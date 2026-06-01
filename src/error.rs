//! Error types for RavenClaw

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RavenClawError {
    #[error("LLM error: {0}")]
    LLM(#[from] crate::llm::LLMError),
    
    #[error("Configuration error: {0}")]
    Config(#[from] crate::config::ConfigError),
    
    #[error("RavenFabric error: {0}")]
    RavenFabric(String),
    
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    
    #[error("Command execution failed: {0}")]
    CommandExecution(String),
    
    #[error("Security violation: {0}")]
    SecurityViolation(String),
}

pub type Result<T> = std::result::Result<T, RavenClawError>;
