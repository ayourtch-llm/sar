use async_trait::async_trait;
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::config::LlmConfig;
use sar_core::message::Message;
use sar_llm::LlmRequest;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};

pub mod calculator;
pub use calculator::CalculatorTool;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> &serde_json::Value;
    async fn execute(&self, arguments: &serde_json::Value) -> Result<String, String>;
}

#[derive(Default)]
pub struct LlmTestLoopToolsActor {
    pub index: usize,
    pub input_topic: String,
    pub llm_in_topic: String,
    pub llm_out_topic: String,
    pub llm_stream_topic: String,
    pub stream_output_topic: String,
    pub llm_base_url: String,
    pub tools: Vec<Box<dyn Tool>>,
}

impl LlmTestLoopToolsActor {
    pub fn new(
        index: usize,
        input_topic: String,
        llm_in_topic: String,
        llm_out_topic: String,
        llm_stream_topic: String,
        stream_output_topic: String,
    ) -> Self {
        Self {
            index,
            input_topic,
            llm_in_topic,
            llm_out_topic,
            llm_stream_topic,
            stream_output_topic,
            llm_base_url: String::new(),
            tools: Vec::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.llm_base_url = base_url;
        self
    }

    pub fn with_tool(mut self, tool: impl Tool + 'static) -> Self {
        self.tools.push(Box::new(tool));
        self
    }
}

#[async_trait::async_trait]
impl Actor for LlmTestLoopToolsActor {
    fn id(&self) -> sar_core::ActorId {
        format!("sar-llm-test-loop-tools-{}", self.index)
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conversation_history: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let mut input_rx = bus.subscribe(&self.id(), &self.input_topic).await.map_err(|e| {
            format!("Failed to subscribe to input topic '{}': {}", self.input_topic, e)
        })?;

        let mut out_rx = bus.subscribe(&self.id(), &self.llm_out_topic).await.map_err(|e| {
            format!("Failed to subscribe to output topic '{}': {}", self.llm_out_topic, e)
        })?;

        let mut stream_rx = bus.subscribe(&self.id(), &self.llm_stream_topic).await.map_err(|e| {
            format!("Failed to subscribe to stream topic '{}': {}", self.llm_stream_topic, e)
        })?;

        let tool_defs: Vec<serde_json::Value> = self
            .tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name(),
                        "description": tool.description(),
                        "parameters": tool.parameters(),
                    }
                })
            })
            .collect();

        info!(
            "LLM test loop tools actor {} listening on '{}' with {} tools",
            self.index,
            self.input_topic,
            self.tools.len()
        );

        loop {
            tokio::select! {
                result = input_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            let user_message = match msg.payload.as_str() {
                                Some(s) => s.to_string(),
                                None => msg.payload.to_string(),
                            };
                            {
                                let mut history = conversation_history.lock().await;
                                history.push(format!("User: {}", user_message));
                            }
                            let history_len = {
                                let history = conversation_history.lock().await;
                                history.len()
                            };
                            info!(
                                "LLM test loop tools actor {} received user message, conversation size: {}",
                                self.index, history_len
                            );

                            let conversation_text = {
                                let history = conversation_history.lock().await;
                                history.join("\n")
                            };

                            let llm_request = LlmRequest {
                                prompt: conversation_text,
                                config: if self.llm_base_url.is_empty() {
                                    None
                                } else {
                                    Some(LlmConfig {
                                        model: String::new(),
                                        base_url: self.llm_base_url.clone(),
                                        api_key: String::new(),
                                        temperature: 0.7,
                                        max_tokens: 65536,
                                    })
                                },
                                tools: if tool_defs.is_empty() {
                                    None
                                } else {
                                    Some(tool_defs.clone())
                                },
                            };

                            let msg = Message::new(
                                &self.llm_in_topic,
                                &self.id(),
                                serde_json::to_value(&llm_request)
                                    .map_err(|e| format!("Failed to serialize LLM request: {}", e))?,
                            );

                            if let Err(e) = bus.publish(&self.id(), msg).await {
                                error!("Failed to publish to LLM input: {}", e);
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test loop tools actor {} lagged behind, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop tools actor {} input topic closed", self.index);
                            break;
                        }
                    }
                }
                result = out_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            let output = match &msg.payload {
                                serde_json::Value::String(s) => s.replace("\\n", "\n"),
                                _ => msg.payload.to_string(),
                            };
                            {
                                let mut history = conversation_history.lock().await;
                                history.push(format!("Assistant: {}", output));
                            }
                            let history_len = {
                                let history = conversation_history.lock().await;
                                history.len()
                            };
                            info!(
                                "LLM test loop tools actor {} received full response, conversation size: {}",
                                self.index, history_len
                            );
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test loop tools actor {} lagged behind on output, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop tools actor {} output topic closed", self.index);
                        }
                    }
                }
                result = stream_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            let mut forwarded = msg.clone();
                            forwarded.topic = self.stream_output_topic.clone();
                            if let Err(e) = bus.publish(&self.id(), forwarded).await {
                                error!("Failed to forward stream chunk: {}", e);
                            }
                            if let serde_json::Value::String(ref s) = msg.payload {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(s) {
                                    if let Some(type_val) = json.get("type").and_then(|t| t.as_str()) {
                                        if type_val == "stream_end" {
                                        }
                                    }
                                }
                            }
                            if let serde_json::Value::Object(ref map) = msg.payload {
                                if let Some(type_val) = map.get("type").and_then(|t| t.as_str()) {
                                    if type_val == "stream_end" {
                                    }
                                }
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test loop tools actor {} lagged behind on stream, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop tools actor {} stream topic closed", self.index);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}