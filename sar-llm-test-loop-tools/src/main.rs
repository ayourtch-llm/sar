use sar_core::SarBus;
use sar_core::config::Config;
use sar_llm_test_loop_tools::{LlmTestLoopToolsActor, ReadSystemMessageTool, ToolActorRunner, WriteSystemMessageTool};
use sar_tool_calculator::CalculatorTool;
use sar_tool_sleep::SleepTool;
use std::sync::{Arc, Mutex};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    // Load config
    let config = Config::from_file(std::path::Path::new("config.toml"))?;

    let bus = SarBus::new();

    // Create tool execution topics
    bus.create_topic("tool:results", 1000).await;
    bus.create_topic("tool:calculator:execute", 100).await;
    bus.create_topic("tool:sleep:execute", 100).await;
    bus.create_topic("tool:read_system_message:execute", 100).await;
    bus.create_topic("user:control", 100).await;

    // Create system message storage
    let system_message = Arc::new(Mutex::new(config.system_message.message.clone()));

    // Spawn tool actor runners (independent async actors)
    let calculator_runner = ToolActorRunner::new(CalculatorTool::new());
    let sleep_runner = ToolActorRunner::new(SleepTool::new());
    let read_system_message_runner = ToolActorRunner::new(ReadSystemMessageTool::new(system_message.clone()));

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

    let bus_for_read_sys_msg = bus.clone();
    tokio::spawn(async move {
        if let Err(e) = read_system_message_runner.run(&bus_for_read_sys_msg).await {
            eprintln!("Read system message tool actor failed: {}", e);
        }
    });

    // Only add write system message tool if allow_edit is true
    if config.system_message.allow_edit {
        let write_system_message_runner = ToolActorRunner::new(WriteSystemMessageTool::new(system_message.clone()));
        let bus_for_write_sys_msg = bus.clone();
        tokio::spawn(async move {
            if let Err(e) = write_system_message_runner.run(&bus_for_write_sys_msg).await {
                eprintln!("Write system message tool actor failed: {}", e);
            }
        });
    }

    info!("Tool actors ready. Waiting for Ctrl+C...");
    tokio::signal::ctrl_c().await?;
    Ok(())
}