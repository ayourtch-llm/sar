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
    pub llm_tool_calls_topic: String,
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
        llm_tool_calls_topic: String,
        stream_output_topic: String,
    ) -> Self {
        Self {
            index,
            input_topic,
            llm_in_topic,
            llm_out_topic,
            llm_stream_topic,
            llm_tool_calls_topic,
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

    async fn find_tool(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    async fn send_conversation(
        &self,
        bus: &SarBus,
        messages: &[serde_json::Value],
        tool_defs: &[serde_json::Value],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let llm_request = LlmRequest {
            messages: if messages.is_empty() {
                None
            } else {
                Some(messages.to_vec())
            },
            prompt: String::new(),
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
                Some(tool_defs.to_vec())
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
        Ok(())
    }
}

#[async_trait::async_trait]
impl Actor for LlmTestLoopToolsActor {
    fn id(&self) -> sar_core::ActorId {
        format!("sar-llm-test-loop-tools-{}", self.index)
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conversation_messages: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));

        let mut input_rx = bus.subscribe(&self.id(), &self.input_topic).await.map_err(|e| {
            format!("Failed to subscribe to input topic '{}': {}", self.input_topic, e)
        })?;

        let mut out_rx = bus.subscribe(&self.id(), &self.llm_out_topic).await.map_err(|e| {
            format!("Failed to subscribe to output topic '{}': {}", self.llm_out_topic, e)
        })?;

        let mut stream_rx = bus.subscribe(&self.id(), &self.llm_stream_topic).await.map_err(|e| {
            format!("Failed to subscribe to stream topic '{}': {}", self.llm_stream_topic, e)
        })?;

        let mut tool_calls_rx = bus.subscribe(&self.id(), &self.llm_tool_calls_topic).await.map_err(|e| {
            format!("Failed to subscribe to tool calls topic '{}': {}", self.llm_tool_calls_topic, e)
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
                                let mut messages = conversation_messages.lock().await;
                                messages.push(serde_json::json!({
                                    "role": "user",
                                    "content": user_message
                                }));
                            }
                            let msg_count = {
                                let messages = conversation_messages.lock().await;
                                messages.len()
                            };
                            info!(
                                "LLM test loop tools actor {} received user message, message count: {}",
                                self.index, msg_count
                            );

                            if let Err(e) = self.send_conversation(bus, &conversation_messages.lock().await, &tool_defs).await {
                                error!("Failed to send conversation: {}", e);
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
                                let mut messages = conversation_messages.lock().await;
                                messages.push(serde_json::json!({
                                    "role": "assistant",
                                    "content": output
                                }));
                            }
                            let msg_count = {
                                let messages = conversation_messages.lock().await;
                                messages.len()
                            };
                            info!(
                                "LLM test loop tools actor {} received full response, message count: {}",
                                self.index, msg_count
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
                result = tool_calls_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            info!("LLM test loop tools actor {} received tool calls from '{}'", self.index, msg.source);
                            
                            let tool_calls: Vec<serde_json::Value> = match serde_json::from_value(msg.payload.clone()) {
                                Ok(tc) => tc,
                                Err(e) => {
                                    error!("Failed to parse tool calls: {}", e);
                                    continue;
                                }
                            };

                            let mut tool_results = Vec::new();
                            for tc in &tool_calls {
                                let func_name = tc["function"]["name"].as_str().unwrap_or("");
                                let func_args_str = tc["function"]["arguments"].as_str().unwrap_or("");
                                let tool_call_id = tc["id"].as_str().unwrap_or("");
                                
                                info!("LLM test loop tools actor {} executing tool call: {}({})", self.index, func_name, func_args_str);

                                if let Some(tool) = self.find_tool(func_name).await {
                                    let args: serde_json::Value = match serde_json::from_str(func_args_str) {
                                        Ok(a) => a,
                                        Err(e) => {
                                            error!("Failed to parse tool arguments for {}: {}", func_name, e);
                                            tool_results.push(serde_json::json!({
                                                "tool_call_id": tool_call_id,
                                                "name": func_name,
                                                "content": format!("Error parsing arguments: {}", e),
                                            }));
                                            continue;
                                        }
                                    };

                                    match tool.execute(&args).await {
                                        Ok(result) => {
                                            info!("LLM test loop tools actor {} tool {} result: {}", self.index, func_name, result);
                                            tool_results.push(serde_json::json!({
                                                "tool_call_id": tool_call_id,
                                                "name": func_name,
                                                "content": result,
                                            }));
                                        }
                                        Err(e) => {
                                            error!("LLM test loop tools actor {} tool {} error: {}", self.index, func_name, e);
                                            tool_results.push(serde_json::json!({
                                                "tool_call_id": tool_call_id,
                                                "name": func_name,
                                                "content": format!("Error: {}", e),
                                            }));
                                        }
                                    }
                                } else {
                                    warn!("LLM test loop tools actor {} tool '{}' not found", self.index, func_name);
                                    tool_results.push(serde_json::json!({
                                        "tool_call_id": tool_call_id,
                                        "name": func_name,
                                        "content": format!("Error: tool '{}' not found", func_name),
                                    }));
                                }
                            }

                            {
                                let mut messages = conversation_messages.lock().await;
                                for tr in &tool_results {
                                    messages.push(serde_json::json!({
                                        "role": "tool",
                                        "tool_call_id": tr["tool_call_id"],
                                        "name": tr["name"],
                                        "content": tr["content"],
                                    }));
                                }
                            }

                            info!("LLM test loop tools actor {} sending tool results back to LLM, message count: {}", self.index, conversation_messages.lock().await.len());
                            
                            if let Err(e) = self.send_conversation(bus, &conversation_messages.lock().await, &tool_defs).await {
                                error!("Failed to send tool results to LLM: {}", e);
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test loop tools actor {} lagged behind on tool calls, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop tools actor {} tool calls topic closed", self.index);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}