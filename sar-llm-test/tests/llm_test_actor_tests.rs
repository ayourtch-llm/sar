use sar_core::{Message, SarBus};
use sar_llm::LlmActor;
use sar_llm_test::LlmTestActor;

#[tokio::test]
async fn test_llm_test_actor_wraps_in_xml() {
    let actor = LlmTestActor::new(
        0,
        "test:input".to_string(),
        "test:llm:in".to_string(),
        "test:llm:out".to_string(),
        "test:llm:stream".to_string(),
    );

    let xml = actor.wrap_in_xml("Hello, world!");
    assert!(xml.contains("<llm-request>"));
    assert!(xml.contains("<prompt>Hello, world!</prompt>"));
    assert!(xml.contains("</llm-request>"));
}

#[tokio::test]
async fn test_llm_test_actor_escapes_xml_special_chars() {
    let actor = LlmTestActor::new(
        0,
        "test:input".to_string(),
        "test:llm:in".to_string(),
        "test:llm:out".to_string(),
        "test:llm:stream".to_string(),
    );

    let xml = actor.wrap_in_xml("Test <tag> & \"quotes\"");
    assert!(xml.contains("&lt;tag&gt;"));
    assert!(xml.contains("&amp;"));
    assert!(xml.contains("&quot;quotes&quot;") || xml.contains("\"quotes\""));
}

#[tokio::test]
async fn test_llm_test_actor_publishes_to_llm() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("LLM response"))
        .build()
        .await;

    let bus = SarBus::new();
    bus.create_topic("test:input", 100).await;
    bus.create_topic("test:llm:in", 100).await;
    bus.create_topic("test:llm:out", 1000).await;
    bus.create_topic("test:llm:stream", 1000).await;

    let llm_actor = LlmActor::new(
        0,
        "test:llm:in".to_string(),
        "test:llm:out".to_string(),
        "test:llm:stream".to_string(),
        sar_core::config::LlmConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: mock.url(),
            api_key: "sk-test".to_string(),
            temperature: 0.7,
            max_tokens: 2048,
        },
    );

    let _llm_handle = bus.spawn_actor(llm_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let test_actor = LlmTestActor::new(
        0,
        "test:input".to_string(),
        "test:llm:in".to_string(),
        "test:llm:out".to_string(),
        "test:llm:stream".to_string(),
    );

    let _test_handle = bus.spawn_actor(test_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let msg = Message::text("test:input", "test-user", "Hello, world!");
    bus.publish(msg).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    assert_eq!(mock.request_count(), 1);
    let request_body = mock.request(0).unwrap();
    assert!(request_body["messages"][0]["content"].is_string());
    let content = request_body["messages"][0]["content"].as_str().unwrap();
    assert!(content.contains("<llm-request>"));
    assert!(content.contains("<prompt>Hello, world!</prompt>"));
}

#[tokio::test]
async fn test_llm_test_actor_receives_output() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("Test response"))
        .build()
        .await;

    let bus = SarBus::new();
    bus.create_topic("test:input2", 100).await;
    bus.create_topic("test:llm:in2", 100).await;
    bus.create_topic("test:llm:out2", 1000).await;
    bus.create_topic("test:llm:stream2", 1000).await;

    let mut out_rx = bus.subscribe("test:llm:out2").await.unwrap();

    let llm_actor = LlmActor::new(
        0,
        "test:llm:in2".to_string(),
        "test:llm:out2".to_string(),
        "test:llm:stream2".to_string(),
        sar_core::config::LlmConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: mock.url(),
            api_key: "sk-test".to_string(),
            temperature: 0.7,
            max_tokens: 2048,
        },
    );

    let _llm_handle = bus.spawn_actor(llm_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let test_actor = LlmTestActor::new(
        0,
        "test:input2".to_string(),
        "test:llm:in2".to_string(),
        "test:llm:out2".to_string(),
        "test:llm:stream2".to_string(),
    );

    let _test_handle = bus.spawn_actor(test_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let msg = Message::text("test:input2", "test-user", "Test input");
    bus.publish(msg).await.unwrap();

    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        out_rx.recv()
    ).await.unwrap().unwrap();

    assert_eq!(response.source, "sar-llm-0");
    assert_eq!(response.payload, serde_json::json!("Test response"));
}

#[tokio::test]
async fn test_llm_test_actor_receives_stream() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("Streamed text"))
        .build()
        .await;

    let bus = SarBus::new();
    bus.create_topic("test:input3", 100).await;
    bus.create_topic("test:llm:in3", 100).await;
    bus.create_topic("test:llm:out3", 1000).await;
    bus.create_topic("test:llm:stream3", 1000).await;

    let mut stream_rx = bus.subscribe("test:llm:stream3").await.unwrap();

    let llm_actor = LlmActor::new(
        0,
        "test:llm:in3".to_string(),
        "test:llm:out3".to_string(),
        "test:llm:stream3".to_string(),
        sar_core::config::LlmConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: mock.url(),
            api_key: "sk-test".to_string(),
            temperature: 0.7,
            max_tokens: 2048,
        },
    );

    let _llm_handle = bus.spawn_actor(llm_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let test_actor = LlmTestActor::new(
        0,
        "test:input3".to_string(),
        "test:llm:in3".to_string(),
        "test:llm:out3".to_string(),
        "test:llm:stream3".to_string(),
    );

    let _test_handle = bus.spawn_actor(test_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let msg = Message::text("test:input3", "test-user", "Stream test");
    bus.publish(msg).await.unwrap();

    let mut full_stream = String::new();
    loop {
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            stream_rx.recv()
        ).await {
            Ok(Ok(chunk_msg)) => {
                if let Some(text) = chunk_msg.payload.as_str() {
                    full_stream.push_str(text);
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                continue;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                break;
            }
            Err(_) => {
                break;
            }
        }
    }

    assert!(!full_stream.is_empty());
    assert!(full_stream.contains("Streamed"));
}

#[tokio::test]
async fn test_llm_test_actor_multiple_requests() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("First"))
        .next(mockllm::Response::text("Second"))
        .next(mockllm::Response::text("Third"))
        .build()
        .await;

    let bus = SarBus::new();
    bus.create_topic("test:input4", 100).await;
    bus.create_topic("test:llm:in4", 100).await;
    bus.create_topic("test:llm:out4", 1000).await;
    bus.create_topic("test:llm:stream4", 1000).await;

    let mut out_rx = bus.subscribe("test:llm:out4").await.unwrap();

    let llm_actor = LlmActor::new(
        0,
        "test:llm:in4".to_string(),
        "test:llm:out4".to_string(),
        "test:llm:stream4".to_string(),
        sar_core::config::LlmConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: mock.url(),
            api_key: "sk-test".to_string(),
            temperature: 0.7,
            max_tokens: 2048,
        },
    );

    let _llm_handle = bus.spawn_actor(llm_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let test_actor = LlmTestActor::new(
        0,
        "test:input4".to_string(),
        "test:llm:in4".to_string(),
        "test:llm:out4".to_string(),
        "test:llm:stream4".to_string(),
    );

    let _test_handle = bus.spawn_actor(test_actor).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    for i in 1..=3 {
        let msg = Message::text("test:input4", "test-user", format!("Request {}", i));
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