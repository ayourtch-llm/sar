use sar_core::{Message, SarBus};

#[tokio::test]
async fn test_bus_new_empty() {
    let bus = SarBus::new();
    let topics = bus.list_topics().await;
    assert!(topics.is_empty());
}

#[tokio::test]
async fn test_bus_create_topic() {
    let bus = SarBus::new();
    bus.create_topic("test:topic", 100).await;
    let topics = bus.list_topics().await;
    assert_eq!(topics.len(), 1);
    assert!(topics.contains(&"test:topic".to_string()));
}

#[tokio::test]
async fn test_bus_create_topic_idempotent() {
    let bus = SarBus::new();
    bus.create_topic("test:topic", 100).await;
    bus.create_topic("test:topic", 200).await;
    let topics = bus.list_topics().await;
    assert_eq!(topics.len(), 1);
}

#[tokio::test]
async fn test_bus_create_multiple_topics() {
    let bus = SarBus::new();
    bus.create_topic("topic:1", 10).await;
    bus.create_topic("topic:2", 20).await;
    bus.create_topic("topic:3", 30).await;
    let mut topics = bus.list_topics().await;
    topics.sort();
    assert_eq!(topics, vec!["topic:1".to_string(), "topic:2".to_string(), "topic:3".to_string()]);
}

#[tokio::test]
async fn test_bus_publish_and_subscribe() {
    let bus = SarBus::new();
    bus.create_topic("test:pubsub", 100).await;
    
    let mut rx = bus.subscribe("test", "test:pubsub").await.unwrap();
    
    let msg = Message::text("test:pubsub", "sender", "hello");
    bus.publish("test", msg).await.unwrap();
    
    let received = rx.recv().await.unwrap();
    assert_eq!(received.payload, serde_json::json!("hello"));
}

#[tokio::test]
async fn test_bus_publish_to_unknown_topic_auto_creates() {
    let bus = SarBus::new();
    let msg = Message::text("unknown:topic", "sender", "data");
    let result = bus.publish("test", msg).await;
    assert!(result.is_ok());
    let topics = bus.list_topics().await;
    assert!(topics.contains(&"unknown:topic".to_string()));
}

#[tokio::test]
async fn test_bus_subscribe_to_unknown_topic_auto_creates() {
    let bus = SarBus::new();
    let result = bus.subscribe("test", "unknown:topic").await;
    assert!(result.is_ok());
    let topics = bus.list_topics().await;
    assert!(topics.contains(&"unknown:topic".to_string()));
}

#[tokio::test]
async fn test_bus_multiple_subscribers() {
    let bus = SarBus::new();
    bus.create_topic("test:multi", 100).await;
    
    let mut rx1 = bus.subscribe("test", "test:multi").await.unwrap();
    let mut rx2 = bus.subscribe("test", "test:multi").await.unwrap();
    
    let msg = Message::text("test:multi", "sender", "broadcast");
    bus.publish("test", msg).await.unwrap();
    
    let m1 = rx1.recv().await.unwrap();
    let m2 = rx2.recv().await.unwrap();
    
    assert_eq!(m1.payload, serde_json::json!("broadcast"));
    assert_eq!(m2.payload, serde_json::json!("broadcast"));
}

#[tokio::test]
async fn test_bus_clone_shares_topics() {
    let bus1 = SarBus::new();
    bus1.create_topic("shared:topic", 50).await;
    
    let bus2 = bus1.clone();
    
    let topics1 = bus1.list_topics().await;
    let topics2 = bus2.list_topics().await;
    
    assert_eq!(topics1, topics2);
    assert_eq!(topics1.len(), 1);
}

#[tokio::test]
async fn test_bus_clone_can_subscribe() {
    let bus1 = SarBus::new();
    bus1.create_topic("clone:test", 100).await;
    
    let bus2 = bus1.clone();
    let mut rx = bus2.subscribe("test", "clone:test").await.unwrap();
    
    let msg = Message::text("clone:test", "sender", "cloned");
    bus1.publish("test", msg).await.unwrap();
    
    let received = rx.recv().await.unwrap();
    assert_eq!(received.payload, serde_json::json!("cloned"));
}

#[tokio::test]
async fn test_bus_publish_no_subscribers() {
    let bus = SarBus::new();
    bus.create_topic("test:nosub", 100).await;
    
    let msg = Message::text("test:nosub", "sender", "no listeners");
    let result = bus.publish("test", msg).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_bus_lagged_receiver() {
    let bus = SarBus::new();
    bus.create_topic("test:lag", 5).await;
    
    let mut rx = bus.subscribe("test", "test:lag").await.unwrap();
    
    for i in 0..10 {
        let msg = Message::text("test:lag", "sender", format!("msg{}", i));
        bus.publish("test", msg).await.unwrap();
    }
    
    match rx.recv().await {
        Ok(msg) => {
            assert!(msg.payload.as_str().unwrap().starts_with("msg"));
        }
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
            assert!(true);
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
            panic!("Channel closed unexpectedly");
        }
    }
}

#[tokio::test]
async fn test_bus_message_format() {
    let bus = SarBus::new();
    bus.create_topic("test:format", 100).await;
    
    let mut rx = bus.subscribe("test", "test:format").await.unwrap();
    
    let msg = Message::new("test:format", "actor-1", "test payload");
    bus.publish("test", msg).await.unwrap();
    
    let received = rx.recv().await.unwrap();
    assert_eq!(received.topic, "test:format");
    assert_eq!(received.source, "actor-1");
    assert_eq!(received.payload, serde_json::json!("test payload"));
}