use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::message::Message;
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info};

const APP_ID: &str = "sar-reverse";

#[derive(Debug, Default)]
pub struct ReverseActor {
    input_topic: String,
    log_topic: String,
}

impl ReverseActor {
    pub fn new(input_topic: String, log_topic: String) -> Self {
        Self {
            input_topic,
            log_topic,
        }
    }
}

#[async_trait::async_trait]
impl Actor for ReverseActor {
    fn id(&self) -> sar_core::ActorId {
        APP_ID.to_string()
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut rx = bus.subscribe(&self.input_topic).await.map_err(|e| {
            format!("Failed to subscribe to input topic '{}': {}", self.input_topic, e)
        })?;

        info!("Reverse actor listening on '{}'", self.input_topic);

        loop {
            match rx.recv().await {
                Ok(msg) => {
                    info!("Reverse: received from '{}': {}", msg.source, msg.payload);
                    let reversed = msg.payload.as_str().map(|s| s.chars().rev().collect::<String>());
                    let echo = Message::new(
                        &self.log_topic,
                        APP_ID,
                        format!("reverse: {}", reversed.unwrap_or(msg.payload.to_string())),
                    );
                    if let Err(e) = bus.publish(echo).await {
                        error!("Failed to publish reverse: {}", e);
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("Reverse actor lagged behind, dropped {} messages", n);
                }
                Err(RecvError::Closed) => {
                    info!("Input topic channel closed, reverse actor stopping");
                    break;
                }
            }
        }

        Ok(())
    }
}