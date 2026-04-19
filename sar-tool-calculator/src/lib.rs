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
            "Evaluate a mathematical expression and return the result. Supports basic arithmetic: +, -, *, /, and parentheses.".to_string(),
            self.parameters.clone(),
        )
    }

    async fn execute_tool(&self, arguments: &serde_json::Value) -> Result<String, String> {
        let expression = arguments["expression"].as_str().ok_or("Missing 'expression' argument")?;

        let result = evaluate_expression(expression)?;
        Ok(result)
    }
}

fn evaluate_expression(expr: &str) -> Result<String, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("Empty expression".to_string());
    }

    // Simple expression evaluator
    let result = parse_expression(expr, &mut 0)?;
    Ok(result.to_string())
}

fn parse_expression(expr: &str, pos: &mut usize) -> Result<f64, String> {
    let mut result = parse_term(expr, pos)?;

    while *pos < expr.len() {
        let ch = expr.chars().nth(*pos).unwrap();
        if ch == '+' {
            *pos += 1;
            result += parse_term(expr, pos)?;
        } else if ch == '-' {
            *pos += 1;
            result -= parse_term(expr, pos)?;
        } else {
            break;
        }
    }

    Ok(result)
}

fn parse_term(expr: &str, pos: &mut usize) -> Result<f64, String> {
    let mut result = parse_factor(expr, pos)?;

    while *pos < expr.len() {
        let ch = expr.chars().nth(*pos).unwrap();
        if ch == '*' {
            *pos += 1;
            result *= parse_factor(expr, pos)?;
        } else if ch == '/' {
            *pos += 1;
            let divisor = parse_factor(expr, pos)?;
            if divisor == 0.0 {
                return Err("Division by zero".to_string());
            }
            result /= divisor;
        } else {
            break;
        }
    }

    Ok(result)
}

fn parse_factor(expr: &str, pos: &mut usize) -> Result<f64, String> {
    // Skip whitespace
    while *pos < expr.len() && expr.chars().nth(*pos).unwrap().is_whitespace() {
        *pos += 1;
    }

    if *pos >= expr.len() {
        return Err("Unexpected end of expression".to_string());
    }

    let ch = expr.chars().nth(*pos).unwrap();

    if ch == '-' {
        *pos += 1;
        let value = parse_factor(expr, pos)?;
        Ok(-value)
    } else if ch == '(' {
        *pos += 1;
        let result = parse_expression(expr, pos)?;
        // Skip whitespace
        while *pos < expr.len() && expr.chars().nth(*pos).unwrap().is_whitespace() {
            *pos += 1;
        }
        if *pos >= expr.len() || expr.chars().nth(*pos).unwrap() != ')' {
            return Err("Missing closing parenthesis".to_string());
        }
        *pos += 1;
        Ok(result)
    } else if ch.is_ascii_digit() || ch == '.' {
        let start = *pos;
        while *pos < expr.len() && (expr.chars().nth(*pos).unwrap().is_ascii_digit() || expr.chars().nth(*pos).unwrap() == '.') {
            *pos += 1;
        }
        let num_str: String = expr.chars().skip(start).take(*pos - start).collect();
        num_str.parse::<f64>().map_err(|e| format!("Invalid number '{}': {}", num_str, e))
    } else {
        Err(format!("Unexpected character '{}'", ch))
    }
}