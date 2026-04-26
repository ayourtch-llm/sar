use async_trait::async_trait;
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::config::LlmConfig;
use sar_core::message::Message;
use sar_llm::LlmRequest;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};

#[derive(Debug, Default)]
pub struct LlmTestLoopActor {
    pub index: usize,
    pub input_topic: String,
    pub llm_in_topic: String,
    pub llm_out_topic: String,
    pub llm_stream_topic: String,
    pub stream_output_topic: String,
    pub llm_base_url: String,
    pub grammar: Arc<StdMutex<Option<String>>>,
}

impl LlmTestLoopActor {
    pub fn new(index: usize, input_topic: String, llm_in_topic: String, llm_out_topic: String, llm_stream_topic: String, stream_output_topic: String) -> Self {
        Self {
            index,
            input_topic,
            llm_in_topic,
            llm_out_topic,
            llm_stream_topic,
            stream_output_topic,
            llm_base_url: String::new(),
            grammar: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.llm_base_url = base_url;
        self
    }
}

#[async_trait::async_trait]
impl Actor for LlmTestLoopActor {
    fn id(&self) -> sar_core::ActorId {
        format!("sar-llm-test-loop-{}", self.index)
    }

    fn announce(&self) -> sar_core::actor::ActorAnnouncement {
        sar_core::actor::ActorAnnouncement {
            id: self.id(),
            subscriptions: vec![
                self.input_topic.clone(),
                self.llm_out_topic.clone(),
                self.llm_stream_topic.clone(),
                "llm-test-loop:0:grammar".to_string(),
            ],
            publications: vec![self.stream_output_topic.clone()],
        }
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

        let grammar_topic = "llm-test-loop:0:grammar".to_string();
        let mut grammar_rx = bus.subscribe(&self.id(), &grammar_topic).await.map_err(|e| {
            format!("Failed to subscribe to grammar topic '{}': {}", grammar_topic, e)
        })?;

        info!(
            "LLM test loop actor {} listening on '{}'",
            self.index, self.input_topic
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
                                "LLM test loop actor {} received user message, conversation size: {}",
                                self.index, history_len
                            );

                            let conversation_text = {
                                let history = conversation_history.lock().await;
                                history.join("\n")
                            };

                            let llm_request = LlmRequest {
                                messages: None,
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
                                tools: if self.grammar.lock().unwrap().is_some() { None } else { None },
                                grammar: self.grammar.lock().unwrap().clone(),
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
                                "LLM test loop actor {} lagged behind, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop actor {} input topic closed", self.index);
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
                                "LLM test loop actor {} received full response, conversation size: {}",
                                self.index, history_len
                            );
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test loop actor {} lagged behind on output, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop actor {} output topic closed", self.index);
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
                                "LLM test loop actor {} lagged behind on stream, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop actor {} stream topic closed", self.index);
                        }
                    }
                }
                result = grammar_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            if let Ok(v) = serde_json::from_value::<serde_json::Value>(msg.payload.clone()) {
                                if let Some(grammar_type) = v.get("type").and_then(|t| t.as_str()) {
                                    if grammar_type == "Grammar" {
                                        let content = v.get("content");
                                        match content {
                                            Some(serde_json::Value::String(s)) => {
                                                info!(
                                                    "LLM test loop actor {} setting grammar, length={}",
                                                    self.index, s.len()
                                                );
                                                let mut g = self.grammar.lock().unwrap();
                                                *g = Some(s.clone());
                                            }
                                            _ => {
                                                info!("LLM test loop actor {} clearing grammar", self.index);
                                                let mut g = self.grammar.lock().unwrap();
                                                *g = None;
                                            }
                                        }
                                        let confirmation = Message::text(
                                            &self.stream_output_topic,
                                            &self.id(),
                                            if content.is_some() {
                                                format!("Grammar set ({} chars)", content.as_ref().unwrap().to_string().len())
                                            } else {
                                                "Grammar cleared".to_string()
                                            },
                                        ).with_type("Info");
                                        if let Err(e) = bus.publish(&self.id(), confirmation).await {
                                            error!("Failed to publish grammar confirmation: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(
                                "LLM test loop actor {} lagged behind on grammar, dropped {} messages",
                                self.index, n
                            );
                        }
                        Err(RecvError::Closed) => {
                            info!("LLM test loop actor {} grammar topic closed", self.index);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}