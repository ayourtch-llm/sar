use async_trait::async_trait;
use rmcp::{
    model::{CallToolRequestParams, RawContent, Tool},
    service::RunningService,
    transport::TokioChildProcess,
    ClientHandler, RoleClient,
};
use sar_core::bus::SarBus;
use sar_tool_actors::{ToolActor, ToolActorRunner, ToolSyntax};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{error, info};

/// Configuration for a single MCP server.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct McpServerConfig {
    /// Command to launch the MCP server (e.g., ["npx", "-y", "@..."])
    pub command: Vec<String>,
    /// Whether to auto-add tools to the LLM test loop (default = true means auto-add)
    #[serde(default)]
    pub default: bool,
    /// Tool names to expose without the server prefix (empty = all prefixed)
    #[serde(default)]
    pub expose: Vec<String>,
}

/// A single MCP tool wrapped as a ToolActor.
pub struct McpToolActor {
    tool_name: String,
    tool_description: String,
    tool_parameters: serde_json::Value,
    peer: Arc<Mutex<rmcp::Peer<RoleClient>>>,
}

impl McpToolActor {
    pub fn new(
        tool: &Tool,
        peer: Arc<Mutex<rmcp::Peer<RoleClient>>>,
    ) -> Self {
        Self {
            tool_name: tool.name.as_ref().to_string(),
            tool_description: tool.description.as_ref().map(|s| s.as_ref().to_string()).unwrap_or_default(),
            tool_parameters: tool_input_schema_to_json(&tool.input_schema),
            peer,
        }
    }
}

fn tool_input_schema_to_json(schema: &Arc<serde_json::Map<String, serde_json::Value>>) -> serde_json::Value {
    serde_json::Value::Object(schema.as_ref().clone())
}

