use axum::{Router, http::{Request, Method, StatusCode}, body::Body};
use axum::routing::{get, post};
use sar_core::SarBus;
use sar_server::{health, list_topics, publish};
use tower::ServiceExt;

async fn setup_test_app() -> (Router, SarBus) {
    let bus = SarBus::new();
    bus.create_topic("test:topic", 100).await;
    
    let app: Router = Router::new()
        .route("/health", get(health))
        .route("/topics", get(list_topics))
        .route("/publish", post(publish))
        .with_state(bus.clone());
    
    (app, bus)
}

#[tokio::test]
async fn test_health_endpoint() {
    let (app, _bus) = setup_test_app().await;
    
    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn test_list_topics_endpoint() {
    let (app, _bus) = setup_test_app().await;
    
    let request = Request::builder()
        .method(Method::GET)
        .uri("/topics")
        .body(Body::empty())
        .unwrap();
    
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_publish_endpoint_success() {
    let (app, bus) = setup_test_app().await;
    
    let mut rx = bus.subscribe("test:topic").await.unwrap();
    
    let request = Request::builder()
        .method(Method::POST)
        .uri("/publish")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&serde_json::json!({
            "topic": "test:topic",
            "source": "test-source",
            "payload": "test message"
        })).unwrap()))
        .unwrap();
    
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["topic"], "test:topic");
    
    let received = rx.recv().await.unwrap();
    assert_eq!(received.payload, serde_json::json!("test message"));
}

#[tokio::test]
async fn test_publish_endpoint_unknown_topic() {
    let (app, _bus) = setup_test_app().await;
    
    let request = Request::builder()
        .method(Method::POST)
        .uri("/publish")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&serde_json::json!({
            "topic": "unknown:topic",
            "source": "test-source",
            "payload": "test message"
        })).unwrap()))
        .unwrap();
    
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["topics"].is_array());
}

#[tokio::test]
async fn test_publish_with_json_payload() {
    let (app, bus) = setup_test_app().await;
    
    let mut rx = bus.subscribe("test:topic").await.unwrap();
    
    let request = Request::builder()
        .method(Method::POST)
        .uri("/publish")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&serde_json::json!({
            "topic": "test:topic",
            "source": "api",
            "payload": {"key": "value", "number": 42}
        })).unwrap()))
        .unwrap();
    
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["success"], true);
    
    let received = rx.recv().await.unwrap();
    assert_eq!(received.payload["key"], "value");
    assert_eq!(received.payload["number"], 42);
}

#[tokio::test]
async fn test_topics_endpoint_returns_all_topics() {
    let (app, bus) = setup_test_app().await;
    
    bus.create_topic("alpha", 10).await;
    bus.create_topic("beta", 20).await;
    bus.create_topic("gamma", 30).await;
    
    let request = Request::builder()
        .method(Method::GET)
        .uri("/topics")
        .body(Body::empty())
        .unwrap();
    
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(json.len(), 4);
    
    let names: Vec<String> = json.iter()
        .map(|v| v["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"alpha".to_string()));
    assert!(names.contains(&"beta".to_string()));
    assert!(names.contains(&"gamma".to_string()));
}