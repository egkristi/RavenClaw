//! RavenClaw — Lightweight, secure Rust agent framework
//!
//! Built for efficiency, security, and easy deployment.

mod agent;
mod config;
mod error;
mod llm;

use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "ravenclaw")]
#[command(author = "Erling G M Kristiansen")]
#[command(version = "0.1.0")]
#[command(about = "Lightweight, secure Rust agent framework", long_about = None)]
struct Args {
    /// Configuration file path
    #[arg(short, long, env = "RAVENCLAW_CONFIG")]
    config: Option<String>,

    /// Agent mode: single, swarm, or supervisor
    #[arg(short, long, default_value = "single")]
    mode: String,

    /// Enable verbose logging
    #[arg(short, long, env = "RAVENCLAW_VERBOSE")]
    verbose: bool,

    /// Run a one-shot command
    #[arg(short, long)]
    exec: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| format!("ravenclaw={}", log_level).into()))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!(version = env!("CARGO_PKG_VERSION"), "RavenClaw starting");

    // Load configuration
    let config = config::Config::load(args.config.as_deref())?;
    info!(mode = %args.mode, "Configuration loaded");

    // Initialize LiteLLM client
    let llm = llm::LiteLLMClient::new(&config.llm)?;
    info!(endpoint = %config.llm.endpoint, "LiteLLM client initialized");

    // Run agent based on mode
    match args.mode.as_str() {
        "single" => {
            info!("Running in single agent mode");
            agent::run_single(llm, config).await?;
        }
        "swarm" => {
            info!("Running in swarm mode");
            agent::run_swarm(llm, config).await?;
        }
        "supervisor" => {
            info!("Running in supervisor mode");
            agent::run_supervisor(llm, config).await?;
        }
        _ => {
            anyhow::bail!("Unknown mode: {}. Use: single, swarm, or supervisor", args.mode);
        }
    }

    info!("RavenClaw shutdown complete");
    Ok(())
}
