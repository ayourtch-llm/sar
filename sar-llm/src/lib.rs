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
    pub prompt: String,
    #[serde(default)]
    pub config: Option<LlmConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEnd {
    pub r#type: String,
}

#[derive(Debug, Default)]
pub struct LlmActor {
    pub index: usize,
    pub input_topic: String,
    pub output_topic: String,
    pub stream_topic: String,
    pub config: LlmConfig,
}

impl LlmActor {
    pub fn new(index: usize, input_topic: String, output_topic: String, stream_topic: String, config: LlmConfig) -> Self {
        Self {
            index,
            input_topic,
            output_topic,
            stream_topic,
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

    async fn send_request(&self, config: &LlmConfig, prompt: &str, bus: &SarBus) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::new();
        
        let body = serde_json::json!({
            "model": config.model,
            "messages": [
                {"role": "user", "content": prompt}
            ],
            "temperature": config.temperature,
            "max_tokens": config.max_tokens,
            "stream": true
        });

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
                        if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                            if !content.is_empty() {
                                let chunk_msg = Message::new(
                                    &self.stream_topic,
                                    &self.id(),
                                    content.to_string(),
                                );
                                if let Err(e) = bus.publish(&self.id(), chunk_msg).await {
                                    error!("Failed to publish stream chunk: {}", e);
                                }
                                full_response.push_str(content);
                            }
                        }
                    }
                }
            }
        }

        let end_msg = Message::new(
            &self.stream_topic,
            &self.id(),
            serde_json::to_string(&StreamEnd { r#type: "stream_end".to_string() }).unwrap(),
        );
        if let Err(e) = bus.publish(&self.id(), end_msg).await {
            error!("Failed to publish stream end: {}", e);
        }

        Ok(full_response)
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
                    
                    let result = self.send_request(&config, &request.prompt, bus).await;
                    
                    match result {
                        Ok(full_response) => {
                            info!("LLM actor {} completed request", self.index);
                            
                            let out_msg = Message::new(
                                &self.output_topic,
                                &self.id(),
                                full_response,
                            );
                            if let Err(e) = bus.publish(&self.id(), out_msg).await {
                                error!("Failed to publish LLM response: {}", e);
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