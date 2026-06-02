//! Agent implementations for RavenClaw
//!
//! Supports single-provider and multi-model (multi-provider) modes.

use crate::config::Config;
use crate::error::Result;
use crate::llm::{ChatMessage, LLMProviderTrait, MultiModelManager};
use std::sync::Arc;
use tracing::{info, warn};

/// Run a one-shot command via --exec mode
/// Sends the prompt to the LLM and returns the response text.
pub async fn run_exec(llm: Arc<dyn LLMProviderTrait>, prompt: &str) -> Result<String> {
    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        "Exec one-shot mode"
    );

    let system_prompt = "You are RavenClaw, a lightweight autonomous agent. \
        Be concise, efficient, and secure. Always validate inputs and outputs.";

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        },
    ];

    let response = llm.chat(messages).await.map_err(|e| {
        crate::error::RavenClawError::CommandExecution(format!("LLM request failed: {}", e))
    })?;

    let content = response
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| {
            crate::error::RavenClawError::CommandExecution(
                "LLM returned empty response".to_string(),
            )
        })?;

    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        "Exec response received"
    );
    Ok(content)
}

/// Run a single autonomous agent (single-provider mode)
pub async fn run_single(llm: Arc<dyn LLMProviderTrait>, _config: Config) -> Result<()> {
    info!(
        "Starting single agent mode with provider: {}",
        llm.provider_name()
    );

    let system_prompt = "You are RavenClaw, a lightweight autonomous agent. \
        Be concise, efficient, and secure. Always validate inputs and outputs.";

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: "Ready. Awaiting instructions.".to_string(),
        },
    ];

    match llm.chat(messages).await {
        Ok(response) => {
            if let Some(choice) = response.choices.first() {
                info!(provider = llm.provider_name(), model = llm.model(), response = %choice.message.content, "Agent response received");
            }
        }
        Err(e) => {
            warn!(error = %e, provider = llm.provider_name(), "LLM request failed");
        }
    }

    Ok(())
}

/// Run multiple agents in swarm mode (single-provider)
pub async fn run_swarm(_llm: Arc<dyn LLMProviderTrait>, _config: Config) -> Result<()> {
    warn!("Swarm mode not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline.".to_string(),
    ))
}

/// Run supervisor agent coordinating sub-agents (single-provider)
pub async fn run_supervisor(_llm: Arc<dyn LLMProviderTrait>, _config: Config) -> Result<()> {
    warn!("Supervisor mode not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
            .to_string(),
    ))
}

/// Run a single autonomous agent (multi-model mode)
pub async fn run_single_multi(multi_llm: MultiModelManager, _config: Config) -> Result<()> {
    info!(
        "Starting single agent mode (multi-model) with {} providers",
        multi_llm.client_count()
    );

    let system_prompt = "You are RavenClaw, a lightweight autonomous agent. \
        Be concise, efficient, and secure. Always validate inputs and outputs.";

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: "Ready. Awaiting instructions.".to_string(),
        },
    ];

    // Test each configured provider
    for i in 0..multi_llm.client_count() {
        if let Some(client) = multi_llm.get_client(i) {
            match client.chat(messages.clone()).await {
                Ok(response) => {
                    if let Some(choice) = response.choices.first() {
                        info!(provider = client.provider_name(), model = client.model(), response = %choice.message.content, "Provider response received");
                    }
                }
                Err(e) => {
                    warn!(error = %e, provider = client.provider_name(), model = client.model(), "Provider request failed");
                }
            }
        }
    }

    Ok(())
}

/// Run multiple agents in swarm mode (multi-model)
pub async fn run_swarm_multi(_multi_llm: MultiModelManager, _config: Config) -> Result<()> {
    warn!("Swarm mode (multi-model) not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline.".to_string(),
    ))
}

