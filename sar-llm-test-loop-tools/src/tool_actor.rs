use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

/// Structured representation of a tool's OpenAI-compatible function definition.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolSyntax {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSyntax {
    pub fn new(
        name: String,
        description: String,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name,
            description,
            parameters,
        }
    }

    /// Serializes to OpenAI-compatible function definition JSON.
    pub fn to_openai_json(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

/// Message sent to a tool actor to execute a tool call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolExecuteMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Message published by a tool actor after execution.
/// Results go to the single `tool:results` topic.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub success: bool,
    pub result: String,
    pub error: Option<String>,
}

/// Message sent to cancel a running tool call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCancelMessage {
    pub tool_call_id: String,
}

/// Message published by the UI hub when the user sends `/continue <reason>`.
/// Tools that support interrupt-by-continue subscribe to `user:control` and
/// cancel their current execution if the `tool_name` matches.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContinueMessage {
    pub tool_name: String,
    pub reason: String,
}

/// Trait for actors that expose tool capabilities.
///
/// This trait is object-safe (dyn compatible), allowing tools to be stored
/// as `Arc<dyn ToolActor>` in the `LlmTestLoopToolsActor`.
///
/// - `tool_syntax()` returns the tool's OpenAI-compatible definition (called synchronously).
/// - `execute_tool()` runs the tool logic (called asynchronously by the tool actor's own task).
/// - `supports_cancel()` indicates whether the tool can be cancelled mid-execution.
#[async_trait]
pub trait ToolActor: Send + Sync {
    fn tool_syntax(&self) -> ToolSyntax;
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String>;
    fn supports_cancel(&self) -> bool { false }
}

/// Runner for a ToolActor — subscribes to bus topics and dispatches tool calls.
///
/// Each tool runs as an independent actor task, subscribed to its own `execute`
/// topic and the shared `user:control` topic for continue/cancel signals.
/// Results are published to a single `tool:results` topic.
///
/// For tools that support cancellation, the execution is spawned as a separate tokio task
/// and can be aborted when a matching ContinueMessage is received on `user:control`.
pub struct ToolActorRunner {
    actor: std::sync::Arc<dyn ToolActor>,
    tool_name: String,
}

impl ToolActorRunner {
    pub fn new(actor: impl ToolActor + 'static) -> Self {
        let name = actor.tool_syntax().name.clone();
        Self {
            actor: std::sync::Arc::new(actor),
            tool_name: name,
        }
    }

    async fn publish_result(
        &self,
        bus: &sar_core::bus::SarBus,
        results_topic: &str,
        tool_call_id: &str,
        success: bool,
        result: String,
        error: Option<String>,
    ) {
        let result_msg = ToolResultMessage {
            tool_call_id: tool_call_id.to_string(),
            tool_name: self.tool_name.clone(),
            success,
            result,
            error,
        };

        let msg = sar_core::message::Message::new(
            results_topic,
            "tool-actor",
            serde_json::to_value(&result_msg).unwrap(),
        );

        if let Err(e) = bus.publish("tool-actor", msg).await {
            error!("Failed to publish tool result for '{}': {}", self.tool_name, e);
        }
    }

    pub async fn run(
        &self,
        bus: &sar_core::bus::SarBus,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let execute_topic = format!("tool:{}:execute", self.tool_name);
        let control_topic = "user:control".to_string();
        let results_topic = "tool:results".to_string();

        let mut execute_rx = bus.subscribe("tool-actor", &execute_topic).await?;

        let mut control_rx = if self.actor.supports_cancel() {
            Some(bus.subscribe("tool-actor", &control_topic).await?)
        } else {
            None
        };

        info!(
            "Tool actor '{}' listening on '{}'",
            self.tool_name, execute_topic
        );

        loop {
            match execute_rx.recv().await {
                Ok(msg) => {
                    let exec_msg: ToolExecuteMessage = match serde_json::from_value(msg.payload) {
                        Ok(m) => m,
                        Err(e) => {
                            error!("Failed to parse tool execute message for '{}': {}", self.tool_name, e);
                            continue;
                        }
                    };

                    info!(
                        "Tool '{}' executing call_id='{}' with args: {}",
                        self.tool_name, exec_msg.tool_call_id, exec_msg.arguments
                    );

                    let actor = self.actor.clone();
                    let args = exec_msg.arguments.clone();
                    let call_id = exec_msg.tool_call_id.clone();

                    let exec_handle = tokio::spawn(async move {
                        actor.execute_tool(&args).await
                    });

                    if let Some(ref mut control_rx) = control_rx {
                        let abort_handle = exec_handle.abort_handle();
                        tokio::select! {
                            result = exec_handle => {
                                match result {
                                    Ok(tool_result) => {
                                        let (success, result, error) = match tool_result {
                                            Ok(r) => (true, r, None),
                                            Err(e) => (false, String::new(), Some(e)),
                                        };
                                        self.publish_result(bus, &results_topic, &call_id, success, result, error).await;
                                    }
                                    Err(e) => {
                                        error!("Tool execution task panicked: {}", e);
                                        self.publish_result(bus, &results_topic, &call_id, false, String::new(), Some("Task panicked".to_string())).await;
                                    }
                                }
                            }
                            control_msg = control_rx.recv() => {
                                match control_msg {
                                    Ok(msg) => {
                                        match serde_json::from_value::<ContinueMessage>(msg.payload) {
                                            Ok(continue_msg) if continue_msg.tool_name == self.tool_name => {
                                                abort_handle.abort();
                                                info!("Tool '{}' cancelled: {}", self.tool_name, continue_msg.reason);
                                                self.publish_result(bus, &results_topic, &call_id, false, String::new(), Some(format!("Cancelled: {}", continue_msg.reason))).await;
                                            }
                                            Ok(_) => {}
                                            Err(_) => {}
                                        }
                                    }
                                    Err(RecvError::Lagged(_)) => {}
                                    Err(RecvError::Closed) => break,
                                }
                            }
                        }
                    } else {
                        match exec_handle.await {
                            Ok(tool_result) => {
                                let (success, result, error) = match tool_result {
                                    Ok(r) => (true, r, None),
                                    Err(e) => (false, String::new(), Some(e)),
                                };
                                self.publish_result(bus, &results_topic, &call_id, success, result, error).await;
                            }
                            Err(e) => {
                                error!("Tool execution task panicked: {}", e);
                                self.publish_result(bus, &results_topic, &call_id, false, String::new(), Some("Task panicked".to_string())).await;
                            }
                        }
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    warn!(
                        "Tool '{}' lagged behind, dropped {} messages",
                        self.tool_name, n
                    );
                }
                Err(RecvError::Closed) => {
                    info!("Tool '{}' execute topic closed", self.tool_name);
                    break;
                }
            }
        }

        Ok(())
    }
}

use tokio::sync::broadcast::error::RecvError;