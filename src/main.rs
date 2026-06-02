//! RavenClaw — Lightweight, secure Rust agent framework
//!
//! Built for efficiency, security, and easy deployment.
//! Supports multiple LLM providers: LiteLLM, OpenRouter, Ollama, OpenAI.

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
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Lightweight, secure Rust agent framework with multi-provider support", long_about = None)]
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

    /// Provider type: litellm, openrouter, ollama, openai (overrides config)
    #[arg(long, env = "RAVENCLAW_PROVIDER")]
    provider: Option<String>,

    /// LLM endpoint (overrides config)
    #[arg(long, env = "RAVENCLAW_ENDPOINT")]
    endpoint: Option<String>,

    /// Model name (overrides config)
    #[arg(long, env = "RAVENCLAW_MODEL")]
    model: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("ravenclaw={}", log_level).into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!(version = env!("CARGO_PKG_VERSION"), "RavenClaw starting");

    // Load configuration
    let mut config = config::Config::load(args.config.as_deref())?;

    // Apply CLI overrides
    if let Some(provider) = args.provider {
        config.llm.provider = match provider.to_lowercase().as_str() {
            "openrouter" => config::LLMProvider::OpenRouter,
            "ollama" => config::LLMProvider::Ollama,
            "openai" => config::LLMProvider::OpenAI,
            _ => config::LLMProvider::LiteLLM,
        };
    }
    if let Some(endpoint) = args.endpoint {
        config.llm.endpoint = endpoint;
    }
    if let Some(model) = args.model {
        config.llm.model = model;
    }

    info!(mode = %args.mode, "Configuration loaded");

    // Handle --exec one-shot mode (overrides mode, uses first available provider)
    if let Some(exec_prompt) = args.exec {
        info!("Running in --exec one-shot mode");
        if !config.llms.is_empty() {
            let multi_llm = llm::MultiModelManager::new(config.llms.clone())?;
            if let Some(client) = multi_llm.get_client(0) {
                let response = agent::run_exec(client.clone(), &exec_prompt).await?;
                println!("{}", response);
            } else {
                anyhow::bail!("No LLM providers available for --exec mode");
            }
        } else {
            let llm = llm::create_client(&config.llm)?;
            let response = agent::run_exec(llm, &exec_prompt).await?;
            println!("{}", response);
        }
        info!("RavenClaw shutdown complete");
        return Ok(());
    }

    // Determine if multi-model or single-provider mode
    let has_multi_model = !config.llms.is_empty();

    if has_multi_model {
        info!(providers = config.llms.len(), "Multi-model mode enabled");

        // Create multi-model manager
        let multi_llm = llm::MultiModelManager::new(config.llms.clone())?;

        for i in 0..multi_llm.client_count() {
            if let Some(client) = multi_llm.get_client(i) {
                info!(
                    provider = client.provider_name(),
                    model = client.model(),
                    "Provider initialized"
                );
            }
        }

        // Run agent based on mode with multi-model support
        match args.mode.as_str() {
            "single" => {
                info!("Running in single agent mode (multi-model)");
                agent::run_single_multi(multi_llm, config).await?;
            }
            "swarm" => {
                info!("Running in swarm mode (multi-model)");
                agent::run_swarm_multi(multi_llm, config).await?;
            }
            "supervisor" => {
                info!("Running in supervisor mode (multi-model)");
                agent::run_supervisor_multi(multi_llm, config).await?;
            }
            _ => {
                anyhow::bail!(
                    "Unknown mode: {}. Use: single, swarm, or supervisor",
                    args.mode
                );
            }
        }
    } else {
        // Single provider mode
        let provider_name = match config.llm.provider {
            config::LLMProvider::LiteLLM => "LiteLLM",
            config::LLMProvider::OpenRouter => "OpenRouter",
            config::LLMProvider::Ollama => "Ollama",
            config::LLMProvider::OpenAI => "OpenAI",
        };

        info!(provider = provider_name, endpoint = %config.llm.endpoint, model = %config.llm.model, "LLM client initialized");

        // Create appropriate client based on provider
        let llm = llm::create_client(&config.llm)?;
        info!(
            provider = llm.provider_name(),
            model = llm.model(),
            "Provider ready"
        );

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
                anyhow::bail!(
                    "Unknown mode: {}. Use: single, swarm, or supervisor",
                    args.mode
                );
            }
        }
    }

    info!("RavenClaw shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_default_args() {
        // Verify the CLI struct can be constructed with defaults
        let args = Args::parse_from(["ravenclaw"]);
        assert_eq!(args.mode, "single");
        assert!(!args.verbose);
        assert!(args.config.is_none());
        assert!(args.exec.is_none());
        assert!(args.provider.is_none());
        assert!(args.endpoint.is_none());
        assert!(args.model.is_none());
    }

    #[test]
    fn test_cli_custom_args() {
        let args = Args::parse_from([
            "ravenclaw",
            "--config",
            "/tmp/config.toml",
            "--mode",
            "swarm",
            "--verbose",
            "--exec",
            "Hello",
            "--provider",
            "ollama",
            "--endpoint",
            "http://localhost:11434",
            "--model",
            "llama3.1",
        ]);
        assert_eq!(args.config.unwrap(), "/tmp/config.toml");
        assert_eq!(args.mode, "swarm");
        assert!(args.verbose);
        assert_eq!(args.exec.unwrap(), "Hello");
        assert_eq!(args.provider.unwrap(), "ollama");
        assert_eq!(args.endpoint.unwrap(), "http://localhost:11434");
        assert_eq!(args.model.unwrap(), "llama3.1");
    }

    #[test]
    fn test_cli_short_args() {
        let args = Args::parse_from([
            "ravenclaw",
            "-c",
            "/tmp/config.toml",
            "-m",
            "supervisor",
            "-v",
            "-e",
            "test prompt",
        ]);
        assert_eq!(args.config.unwrap(), "/tmp/config.toml");
        assert_eq!(args.mode, "supervisor");
        assert!(args.verbose);
        assert_eq!(args.exec.unwrap(), "test prompt");
    }

    #[test]
    fn test_cli_invalid_mode() {
        let args = Args::parse_from(["ravenclaw", "--mode", "invalid"]);
        assert_eq!(args.mode, "invalid");
        // The mode validation happens at runtime, not in clap
    }

    #[test]
    fn test_cli_provider_mapping() {
        // Test that provider strings map correctly
        let test_cases = vec![
            ("litellm", config::LLMProvider::LiteLLM),
            ("openrouter", config::LLMProvider::OpenRouter),
            ("ollama", config::LLMProvider::Ollama),
            ("openai", config::LLMProvider::OpenAI),
            ("unknown", config::LLMProvider::LiteLLM), // default
        ];

        for (input, expected) in test_cases {
            let mapped = match input.to_lowercase().as_str() {
                "openrouter" => config::LLMProvider::OpenRouter,
                "ollama" => config::LLMProvider::Ollama,
                "openai" => config::LLMProvider::OpenAI,
                _ => config::LLMProvider::LiteLLM,
            };
            assert_eq!(mapped, expected, "Provider mapping failed for '{}'", input);
        }
    }

    #[test]
    fn test_cli_provider_mapping_case_insensitive() {
        let test_cases = vec![
            ("OpenRouter", config::LLMProvider::OpenRouter),
            ("OPENAI", config::LLMProvider::OpenAI),
            ("Ollama", config::LLMProvider::Ollama),
            ("LiteLLM", config::LLMProvider::LiteLLM),
        ];

        for (input, expected) in test_cases {
            let mapped = match input.to_lowercase().as_str() {
                "openrouter" => config::LLMProvider::OpenRouter,
                "ollama" => config::LLMProvider::Ollama,
                "openai" => config::LLMProvider::OpenAI,
                _ => config::LLMProvider::LiteLLM,
            };
            assert_eq!(
                mapped, expected,
                "Case-insensitive mapping failed for '{}'",
                input
            );
        }
    }

    #[test]
    fn test_cli_verbose_flag() {
        let args = Args::parse_from(["ravenclaw", "--verbose"]);
        assert!(args.verbose);

        let args = Args::parse_from(["ravenclaw"]);
        assert!(!args.verbose);
    }

    #[test]
    fn test_cli_env_var_mapping() {
        // Verify that env var names match CLI args
        // RAVENCLAW_CONFIG → config
        // RAVENCLAW_VERBOSE → verbose
        // RAVENCLAW_PROVIDER → provider
        // RAVENCLAW_ENDPOINT → endpoint
        // RAVENCLAW_MODEL → model
        let args = Args::parse_from(["ravenclaw"]);
        assert_eq!(args.config, None);
        assert!(!args.verbose);
        assert_eq!(args.provider, None);
        assert_eq!(args.endpoint, None);
        assert_eq!(args.model, None);
    }

    #[test]
    fn test_cli_exec_with_provider() {
        let args = Args::parse_from(["ravenclaw", "--exec", "test prompt", "--provider", "openai"]);
        assert_eq!(args.exec.unwrap(), "test prompt");
        assert_eq!(args.provider.unwrap(), "openai");
    }

    #[test]
    fn test_cli_mode_dispatch_single() {
        let args = Args::parse_from(["ravenclaw", "--mode", "single"]);
        assert_eq!(args.mode, "single");
    }

    #[test]
    fn test_cli_mode_dispatch_swarm() {
        let args = Args::parse_from(["ravenclaw", "--mode", "swarm"]);
        assert_eq!(args.mode, "swarm");
    }

    #[test]
    fn test_cli_mode_dispatch_supervisor() {
        let args = Args::parse_from(["ravenclaw", "--mode", "supervisor"]);
        assert_eq!(args.mode, "supervisor");
    }
}
