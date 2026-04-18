use async_trait::async_trait;
use futures::StreamExt;
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::config::LlmConfig;
use sar_core::message::Message;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    #[serde(default)]
    pub messages: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub config: Option<LlmConfig>,
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStats {
    pub rxtokens: usize,
}

#[derive(Debug, Default)]
pub struct LlmActor {
    pub index: usize,
    pub input_topic: String,
    pub output_topic: String,
    pub stream_topic: String,
    pub stats_topic: String,
    pub tool_calls_topic: String,
    pub config: LlmConfig,
}

impl LlmActor {
    pub fn new(index: usize, input_topic: String, output_topic: String, stream_topic: String, stats_topic: String, tool_calls_topic: String, config: LlmConfig) -> Self {
        Self {
            index,
            input_topic,
            output_topic,
            stream_topic,
            stats_topic,
            tool_calls_topic,
            config,
        }
    }

    fn merge_config(&self, request_config: Option<LlmConfig>) -> LlmConfig {
        match request_config {
            Some(req) => LlmConfig {
                model: if req.model.is_empty() { self.config.model.clone() } else { req.model },
                base_url: if req.base_url.is_empty() { self.config.base_url.clone() } else { req.base_url },
                api_key: if req.api_key.is_empty() { self.config.api_key.clone() } else { req.api_key },
                temperature: req.temperature,
                max_tokens: if req.max_tokens == 0 { self.config.max_tokens } else { req.max_tokens },
            },
            None => self.config.clone(),
        }
    }

