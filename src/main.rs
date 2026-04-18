use std::path::PathBuf;

use clap::Parser;
use sar_core::{Config, SarBus};
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "sar", about = "Simple Agent in Rust")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Enable verbose logging
    #[arg(short, long, default_value = "false")]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    // Setup tracing
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            if cli.verbose {
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
            } else {
                tracing_subscriber::EnvFilter::new("info")
            },
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Load config
    let config = if cli.config.exists() {
        Config::from_file(&cli.config)?
    } else {
        info!("Config file not found, using defaults");
        Config::default()
    };

    info!("Starting SAR with config: {:?}", config);

    // Create pub-sub bus
    let bus = SarBus::new();

    // Create topics
    bus.create_topic(&config.topics.log, 1000).await;
    bus.create_topic(&config.topics.input, 100).await;
    bus.create_topic(&config.topics.echo, 100).await;
    bus.create_topic(&config.topics.server, 100).await;

    info!("Topics initialized: {:?}", bus.list_topics().await);

    // Spawn echo actor
    let echo_actor = sar_echo::EchoActor::new(
        config.topics.input.clone(),
        config.topics.log.clone(),
    );
    bus.spawn_actor(echo_actor).await?;

    // Spawn server (detached - runs in background)
    let bus_for_server = bus.clone();
    let host = config.server.host.clone();
    let port = config.server.port;
    tokio::spawn(async move {
        if let Err(e) = sar_server::run_server(bus_for_server, host, port).await {
            eprintln!("Server error: {}", e);
        }
    });

    // Spawn TUI actor (this blocks until the user quits)
    let tui_actor = sar_tui::TuiActor::new(
        config.topics.log.clone(),
        config.topics.input.clone(),
        config.ui.show_bottom_panel,
    );
    bus.spawn_actor(tui_actor).await?.wait().await?;

    info!("SAR shutting down");
    Ok(())
}