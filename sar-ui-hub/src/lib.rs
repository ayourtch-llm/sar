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
        info!(
            "UI hub '{}' starting: user_topic={}, input_topic={}, subscribe_to={}, route_to={}",
            self.config.name,
            self.config.user_topic,
            self.config.input_topic,
            self.config.subscribe_to.len(),
            self.config.route_to.len()
        );

        let mut input_rx = bus.subscribe(&self.config.input_topic).await
            .map_err(|e| format!("Failed to subscribe to input topic '{}': {}", self.config.input_topic, e))?;

        info!("UI hub '{}' subscribed to input topic: {}", self.config.name, self.config.input_topic);

        for topic in &self.config.subscribe_to {
            let bus = bus.clone();
            let hub_name = self.config.name.clone();
            let topic_clone = topic.clone();
            tokio::spawn(async move {
                let mut rx = match bus.subscribe(&topic_clone).await {
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
                            if let Err(e) = bus.publish(msg).await {
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

        Ok(())
    }
}