use async_trait::async_trait;
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::config::LlmConfig;
use sar_core::message::Message;
use sar_llm::LlmRequest;
use sar_tool_actors::ToolResultMessage;
use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};

pub use sar_tool_actors::{ToolActor, ToolActorRunner, ToolExecuteMessage, ToolSyntax};
pub use sar_tool_calculator::CalculatorTool;
pub use sar_tool_sleep::SleepTool;

pub mod tool_actor_wrapper;
pub use tool_actor_wrapper::ToolActorWrapper;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> &serde_json::Value;
    async fn execute(&self, arguments: &serde_json::Value) -> Result<String, String>;
}

pub const TOOLS_RESULTS_TOPIC: &str = "tool:results";

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
    pub tools: StdMutex<Vec<Arc<dyn ToolActor>>>,
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
            tools: StdMutex::new(Vec::new()),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.llm_base_url = base_url;
        self
    }

    pub fn with_tool(self, tool: impl ToolActor + 'static) -> Self {
        {
            let mut tools = self.tools.lock().unwrap();
            tools.push(Arc::new(tool));
        }
        self
    }

    pub async fn add_tool_arc(&self, tool: Arc<dyn ToolActor>) {
        self.tools.lock().unwrap().push(tool);
    }

    pub async fn remove_tool(&self, name: &str) {
        self.tools.lock().unwrap().retain(|t| t.tool_syntax().name != name);
    }

    fn format_dump(&self, messages: &[serde_json::Value], tool_defs: &[serde_json::Value]) -> String {
        let mut dump = String::new();
        dump.push_str("=== LLM Request Dump ===\n\n");

        dump.push_str("Model: (default)\n");
        dump.push_str(&format!("Temperature: 0.7\n"));
        dump.push_str(&format!("Max tokens: 65536\n\n"));

        if !tool_defs.is_empty() {
            dump.push_str("Tools:\n");
            for tool in tool_defs {
                let name = tool["function"]["name"].as_str().unwrap_or("");
                let desc = tool["function"]["description"].as_str().unwrap_or("");
                dump.push_str(&format!("  - {}:\n", name));
                dump.push_str(&format!("    {}\n", desc));
            }
            dump.push('\n');
        }

        dump.push_str("Messages:\n");
        for (i, msg) in messages.iter().enumerate() {
            let role = msg["role"].as_str().unwrap_or("unknown");
            dump.push_str(&format!("\n--- Message {} ({}): ---\n", i + 1, role));

            match role {
                "user" => {
                    let content = msg["content"].as_str().unwrap_or("");
                    dump.push_str(&format!("{}\n", content));
                }
                "assistant" => {
                    let content = msg["content"].as_str().unwrap_or("");
                    dump.push_str(&format!("{}\n", content));
                    if let Some(tool_calls) = msg["tool_calls"].as_array() {
                        for tc in tool_calls {
                            let func_name = tc["function"]["name"].as_str().unwrap_or("");
                            let args = tc["function"]["arguments"].as_str().unwrap_or("");
                            dump.push_str(&format!("  [tool_call] {}({})\n", func_name, args));
                        }
                    }
                }
                "tool" => {
                    let tool_call_id = msg["tool_call_id"].as_str().unwrap_or("");
                    let name = msg["name"].as_str().unwrap_or("");
                    let content = msg["content"].as_str().unwrap_or("");
                    dump.push_str(&format!("  tool_call_id: {}\n", tool_call_id));
                    dump.push_str(&format!("  name: {}\n", name));
                    dump.push_str(&format!("  result: {}\n", content));
                }
                _ => {
                    dump.push_str(&format!("{:?}\n", msg));
                }
            }
        }

        dump.push_str("\n=== End Dump ===\n");
        dump
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

        let mut tool_results_rx = bus.subscribe(&self.id(), TOOLS_RESULTS_TOPIC).await.map_err(|e| {
            format!("Failed to subscribe to tool results topic '{}': {}", TOOLS_RESULTS_TOPIC, e)
        })?;

        let tool_defs: Vec<serde_json::Value> = {
            let tools = self.tools.lock().unwrap();
            tools.iter()
                .map(|tool| tool.tool_syntax().to_openai_json())
                .collect()
        };

        info!(
            "LLM test loop tools actor {} listening on '{}' with {} tools",
            self.index,
            self.input_topic,
            self.tools.lock().unwrap().len()
        );

        let mut pending_tool_calls: HashSet<String> = HashSet::new();
        let mut pending_messages: Vec<String> = Vec::new();

        loop {
            tokio::select! {
                result = input_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            let user_message = match msg.payload.as_str() {
                                Some(s) => s.to_string(),
                                None => msg.payload.to_string(),
                            };

                            if user_message.starts_with("/dump") {
                                let dump_text = self.format_dump(&conversation_messages.lock().await, &tool_defs);
                                let dump_msg = Message::new(
                                    &self.stream_output_topic,
                                    &self.id(),
                                    dump_text,
                                ).with_type("LlmDump");
                                if let Err(e) = bus.publish(&self.id(), dump_msg).await {
                                    error!("Failed to publish dump: {}", e);
                                }
                                continue;
                            }

                            if !pending_tool_calls.is_empty() {
                                info!(
                                    "LLM test loop tools actor {} buffering user message while {} tool calls pending",
                                    self.index, pending_tool_calls.len()
                                );
                                pending_messages.push(user_message);
                                continue;
                            }

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

                            let tool_calls: Vec<serde_json::Value> = match msg.payload {
                                serde_json::Value::Array(arr) => arr.into_iter().collect(),
                                serde_json::Value::Object(_) => {
                                    info!("LLM test loop tools actor {} skipping individual tool call chunk", self.index);
                                    continue;
                                }
                                _ => {
                                    error!("Failed to parse tool calls: unexpected type");
                                    continue;
                                }
                            };

                            {
                                let mut messages = conversation_messages.lock().await;
                                messages.push(serde_json::json!({
                                    "role": "assistant",
                                    "tool_calls": tool_calls.clone()
                                }));
                            }

                            // Track pending tool calls by ID
                            for tc in &tool_calls {
                                let tool_call_id = tc["id"].as_str().unwrap_or("");
                                pending_tool_calls.insert(tool_call_id.to_string());

                                let func_name = tc["function"]["name"].as_str().unwrap_or("");
                                let func_args_str = tc["function"]["arguments"].as_str().unwrap_or("");

                                info!("LLM test loop tools actor {} publishing tool call: {}({})", self.index, func_name, func_args_str);

                                let args: serde_json::Value = match serde_json::from_str(func_args_str) {
                                    Ok(a) => a,
                                    Err(e) => {
                                        error!("Failed to parse tool arguments for {}: {}", func_name, e);
                                        // Publish error result directly to tool:results
                                        pending_tool_calls.remove(tool_call_id);
                                        let error_result = ToolResultMessage {
                                            tool_call_id: tool_call_id.to_string(),
                                            tool_name: func_name.to_string(),
                                            success: false,
                                            result: String::new(),
                                            error: Some(format!("Error parsing arguments: {}", e)),
                                        };
                                        let result_msg = Message::new(
                                            TOOLS_RESULTS_TOPIC,
                                            &self.id(),
                                            serde_json::to_value(&error_result).unwrap(),
                                        );
                                        let _ = bus.publish(&self.id(), result_msg).await;
                                        continue;
                                    }
                                };

                                // Publish to tool actor topic (async, non-blocking)
                                let bus_clone = bus.clone();
                                let self_id = self.id();
                                let func_name_clone = func_name.to_string();
                                let tool_call_id_clone = tool_call_id.to_string();
                                let args_clone = args.clone();

                                tokio::spawn(async move {
                                    let exec_msg = ToolExecuteMessage {
                                        tool_call_id: tool_call_id_clone,
                                        tool_name: func_name_clone,
                                        arguments: args_clone,
                                    };
                                    let msg = Message::new(
                                        &format!("tool:{}:execute", exec_msg.tool_name),
                                        &self_id,
                                        serde_json::to_value(&exec_msg).unwrap(),
                                    );
                                    if let Err(e) = bus_clone.publish(&self_id, msg).await {
                                        error!("Failed to publish tool execute: {}", e);
                                    }
                                });
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
                result = tool_results_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            let tool_result: ToolResultMessage = match serde_json::from_value(msg.payload) {
                                Ok(tr) => tr,
                                Err(e) => {
                                    error!("Failed to parse tool result message: {}", e);
                                    continue;
                                }
                            };

                            info!(
                                "LLM test loop tools actor {} received tool result: tool='{}' success={} call_id='{}'",
                                self.index, tool_result.tool_name, tool_result.success, tool_result.tool_call_id
                            );

                            if tool_result.success {
                                {
                                    let mut messages = conversation_messages.lock().await;
                                    messages.push(serde_json::json!({
                                        "role": "tool",
                                        "tool_call_id": tool_result.tool_call_id,
                                        "name": tool_result.tool_name,
                                        "content": tool_result.result,
                                    }));
                                }

                                let stream_msg = Message::new(
                                    &self.stream_output_topic,
                                    &self.id(),
                                    format!("  [tool_result] {}({}) => {}", tool_result.tool_name, tool_result.tool_call_id, tool_result.result),
                                ).with_type("LlmToolResult");
                                if let Err(e) = bus.publish(&self.id(), stream_msg).await {
                                    error!("Failed to publish tool result stream: {}", e);
                                }
                            } else {
                                {
                                    let mut messages = conversation_messages.lock().await;
                                    messages.push(serde_json::json!({
                                        "role": "tool",
                                        "tool_call_id": tool_result.tool_call_id,
                                        "name": tool_result.tool_name,
                                        "content": format!("Error: {}", tool_result.error.as_deref().unwrap_or("unknown")),
                                    }));
                                }

                                let stream_msg = Message::new(
                                    &self.stream_output_topic,
                                    &self.id(),
                                    format!("  [tool_error] {}({}) => {}", tool_result.tool_name, tool_result.tool_call_id, tool_result.error.as_deref().unwrap_or("unknown")),
                                ).with_type("LlmToolResult");
                                if let Err(e) = bus.publish(&self.id(), stream_msg).await {
                                    error!("Failed to publish tool error stream: {}", e);
                                }
                            }

                            pending_tool_calls.remove(&tool_result.tool_call_id);

                            if pending_tool_calls.is_empty() {
                                info!("LLM test loop tools actor {} all tool calls resolved, message count: {}", self.index, conversation_messages.lock().await.len());

                                for buffered_msg in pending_messages.drain(..) {
                                    {
                                        let mut messages = conversation_messages.lock().await;
                                        messages.push(serde_json::json!({
                                            "role": "user",
                                            "content": buffered_msg
                                        }));
                                    }
                                    let buffered_count = {
                                        let messages = conversation_messages.lock().await;
                                        messages.len()
                                    };
                                    info!(
                                        "LLM test loop tools actor {} processing buffered message, message count: {}",
                                        self.index, buffered_count
                                    );

                                    if let Err(e) = self.send_conversation(bus, &conversation_messages.lock().await, &tool_defs).await {
                                        error!("Failed to send buffered message: {}", e);
                                    }
                                }

                                if let Err(e) = self.send_conversation(bus, &conversation_messages.lock().await, &tool_defs).await {
                                    error!("Failed to send tool results to LLM: {}", e);
                                }
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test loop tools actor {} lagged behind on tool results, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            warn!("LLM test loop tools actor {} tool results topic closed", self.index);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}