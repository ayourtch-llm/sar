use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use crate::tool_actor::{ToolActor, ToolSyntax};

#[derive(Debug, Clone)]
pub struct SleepTool {
    parameters: Value,
}

impl SleepTool {
    pub fn new() -> Self {
        Self {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "duration_ms": {
                        "type": "integer",
                        "description": "Duration in milliseconds to sleep"
                    }
                },
                "required": ["duration_ms"]
            }),
        }
    }
}

#[async_trait]
impl ToolActor for SleepTool {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            "sleep".to_string(),
            "Sleep for a specified duration in milliseconds. Useful for testing tool cancellation.".to_string(),
            self.parameters.clone(),
        )
    }

    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String> {
        let duration_ms = arguments["duration_ms"].as_u64().ok_or("Missing 'duration_ms' argument")?;

        if duration_ms == 0 {
            return Ok("Slept for 0ms".to_string());
        }

        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        Ok(format!("Slept for {}ms", duration_ms))
    }

    fn supports_cancel(&self) -> bool {
        true
    }
}