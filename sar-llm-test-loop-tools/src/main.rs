use sar_core::SarBus;
use sar_llm_test_loop_tools::calculator::CalculatorTool;
use sar_llm_test_loop_tools::sleep_tool::SleepTool;
use sar_llm_test_loop_tools::tool_actor::ToolActorRunner;
use sar_llm_test_loop_tools::ToolActorWrapper;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let bus = SarBus::new();

    // Create tool execution topics
    bus.create_topic("tool:results", 1000).await;
    bus.create_topic("tool:calculator:execute", 100).await;
    bus.create_topic("tool:calculator:cancel", 100).await;
    bus.create_topic("tool:sleep:execute", 100).await;
    bus.create_topic("tool:sleep:cancel", 100).await;

    // Spawn tool actor runners (independent async actors)
    let calculator_runner = ToolActorRunner::new(ToolActorWrapper::new(CalculatorTool::new()));
    let sleep_runner = ToolActorRunner::new(SleepTool::new());

    let bus_for_calc = bus.clone();
    tokio::spawn(async move {
        if let Err(e) = calculator_runner.run(&bus_for_calc).await {
            eprintln!("Calculator tool actor failed: {}", e);
        }
    });

    let bus_for_sleep = bus.clone();
    tokio::spawn(async move {
        if let Err(e) = sleep_runner.run(&bus_for_sleep).await {
            eprintln!("Sleep tool actor failed: {}", e);
        }
    });

    info!("Tool actors ready. Waiting for Ctrl+C...");
    tokio::signal::ctrl_c().await?;
    Ok(())
}