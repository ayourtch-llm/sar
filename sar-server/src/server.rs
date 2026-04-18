use axum::{
    extract::State,
    http::StatusCode,
    Json,
    routing::{get, post},
    Router,
};
use sar_core::bus::SarBus;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tracing::{info, error};

#[derive(Debug, Serialize)]
pub struct TopicInfo {
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct PublishRequest {
    topic: String,
    source: String,
    payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct PublishResponse {
    success: bool,
    topic: String,
}

#[derive(Debug, Serialize)]
pub struct ServerState {
    topics: Vec<String>,
}

pub async fn list_topics(State(bus): State<SarBus>) -> Json<Vec<TopicInfo>> {
    let topics = bus.list_topics().await;
    Json(topics.into_iter().map(|t| TopicInfo { name: t }).collect())
}

pub async fn publish(
    State(bus): State<SarBus>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<PublishResponse>, (StatusCode, Json<ServerState>)> {
    let msg = sar_core::message::Message::new(&req.topic, req.source, req.payload);
    match bus.publish("sar-server", msg).await {
        Ok(()) => {
            info!("Published to topic '{}'", req.topic);
            Ok(Json(PublishResponse {
                success: true,
                topic: req.topic,
            }))
        }
        Err(e) => {
            error!("Failed to publish: {}", e);
            let topics = bus.list_topics().await;
            Err((
                StatusCode::BAD_REQUEST,
                Json(ServerState {
                    topics: topics.iter().map(|t| t.clone()).collect(),
                }),
            ))
        }
    }
}

pub async fn health() -> &'static str {
    "ok"
}

pub async fn run_server(bus: SarBus, host: String, port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/topics", get(list_topics))
        .route("/publish", post(publish))
        .route("/health", get(health))
        .with_state(bus)
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("Server listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}