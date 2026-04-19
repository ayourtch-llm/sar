use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Structured representation of a tool's OpenAI-compatible function definition.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolSyntax {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSyntax {
    pub fn new(
        name: String,
        description: String,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name,
            description,
            parameters,
        }
    }

    /// Serializes to OpenAI-compatible function definition JSON.
    pub fn to_openai_json(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

/// Trait for actors that expose tool capabilities.
///
/// This trait is object-safe (dyn compatible), allowing tools to be stored
/// as `Arc<dyn ToolActor>` in the `LlmTestLoopToolsActor`.
///
/// - `tool_syntax()` returns the tool's OpenAI-compatible definition (called synchronously).
/// - `execute_tool()` runs the tool logic (called asynchronously).
#[async_trait]
pub trait ToolActor: Send + Sync {
    fn tool_syntax(&self) -> ToolSyntax;
    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String>;
}