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
    pub control_topic: String,
    pub config: LlmConfig,
}

impl LlmActor {
    pub fn new(index: usize, input_topic: String, output_topic: String, stream_topic: String, stats_topic: String, tool_calls_topic: String, control_topic: String, config: LlmConfig) -> Self {
        Self {
            index,
            input_topic,
            output_topic,
            stream_topic,
            stats_topic,
            tool_calls_topic,
            control_topic,
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

    fn announce(&self) -> sar_core::actor::ActorAnnouncement {
        sar_core::actor::ActorAnnouncement {
            id: self.id(),
            subscriptions: vec![self.input_topic.clone(), self.control_topic.clone()],
            publications: vec![
                self.output_topic.clone(),
                self.stream_topic.clone(),
                self.stats_topic.clone(),
                self.tool_calls_topic.clone(),
            ],
        }
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("[llm{}] Subscribing to input topic: '{}'", self.index, self.input_topic);
        let mut rx = bus.subscribe(&self.id(), &self.input_topic).await.map_err(|e| {
            format!("Failed to subscribe to input topic '{}': {}", self.input_topic, e)
        })?;

        info!("[llm{}] Subscribing to control topic: '{}'", self.index, self.control_topic);
        let mut control_rx = bus.subscribe(&self.id(), &self.control_topic).await.map_err(|e| {
            format!("Failed to subscribe to control topic '{}': {}", self.control_topic, e)
        })?;

        info!("[llm{}] Listening on input='{}' control='{}'", self.index, self.input_topic, self.control_topic);

        let mut active_request: Option<tokio::task::JoinHandle<Result<(String, Vec<serde_json::Value>), Box<dyn std::error::Error + Send + Sync>>>> = None;

        loop {
            if let Some(req_handle) = active_request.take() {
                let abort_handle = req_handle.abort_handle();
                let mut req_handle_opt = Some(req_handle);
                // We have an active request — select between control and request completion
                loop {
                    let control_msg = tokio::select! {
                        control_msg = control_rx.recv() => {
                            info!("[llm{}] Control branch ready (with active request)", self.index);
                            control_msg
                        }
                        request_result = async {
                            if let Some(handle) = req_handle_opt.take() {
                                handle.await
                            } else {
                                std::future::pending().await
                            }
                        } => {
                            info!("[llm{}] Request task completed", self.index);
                            match request_result {
                                Ok(Ok((full_response, tool_calls))) => {
                                    info!("[llm{}] Request succeeded, tool_calls={}", self.index, tool_calls.len());
                                    if !tool_calls.is_empty() {
                                        let tc_msg = Message::new(
                                            &self.tool_calls_topic,
                                            &self.id(),
                                            serde_json::to_value(&tool_calls).unwrap(),
                                        ).with_type("LlmToolCalls");
                                        if let Err(e) = bus.publish(&self.id(), tc_msg).await {
                                            error!("[llm{}] Failed to publish tool calls: {}", self.index, e);
                                        }
                                    } else {
                                        let out_msg = Message::new(
                                            &self.output_topic,
                                            &self.id(),
                                            full_response,
                                        );
                                        if let Err(e) = bus.publish(&self.id(), out_msg).await {
                                            error!("[llm{}] Failed to publish LLM response: {}", self.index, e);
                                        }
                                    }
                                }
                                Ok(Err(e)) => {
                                    error!("[llm{}] Request failed: {}", self.index, e);
                                    let error_msg = Message::new(
                                        &self.output_topic,
                                        &self.id(),
                                        format!("Error: {}", e),
                                    );
                                    if let Err(e) = bus.publish(&self.id(), error_msg).await {
                                        error!("[llm{}] Failed to publish error message: {}", self.index, e);
                                    }
                                }
                                Err(e) => {
                                    error!("[llm{}] Request task was aborted: {}", self.index, e);
                                }
                            }
                            break;
                        }
                    };

                    match control_msg {
                        Ok(msg) => {
                            info!("[llm{}] Received control message, payload={:?}", self.index, msg.payload);
                            if let Ok(v) = serde_json::from_value::<serde_json::Value>(msg.payload) {
                                if let Some(type_str) = v.get("type").and_then(|t| t.as_str()) {
                                    info!("[llm{}] Control message type='{}'", self.index, type_str);
                                    if type_str == "interrupt" {
                                        let reason = v.get("reason").and_then(|r| r.as_str()).unwrap_or("interrupted");
                                        info!("[llm{}] Processing interrupt, reason='{}'", self.index, reason);
                                        info!("[llm{}] Aborting active request handle", self.index);
                                        abort_handle.abort();
                                        if let Some(handle) = req_handle_opt.take() {
                                            info!("[llm{}] Waiting for aborted request to finish", self.index);
                                            match handle.await {
                                                Ok(Err(e)) => info!("[llm{}] Aborted request returned error: {}", self.index, e),
                                                Ok(Ok(_)) => info!("[llm{}] Aborted request finished normally (was already done)", self.index),
                                                Err(e) => info!("[llm{}] Aborted request task was cancelled: {}", self.index, e),
                                            }
                                            info!("[llm{}] Request handle fully resolved", self.index);
                                        }
                                        info!("[llm{}] Returning interrupt error", self.index);
                                        return Err(format!("Interrupted: {}", reason).into());
                                    }
                                }
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!("[llm{}] Control lagged, dropped {} messages", self.index, n);
                        }
                        Err(RecvError::Closed) => {
                            info!("[llm{}] Control topic closed", self.index);
                            info!("[llm{}] Aborting active request due to control close", self.index);
                            abort_handle.abort();
                            if let Some(handle) = req_handle_opt.take() {
                                let _ = handle.await;
                            }
                            break;
                        }
                    }
                    // Put request back for next select! iteration
                    if let Some(handle) = req_handle_opt.take() {
                        active_request = Some(handle);
                    }
                    break;
                }
            } else {
                // No active request — select between input and control
                tokio::select! {
                    result = rx.recv() => {
                        info!("[llm{}] Input branch ready (no active request)", self.index);

                        match result {
                            Ok(msg) => {
                                info!("[llm{}] Received request from '{}'", self.index, msg.source);

                                let request: LlmRequest = match serde_json::from_value(msg.payload.clone()) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        error!("[llm{}] Failed to parse LLM request: {}", self.index, e);
                                        continue;
                                    }
                                };

                                let config = self.merge_config(request.config);

                                let messages_vec: Vec<serde_json::Value> = if let Some(ref messages) = request.messages {
                                    messages.clone()
                                } else {
                                    vec![serde_json::json!({"role": "user", "content": request.prompt})]
                                };
                                info!("[llm{}] Preparing to send {} messages to API", self.index, messages_vec.len());
                                let tools_vec = request.tools.clone();
                                let bus_clone = bus.clone();
                                let stream_topic = self.stream_topic.clone();
                                let stats_topic = self.stats_topic.clone();
                                let tool_calls_topic = self.tool_calls_topic.clone();
                                let index = self.index;
                                let config_merged = config.clone();
                                let messages_clone = messages_vec.clone();

                                active_request = Some(tokio::spawn(async move {
                                    info!("[llm{}] Request task spawned, starting send_request", index);
                                    let llm = LlmActor {
                                        index,
                                        input_topic: String::new(),
                                        output_topic: String::new(),
                                        stream_topic,
                                        stats_topic,
                                        tool_calls_topic,
                                        control_topic: String::new(),
                                        config: config_merged,
                                    };
                                    let tools = tools_vec.as_deref();
                                    llm.send_request(&llm.config, &messages_clone, tools, &bus_clone).await
                                }));
                            }
                            Err(RecvError::Lagged(n)) => {
                                warn!("[llm{}] Input lagged, dropped {} messages", self.index, n);
                            }
                            Err(RecvError::Closed) => {
                                info!("[llm{}] Input topic closed", self.index);
                                break;
                            }
                        }
                    }
                    control_msg = control_rx.recv() => {
                        info!("[llm{}] Control branch ready (no active request)", self.index);
                        match control_msg {
                            Ok(msg) => {
                                info!("[llm{}] Received control message, payload={:?}", self.index, msg.payload);
                                if let Ok(v) = serde_json::from_value::<serde_json::Value>(msg.payload) {
                                    if let Some(type_str) = v.get("type").and_then(|t| t.as_str()) {
                                        info!("[llm{}] Control message type='{}'", self.index, type_str);
                                        if type_str == "interrupt" {
                                            let reason = v.get("reason").and_then(|r| r.as_str()).unwrap_or("interrupted");
                                            info!("[llm{}] Processing interrupt, reason='{}'", self.index, reason);
                                            info!("[llm{}] No active request to abort", self.index);
                                            info!("[llm{}] Returning interrupt error", self.index);
                                            return Err(format!("Interrupted: {}", reason).into());
                                        }
                                    }
                                }
                            }
                            Err(RecvError::Lagged(n)) => {
                                warn!("[llm{}] Control lagged, dropped {} messages", self.index, n);
                            }
                            Err(RecvError::Closed) => {
                                info!("[llm{}] Control topic closed", self.index);
                                break;
                            }
                        }
                    }
                }
            }
        }

        info!("[llm{}] Actor run loop exited", self.index);
        Ok(())
    }
}
