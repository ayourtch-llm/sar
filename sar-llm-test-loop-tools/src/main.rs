use sar_llm_test_loop_tools::LlmTestLoopToolsActor;
use sar_llm_test_loop_tools::calculator::CalculatorTool;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let actor = LlmTestLoopToolsActor::new(
        0,
        "llm-test-tools:0:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm-test-tools:0:stream".to_string(),
    )
    .with_tool(CalculatorTool::new());

    println!("LLM Test Loop Tools actor ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}