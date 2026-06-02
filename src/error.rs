//! Error types for RavenClaw

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RavenClawError {
    #[error("LLM error: {0}")]
    Llm(#[from] crate::llm::LLMError),

    #[error("Configuration error: {0}")]
    Config(#[from] crate::config::ConfigError),

    #[error("RavenFabric error: {0}")]
    #[allow(dead_code)]
    RavenFabric(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),

    #[error("Command execution failed: {0}")]
    CommandExecution(String),

    #[error("Security violation: {0}")]
    #[allow(dead_code)]
    SecurityViolation(String),
}

pub type Result<T> = std::result::Result<T, RavenClawError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_error_variant() {
        let err = RavenClawError::Llm(crate::llm::LLMError::RequestFailed("timeout".to_string()));
        assert_eq!(format!("{}", err), "LLM error: Request failed: timeout");
    }

    #[test]
    fn test_config_error_variant() {
        let err = RavenClawError::Config(crate::config::ConfigError::ValidationError(
            "bad field".to_string(),
        ));
        assert_eq!(
            format!("{}", err),
            "Configuration error: Invalid configuration: bad field"
        );
    }

    #[test]
    fn test_ravenfabric_error_variant() {
        let err = RavenClawError::RavenFabric("connection refused".to_string());
        assert_eq!(format!("{}", err), "RavenFabric error: connection refused");
    }

    #[test]
    fn test_command_execution_error_variant() {
        let err = RavenClawError::CommandExecution("command failed".to_string());
        assert_eq!(
            format!("{}", err),
            "Command execution failed: command failed"
        );
    }

    #[test]
    fn test_security_violation_error_variant() {
        let err = RavenClawError::SecurityViolation("unauthorized access".to_string());
        assert_eq!(
            format!("{}", err),
            "Security violation: unauthorized access"
        );
    }

    #[test]
    fn test_result_type_alias() {
        let ok: i32 = 42;
        assert_eq!(ok, 42);

        let err: Result<i32> = Err(RavenClawError::CommandExecution("fail".to_string()));
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_network_error_variant() {
        // Network error from reqwest — we can construct it via the From impl
        // by creating a reqwest error. Since reqwest::Error is opaque, we
        // test the variant via the Display trait.
        let err = RavenClawError::Network(
            reqwest::Client::builder()
                .build()
                .unwrap()
                .get("http://invalid.example.com")
                .send()
                .await
                .unwrap_err(),
        );
        assert!(format!("{}", err).contains("Network error"));
    }

    #[test]
    fn test_io_error_variant() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = RavenClawError::IO(io_err);
        assert!(format!("{}", err).contains("IO error"));
        assert!(format!("{}", err).contains("file not found"));
    }

    #[test]
    fn test_error_is_debug() {
        let err = RavenClawError::CommandExecution("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("CommandExecution"));
    }

    #[test]
    fn test_error_is_send() {
        fn check_send<T: Send>() {}
        check_send::<RavenClawError>();
    }

    #[test]
    fn test_error_is_sync() {
        fn check_sync<T: Sync>() {}
        check_sync::<RavenClawError>();
    }

    #[test]
    fn test_from_llm_error_conversion() {
        let llm_err = crate::llm::LLMError::RequestFailed("timeout".to_string());
        let err: RavenClawError = llm_err.into();
        assert!(format!("{}", err).contains("LLM error"));
        assert!(format!("{}", err).contains("timeout"));
    }

    #[test]
    fn test_from_config_error_conversion() {
        let cfg_err = crate::config::ConfigError::ValidationError("bad config".to_string());
        let err: RavenClawError = cfg_err.into();
        assert!(format!("{}", err).contains("Configuration error"));
        assert!(format!("{}", err).contains("bad config"));
    }

    #[test]
    fn test_from_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        let err: RavenClawError = io_err.into();
        assert!(format!("{}", err).contains("IO error"));
        assert!(format!("{}", err).contains("permission denied"));
    }

    #[test]
    fn test_error_source_chain() {
        // RavenClawError doesn't implement std::error::Error::source() directly
        // for all variants, but the Display impl should contain the inner message
        let inner = crate::llm::LLMError::AuthFailed;
        let err = RavenClawError::Llm(inner);
        let display = format!("{}", err);
        assert!(display.contains("Authentication failed"));
    }

    #[test]
    fn test_ravenfabric_error_construction() {
        let err = RavenClawError::RavenFabric("connection timeout".to_string());
        assert_eq!(format!("{}", err), "RavenFabric error: connection timeout");
    }

    #[test]
    fn test_security_violation_construction() {
        let err = RavenClawError::SecurityViolation("invalid token".to_string());
        assert_eq!(format!("{}", err), "Security violation: invalid token");
    }

    #[test]
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn test_result_type_alias_ok() {
        let result: Result<i32> = Ok(42);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn test_result_type_alias_err() {
        let result: Result<i32> = Err(RavenClawError::CommandExecution("fail".to_string()));
        assert!(result.is_err());
        assert_eq!(
            format!("{}", result.unwrap_err()),
            "Command execution failed: fail"
        );
    }
}
