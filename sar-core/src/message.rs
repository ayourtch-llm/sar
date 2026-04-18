use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UiMessageType {
    UserInput,
    LlmThinking,
    LlmStream,
    LlmStreamEnd,
    LlmToolCall,
    LlmToolResult,
    Echo,
    Reverse,
    Log,
    Info,
    Error,
    Warning,
}

impl fmt::Display for UiMessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiMessageType::UserInput => write!(f, "UserInput"),
            UiMessageType::LlmThinking => write!(f, "LlmThinking"),
            UiMessageType::LlmStream => write!(f, "LlmStream"),
            UiMessageType::LlmStreamEnd => write!(f, "LlmStreamEnd"),
            UiMessageType::LlmToolCall => write!(f, "LlmToolCall"),
            UiMessageType::LlmToolResult => write!(f, "LlmToolResult"),
            UiMessageType::Echo => write!(f, "Echo"),
            UiMessageType::Reverse => write!(f, "Reverse"),
            UiMessageType::Log => write!(f, "Log"),
            UiMessageType::Info => write!(f, "Info"),
            UiMessageType::Error => write!(f, "Error"),
            UiMessageType::Warning => write!(f, "Warning"),
        }
    }
}

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

    pub fn with_type(mut self, item_type: impl Into<String>) -> Self {
        self.meta = serde_json::json!({"type": item_type.into()});
        self
    }

    pub fn with_stream_id(mut self, stream_id: String) -> Self {
        if let serde_json::Value::Object(ref mut obj) = self.meta {
            obj.insert("stream_id".to_string(), serde_json::Value::String(stream_id));
        } else {
            self.meta = serde_json::json!({"stream_id": stream_id});
        }
        self
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} -> {}", self.topic, self.source, self.payload)
    }
}
