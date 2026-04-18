use crate::Tool;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct CalculatorTool {
    pub base_url: String,
    parameters: Value,
}

impl CalculatorTool {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
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
        "Evaluate mathematical expressions. Supports basic arithmetic (+, -, *, /), parentheses, and common functions like sin, cos, tan, sqrt, log, exp, pow."
    }

    fn parameters(&self) -> &Value {
        &self.parameters
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let expression = arguments
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'expression' argument")?;

        let client = reqwest::Client::new();
        let url = format!("{}/evaluate", self.base_url);

        let response = client
            .post(&url)
            .json(&serde_json::json!({ "expression": expression }))
            .send()
            .await
            .map_err(|e| format!("Failed to send request to aycalc: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("aycalc error {}: {}", status, body));
        }

        let result = response
            .json::<Value>()
            .await
            .map_err(|e| format!("Failed to parse aycalc response: {}", e))?;

        Ok(result
            .get("result")
            .or_else(|| result.get("answer"))
            .map(|v| v.to_string())
            .unwrap_or_else(|| result.to_string()))
    }
}