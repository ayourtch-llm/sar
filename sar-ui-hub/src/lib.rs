use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::message::Message;
use sar_core::config::UiHubConfig;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

#[derive(Debug, Default)]
pub struct UiHubActor {
    config: UiHubConfig,
}

impl UiHubActor {
    pub fn new(config: UiHubConfig) -> Self {
        Self { config }
    }
}

fn classify_message(topic: &str, payload: &serde_json::Value) -> &'static str {
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
            return "LlmStreamEnd";
        }
        return "LlmStream";
    }
    match topic {
        t if t.contains("echo") => "Echo",
        t if t.contains("reverse") => "Reverse",
        t if t.contains("log") => "Log",
        _ => "Info",
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
                            let mut forwarded = msg.clone();
                            forwarded.topic = user_topic.clone();
                            let msg_type = classify_message(&topic_clone, &msg.payload);
                            forwarded.meta = serde_json::json!({"type": msg_type});
                            if let Err(e) = bus.publish(&hub_id, forwarded).await {
                                error!("UI hub '{}' failed to publish to user topic: {}", hub_name, e);
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