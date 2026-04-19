use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use sar_core::{Config, SarBus};
use sar_llm::LlmActor;
use sar_llm_test_loop::LlmTestLoopActor;
use sar_llm_test_loop_tools::LlmTestLoopToolsActor;
use sar_tool_actors::{ToolActor, ToolActorRunner};
use sar_tool_calculator::CalculatorTool;
use sar_tool_mcp::McpServerRunner;
use sar_tool_sleep::SleepTool;
use sar_ui_hub::UiHubActor;
use tracing::{error, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::fmt::MakeWriter;

struct LogWriter {
    path: Option<PathBuf>,
}

impl LogWriter {
    fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = Box<dyn Write + Send + Sync>;
    
    fn make_writer(&self) -> Self::Writer {
        match &self.path {
            Some(path) => {
                match File::options().append(true).create(true).open(path) {
                    Ok(file) => Box::new(file),
                    Err(e) => panic!("Failed to open log file '{}': {}", path.display(), e),
                }
            }
            None => Box::new(std::io::sink()),
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "sar", about = "Simple Agent in Rust")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Enable verbose logging
    #[arg(short, long, default_value = "false")]
    verbose: bool,

    /// Log file path
    #[arg(long, value_name = "FILE", num_args = 1)]
    log: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    // Load or create config
    let config = if cli.config.exists() {
        Config::from_file(&cli.config)?
    } else {
        let default_config = Config::default();
        let default_toml = default_config.to_toml()
            .map_err(|e| format!("Failed to serialize default config: {}", e))?;
        std::fs::write(&cli.config, &default_toml)
            .map_err(|e| format!("Failed to write default config to '{}': {}", cli.config.display(), e))?;
        info!("Created default config at: {}", cli.config.display());
        default_config
    };

    // Create pub-sub bus
    let bus = Arc::new(SarBus::new());

    // Create topics
    bus.create_topic(&config.topics.log, 1000).await;
    bus.create_topic(&config.topics.input, 100).await;
    bus.create_topic(&config.topics.echo, 100).await;
    bus.create_topic(&config.topics.reverse, 100).await;
    bus.create_topic(&config.topics.server, 100).await;
    bus.create_topic("llm:0:in", 100).await;
    bus.create_topic("llm:0:out", 1000).await;
    bus.create_topic("llm:0:stream", 1000).await;
    bus.create_topic("llm:0:stats", 100).await;
    bus.create_topic("llm:0:tool_calls", 100).await;
    bus.create_topic("llm-test:0:in", 100).await;
    bus.create_topic("llm-test:0:stream", 1000).await;
    bus.create_topic("llm-test-loop:0:in", 100).await;
    bus.create_topic("llm-test-loop:0:stream", 1000).await;
    bus.create_topic("llm-test-tools:0:in", 100).await;
    bus.create_topic("llm-test-tools:0:stream", 1000).await;
    bus.create_topic("ui:user", 1000).await;
    bus.create_topic("ui:input", 100).await;

    // Tool execution topics
    bus.create_topic("tool:results", 1000).await;
    bus.create_topic("tool:calculator:execute", 100).await;
    bus.create_topic("tool:sleep:execute", 100).await;
    bus.create_topic("user:control", 100).await;

    // Spawn UI hub actors
    for (name, hub_config) in &config.ui_hubs {
        info!("Spawning UI hub '{}'", name);
        let hub_actor = UiHubActor::new(hub_config.clone());
        (*bus).spawn_actor(hub_actor).await?;
    }

    // Setup tracing with bus layer
    bus.register_announcement(sar_core::actor::ActorAnnouncement {
        id: "sar-tracing".to_string(),
        subscriptions: Vec::new(),
        publications: vec![config.topics.log.clone()],
    }).await;
    let bus_layer = sar_tracing::BusLayer::new(
        bus.clone(),
        config.topics.log.clone(),
    );
    
    let env_filter = if cli.verbose {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
    } else {
        tracing_subscriber::EnvFilter::new("info")
    };
    
    let log_writer = LogWriter::new(cli.log);
    
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(log_writer)
        .finish();
    
    let subscriber = subscriber.with(bus_layer);
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting SAR with config: {:?}", config);
    info!("Topics initialized: {:?}", (*bus).list_topics().await);

    // Spawn echo actor
    let echo_actor = sar_echo::EchoActor::new(
        config.topics.input.clone(),
        config.topics.log.clone(),
    );
    (*bus).spawn_actor(echo_actor).await?;

    // Spawn reverse actor
    let reverse_actor = sar_reverse::ReverseActor::new(
        config.topics.reverse.clone(),
        config.topics.log.clone(),
    );
    (*bus).spawn_actor(reverse_actor).await?;

    // Spawn LLM actor
    let llm_actor = LlmActor::new(
        0,
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm:0:stats".to_string(),
        "llm:0:tool_calls".to_string(),
        "user:control".to_string(),
        config.llm.clone(),
    );
    (*bus).spawn_actor(llm_actor).await?;

    // Spawn LLM test actor
    let llm_test_actor = sar_llm_test::LlmTestActor::new(
        0,
        "llm-test:0:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm-test:0:stream".to_string(),
    );
    (*bus).spawn_actor(llm_test_actor).await?;

    // Spawn LLM test loop actor
    let llm_test_loop_actor = LlmTestLoopActor::new(
        0,
        "llm-test-loop:0:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm-test-loop:0:stream".to_string(),
    );
    (*bus).spawn_actor(llm_test_loop_actor).await?;

    // Spawn tool actor runners (independent async actors)
    let calculator_runner = ToolActorRunner::new(CalculatorTool::new());
    let sleep_runner = ToolActorRunner::new(SleepTool::new());
    let bus_for_tools = bus.clone();
    tokio::spawn(async move {
        if let Err(e) = calculator_runner.run(&bus_for_tools).await {
            error!("Calculator tool actor failed: {}", e);
        }
    });
    let bus_for_tools = bus.clone();
    tokio::spawn(async move {
        if let Err(e) = sleep_runner.run(&bus_for_tools).await {
            error!("Sleep tool actor failed: {}", e);
        }
    });

    // Spawn MCP servers and their tool runners
    let mut all_mcp_actors: Vec<std::sync::Arc<dyn ToolActor>> = Vec::new();
    for (name, mcp_config) in &config.mcp_servers {
        info!("Spawning MCP server '{}'", name);
        let mcp_runner = McpServerRunner::new(name.clone(), mcp_config.clone());
        let handle = match mcp_runner.spawn(&bus).await {
            Ok(handle) => {
                info!(
                    "MCP server '{}' discovered {} tools",
                    name,
                    handle.tool_names().join(", ")
                );
                handle
            }
            Err(e) => {
                error!("Failed to spawn MCP server '{}': {}", name, e);
                continue;
            }
        };

        // Add MCP tool actors to the loop actor
        all_mcp_actors.extend(handle.tool_actors());
    }

    // Build tool list: built-in tools + MCP tools
    let llm_test_tools_actor = LlmTestLoopToolsActor::new(
        0,
        "llm-test-tools:0:in".to_string(),
        "llm:0:in".to_string(),
        "llm:0:out".to_string(),
        "llm:0:stream".to_string(),
        "llm:0:tool_calls".to_string(),
        "llm-test-tools:0:stream".to_string(),
    )
    .with_tool(CalculatorTool::new())
    .with_tool(SleepTool::new());

    for actor in all_mcp_actors {
        let name = actor.tool_syntax().name.clone();
        info!("Adding MCP tool to LLM loop: {}", name);
        llm_test_tools_actor.add_tool_arc(actor).await;
    }

    (*bus).spawn_actor(llm_test_tools_actor).await?;

    // Spawn server (detached - runs in background)
    let bus_for_server = bus.clone();
    let host = config.server.host.clone();
    let port = config.server.port;
    tokio::spawn(async move {
        if let Err(e) = sar_server::run_server((*bus_for_server).clone(), host, port).await {
            eprintln!("Server error: {}", e);
        }
    });

    // Spawn TUI actor (this blocks until the user quits)
    let first_hub = config.ui_hubs.values().next()
        .ok_or("No UI hub configured")?;
    let (interrupt_tx, mut interrupt_rx) = tokio::sync::mpsc::channel::<&'static str>(1);
    
    let bus_for_interrupt = bus.clone();
    let first_hub_for_hint = first_hub.clone();
    
    bus_for_interrupt.register_announcement(sar_core::actor::ActorAnnouncement {
        id: "sar-interrupt".to_string(),
        subscriptions: Vec::new(),
        publications: vec!["user:control".to_string(), first_hub.user_topic.clone()],
    }).await;
    
    let tui_actor = sar_tui::TuiActor::new(
        first_hub.user_topic.clone(),
        first_hub.input_topic.clone(),
        config.topics.log.clone(),
        config.ui.show_bottom_panel,
        interrupt_tx,
    );
    let handle = (*bus).spawn_actor(tui_actor).await?;

    tokio::spawn(async move {
        let mut first_press = std::time::Instant::now();
        
        loop {
            match interrupt_rx.recv().await {
                Some("first") => {
                    tracing::info!("[interrupt] First Ctrl+C detected");
                    
                    let hint_msg = sar_core::message::Message::new(
                        &first_hub_for_hint.user_topic,
                        "sar-interrupt",
                        "Ctrl+C detected. Press Ctrl+C again within 2 seconds to interrupt LLM.",
                    ).with_type("Info");
                    if let Err(e) = bus_for_interrupt.publish("sar-interrupt", hint_msg).await {
                        tracing::error!("Failed to publish hint to TUI: {}", e);
                    }
                    
                    first_press = std::time::Instant::now();
                    
                    // Wait for second Ctrl-C or timeout
                    match tokio::time::timeout(std::time::Duration::from_secs(2), interrupt_rx.recv()).await {
                        Ok(Some(_)) => {
                            let elapsed = first_press.elapsed();
                            tracing::info!("[interrupt] Second Ctrl+C detected, elapsed={:?}ms", elapsed.as_millis());
                            if elapsed.as_millis() < 2000 {
                                tracing::info!("[interrupt] Publishing interrupt to user:control");
                                let interrupt_msg = sar_core::message::Message::new(
                                    "user:control",
                                    "sar-interrupt",
                                    serde_json::json!({
                                        "type": "interrupt",
                                        "reason": "User sent double Ctrl+C",
                                    }),
                                ).with_type("Interrupt");
                                if let Err(e) = bus_for_interrupt.publish("sar-interrupt", interrupt_msg).await {
                                    tracing::error!("Failed to publish interrupt to user:control: {}", e);
                                } else {
                                    let done_msg = sar_core::message::Message::new(
                                        &first_hub_for_hint.user_topic,
                                        "sar-interrupt",
                                        "Interrupt sent to LLM actor.",
                                    ).with_type("Info");
                                    if let Err(e) = bus_for_interrupt.publish("sar-interrupt", done_msg).await {
                                        tracing::error!("Failed to publish interrupt confirmation: {}", e);
                                    }
                                }
                            } else {
                                tracing::info!("[interrupt] Second Ctrl+C too late (>2s), ignoring");
                            }
                        }
                        Ok(None) => {
                            tracing::info!("[interrupt] Interrupt channel closed");
                            break;
                        }
                        Err(_) => {
                            tracing::info!("[interrupt] Timeout waiting for second Ctrl+C");
                        }
                    }
                }
                Some(_) => {
                    tracing::warn!("[interrupt] Unknown interrupt signal");
                }
                None => {
                    tracing::info!("[interrupt] Interrupt channel closed");
                    break;
                }
            }
        }
    });

    handle.wait().await?;

    info!("SAR shutting down");
    Ok(())
}