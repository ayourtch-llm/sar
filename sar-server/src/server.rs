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
pub struct ActorInfoResponse {
    pub id: String,
    pub subscriptions: Vec<String>,
    pub publications: Vec<String>,
    pub announced: bool,
}

#[derive(Debug, Serialize)]
pub struct TopicAnnouncementResponse {
    pub name: String,
    pub subscribers: Vec<String>,
    pub publishers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TopicInfo {
    pub name: String,
    pub subscribers: Vec<String>,
    pub publishers: Vec<String>,
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
    let announced = bus.list_announced_topics().await;
    let runtime = bus.list_topic_info().await;
    let runtime_map: std::collections::HashMap<String, _> = runtime.into_iter().map(|t| (t.name.clone(), t)).collect();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for t in &announced {
        seen.insert(t.name.clone());
        let mut subs = t.subscribers.clone();
        let mut pubs = t.publishers.clone();
        if let Some(rt) = runtime_map.get(&t.name) {
            for s in &rt.subscribers {
                if !subs.contains(s) { subs.push(s.clone()); }
            }
            for p in &rt.publishers {
                if !pubs.contains(p) { pubs.push(p.clone()); }
            }
        }
        result.push(TopicInfo {
            name: t.name.clone(),
            subscribers: subs,
            publishers: pubs,
        });
    }
    for (name, t) in runtime_map {
        if !seen.contains(&name) {
            result.push(TopicInfo {
                name,
                subscribers: t.subscribers,
                publishers: t.publishers,
            });
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Json(result)
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

pub async fn list_actors(State(bus): State<SarBus>) -> Json<Vec<ActorInfoResponse>> {
    let announced = bus.list_announced_actors().await;
    let announced_map: std::collections::HashMap<String, _> = announced.into_iter().map(|a| (a.id.clone(), a)).collect();
    let actors = bus.list_actors().await;
    Json(actors.into_iter().map(|a| {
        let id = a.id.clone();
        let ann = announced_map.get(&id);
        let (subscriptions, publications) = if let Some(a) = ann {
            (a.subscriptions.clone(), a.publications.clone())
        } else {
            (a.subscriptions.clone(), a.publications.clone())
        };
        ActorInfoResponse {
            id: a.id,
            subscriptions,
            publications,
            announced: ann.is_some(),
        }
    }).collect())
}

pub async fn list_announced_actors(State(bus): State<SarBus>) -> Json<Vec<ActorInfoResponse>> {
    let announced = bus.list_announced_actors().await;
    Json(announced.into_iter().map(|a| ActorInfoResponse {
        id: a.id,
        subscriptions: a.subscriptions,
        publications: a.publications,
        announced: true,
    }).collect())
}

pub async fn list_announced_topics(State(bus): State<SarBus>) -> Json<Vec<TopicAnnouncementResponse>> {
    let topics = bus.list_announced_topics().await;
    Json(topics.into_iter().map(|t| TopicAnnouncementResponse {
        name: t.name,
        subscribers: t.subscribers,
        publishers: t.publishers,
    }).collect())
}

pub async fn run_server(bus: SarBus, host: String, port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/api/topics", get(list_topics))
        .route("/api/actors", get(list_actors))
        .route("/api/announced", get(list_announced_actors))
        .route("/api/announced-topics", get(list_announced_topics))
        .route("/api/publish", post(publish))
        .route("/health", get(health))
        .with_state(bus)
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("Server listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}