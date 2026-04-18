use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::actor::{Actor, ActorJoinHandle};
use crate::message::Message;

pub struct SarBus {
    topics: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, broadcast::Sender<Message>>>>,
}

impl SarBus {
    pub fn new() -> Self {
        Self {
            topics: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn create_topic(&self, topic: &str, capacity: usize) {
        let mut topics = self.topics.write().await;
        if topics.contains_key(topic) {
            warn!("Topic '{}' already exists, skipping creation", topic);
            return;
        }
        let (sender, _) = broadcast::channel::<Message>(capacity);
        topics.insert(topic.to_string(), sender);
        info!("Created topic: {}", topic);
    }

    pub async fn publish(&self, message: Message) -> Result<(), BusError> {
        let topic = message.topic.clone();
        let topics = self.topics.read().await;
        
        match topics.get(&topic) {
            Some(sender) => {
                if sender.send(message).is_err() {
                    debug!("No subscribers for topic: {}", topic);
                }
                Ok(())
            }
            None => {
                error!("Published to unknown topic: {}", topic);
                Err(BusError::UnknownTopic(topic))
            }
        }
    }

    pub async fn subscribe(&self, topic: &str) -> Result<broadcast::Receiver<Message>, BusError> {
        let topics = self.topics.read().await;
        match topics.get(topic) {
            Some(sender) => {
                let receiver = sender.subscribe();
                debug!("Subscribed to topic: {}", topic);
                Ok(receiver)
            }
            None => Err(BusError::UnknownTopic(topic.to_string())),
        }
    }

    pub async fn list_topics(&self) -> Vec<String> {
        let topics = self.topics.read().await;
        topics.keys().cloned().collect()
    }

    pub async fn spawn_actor<A: Actor + Send + 'static>(
        &self,
        actor: A,
    ) -> Result<ActorJoinHandle, BusError> {
        let actor_id = actor.id().clone();
        info!("Spawning actor: {}", actor_id);

        let bus = self.clone();
        let id_for_task = actor_id.clone();

        let join_handle = tokio::spawn(async move {
            info!("Actor '{}' starting", id_for_task);
            if let Err(e) = actor.run(&bus).await {
                error!("Actor '{}' failed: {}", id_for_task, e);
            }
            info!("Actor '{}' stopped", id_for_task);
        });

        Ok(ActorJoinHandle::new(actor_id, join_handle))
    }

    pub async fn spawn_actor_detached<A: Actor + Send + 'static>(
        &self,
        actor: A,
    ) -> Result<(), BusError> {
        let actor_id = actor.id().clone();
        info!("Spawning detached actor: {}", actor_id);

        let bus = self.clone();

        tokio::spawn(async move {
            if let Err(e) = actor.run(&bus).await {
                error!("Detached actor '{}' failed: {}", actor_id, e);
            }
            info!("Detached actor '{}' stopped", actor_id);
        });

        Ok(())
    }
}

impl Clone for SarBus {
    fn clone(&self) -> Self {
        Self {
            topics: self.topics.clone(),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum BusError {
    #[error("Unknown topic: {0}")]
    UnknownTopic(String),
    #[error("Actor error: {0}")]
    Actor(String),
    #[error("Channel closed")]
    ChannelClosed,
}