#[async_trait]
impl ToolActor for McpToolActor {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            self.tool_name.clone(),
            self.tool_description.clone(),
            self.tool_parameters.clone(),
        )
    }

    async fn execute_tool(&self, arguments: &serde_json::Value) -> std::result::Result<String, String> {
        let peer = self.peer.lock().await;
        let args_map: serde_json::Map<String, serde_json::Value> = if let serde_json::Value::Object(m) = arguments {
            m.clone()
        } else {
            serde_json::Map::new()
        };
        let call_params = CallToolRequestParams {
            name: self.tool_name.clone().into(),
            arguments: Some(args_map),
            meta: None,
            task: None,
        };

        match peer.call_tool(call_params).await {
            Ok(result) => {
                let content = result
                    .content
                    .iter()
                    .filter_map(|c| {
                        match &c.raw {
                            RawContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                let is_error = result.is_error.unwrap_or(false);
                if is_error {
                    Err(format!("MCP error: {}", content))
                } else {
                    Ok(content)
                }
            }
            Err(e) => Err(format!("MCP call failed: {}", e)),
        }
    }
}

/// Handler for MCP client messages (empty - we don't need to handle incoming messages).
#[derive(Clone)]
pub struct McpClientHandler;

impl ClientHandler for McpClientHandler {}

/// Manages an MCP server: spawns the process, creates the client, discovers tools.
pub struct McpServerRunner {
    prefix: String,
    config: McpServerConfig,
}

impl McpServerRunner {
    pub fn new(prefix: String, config: McpServerConfig) -> Self {
        Self { prefix, config }
    }

    /// Spawn the MCP server process and return a handle that can be used to
    /// create tool actors and runners.
    pub async fn spawn(
        &self,
        _bus: &SarBus,
    ) -> std::result::Result<McpServerHandle, Box<dyn std::error::Error + Send + Sync>> {
        let cmd = {
            let mut c = tokio::process::Command::new(&self.config.command[0]);
            c.args(&self.config.command[1..]);
            c
        };
        let transport = TokioChildProcess::builder(cmd).spawn()?.0;
        let service = McpClientHandler;
        let running_service: Arc<RunningService<RoleClient, McpClientHandler>> = Arc::new(rmcp::serve_client(service, transport).await?);
        let peer = Arc::new(Mutex::new(running_service.peer().clone()));

        info!(
            "MCP server '{}' started, discovering tools...",
            self.prefix
        );

        // Discover tools
        let tools: Vec<Tool> = peer.lock().await.list_all_tools().await?;

        info!(
            "MCP server '{}' exposed {} tools",
            self.prefix,
            tools.len()
        );

        Ok(McpServerHandle {
            prefix: self.prefix.clone(),
            peer,
            _running_service: running_service,
            tools,
            default: self.config.default,
            expose: self.config.expose.clone(),
        })
    }
}

/// Handle to a running MCP server with discovered tools.
pub struct McpServerHandle {
    prefix: String,
    peer: Arc<Mutex<rmcp::Peer<RoleClient>>>,
    _running_service: Arc<RunningService<RoleClient, McpClientHandler>>,
    tools: Vec<Tool>,
    default: bool,
    expose: Vec<String>,
}

impl McpServerHandle {
    /// Create tool actors and runners for all tools from this MCP server.
    pub fn create_tool_runners(&self, bus: &SarBus) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();

        let peer = self.peer.clone();
        let prefix = self.prefix.clone();

        for tool in &self.tools {
            let tool_actor = McpToolActor::new(tool, peer.clone());
            let runner = ToolActorRunner::new(tool_actor);
            let bus = bus.clone();
            let tool_name_prefix = format!("{}:{}", prefix, tool.name);

            let handle = tokio::spawn(async move {
                if let Err(e) = runner.run(&bus).await {
                    error!("MCP tool '{}' runner failed: {}", tool_name_prefix, e);
                }
            });
            handles.push(handle);
        }

        handles
    }

    /// Get tool actors for tools that should be exposed to the LLM.
    /// These can be added to LlmTestLoopToolsActor via with_tool().
    pub fn tool_actors(&self) -> Vec<std::sync::Arc<dyn ToolActor>> {
        self.tools
            .iter()
            .filter(|t| {
                let should_include = |name: &str| -> bool {
                    if self.default {
                        return true;
                    }
                    if !self.expose.is_empty() {
                        return self.expose.contains(&name.to_string());
                    }
                    false
                };
                should_include(t.name.as_ref())
            })
            .map(|t| {
                std::sync::Arc::new(McpToolActor::new(t, self.peer.clone()))
                    as std::sync::Arc<dyn ToolActor>
            })
            .collect()
    }

    /// Get tool syntaxes for tools that should be exposed to the LLM.
    /// - If `default = true`, all tools are included.
    /// - If `expose` is non-empty, only exposed tools are included.
    /// - Otherwise, no tools are included.
    pub fn get_tool_syntaxes(&self) -> Vec<ToolSyntax> {
        if self.tools.is_empty() {
            return vec![];
        }

        let should_include = |name: &str| -> bool {
            if self.default {
                return true;
            }
            if !self.expose.is_empty() {
                return self.expose.contains(&name.to_string());
            }
            false
        };

        self.tools
            .iter()
            .filter(|t| should_include(t.name.as_ref()))
            .map(|t| {
                ToolSyntax::new(
                    t.name.as_ref().to_string(),
                    t.description.as_ref().map(|s| s.as_ref().to_string()).unwrap_or_default(),
                    tool_input_schema_to_json(&t.input_schema),
                )
            })
            .collect()
    }

    /// Get the prefix for this MCP server.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Get the list of tool names from this MCP server.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name.as_ref().to_string()).collect()
    }
}

impl From<sar_core::config::McpServerConfig> for McpServerConfig {
    fn from(config: sar_core::config::McpServerConfig) -> Self {
        Self {
            command: config.command,
            default: config.default,
            expose: config.expose,
        }
    }
}