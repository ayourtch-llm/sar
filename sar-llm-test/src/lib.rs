use async_trait::async_trait;
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::config::LlmConfig;
use sar_core::message::Message;
use sar_llm::LlmRequest;
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};

#[derive(Debug, Default)]
pub struct LlmTestActor {
    pub index: usize,
    pub input_topic: String,
    pub llm_in_topic: String,
    pub llm_out_topic: String,
    pub llm_stream_topic: String,
    pub stream_output_topic: String,
    pub llm_base_url: String,
}

impl LlmTestActor {
    pub fn new(index: usize, input_topic: String, llm_in_topic: String, llm_out_topic: String, llm_stream_topic: String, stream_output_topic: String) -> Self {
        Self {
            index,
            input_topic,
            llm_in_topic,
            llm_out_topic,
            llm_stream_topic,
            stream_output_topic,
            llm_base_url: String::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.llm_base_url = base_url;
        self
    }

}

#[async_trait::async_trait]
impl Actor for LlmTestActor {
    fn id(&self) -> sar_core::ActorId {
        format!("sar-llm-test-{}", self.index)
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut input_rx = bus.subscribe(&self.id(), &self.input_topic).await.map_err(|e| {
            format!("Failed to subscribe to input topic '{}': {}", self.input_topic, e)
        })?;

        let mut out_rx = bus.subscribe(&self.id(), &self.llm_out_topic).await.map_err(|e| {
            format!("Failed to subscribe to output topic '{}': {}", self.llm_out_topic, e)
        })?;

        let mut stream_rx = bus.subscribe(&self.id(), &self.llm_stream_topic).await.map_err(|e| {
            format!("Failed to subscribe to stream topic '{}': {}", self.llm_stream_topic, e)
        })?;

        info!(
            "LLM test actor {} listening on '{}'",
            self.index, self.input_topic
        );

        loop {
            tokio::select! {
                result = input_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            info!(
                                "LLM test actor {} received from '{}'",
                                self.index, msg.source
                            );

                            let prompt = match msg.payload.as_str() {
                                Some(s) => s.to_string(),
                                None => msg.payload.to_string(),
                            };

                            let llm_request = LlmRequest {
                                prompt,
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
                                "LLM test actor {} lagged behind, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test actor {} input topic closed", self.index);
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
                            info!(
                                "LLM test actor {} received output from '{}': {}",
                                self.index, msg.source, output
                            );
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test actor {} lagged behind on output, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test actor {} output topic closed", self.index);
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
                            info!(
                                "LLM test actor {} forwarded stream chunk to '{}': {:?}",
                                self.index, self.stream_output_topic, msg
                            );
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test actor {} lagged behind on stream, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test actor {} stream topic closed", self.index);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}