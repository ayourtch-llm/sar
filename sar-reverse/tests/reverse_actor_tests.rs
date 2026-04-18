use sar_core::{Actor, Message, SarBus};
use sar_reverse::ReverseActor;

#[tokio::test]
async fn test_reverse_actor_id() {
    let actor = ReverseActor::new("sar:input".to_string(), "sar:log".to_string());
    assert_eq!(actor.id(), "sar-reverse");
}

#[tokio::test]
async fn test_reverse_actor_reverses_text() {
    let bus = SarBus::new();
    bus.create_topic("test:input", 100).await;
    bus.create_topic("test:log", 100).await;
    
    let mut log_rx = bus.subscribe("test:log").await.unwrap();
    
    let actor = ReverseActor::new("test:input".to_string(), "test:log".to_string());
    let handle = bus.spawn_actor(actor).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let msg = Message::text("test:input", "test-source", "hello");
    bus.publish(msg).await.unwrap();
    
    let received = log_rx.recv().await.unwrap();
    assert_eq!(received.source, "sar-reverse");
    assert_eq!(received.payload, serde_json::json!("reverse: olleh"));
    
    handle.stop().await;
}

#[tokio::test]
async fn test_reverse_actor_reverses_longer_text() {
    let bus = SarBus::new();
    bus.create_topic("test:input2", 100).await;
    bus.create_topic("test:log2", 100).await;
    
    let mut log_rx = bus.subscribe("test:log2").await.unwrap();
    
    let actor = ReverseActor::new("test:input2".to_string(), "test:log2".to_string());
    let handle = bus.spawn_actor(actor).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let msg = Message::text("test:input2", "source", "test reverse");
    bus.publish(msg).await.unwrap();
    
    let received = log_rx.recv().await.unwrap();
    assert_eq!(received.payload, serde_json::json!("reverse: esrever tset"));
    
    handle.stop().await;
}

#[tokio::test]
async fn test_reverse_actor_multiple_messages() {
    let bus = SarBus::new();
    bus.create_topic("test:input3", 100).await;
    bus.create_topic("test:log3", 100).await;
    
    let mut log_rx = bus.subscribe("test:log3").await.unwrap();
    
    let actor = ReverseActor::new("test:input3".to_string(), "test:log3".to_string());
    let handle = bus.spawn_actor(actor).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let test_cases = vec![
        ("abc", "cba"),
        ("hello", "olleh"),
        ("world", "dlrow"),
    ];
    
    for (input, _expected) in &test_cases {
        let msg = Message::text("test:input3", "source", *input);
        bus.publish(msg).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    for (input, expected) in &test_cases {
        let received = log_rx.recv().await.unwrap();
        assert_eq!(received.source, "sar-reverse");
        assert_eq!(received.payload, serde_json::json!(format!("reverse: {}", expected)));
    }
    
    handle.stop().await;
}

#[tokio::test]
async fn test_reverse_actor_with_non_string_payload() {
    let bus = SarBus::new();
    bus.create_topic("test:input4", 100).await;
    bus.create_topic("test:log4", 100).await;
    
    let mut log_rx = bus.subscribe("test:log4").await.unwrap();
    
    let actor = ReverseActor::new("test:input4".to_string(), "test:log4".to_string());
    let handle = bus.spawn_actor(actor).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let json_payload = serde_json::json!({"key": "value"});
    let msg = Message::new("test:input4", "source", json_payload);
    bus.publish(msg).await.unwrap();
    
    let received = log_rx.recv().await.unwrap();
    assert_eq!(received.source, "sar-reverse");
    assert!(received.payload.to_string().contains("reverse:"));
    
    handle.stop().await;
}