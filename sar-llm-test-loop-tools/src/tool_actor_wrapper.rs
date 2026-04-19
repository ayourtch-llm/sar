use serde_json::Value;

use crate::Tool;
use sar_tool_actors::{ToolActor, ToolSyntax};

/// Wraps a `Tool` implementation to make it compatible with `ToolActor`.
///
/// This allows existing `Tool` implementations (like `CalculatorTool`) to be used
/// with `LlmTestLoopToolsActor` which now requires `ToolActor`.
pub struct ToolActorWrapper<T: Tool> {
    tool: T,
}

impl<T: Tool> ToolActorWrapper<T> {
    pub fn new(tool: T) -> Self {
        Self { tool }
    }
}

#[async_trait::async_trait]
impl<T: Tool + Send + Sync> ToolActor for ToolActorWrapper<T> {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            self.tool.name().to_string(),
            self.tool.description().to_string(),
            self.tool.parameters().clone(),
        )
    }

    async fn execute_tool(&self, arguments: &Value) -> Result<String, String> {
        self.tool.execute(arguments).await
    }
}