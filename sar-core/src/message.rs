use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub topic: String,
    pub source: String,
    pub payload: serde_json::Value,
}

impl Message {
    pub fn new(
        topic: impl Into<String>,
        source: impl Into<String>,
        payload: impl Into<serde_json::Value>,
    ) -> Self {
        Self {
            topic: topic.into(),
            source: source.into(),
            payload: payload.into(),
        }
    }

    pub fn text(
        topic: impl Into<String>,
        source: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self::new(topic, source, text.into())
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} -> {}", self.topic, self.source, self.payload)
    }
}
