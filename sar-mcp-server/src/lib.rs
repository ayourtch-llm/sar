use async_trait::async_trait;
use rmcp::{
    model::{CallToolRequestParams, RawContent, Tool},
    service::RunningService,
    transport::TokioChildProcess,
    ClientHandler, RoleClient,
};
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::config::McpServerConfig;
use sar_tool_actors::{ToolActor, ToolActorRunner};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{error, info};

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
    fn tool_syntax(&self) -> sar_tool_actors::ToolSyntax {
        sar_tool_actors::ToolSyntax::new(
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

/// Actor for a single MCP server. Owns the service loop handle to keep the transport alive.
pub struct McpServerActor {
    prefix: String,
    peer: Arc<Mutex<rmcp::Peer<RoleClient>>>,
    tools: Vec<Tool>,
    _service_handle: JoinHandle<()>,
}

/// Lightweight handle returned after spawning an MCP server actor.
/// Does NOT own the service loop — the actor does.
pub struct McpServerHandle {
    prefix: String,
    peer: Arc<Mutex<rmcp::Peer<RoleClient>>>,
    tools: Vec<Tool>,
    config: McpServerConfig,
}

impl McpServerHandle {
    /// Get tool actors for tools that should be exposed to the LLM.
    pub fn tool_actors(&self) -> Vec<std::sync::Arc<dyn ToolActor>> {
        self.tools
            .iter()
            .filter(|t| {
                let should_include = |name: &str| -> bool {
                    if self.config.default {
                        return true;
                    }
                    if !self.config.expose.is_empty() {
                        return self.config.expose.contains(&name.to_string());
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
    pub fn get_tool_syntaxes(&self) -> Vec<sar_tool_actors::ToolSyntax> {
        if self.tools.is_empty() {
            return vec![];
        }

        let should_include = |name: &str| -> bool {
            if self.config.default {
                return true;
            }
            if !self.config.expose.is_empty() {
                return self.config.expose.contains(&name.to_string());
            }
            false
        };

        self.tools
            .iter()
            .filter(|t| should_include(t.name.as_ref()))
            .map(|t| {
                sar_tool_actors::ToolSyntax::new(
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

#[async_trait::async_trait]
impl Actor for McpServerActor {
    fn id(&self) -> sar_core::ActorId {
        format!("mcp-{}", self.prefix)
    }

    fn announce(&self) -> sar_core::actor::ActorAnnouncement {
        let mut pubs = Vec::new();
        for tool in &self.tools {
            pubs.push(format!("tool:{}:execute", tool.name));
        }
        sar_core::actor::ActorAnnouncement {
            id: self.id(),
            subscriptions: Vec::new(),
            publications: pubs,
        }
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Spawn tool runners for this MCP server
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
            let _ = handle.await;
        }

        Ok(())
    }
}

/// Runner to spawn an MCP server actor.
pub struct McpServerRunner {
    prefix: String,
    config: McpServerConfig,
}

impl McpServerRunner {
    pub fn new(prefix: String, config: McpServerConfig) -> Self {
        Self { prefix, config }
    }

    /// Spawn the MCP server process, initialize the client, discover tools,
    /// and return an McpServerHandle. The McpServerActor (with service loop)
    /// is spawned internally and stays alive for the lifetime of the program.
    pub async fn spawn(
        &self,
        bus: &SarBus,
    ) -> std::result::Result<McpServerHandle, Box<dyn std::error::Error + Send + Sync>> {
        let cmd = {
            let mut c = tokio::process::Command::new(&self.config.command[0]);
            c.args(&self.config.command[1..]);
            c
        };
        let transport = TokioChildProcess::builder(cmd).spawn()?.0;
        let service = McpClientHandler;
        let running_service: RunningService<RoleClient, McpClientHandler> = rmcp::serve_client(service, transport).await?;
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

        // Spawn the service loop in a background task so it stays alive.
        // RunningService::waiting() consumes the service and polls the transport loop.
        // As long as this JoinHandle is alive, the transport stays open.
        let service_handle = {
            let rs = running_service;
            tokio::spawn(async move {
                let _ = rs.waiting().await;
            })
        };

        let actor = McpServerActor {
            prefix: self.prefix.clone(),
            peer: peer.clone(),
            tools: tools.clone(),
            _service_handle: service_handle,
        };

        // Spawn the actor so it stays alive for the lifetime of the program
        bus.spawn_actor(actor).await?;

        // Return a lightweight handle (the actor owns the service loop)
        Ok(McpServerHandle {
            prefix: self.prefix.clone(),
            peer,
            tools,
            config: self.config.clone(),
        })
    }
}