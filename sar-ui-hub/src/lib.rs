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

#[async_trait::async_trait]
impl Actor for UiHubActor {
    fn id(&self) -> sar_core::ActorId {
        format!("sar-ui-hub-{}", self.config.name)
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let buffer_size = self.config.buffer_size;

        let mut user_rx = bus.subscribe(&self.config.user_topic).await
            .map_err(|e| format!("Failed to subscribe to user topic '{}': {}", self.config.user_topic, e))?;

        let mut input_rx = bus.subscribe(&self.config.input_topic).await
            .map_err(|e| format!("Failed to subscribe to input topic '{}': {}", self.config.input_topic, e))?;

        info!(
            "UI hub '{}' started: user={}, input={}, subscribe_to={}, route_to={}",
            self.config.name,
            self.config.user_topic,
            self.config.input_topic,
            self.config.subscribe_to.len(),
            self.config.route_to.len()
        );

        let mut user_subscribers = Vec::new();
        for topic in &self.config.subscribe_to {
            match bus.subscribe(topic).await {
                Ok(rx) => {
                    info!("UI hub '{}' subscribed to producer topic: {}", self.config.name, topic);
                    user_subscribers.push((topic.clone(), rx));
                }
                Err(e) => {
                    warn!("UI hub '{}' failed to subscribe to producer topic '{}': {}", self.config.name, topic, e);
                }
            }
        }

        let mut input_routers = Vec::new();
        for topic in &self.config.route_to {
            match bus.subscribe(topic).await {
                Ok(rx) => {
                    info!("UI hub '{}' subscribed to consumer topic: {}", self.config.name, topic);
                    input_routers.push((topic.clone(), rx));
                }
                Err(e) => {
                    warn!("UI hub '{}' failed to subscribe to consumer topic '{}': {}", self.config.name, topic, e);
                }
            }
        }

        loop {
            tokio::select! {
                biased;

                result = user_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            if let Err(e) = bus.publish(msg).await {
                                error!("UI hub '{}' failed to publish to user topic: {}", self.config.name, e);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("UI hub '{}' user subscriber lagged behind, dropped {} messages", self.config.name, n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("UI hub '{}' user topic closed", self.config.name);
                            break;
                        }
                    }
                }

                result = input_rx.recv() => {
                    match result {
                        Ok(msg) => {
                            for (route_topic, _) in &input_routers {
                                let routed_msg = Message::new(
                                    route_topic,
                                    &msg.source,
                                    msg.payload.clone(),
                                );
                                if let Err(e) = bus.publish(routed_msg).await {
                                    error!("UI hub '{}' failed to route to '{}': {}", self.config.name, route_topic, e);
                                }
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

                else => break,
            }
        }

        Ok(())
    }
}