    async fn send_request(&self, config: &LlmConfig, messages: &[serde_json::Value], tools: Option<&[serde_json::Value]>, bus: &SarBus) -> Result<(String, Vec<serde_json::Value>), Box<dyn std::error::Error + Send + Sync>> {
        let stream_id = uuid::Uuid::new_v4().to_string();
        let client = reqwest::Client::new();
        
        let mut body = serde_json::json!({
            "model": config.model,
            "messages": messages,
            "temperature": config.temperature,
            "max_tokens": config.max_tokens,
            "stream": true
        });
        if let Some(tools) = tools {
            body["tools"] = serde_json::to_value(tools).unwrap();
        }

        let dump = format!("=== LLM Request (index={}) ===\n{}\n=================\n", self.index, serde_json::to_string_pretty(&body).unwrap());
        if let Err(e) = std::fs::write("/tmp/sar.txt", &dump) {
            error!("Failed to write dump to /tmp/sar.txt: {}", e);
        }

        let response = client
            .post(format!("{}/chat/completions", config.base_url))
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, body).into());
        }

        let mut full_response = String::new();
        let mut stream = response.bytes_stream();
        let mut rxtokens: usize = 0;
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut tool_call_accumulators: std::collections::HashMap<usize, (String, String)> = std::collections::HashMap::new();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    error!("Stream error: {}", e);
                    break;
                }
            };
            let text = String::from_utf8_lossy(&chunk);
            
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        continue;
                    }
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(reasoning) = json["choices"][0]["delta"]["reasoning_content"].as_str() {
                            if !reasoning.is_empty() {
                                rxtokens += reasoning.chars().count();
                                let chunk_msg = Message::new(
                                    &self.stream_topic,
                                    &self.id(),
                                    reasoning.to_string(),
                                ).with_type("LlmThinking").with_stream_id(stream_id.clone());
                                if let Err(e) = bus.publish(&self.id(), chunk_msg).await {
                                    error!("Failed to publish thinking chunk: {}", e);
                                }
                            }
                        }
                        if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                            if !content.is_empty() {
                                rxtokens += content.chars().count();
                                let mut remaining = content.to_string();
                                while let Some(think_start) = remaining.find("<thinking>") {
                                    let thinking_content = &remaining[..think_start];
                                    if !thinking_content.is_empty() {
                                        let chunk_msg = Message::new(
                                            &self.stream_topic,
                                            &self.id(),
                                            thinking_content.to_string(),
                                        ).with_type("LlmStream").with_stream_id(stream_id.clone());
                                        if let Err(e) = bus.publish(&self.id(), chunk_msg).await {
                                            error!("Failed to publish stream chunk: {}", e);
                                        }
                                        full_response.push_str(thinking_content);
                                    }
                                    remaining = remaining[think_start + "<thinking>".len()..].to_string();
                                }
                                if let Some(think_end) = remaining.find("</thinking>") {
                                    let thinking_content = &remaining[..think_end];
                                    if !thinking_content.is_empty() {
                                        let chunk_msg = Message::new(
                                            &self.stream_topic,
                                            &self.id(),
                                            thinking_content.to_string(),
                                        ).with_type("LlmThinking").with_stream_id(stream_id.clone());
                                        if let Err(e) = bus.publish(&self.id(), chunk_msg).await {
                                            error!("Failed to publish thinking chunk: {}", e);
                                        }
                                    }
                                    remaining = remaining[think_end + "</thinking>".len()..].to_string();
                                }
                                if !remaining.is_empty() {
                                    let chunk_msg = Message::new(
                                        &self.stream_topic,
                                        &self.id(),
                                        remaining.to_string(),
                                    ).with_type("LlmStream").with_stream_id(stream_id.clone());
                                    if let Err(e) = bus.publish(&self.id(), chunk_msg).await {
                                        error!("Failed to publish stream chunk: {}", e);
                                    }
                                    full_response.push_str(&remaining);
                                }
                            }
                        }
                        if let Some(tool_calls_arr) = json["choices"][0]["delta"]["tool_calls"].as_array() {
                            for tool_call in tool_calls_arr {
                                if let Some(index) = tool_call["index"].as_u64() {
                                    let idx = index as usize;
                                    let name = tool_call["function"]["name"].as_str().unwrap_or("");
                                    let args = tool_call["function"]["arguments"].as_str().unwrap_or("");
                                    if !name.is_empty() {
                                        tool_call_accumulators.entry(idx).or_insert_with(|| (String::new(), String::new())).0.push_str(name);
                                    }
                                    if !args.is_empty() {
                                        tool_call_accumulators.entry(idx).or_insert_with(|| (String::new(), String::new())).1.push_str(args);
                                    }
                                    let (func_name, func_args) = tool_call_accumulators.get(&idx).unwrap();
                                    let tc_msg = Message::new(
                                        &self.tool_calls_topic,
                                        &self.id(),
                                        serde_json::json!({
                                            "index": idx,
                                            "function": {
                                                "name": func_name,
                                                "arguments": func_args,
                                            }
                                        }),
                                    ).with_type("LlmToolCall").with_stream_id(stream_id.clone());
                                    if let Err(e) = bus.publish(&self.id(), tc_msg).await {
                                        error!("Failed to publish tool call chunk: {}", e);
                                    }
                                    let stream_tc_msg = Message::new(
                                        &self.stream_topic,
                                        &self.id(),
                                        format!("  [tool_call] {}({})", func_name, func_args),
                                    ).with_type("LlmToolCall").with_stream_id(stream_id.clone());
                                    if let Err(e) = bus.publish(&self.id(), stream_tc_msg).await {
                                        error!("Failed to publish tool call stream chunk: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let end_msg = Message::new(
            &self.stream_topic,
            &self.id(),
            serde_json::json!({"type": "stream_end"}),
        ).with_type("LlmStreamEnd").with_stream_id(stream_id.clone());
        if let Err(e) = bus.publish(&self.id(), end_msg).await {
            error!("Failed to publish stream end: {}", e);
        }

        info!("LLM actor {} publishing stats: rxtokens={}", self.index, rxtokens);
        let stats_msg = Message::new(
            &self.stats_topic,
            &self.id(),
            serde_json::to_value(&StreamStats { rxtokens }).unwrap(),
        ).with_type("StreamStats").with_stream_id(stream_id.clone());
        if let Err(e) = bus.publish(&self.id(), stats_msg).await {
            error!("Failed to publish stream stats: {}", e);
        } else {
            info!("LLM actor {} published stats to '{}'", self.index, self.stats_topic);
        }

        for (idx, (func_name, func_args)) in &tool_call_accumulators {
            let full_tc = serde_json::json!({
                "id": format!("call_{}", stream_id).replace('-', "_"),
                "type": "function",
                "function": {
                    "name": func_name,
                    "arguments": func_args,
                }
            });
            info!("LLM actor {} accumulated tool call {}: {}", self.index, idx, func_name);
            tool_calls.push(full_tc);
        }

        Ok((full_response, tool_calls))
    }
}

#[async_trait::async_trait]
impl Actor for LlmActor {
    fn id(&self) -> sar_core::ActorId {
        format!("sar-llm-{}", self.index)
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut rx = bus.subscribe(&self.id(), &self.input_topic).await.map_err(|e| {
            format!("Failed to subscribe to input topic '{}': {}", self.input_topic, e)
        })?;

        info!("LLM actor {} listening on '{}'", self.index, self.input_topic);

        loop {
            match rx.recv().await {
                Ok(msg) => {
                    info!("LLM actor {} received request from '{}'", self.index, msg.source);
                    
                    let request: LlmRequest = match serde_json::from_value(msg.payload.clone()) {
                        Ok(r) => r,
                        Err(e) => {
                            error!("Failed to parse LLM request: {}", e);
                            continue;
                        }
                    };

                    let config = self.merge_config(request.config);
                    
                    let messages_vec: Vec<serde_json::Value> = if let Some(ref messages) = request.messages {
                        messages.clone()
                    } else {
                        vec![serde_json::json!({"role": "user", "content": request.prompt})]
                    };
                    info!("LLM actor {} sending {} messages to API", self.index, messages_vec.len());
                    let tools = request.tools.as_deref();
                    let result = self.send_request(&config, &messages_vec, tools, bus).await;
                    
                    match result {
                        Ok((full_response, tool_calls)) => {
                            info!("LLM actor {} completed request, tool_calls count: {}", self.index, tool_calls.len());
                            
                            if !tool_calls.is_empty() {
                                let tc_msg = Message::new(
                                    &self.tool_calls_topic,
                                    &self.id(),
                                    serde_json::to_value(&tool_calls).unwrap(),
                                ).with_type("LlmToolCalls");
                                if let Err(e) = bus.publish(&self.id(), tc_msg).await {
                                    error!("Failed to publish tool calls: {}", e);
                                }
                            } else {
                                let out_msg = Message::new(
                                    &self.output_topic,
                                    &self.id(),
                                    full_response,
                                );
                                if let Err(e) = bus.publish(&self.id(), out_msg).await {
                                    error!("Failed to publish LLM response: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            error!("LLM actor {} request failed: {}", self.index, e);
                            let error_msg = Message::new(
                                &self.output_topic,
                                &self.id(),
                                format!("Error: {}", e),
                            );
                            if let Err(e) = bus.publish(&self.id(), error_msg).await {
                                error!("Failed to publish error message: {}", e);
                            }
                        }
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    warn!("LLM actor {} lagged behind, dropped {} messages", self.index, n);
                }
                Err(RecvError::Closed) => {
                    info!("LLM actor {} input topic closed", self.index);
                    break;
                }
            }
        }

        Ok(())
    }
}
