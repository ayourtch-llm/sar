use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use sar_core::{Config, SarBus};
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;

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

    // Load config
    let config = if cli.config.exists() {
        Config::from_file(&cli.config)?
    } else {
        Config::default()
    };

    // Create pub-sub bus
    let bus = Arc::new(SarBus::new());

    // Create topics
    bus.create_topic(&config.topics.log, 1000).await;
    bus.create_topic(&config.topics.input, 100).await;
    bus.create_topic(&config.topics.echo, 100).await;
    bus.create_topic(&config.topics.server, 100).await;

    // Setup tracing with bus layer
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
    
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::sink)
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
    let tui_actor = sar_tui::TuiActor::new(
        config.topics.log.clone(),
        config.topics.input.clone(),
        config.ui.show_bottom_panel,
    );
    (*bus).spawn_actor(tui_actor).await?.wait().await?;

    info!("SAR shutting down");
    Ok(())
}