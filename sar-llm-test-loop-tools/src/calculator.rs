use crate::Tool;
use async_trait::async_trait;
use serde_json::Value;

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
                        "description": "The mathematical expression to evaluate"
                    }
                },
                "required": ["expression"]
            }),
        }
    }
}

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculator"
    }

    fn description(&self) -> &str {
        "Evaluate mathematical expressions. Supports basic arithmetic (+, -, *, /), parentheses, bitwise operators (&, |, ^, <<, >>), comparisons (==, !=, >, >=, <, <=), modulo (%), string concatenation, and the ternary conditional operator (a ? b : c)."
    }

    fn parameters(&self) -> &Value {
        &self.parameters
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let expression = arguments
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'expression' argument")?;

        let result = aycalc::eval(expression)
            .map_err(|e| format!("aycalc error: {:?}", e))?;

        Ok(result.to_string())
    }
}