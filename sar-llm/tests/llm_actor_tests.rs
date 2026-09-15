use sar_core::{Message, SarBus};
use sar_llm::{LlmActor, LlmRequest};

async fn setup_test_actors(mock_url: &str) -> (
    SarBus,
    tokio::sync::broadcast::Receiver<Message>,
    tokio::sync::broadcast::Receiver<Message>,
) {
    let bus = SarBus::new();
    bus.register_announcement(sar_core::actor::ActorAnnouncement { id: "test".to_string(), subscriptions: vec![], publications: vec![] }).await;
    bus.create_topic("test:llm:in", 100).await;
    bus.create_topic("test:llm:out", 1000).await;
    bus.create_topic("test:llm:stream", 1000).await;

    let out_rx = bus.subscribe("test", "test:llm:out").await.unwrap();
    let stream_rx = bus.subscribe("test", "test:llm:stream").await.unwrap();

    let actor = LlmActor::new(
        0,
        "test:llm:in".to_string(),
        "test:llm:out".to_string(),
        "test:llm:stream".to_string(),
        "test:llm:stats".to_string(),
        "test:llm:tool_calls".to_string(),
        "test:llm:control".to_string(),
        sar_core::config::LlmConfig {
            model: "gpt-4o-mini".to_string(),
            base_url: mock_url.to_string(),
            api_key: "sk-test".to_string(),
            temperature: 0.7,
            max_tokens: 65536,
            ..Default::default()
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
        messages: None,
        tools: None,
        grammar: None,
        prompt: "Hello".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish("test", msg).await.unwrap();

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
        messages: None,
        tools: None,
        grammar: None,
        prompt: "Test override".to_string(),
        config: Some(sar_core::config::LlmConfig {
            model: "custom-model".to_string(),
            base_url: mock.url(),
            api_key: "sk-custom".to_string(),
            temperature: 1.0,
            max_tokens: 100,
            ..Default::default()
        }),
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish("test", msg).await.unwrap();

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
            messages: None,
            tools: None,
            grammar: None,
            prompt: format!("Request {}", i),
            config: None,
        };

        let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
        bus.publish("test", msg).await.unwrap();
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
        messages: None,
        tools: None,
        grammar: None,
        prompt: "Echo this text".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish("test", msg).await.unwrap();

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
        messages: None,
        tools: None,
        grammar: None,
        prompt: "Verify messages".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish("test", msg).await.unwrap();

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
        messages: None,
        tools: None,
        grammar: None,
        prompt: "Use defaults".to_string(),
        config: None,
    };

    let msg = Message::new("test:llm:in", "test-source", serde_json::to_value(&request).unwrap());
    bus.publish("test", msg).await.unwrap();

    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        out_rx.recv()
    ).await.unwrap().unwrap();
    
    assert_eq!(response.payload, serde_json::json!("Default config"));

    let request_body = mock.request(0).unwrap();
    assert_eq!(request_body["model"].as_str().unwrap(), "gpt-4o-mini");
}

// Raw HTTP chunks let these regressions split an SSE line inside a UTF-8
// character and keep a request in flight while control messages arrive.
async fn chunked_server() -> (
    String,
    tokio::sync::mpsc::Sender<Vec<u8>>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        socket.set_nodelay(true).unwrap();
        let mut socket = BufReader::new(socket);
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            assert_ne!(socket.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse::<usize>().unwrap();
            }
        }
        socket
            .read_exact(&mut vec![0; content_length])
            .await
            .unwrap();
        let socket = socket.get_mut();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
        while let Some(chunk) = rx.recv().await {
            socket
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(&chunk).await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
        }
        socket.write_all(b"0\r\n\r\n").await.unwrap();
    });
    (url, tx, task)
}

async fn request_test_response(bus: &SarBus) {
    bus.publish(
        "test",
        Message::new("test:llm:in", "test", serde_json::json!({"prompt": "test"})),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_sse_split_utf8_lines_and_tool_arguments() {
    let (url, chunks, server) = chunked_server().await;
    let (bus, mut out_rx, mut stream_rx) = setup_test_actors(&url).await;
    let mut tool_rx = bus.subscribe("test", "test:llm:tool_calls").await.unwrap();
    let mut stats_rx = bus.subscribe("test", "test:llm:stats").await.unwrap();
    request_test_response(&bus).await;

    let events = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hé🙂\"}}]}\r\n\r\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"search\",\"arguments\":\"{\\\"q\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"café\\\"}\"}}]}}]}\n\n",
        "data: [DONE]\n\n",
        // A final unterminated line must not be consumed.
        "data: {\"choices\":[{\"delta\":{\"content\":\"discard\"}}]}"
    );
    let utf8_split = events.find('é').unwrap() + 1;
    for part in [
        &events.as_bytes()[..utf8_split],
        &events.as_bytes()[utf8_split..utf8_split + 4],
        &events.as_bytes()[utf8_split + 4..],
    ] {
        chunks.send(part.to_vec()).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
    drop(chunks);
    server.await.unwrap();

    let calls = tokio::time::timeout(tokio::time::Duration::from_secs(2), async {
        loop {
            let msg = tool_rx.recv().await.unwrap();
            if msg.payload.is_array() {
                break msg.payload;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(calls[0]["id"], "call_1");
    assert_eq!(calls[0]["function"]["name"], "search");
    assert_eq!(calls[0]["function"]["arguments"], r#"{"q":"café"}"#);
    assert_eq!(stats_rx.recv().await.unwrap().payload["rx_chars"], 3);
    let mut content = String::new();
    while let Ok(msg) = stream_rx.try_recv() {
        if msg.meta["type"] == "LlmStream" {
            content.push_str(msg.payload.as_str().unwrap());
        }
    }
    assert_eq!(content, "hé🙂");
    assert!(out_rx.try_recv().is_err());
}

#[tokio::test]
async fn test_unrelated_control_preserves_active_request() {
    let (url, chunks, server) = chunked_server().await;
    let (bus, mut out_rx, mut stream_rx) = setup_test_actors(&url).await;
    request_test_response(&bus).await;
    chunks
        .send(b"data: {\"choices\":[{\"delta\":{\"content\":\"started\"}}]}\n\n".to_vec())
        .await
        .unwrap();
    tokio::time::timeout(tokio::time::Duration::from_secs(2), stream_rx.recv())
        .await
        .unwrap()
        .unwrap();
    bus.publish(
        "test",
        Message::new(
            "test:llm:control",
            "test",
            serde_json::json!({"type": "unrelated"}),
        ),
    )
    .await
    .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    drop(chunks);
    server.await.unwrap();
    let response = tokio::time::timeout(tokio::time::Duration::from_secs(2), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.payload, "started");
}

#[tokio::test]
async fn test_idle_interrupt_does_not_publish_a_turn() {
    let mock = mockllm::MockLlmServer::builder()
        .next(mockllm::Response::text("real response"))
        .build()
        .await;
    let (bus, mut out_rx, _) = setup_test_actors(&mock.url()).await;
    bus.publish(
        "test",
        Message::new(
            "test:llm:control",
            "test",
            serde_json::json!({"type": "interrupt"}),
        ),
    )
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(tokio::time::Duration::from_millis(50), out_rx.recv())
            .await
            .is_err()
    );
    request_test_response(&bus).await;
    let response = tokio::time::timeout(tokio::time::Duration::from_secs(2), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.payload, "real response");
}
