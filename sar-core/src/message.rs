use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LogItemType {
    UserInput,
    System,
    LlmStream,
    LlmResponse,
    Log,
    Info,
    Error,
    Warning,
}

impl Default for LogItemType {
    fn default() -> Self {
        LogItemType::Info
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub reference: String,
    pub topic: String,
    pub source: String,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub meta: serde_json::Value,
}

impl Message {
    pub fn new(
        topic: impl Into<String>,
        source: impl Into<String>,
        payload: impl Into<serde_json::Value>,
    ) -> Self {
        Self {
            reference: uuid::Uuid::new_v4().to_string(),
            topic: topic.into(),
            source: source.into(),
            payload: payload.into(),
            meta: serde_json::Value::Null,
        }
    }

    pub fn with_reference(
        reference: String,
        topic: impl Into<String>,
        source: impl Into<String>,
        payload: impl Into<serde_json::Value>,
    ) -> Self {
        Self {
            reference,
            topic: topic.into(),
            source: source.into(),
            payload: payload.into(),
            meta: serde_json::Value::Null,
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
