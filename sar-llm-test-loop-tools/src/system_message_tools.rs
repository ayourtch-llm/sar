use async_trait::async_trait;
use sar_tool_actors::{ToolActor, ToolSyntax};
use std::sync::{Arc, Mutex as StdMutex};

/// Internal tool to read the current system message
pub struct ReadSystemMessageTool {
    message: Arc<StdMutex<String>>,
}

impl ReadSystemMessageTool {
    pub fn new(message: Arc<StdMutex<String>>) -> Self {
        Self { message }
    }
}

#[async_trait]
impl ToolActor for ReadSystemMessageTool {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            "read_system_message".to_string(),
            "Read the current system message used by the LLM. Returns the full system message text.".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        )
    }

    async fn execute_tool(&self, _arguments: &serde_json::Value) -> Result<String, String> {
        let message = self.message.lock().map_err(|e| format!("Failed to lock message: {}", e))?;
        Ok(message.to_string())
    }
}

/// Internal tool to write/update the system message
pub struct WriteSystemMessageTool {
    message: Arc<StdMutex<String>>,
}

impl WriteSystemMessageTool {
    pub fn new(message: Arc<StdMutex<String>>) -> Self {
        Self { message }
    }
}

#[async_trait]
impl ToolActor for WriteSystemMessageTool {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            "write_system_message".to_string(),
            "Write or update the system message used by the LLM. The new system message will be effective for subsequent conversation turns.".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The new system message text"
                    }
                },
                "required": ["message"]
            }),
        )
    }

    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String> {
        let new_message = arguments.get("message")
            .and_then(|m| m.as_str())
            .ok_or("Missing 'message' argument")?
            .to_string();

        let mut message = self.message.lock().map_err(|e| format!("Failed to lock message: {}", e))?;
        *message = new_message.clone();
        Ok(format!("System message updated successfully."))
    }
}