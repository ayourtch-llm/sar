use sar_core::{Message, SarBus};
use sar_llm::{LlmActor, LlmRequest};

async fn setup_test_actors(mock_url: &str) -> (
    SarBus,
    tokio::sync::broadcast::Receiver<Message>,
    tokio::sync::broadcast::Receiver<Message>,
) {
    let bus = SarBus::new();
    bus.create_topic("test:llm:in", 100).await;
    bus.create_topic("test:llm:out", 1000).await;
    bus.create_topic("test:llm:stream", 1000).await;

    let out_rx = bus.subscribe("test:llm:out").await.unwrap();
    let stream_rx = bus.subscribe("test:llm:stream").await.unwrap();

    let actor = LlmActor::new(
        0,
        "test:llm:in".to_string(),
        "test:llm:out".to_string(),
        "test:llm:stream".to_string(),
        sar_core::config::LlmConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: mock_url.to_string(),
            api_key: "sk-test".to_string(),
            temperature: 0.7,
            max_tokens: 2048,
        },
    );

    let _handle = bus.spawn_actor(actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (bus, out_rx, stream_rx)
}

#[tokio::test]
async fn test_llm_actor_simple_response() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("Hello from mock!"))
        .build()
        .await;

    let (bus, mut out_rx, _stream_rx) = setup_test_actors(&mock.url()).await;

    let request = LlmRequest {
        prompt: "Hello".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish(msg).await.unwrap();

    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        out_rx.recv()
    ).await.unwrap().unwrap();
    
    assert_eq!(response.source, "sar-llm-0");
    assert_eq!(response.payload, serde_json::json!("Hello from mock!"));
}

#[tokio::test]
async fn test_llm_actor_config_override() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("Overridden!"))
        .build()
        .await;

    let (bus, mut out_rx, _stream_rx) = setup_test_actors(&mock.url()).await;

    let request = LlmRequest {
        prompt: "Test override".to_string(),
        config: Some(sar_core::config::LlmConfig {
            model: "custom-model".to_string(),
            base_url: mock.url(),
            api_key: "sk-custom".to_string(),
            temperature: 1.0,
            max_tokens: 100,
        }),
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish(msg).await.unwrap();

    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        out_rx.recv()
    ).await.unwrap().unwrap();
    
    assert_eq!(response.payload, serde_json::json!("Overridden!"));

    assert_eq!(mock.request_count(), 1);
    let request_body = mock.request(0).unwrap();
    assert_eq!(request_body["model"].as_str().unwrap(), "custom-model");
}

#[tokio::test]
async fn test_llm_actor_multiple_requests() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("First"))
        .next(mockllm::Response::text("Second"))
        .next(mockllm::Response::text("Third"))
        .build()
        .await;

    let (bus, mut out_rx, _stream_rx) = setup_test_actors(&mock.url()).await;

    for i in 1..=3 {
        let request = LlmRequest {
            prompt: format!("Request {}", i),
            config: None,
        };

        let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
        bus.publish(msg).await.unwrap();
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    for expected in ["First", "Second", "Third"] {
        let response = tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            out_rx.recv()
        ).await.unwrap().unwrap();
        assert_eq!(response.payload, serde_json::json!(expected));
    }

    assert_eq!(mock.request_count(), 3);
}

#[tokio::test]
async fn test_llm_actor_echo_fallback() {
    let mock = mockllm::MockLlmServer::builder()
        .fallback_echo()
        .build()
        .await;

    let (bus, mut out_rx, _stream_rx) = setup_test_actors(&mock.url()).await;

    let request = LlmRequest {
        prompt: "Echo this text".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish(msg).await.unwrap();

    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        out_rx.recv()
    ).await.unwrap().unwrap();
    
    // Echo fallback returns the full request body as JSON
    let response_str = response.payload.as_str().unwrap();
    assert!(response_str.contains("Echo this text"));
    assert!(response_str.contains("messages"));
}

#[tokio::test]
async fn test_llm_actor_verify_request_messages() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("Verified"))
        .build()
        .await;

    let (bus, mut out_rx, _stream_rx) = setup_test_actors(&mock.url()).await;

    let request = LlmRequest {
        prompt: "Verify messages".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish(msg).await.unwrap();

    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        out_rx.recv()
    ).await.unwrap().unwrap();
    
    assert_eq!(response.payload, serde_json::json!("Verified"));

    let messages = mock.request_messages(0);
    assert!(!messages.is_empty());
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "Verify messages");
}

#[tokio::test]
async fn test_llm_actor_config_defaults_used() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("Default config"))
        .build()
        .await;

    let (bus, mut out_rx, _stream_rx) = setup_test_actors(&mock.url()).await;

    let request = LlmRequest {
        prompt: "Use defaults".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish(msg).await.unwrap();

    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        out_rx.recv()
    ).await.unwrap().unwrap();
    
    assert_eq!(response.payload, serde_json::json!("Default config"));

    let request_body = mock.request(0).unwrap();
    assert_eq!(request_body["model"].as_str().unwrap(), "gpt-4o-mini");
}