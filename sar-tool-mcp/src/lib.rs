use async_trait::async_trait;
use rust_mcp_sdk::mcp_client::{client_runtime, ClientHandler, ClientRuntime, McpClientOptions, ToMcpClientHandler};
use rust_mcp_sdk::schema::*;
use rust_mcp_sdk::{McpClient, StdioTransport, TransportOptions};
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
    client: Arc<Mutex<Arc<ClientRuntime>>>,
}

impl McpToolActor {
    pub fn new(
        tool: &Tool,
        client: Arc<Mutex<Arc<ClientRuntime>>>,
    ) -> Self {
        Self {
            tool_name: tool.name.clone(),
            tool_description: tool.description.clone().unwrap_or_default(),
            tool_parameters: serde_json::to_value(&tool.input_schema).unwrap_or(serde_json::json!({})),
            client,
        }
    }
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
        let client = self.client.lock().await;
        let args_map: serde_json::Map<String, serde_json::Value> = if let serde_json::Value::Object(m) = arguments {
            m.clone()
        } else {
            serde_json::Map::new()
        };
        let call_params = CallToolRequestParams {
            name: self.tool_name.clone(),
            arguments: Some(args_map),
            meta: None,
            task: None,
        };

        match client.request_tool_call(call_params).await {
            Ok(result) => {
                let content = result
                    .content
                    .iter()
                    .filter_map(|c| {
                        if let ContentBlock::TextContent(text) = c {
                            Some(text.text.clone())
                        } else {
                            None
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
        let transport = StdioTransport::create_with_server_launch(
            self.config.command[0].clone(),
            self.config.command[1..].to_vec(),
            None,
            TransportOptions::default(),
        )
        .map_err(|e| format!("Failed to create MCP transport: {}", e))?;

        let client_details = InitializeRequestParams {
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "sar-mcp-client".into(),
                version: "0.1.0".into(),
                description: None,
                icons: vec![],
                title: None,
                website_url: None,
            },
            protocol_version: ProtocolVersion::V2025_11_25.into(),
            meta: None,
        };

        let options = McpClientOptions {
            client_details,
            transport,
            handler: McpClientHandler.to_mcp_client_handler(),
            task_store: None,
            server_task_store: None,
            message_observer: None,
        };

        let client = client_runtime::create_client(options);
        client.clone().start().await.map_err(|e| format!("Failed to start MCP client: {}", e))?;

        info!(
            "MCP server '{}' started, discovering tools...",
            self.prefix
        );

        // Discover tools
        let tools_result = client.request_tool_list(None).await.map_err(|e| format!("Failed to list tools: {}", e))?;
        let tools = tools_result.tools;

        info!(
            "MCP server '{}' exposed {} tools",
            self.prefix,
            tools.len()
        );

        let client = Arc::new(Mutex::new(client));

        Ok(McpServerHandle {
            prefix: self.prefix.clone(),
            client,
            tools,
            default: self.config.default,
            expose: self.config.expose.clone(),
        })
    }
}

/// Handle to a running MCP server with discovered tools.
pub struct McpServerHandle {
    prefix: String,
    client: Arc<Mutex<Arc<ClientRuntime>>>,
    tools: Vec<Tool>,
    default: bool,
    expose: Vec<String>,
}

impl McpServerHandle {
    /// Create tool actors and runners for all tools from this MCP server.
    pub fn create_tool_runners(&self, bus: &SarBus) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();

        let client = self.client.clone();
        let prefix = self.prefix.clone();

        for tool in &self.tools {
            let tool_actor = McpToolActor::new(tool, client.clone());
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
            .filter(|t| should_include(&t.name))
            .map(|t| {
                ToolSyntax::new(
                    t.name.clone(),
                    t.description.clone().unwrap_or_default(),
                    serde_json::to_value(&t.input_schema).unwrap_or(serde_json::json!({})),
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
        self.tools.iter().map(|t| t.name.clone()).collect()
    }
}