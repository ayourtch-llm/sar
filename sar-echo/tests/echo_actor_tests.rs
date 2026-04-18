use sar_core::{Actor, Message, SarBus};
use sar_echo::EchoActor;

#[tokio::test]
async fn test_echo_actor_id() {
    let actor = EchoActor::new("sar:input".to_string(), "sar:log".to_string());
    assert_eq!(actor.id(), "sar-echo");
}

#[tokio::test]
async fn test_echo_actor_receives_and_publishes() {
    let bus = SarBus::new();
    bus.create_topic("test:input", 100).await;
    bus.create_topic("test:log", 100).await;
    
    let mut log_rx = bus.subscribe("test", "test:log").await.unwrap();
    
    let actor = EchoActor::new("test:input".to_string(), "test:log".to_string());
    let handle = bus.spawn_actor(actor).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let msg = Message::text("test:input", "test-source", "hello");
    bus.publish("test", msg).await.unwrap();
    
    let received = log_rx.recv().await.unwrap();
    assert_eq!(received.source, "sar-echo");
    assert_eq!(received.payload, serde_json::json!("echo: \"hello\""));
    
    handle.stop().await;
}

#[tokio::test]
async fn test_echo_actor_multiple_messages() {
    let bus = SarBus::new();
    bus.create_topic("test:input2", 100).await;
    bus.create_topic("test:log2", 100).await;
    
    let mut log_rx = bus.subscribe("test", "test:log2").await.unwrap();
    
    let actor = EchoActor::new("test:input2".to_string(), "test:log2".to_string());
    let handle = bus.spawn_actor(actor).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    for i in 1..=3 {
        let msg = Message::text("test:input2", "source", format!("message {}", i));
        bus.publish("test", msg).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    for i in 1..=3 {
        let received = log_rx.recv().await.unwrap();
        assert_eq!(received.source, "sar-echo");
        assert_eq!(received.payload, serde_json::json!(format!("echo: \"message {}\"", i)));
    }
    
    handle.stop().await;
}

#[tokio::test]
async fn test_echo_actor_with_json_payload() {
    let bus = SarBus::new();
    bus.create_topic("test:input3", 100).await;
    bus.create_topic("test:log3", 100).await;
    
    let mut log_rx = bus.subscribe("test", "test:log3").await.unwrap();
    
    let actor = EchoActor::new("test:input3".to_string(), "test:log3".to_string());
    let handle = bus.spawn_actor(actor).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let json_payload = serde_json::json!({"key": "value"});
    let msg = Message::new("test:input3", "source", json_payload);
    bus.publish("test", msg).await.unwrap();
    
    let received = log_rx.recv().await.unwrap();
    assert_eq!(received.source, "sar-echo");
    assert!(received.payload.to_string().contains("echo:"));
    
    handle.stop().await;
}