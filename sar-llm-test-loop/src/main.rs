use sar_llm_test_loop::LlmTestLoopActor;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let actor = LlmTestLoopActor::new(
        0,
        "llm-test-loop:0:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm-test-loop:0:stream".to_string(),
    );
    
    println!("LLM Test Loop actor ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}