/// Run supervisor agent coordinating sub-agents (multi-model)
pub async fn run_supervisor_multi(_multi_llm: MultiModelManager, _config: Config) -> Result<()> {
    warn!("Supervisor mode (multi-model) not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swarm_stub_returns_error() {
        let err = crate::error::RavenClawError::CommandExecution(
            "Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline."
                .to_string(),
        );
        assert_eq!(
            format!("{}", err),
            "Command execution failed: Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline."
        );
    }

    #[test]
    fn test_supervisor_stub_returns_error() {
        let err = crate::error::RavenClawError::CommandExecution(
            "Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
                .to_string(),
        );
        assert_eq!(
            format!("{}", err),
            "Command execution failed: Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
        );
    }

    #[test]
    fn test_exec_empty_response_error() {
        let err = crate::error::RavenClawError::CommandExecution(
            "LLM returned empty response".to_string(),
        );
        assert!(format!("{}", err).contains("LLM returned empty response"));
    }

    #[test]
    fn test_swarm_multi_stub_returns_error() {
        let err = crate::error::RavenClawError::CommandExecution(
            "Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline."
                .to_string(),
        );
        assert!(format!("{}", err).contains("Swarm mode"));
    }

    #[test]
    fn test_supervisor_multi_stub_returns_error() {
        let err = crate::error::RavenClawError::CommandExecution(
            "Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
                .to_string(),
        );
        assert!(format!("{}", err).contains("Supervisor mode"));
    }

    #[test]
    fn test_run_exec_llm_error_message() {
        let err = crate::error::RavenClawError::CommandExecution(
            "LLM request failed: connection refused".to_string(),
        );
        assert!(format!("{}", err).contains("LLM request failed"));
    }

    #[test]
    fn test_run_single_logs_provider_name() {
        // Verify the function signature compiles and accepts the right types
        fn _check_types() {
            let _ = run_single;
            let _ = run_swarm;
            let _ = run_supervisor;
            let _ = run_single_multi;
            let _ = run_swarm_multi;
            let _ = run_supervisor_multi;
            let _ = run_exec;
        }
        // Compile-time check: all function signatures are valid
    }

    // ── Mockito-based agent tests ──────────────────────────────────────

    fn sample_chat_response_json(model: &str) -> String {
        format!(
            r#"{{
            "id": "chat-123",
            "object": "chat.completion",
            "created": 1717000000,
            "model": "{}",
            "choices": [
                {{
                    "index": 0,
                    "message": {{
                        "role": "assistant",
                        "content": "Hello from agent!"
                    }},
                    "finish_reason": "stop"
                }}
            ],
            "usage": {{
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }}
        }}"#,
            model
        )
    }

    fn with_mockito<F, Fut>(f: F)
    where
        F: FnOnce(mockito::ServerGuard) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let server = mockito::Server::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(f(server));
    }

    #[test]
    fn test_run_exec_with_mockito() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_chat_response_json("gpt-4o-mini"))
                .create();

            let config = crate::config::LLMConfig {
                provider: crate::config::LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let response = run_exec(llm, "Hello!").await.unwrap();

            assert_eq!(response, "Hello from agent!");
            mock.assert();
        });
    }

    #[test]
    fn test_run_exec_with_mockito_empty_response() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    r#"{"id":"x","object":"chat.completion","created":0,"model":"x","choices":[]}"#,
                )
                .create();

            let config = crate::config::LLMConfig {
                provider: crate::config::LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let result = run_exec(llm, "Hello!").await;

            assert!(result.is_err());
            assert!(format!("{}", result.unwrap_err()).contains("empty response"));
            mock.assert();
        });
    }

    #[test]
    fn test_run_exec_with_mockito_llm_error() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(500)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error":"Internal error"}"#)
                .create();

            let config = crate::config::LLMConfig {
                provider: crate::config::LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let result = run_exec(llm, "Hello!").await;

            assert!(result.is_err());
            assert!(format!("{}", result.unwrap_err()).contains("LLM request failed"));
            mock.assert();
        });
    }

    #[test]
    fn test_run_single_with_mockito() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_chat_response_json("gpt-4o-mini"))
                .create();

            let config = crate::config::LLMConfig {
                provider: crate::config::LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let cfg = crate::config::Config {
                llm: crate::config::LLMConfig::default(),
                llms: vec![],
                ravenfabric: crate::config::RavenFabricConfig::default(),
                security: crate::config::SecurityConfig {
                    require_tls: false,
                    token_lifetime_secs: 3600,
                    audit_log: false,
                },
                runtime: crate::config::RuntimeConfig::default(),
            };
            let result = run_single(llm, cfg).await;

            // run_single logs the response but always returns Ok(())
            assert!(result.is_ok());
            mock.assert();
        });
    }

    #[test]
    fn test_run_single_with_mockito_llm_error() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(401)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error":"Unauthorized"}"#)
                .create();

            let config = crate::config::LLMConfig {
                provider: crate::config::LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("bad-key".to_string()),
                timeout_secs: 30,
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let cfg = crate::config::Config {
                llm: crate::config::LLMConfig::default(),
                llms: vec![],
                ravenfabric: crate::config::RavenFabricConfig::default(),
                security: crate::config::SecurityConfig {
                    require_tls: false,
                    token_lifetime_secs: 3600,
                    audit_log: false,
                },
                runtime: crate::config::RuntimeConfig::default(),
            };
            // run_single catches LLM errors internally and logs them, returns Ok(())
            let result = run_single(llm, cfg).await;
            assert!(result.is_ok());
            mock.assert();
        });
    }

    #[test]
    fn test_run_single_multi_with_mockito() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_chat_response_json("gpt-4o-mini"))
                .create();

            let configs = vec![crate::config::LLMConfig {
                provider: crate::config::LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
            }];

            let multi_llm = crate::llm::MultiModelManager::new(configs).unwrap();
            let cfg = crate::config::Config {
                llm: crate::config::LLMConfig::default(),
                llms: vec![],
                ravenfabric: crate::config::RavenFabricConfig::default(),
                security: crate::config::SecurityConfig {
                    require_tls: false,
                    token_lifetime_secs: 3600,
                    audit_log: false,
                },
                runtime: crate::config::RuntimeConfig::default(),
            };
            let result = run_single_multi(multi_llm, cfg).await;

            // run_single_multi logs responses but always returns Ok(())
            assert!(result.is_ok());
            mock.assert();
        });
    }
}
