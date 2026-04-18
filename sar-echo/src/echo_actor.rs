
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::message::Message;
use tracing::{error, info};

const APP_ID: &str = "sar-echo";

#[derive(Debug, Default)]
pub struct EchoActor {
    input_topic: String,
    log_topic: String,
}

impl EchoActor {
    pub fn new(input_topic: String, log_topic: String) -> Self {
        Self {
            input_topic,
            log_topic,
        }
    }
}

#[async_trait::async_trait]
impl Actor for EchoActor {
    fn id(&self) -> sar_core::ActorId {
        APP_ID.to_string()
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut rx = bus.subscribe(&self.id(), &self.input_topic).await.map_err(|e| {
            format!("Failed to subscribe to input topic '{}': {}", self.input_topic, e)
        })?;

        info!("Echo actor listening on '{}'", self.input_topic);

        loop {
            match rx.recv().await {
                Ok(msg) => {
                    info!("Echo: received from '{}': {}", msg.source, msg.payload);
                    let echo = Message::new(
                        &self.log_topic,
                        APP_ID,
                        format!("echo: {}", msg.payload),
                    );
                    if let Err(e) = bus.publish(&self.id(), echo).await {
                        error!("Failed to publish echo: {}", e);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Echo actor lagged behind, dropped {} messages", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("Input topic channel closed, echo actor stopping");
                    break;
                }
            }
        }

        Ok(())
    }
}