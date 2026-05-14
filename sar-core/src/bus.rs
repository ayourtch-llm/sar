use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::actor::{Actor, ActorAnnouncement, ActorJoinHandle, TopicAnnouncement};
use crate::message::Message;

#[derive(Debug, Clone)]
pub struct ActorInfo {
    pub id: String,
    pub subscriptions: Vec<String>,
    pub publications: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TopicInfo {
    pub name: String,
    pub capacity: usize,
    pub subscribers: Vec<String>,
    pub publishers: Vec<String>,
}

pub struct SarBus {
    topics: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, (broadcast::Sender<Message>, usize)>>>,
    actors: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, ActorInfo>>>,
    topic_subscribers: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, std::collections::HashSet<String>>>>,
    topic_publishers: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, std::collections::HashSet<String>>>>,
    announced_actors: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, ActorAnnouncement>>>,
}

impl SarBus {
    pub fn new() -> Self {
        Self {
            topics: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            actors: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            topic_subscribers: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            topic_publishers: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            announced_actors: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn create_topic(&self, topic: &str, capacity: usize) {
        let mut topics = self.topics.write().await;
        if topics.contains_key(topic) {
            warn!("Topic '{}' already exists, skipping creation", topic);
            return;
        }
        let (sender, _) = broadcast::channel::<Message>(capacity);
        topics.insert(topic.to_string(), (sender, capacity));
        info!("Created topic: {}", topic);
    }

    async fn ensure_topic(&self, topic: &str, capacity: usize) {
        let mut topics = self.topics.write().await;
        if !topics.contains_key(topic) {
            let (sender, _) = broadcast::channel::<Message>(capacity);
            topics.insert(topic.to_string(), (sender, capacity));
            info!("Auto-created topic: {}", topic);
        }
    }

    pub async fn publish(&self, actor_id: &str, message: Message) -> Result<(), BusError> {
        if !self.is_announced(actor_id).await {
            error!("Actor '{}' has not announced and cannot publish", actor_id);
            return Err(BusError::UnannouncedActor(actor_id.to_string()));
        }

        let topic = message.topic.clone();
        let capacity = 1000;
        self.ensure_topic(&topic, capacity).await;
        
        let topics = self.topics.read().await;
        match topics.get(&topic) {
            Some((sender, _)) => {
                if sender.send(message).is_err() {
                    debug!("No subscribers for topic: {}", topic);
                }
                self.register_actor(actor_id, &topic, false).await;
                Ok(())
            }
            None => {
                error!("Published to unknown topic: {}", topic);
                Err(BusError::UnknownTopic(topic))
            }
        }
    }

    pub async fn subscribe(&self, actor_id: &str, topic: &str) -> Result<broadcast::Receiver<Message>, BusError> {
        let capacity = 1000;
        self.ensure_topic(topic, capacity).await;
        
        let topics = self.topics.read().await;
        match topics.get(topic) {
            Some((sender, _)) => {
                let receiver = sender.subscribe();
                debug!("Subscribed to topic: {}", topic);
                self.register_actor(actor_id, topic, true).await;
                Ok(receiver)
            }
            None => Err(BusError::UnknownTopic(topic.to_string())),
        }
    }

    /// Subscribe to a topic without registering an actor — for external consumers like SSE.
    /// Returns a broadcast receiver that starts receiving from the next message onward.
    pub async fn subscribe_stream(&self, topic: &str) -> Result<broadcast::Receiver<Message>, BusError> {
        let capacity = 1000;
        self.ensure_topic(topic, capacity).await;
        let topics = self.topics.read().await;
        match topics.get(topic) {
            Some((sender, _)) => {
                let receiver = sender.subscribe();
                debug!("External stream subscribed to topic: {}", topic);
                Ok(receiver)
            }
            None => Err(BusError::UnknownTopic(topic.to_string())),
        }
    }

    pub async fn list_topics(&self) -> Vec<String> {
        let topics = self.topics.read().await;
        topics.keys().cloned().collect()
    }

    pub async fn get_topic_capacity(&self, topic: &str) -> Option<usize> {
        let topics = self.topics.read().await;
        topics.get(topic).map(|(_, capacity)| *capacity)
    }

    pub async fn register_actor(&self, actor_id: &str, topic: &str, is_subscription: bool) {
        let mut actors = self.actors.write().await;
        let actor_info = actors.entry(actor_id.to_string()).or_insert_with(|| ActorInfo {
            id: actor_id.to_string(),
            subscriptions: Vec::new(),
            publications: Vec::new(),
        });
        
        if is_subscription {
            if !actor_info.subscriptions.contains(&topic.to_string()) {
                actor_info.subscriptions.push(topic.to_string());
            }
        } else {
            if !actor_info.publications.contains(&topic.to_string()) {
                actor_info.publications.push(topic.to_string());
            }
        }

        if is_subscription {
            let mut subscribers = self.topic_subscribers.write().await;
            let subs = subscribers.entry(topic.to_string()).or_insert_with(std::collections::HashSet::new);
            subs.insert(actor_id.to_string());
        } else {
            let mut publishers = self.topic_publishers.write().await;
            let pubs = publishers.entry(topic.to_string()).or_insert_with(std::collections::HashSet::new);
            pubs.insert(actor_id.to_string());
        }
    }

    pub async fn unregister_actor(&self, actor_id: &str, topic: &str, is_subscription: bool) {
        let mut actors = self.actors.write().await;
        if let Some(actor_info) = actors.get_mut(actor_id) {
            if is_subscription {
                actor_info.subscriptions.retain(|t| t != topic);
            } else {
                actor_info.publications.retain(|t| t != topic);
            }
        }

        let mut subscribers = self.topic_subscribers.write().await;
        if let Some(subs) = subscribers.get_mut(topic) {
            subs.remove(actor_id);
        }

        let mut publishers = self.topic_publishers.write().await;
        if !is_subscription {
            if let Some(pubs) = publishers.get_mut(topic) {
                pubs.remove(actor_id);
            }
        }
    }

    pub async fn list_actors(&self) -> Vec<ActorInfo> {
        let actors = self.actors.read().await;
        actors.values().cloned().collect()
    }

    pub async fn register_announcement(&self, announcement: ActorAnnouncement) {
        let mut actors = self.announced_actors.write().await;
        actors.insert(announcement.id.clone(), announcement);
    }

    pub async fn list_announced_actors(&self) -> Vec<ActorAnnouncement> {
        let actors = self.announced_actors.read().await;
        actors.values().cloned().collect()
    }

    pub async fn list_announced_topics(&self) -> Vec<TopicAnnouncement> {
        let actors = self.announced_actors.read().await;
        let mut topics: std::collections::HashMap<String, TopicAnnouncement> = std::collections::HashMap::new();

        for announcement in actors.values() {
            for sub in &announcement.subscriptions {
                topics.entry(sub.clone()).or_insert_with(|| TopicAnnouncement {
                    name: sub.clone(),
                    subscribers: Vec::new(),
                    publishers: Vec::new(),
                }).subscribers.push(announcement.id.clone());
            }
            for pub_topic in &announcement.publications {
                topics.entry(pub_topic.clone()).or_insert_with(|| TopicAnnouncement {
                    name: pub_topic.clone(),
                    subscribers: Vec::new(),
                    publishers: Vec::new(),
                }).publishers.push(announcement.id.clone());
            }
        }

        let mut result: Vec<TopicAnnouncement> = topics.into_values().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }

    pub async fn is_announced(&self, actor_id: &str) -> bool {
        let actors = self.announced_actors.read().await;
        actors.contains_key(actor_id)
    }

    pub async fn get_announced(&self, actor_id: &str) -> Option<ActorAnnouncement> {
        let actors = self.announced_actors.read().await;
        actors.get(actor_id).cloned()
    }

    pub async fn list_topic_info(&self) -> Vec<TopicInfo> {
        let topics = self.topics.read().await;
        let subscribers = self.topic_subscribers.read().await;
        let publishers = self.topic_publishers.read().await;
        
        let mut result = Vec::new();
        for (name, (_, capacity)) in topics.iter() {
            let subs = subscribers.get(name).map(|s| s.iter().cloned().collect()).unwrap_or_default();
            let pubs = publishers.get(name).map(|p| p.iter().cloned().collect()).unwrap_or_default();
            
            result.push(TopicInfo {
                name: name.clone(),
                capacity: *capacity,
                subscribers: subs,
                publishers: pubs,
            });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }

    pub async fn spawn_actor<A: Actor + Send + 'static>(
        &self,
        actor: A,
    ) -> Result<ActorJoinHandle, BusError> {
        let actor_id = actor.id().clone();
        let announcement = actor.announce();
        info!("Spawning actor: {} (announced {} subs, {} pubs)", actor_id, announcement.subscriptions.len(), announcement.publications.len());
        self.register_announcement(announcement.clone()).await;

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
            actors: self.actors.clone(),
            topic_subscribers: self.topic_subscribers.clone(),
            topic_publishers: self.topic_publishers.clone(),
            announced_actors: self.announced_actors.clone(),
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
    #[error("Actor '{}' has not announced and cannot publish", _0)]
    UnannouncedActor(String),
}