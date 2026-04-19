use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::message::Message;
use sar_core::config::UiHubConfig;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

pub const USER_CONTROL_TOPIC: &str = "user:control";

pub fn is_continue_message(input: &str) -> Option<String> {
    if let Some(rest) = input.strip_prefix("/continue ") {
        return Some(rest.to_string());
    }
    if input == "/continue" {
        return Some("".to_string());
    }
    None
}

#[derive(Debug, Default)]
pub struct UiHubActor {
    config: UiHubConfig,
}

impl UiHubActor {
    pub fn new(config: UiHubConfig) -> Self {
        Self { config }
    }
}

fn classify_message(topic: &str, payload: &serde_json::Value, meta: &serde_json::Value) -> String {
    if let serde_json::Value::Object(meta_obj) = meta {
        if let Some(existing_type) = meta_obj.get("type").and_then(|t| t.as_str()) {
            if matches!(existing_type, "LlmThinking" | "LlmStream" | "LlmStreamEnd" | "LlmToolCall" | "LlmToolResult" | "LlmDump" | "StreamStats") {
                return existing_type.to_string();
            }
        }
    }
    if topic.ends_with(":stream") {
        let is_stream_end = match payload {
            serde_json::Value::String(s) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(s) {
                    json.get("type").and_then(|t| t.as_str()) == Some("stream_end")
                } else {
                    false
                }
            }
            serde_json::Value::Object(map) => {
                map.get("type").and_then(|t| t.as_str()) == Some("stream_end")
            }
            _ => false,
        };
        if is_stream_end {
            return "LlmStreamEnd".to_string();
        }
        return "LlmStream".to_string();
    }
    match topic {
        t if t.contains("echo") => "Echo".to_string(),
        t if t.contains("reverse") => "Reverse".to_string(),
        t if t.contains("log") => "Log".to_string(),
        _ => "Info".to_string(),
    }
}

#[async_trait::async_trait]
impl Actor for UiHubActor {
    fn id(&self) -> sar_core::ActorId {
        format!("sar-ui-hub-{}", self.config.name)
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            "UI hub '{}' starting: user_topic={}, input_topic={}, subscribe_to={}, route_to={}",
            self.config.name,
            self.config.user_topic,
            self.config.input_topic,
            self.config.subscribe_to.len(),
            self.config.route_to.len()
        );

        let hub_id = self.id();
        let mut input_rx = bus.subscribe(&hub_id, &self.config.input_topic).await
            .map_err(|e| format!("Failed to subscribe to input topic '{}': {}", self.config.input_topic, e))?;

        info!("UI hub '{}' subscribed to input topic: {}", self.config.name, self.config.input_topic);

        for topic in &self.config.subscribe_to {
            let bus = bus.clone();
            let hub_id = format!("sar-ui-hub-{}", self.config.name);
            let hub_name = self.config.name.clone();
            let topic_clone = topic.clone();
            let user_topic = self.config.user_topic.clone();
            tokio::spawn(async move {
                let mut rx = match bus.subscribe(&hub_id, &topic_clone).await {
                    Ok(rx) => rx,
                    Err(e) => {
                        error!("UI hub '{}' failed to subscribe to producer topic '{}': {}", hub_name, topic_clone, e);
                        return;
                    }
                };
                info!("UI hub '{}' spawned forwarder for producer topic: {}", hub_name, topic_clone);

                loop {
                    match rx.recv().await {
                        Ok(msg) => {
                            if msg.source == hub_id {
                                continue;
                            }
                            let msg_type = classify_message(&topic_clone, &msg.payload, &msg.meta);
                            info!("UI hub '{}' forwarder received from '{}': type={}, payload={}", hub_name, topic_clone, msg_type, msg.payload);
                            let mut forwarded = msg.clone();
                            forwarded.topic = user_topic.clone();
                            forwarded.meta = serde_json::json!({"type": msg_type});
                            if let Err(e) = bus.publish(&hub_id, forwarded).await {
                                error!("UI hub '{}' failed to publish to user topic: {}", hub_name, e);
                            } else {
                                info!("UI hub '{}' forwarded to '{}'", hub_name, user_topic);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("UI hub '{}' forwarder for '{}' lagged behind, dropped {} messages", hub_name, topic_clone, n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("UI hub '{}' producer topic '{}' closed", hub_name, topic_clone);
                            break;
                        }
                    }
                }
            });
        }

        loop {
            match input_rx.recv().await {
                Ok(msg) => {
                    let payload_str = match &msg.payload {
                        serde_json::Value::String(s) => s.clone(),
                        _ => msg.payload.to_string(),
                    };

                    if let Some(reason) = is_continue_message(&payload_str) {
                        let control_msg = Message::new(
                            crate::USER_CONTROL_TOPIC,
                            &hub_id,
                            serde_json::json!({
                                "type": "continue",
                                "reason": reason,
                            }),
                        ).with_type("Continue");
                        if let Err(e) = bus.publish(&hub_id, control_msg).await {
                            error!("UI hub '{}' failed to publish continue to '{}': {}", self.config.name, USER_CONTROL_TOPIC, e);
                        } else {
                            info!("UI hub '{}' published continue to '{}': reason='{}'", self.config.name, USER_CONTROL_TOPIC, reason);
                        }
                        continue;
                    }

                    for route_topic in &self.config.route_to {
                        let routed_msg = Message::new(
                            route_topic,
                            &msg.source,
                            msg.payload.clone(),
                        );
                        if let Err(e) = bus.publish(&hub_id, routed_msg).await {
                            error!("UI hub '{}' failed to route to '{}': {}", self.config.name, route_topic, e);
                        }
                    }
                    let user_msg = Message::new(
                        &self.config.user_topic,
                        &msg.source,
                        msg.payload.clone(),
                    ).with_type("UserInput");
                    if let Err(e) = bus.publish(&hub_id, user_msg).await {
                        error!("UI hub '{}' failed to publish user input to user topic: {}", self.config.name, e);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("UI hub '{}' input subscriber lagged behind, dropped {} messages", self.config.name, n);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("UI hub '{}' input topic closed", self.config.name);
                    break;
                }
            }
        }

        Ok(())
    }
}