use async_trait::async_trait;
use serde_json::Value;
use sar_tool_actors::{ToolActor, ToolSyntax};

#[derive(Debug, Clone)]
pub struct CalculatorTool {
    parameters: Value,
}

impl CalculatorTool {
    pub fn new() -> Self {
        Self {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "The mathematical expression to evaluate (e.g., '2 + 2', '3 * (4 + 5)')"
                    }
                },
                "required": ["expression"]
            }),
        }
    }
}

#[async_trait]
impl ToolActor for CalculatorTool {
    fn tool_syntax(&self) -> ToolSyntax {
        ToolSyntax::new(
            "calculator".to_string(),
            "Evaluate mathematical expressions. Supports basic arithmetic (+, -, *, /), parentheses, bitwise operators (&, |, ^, <<, >>), comparisons (==, !=, >, >=, <, <=), modulo (%), string concatenation, and the ternary conditional operator (a ? b : c).".to_string(),
            self.parameters.clone(),
        )
    }

    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String> {
        let expression = arguments["expression"].as_str().ok_or("Missing 'expression' argument")?;

        let result = aycalc::eval(expression)
            .map_err(|e| format!("aycalc error: {:?}", e))?;

        Ok(result.to_string())
    }
}

// Remove the simple evaluator functions below
