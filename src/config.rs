//! Configuration management for RavenClaw
//!
//! Secure by default: no credentials in config files, use environment variables.

use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load config: {0}")]
    LoadError(String),
    #[error("Invalid configuration: {0}")]
    ValidationError(String),
    #[error("Missing required environment variable: {0}")]
    MissingEnvVar(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// LiteLLM configuration
    pub llm: LLMConfig,
    
    /// RavenFabric configuration
    #[serde(default)]
    pub ravenfabric: RavenFabricConfig,
    
    /// Security settings
    #[serde(default)]
    pub security: SecurityConfig,
    
    /// Runtime settings
    #[serde(default)]
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LLMConfig {
    /// LiteLLM endpoint (e.g., http://litellm:4000)
    pub endpoint: String,
    
    /// Default model to use
    #[serde(default = "default_model")]
    pub model: String,
    
    /// API key (prefer env var LITELLM_API_KEY)
    #[serde(default)]
    pub api_key: Option<String>,
    
    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RavenFabricConfig {
    /// RavenFabric endpoint
    #[serde(default)]
    pub endpoint: Option<String>,
    
    /// Agent ID for identification
    #[serde(default)]
    pub agent_id: Option<String>,
    
    /// Enable remote command execution
    #[serde(default = "default_true")]
    pub remote_exec: bool,
    
    /// Allowed remote hosts (whitelist)
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    /// Require TLS for all connections
    #[serde(default = "default_true")]
    pub require_tls: bool,
    
    /// Maximum token lifetime in seconds
    #[serde(default = "default_token_lifetime")]
    pub token_lifetime_secs: u64,
    
    /// Enable audit logging
    #[serde(default = "default_true")]
    pub audit_log: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
    /// Working directory
    #[serde(default = "default_workdir")]
    pub workdir: String,
    
    /// Maximum concurrent agents
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    
    /// Health check interval in seconds
    #[serde(default = "default_health_interval")]
    pub health_interval_secs: u64,
}

fn default_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_timeout() -> u64 {
    30
}

fn default_true() -> bool {
    true
}

fn default_token_lifetime() -> u64 {
    3600
}

fn default_workdir() -> String {
    "/workspace".to_string()
}

fn default_max_agents() -> usize {
    10
}

fn default_health_interval() -> u64 {
    60
}

impl Config {
    /// Load configuration from file and environment
    pub fn load(config_path: Option<&str>) -> Result<Self, ConfigError> {
        // Start with defaults from environment
        dotenvy::dotenv().ok();
        
        let mut config_builder = config::Config::builder();
        
        // Load from file if provided
        if let Some(path) = config_path {
            config_builder = config_builder.add_source(config::File::with_name(path).required(false));
        }
        
        // Load from environment (RAVENCLAW_* prefix)
        config_builder = config_builder.add_source(
            config::Environment::with_prefix("RAVENCLAW").separator("__")
        );
        
        let config = config_builder
            .build()
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;
        
        let mut cfg: Config = config
            .try_deserialize()
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;
        
        // Override sensitive values from environment
        if let Ok(key) = std::env::var("LITELLM_API_KEY") {
            cfg.llm.api_key = Some(key);
        }
        
        if let Ok(endpoint) = std::env::var("RAVENFABRIC_ENDPOINT") {
            cfg.ravenfabric.endpoint = Some(endpoint);
        }
        
        // Validate
        cfg.validate()?;
        
        Ok(cfg)
    }
    
    /// Validate configuration
    fn validate(&self) -> Result<(), ConfigError> {
        if self.llm.endpoint.is_empty() {
            return Err(ConfigError::ValidationError(
                "LLM endpoint is required".to_string()
            ));
        }
        
        if self.security.require_tls && !self.llm.endpoint.starts_with("https://") {
            // Allow localhost for development
            if !self.llm.endpoint.contains("localhost") && !self.llm.endpoint.contains("127.0.0.1") {
                return Err(ConfigError::ValidationError(
                    "TLS required but endpoint is not HTTPS".to_string()
                ));
            }
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        std::env::set_var("LITELLM_API_KEY", "test-key");
        std::env::set_var("RAVENCLAW__LLM__ENDPOINT", "http://localhost:4000");
        
        let config = Config::load(None).unwrap();
        assert_eq!(config.llm.model, "gpt-4o-mini");
        assert_eq!(config.llm.timeout_secs, 30);
        assert!(config.security.require_tls);
    }
}
