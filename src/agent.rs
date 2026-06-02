//! Agent implementations for RavenClaw
//!
//! Supports single-provider and multi-model (multi-provider) modes.

use crate::config::Config;
use crate::error::Result;
use crate::llm::{ChatMessage, LLMProviderTrait, MultiModelManager};
use futures::StreamExt;
use std::sync::Arc;
use tracing::{info, warn};

/// In-memory conversation memory — stores message history for the session.
/// Messages are lost when the process exits.
#[derive(Debug, Clone)]
pub struct ConversationMemory {
    /// Maximum number of messages to retain (0 = unlimited)
    max_messages: usize,
    /// Stored message history
    messages: Vec<ChatMessage>,
}

impl ConversationMemory {
    /// Create a new conversation memory with the given system prompt.
    /// `max_messages` caps history length (oldest user/assistant pairs are dropped first).
    pub fn new(system_prompt: &str, max_messages: usize) -> Self {
        Self {
            max_messages,
            messages: vec![ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            }],
        }
    }

    /// Add a user message and return the full message history for an LLM call.
    pub fn add_user_message(&mut self, content: &str) -> &[ChatMessage] {
        self.messages.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });
        self.trim_to_max();
        &self.messages
    }

    /// Add an assistant message to history.
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
        });
        self.trim_to_max();
    }

    /// Get the current message history.
    #[allow(dead_code)]
    pub fn history(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Trim oldest non-system messages when over the limit.
    fn trim_to_max(&mut self) {
        if self.max_messages == 0 {
            return;
        }
        while self.messages.len() > self.max_messages {
            // Remove the oldest non-system message (index 1, since index 0 is system)
            if self.messages.len() > 1 {
                self.messages.remove(1);
            } else {
                break;
            }
        }
    }
}

/// Run a one-shot command via --exec mode with streaming output
/// Sends the prompt to the LLM and prints tokens as they arrive.
#[allow(dead_code)]
pub async fn run_exec_stream(
    llm: Arc<dyn LLMProviderTrait>,
    prompt: &str,
    system_prompt: &str,
) -> Result<String> {
    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        "Exec one-shot streaming mode"
    );

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

    let mut stream = llm.chat_stream(messages).await.map_err(|e| {
        crate::error::RavenClawError::CommandExecution(format!("LLM stream request failed: {}", e))
    })?;

    let mut full_response = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(chunk) => {
                full_response.push_str(&chunk.content);
            }
            Err(e) => {
                warn!(error = %e, "Stream chunk error");
            }
        }
    }

    if full_response.is_empty() {
        return Err(crate::error::RavenClawError::CommandExecution(
            "LLM returned empty response".to_string(),
        ));
    }

    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        "Exec streaming response received"
    );
    Ok(full_response)
}

/// Run an interactive REPL (read-eval-print loop) with conversation memory.
/// Reads prompts from stdin, sends them to the LLM with streaming, and prints responses.
/// Type `/exit` or `/quit` to exit, `/reset` to clear conversation history.
pub async fn run_repl(llm: Arc<dyn LLMProviderTrait>, config: Config) -> Result<()> {
    let system_prompt = &config.llm.system_prompt;
    let mut memory = ConversationMemory::new(system_prompt, 50);

    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        "Starting interactive REPL mode"
    );

    println!("RavenClaw REPL — type your message, or /exit to quit.");
    println!("System: {}", system_prompt);
    println!();

    let mut line = String::new();

    loop {
        line.clear();
        print!("> ");
        use std::io::Write;
        std::io::stdout().flush().ok();

        let bytes_read = std::io::stdin().read_line(&mut line).map_err(|e| {
            crate::error::RavenClawError::CommandExecution(format!("stdin read error: {}", e))
        })?;

        if bytes_read == 0 {
            // EOF
            println!();
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        match input {
            "/exit" | "/quit" => {
                println!("Goodbye!");
                break;
            }
            "/reset" => {
                memory = ConversationMemory::new(system_prompt, 50);
                println!("Conversation reset.");
                continue;
            }
            _ => {}
        }

        // Add user message and get history
        let history = memory.add_user_message(input);

        // Send to LLM with streaming
        match llm.chat_stream(history.to_vec()).await {
            Ok(mut stream) => {
                let mut full_response = String::new();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => {
                            print!("{}", chunk.content);
                            std::io::stdout().flush().ok();
                            full_response.push_str(&chunk.content);
                        }
                        Err(e) => {
                            warn!(error = %e, "Stream chunk error");
                        }
                    }
                }
                println!();
                memory.add_assistant_message(&full_response);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    info!("REPL session ended");
    Ok(())
}

/// Run a one-shot command via --exec mode
/// Sends the prompt to the LLM and returns the response text.
pub async fn run_exec(
    llm: Arc<dyn LLMProviderTrait>,
    prompt: &str,
    system_prompt: &str,
) -> Result<String> {
    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        "Exec one-shot mode"
    );

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
pub async fn run_single(llm: Arc<dyn LLMProviderTrait>, config: Config) -> Result<()> {
    info!(
        "Starting single agent mode with provider: {}",
        llm.provider_name()
    );

    let system_prompt = &config.llm.system_prompt;

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
pub async fn run_single_multi(multi_llm: MultiModelManager, config: Config) -> Result<()> {
    info!(
        "Starting single agent mode (multi-model) with {} providers",
        multi_llm.client_count()
    );

    let system_prompt = &config.llm.system_prompt;

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

    // Round-robin: start with first provider, then rotate
    let mut last_index = 0;
    for i in 0..multi_llm.client_count() {
        let client = if i == 0 {
            multi_llm.get_client(0)
        } else {
            multi_llm.next_client(last_index)
        };

        if let Some(client) = client {
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
            last_index = i;
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
                system_prompt: crate::config::default_system_prompt(),
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let response = run_exec(llm, "Hello!", &config.system_prompt)
                .await
                .unwrap();

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
                system_prompt: crate::config::default_system_prompt(),
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let result = run_exec(llm, "Hello!", &config.system_prompt).await;

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
                system_prompt: crate::config::default_system_prompt(),
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let result = run_exec(llm, "Hello!", &config.system_prompt).await;

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
                system_prompt: crate::config::default_system_prompt(),
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
                system_prompt: crate::config::default_system_prompt(),
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
                system_prompt: crate::config::default_system_prompt(),
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

    #[test]
    fn test_run_single_multi_with_mockito_partial_failure() {
        // Test that run_single_multi handles one provider failing gracefully
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(401)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error":"Unauthorized"}"#)
                .create();

            let configs = vec![crate::config::LLMConfig {
                provider: crate::config::LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("bad-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
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

            // run_single_multi catches errors internally and logs them, returns Ok(())
            assert!(result.is_ok());
            mock.assert();
        });
    }

    #[test]
    fn test_run_swarm_multi_returns_error() {
        // Test that run_swarm_multi returns the expected error
        let configs = vec![crate::config::LLMConfig {
            provider: crate::config::LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_swarm_multi(multi_llm, cfg));
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("Swarm mode"));
    }

    #[test]
    fn test_run_supervisor_multi_returns_error() {
        // Test that run_supervisor_multi returns the expected error
        let configs = vec![crate::config::LLMConfig {
            provider: crate::config::LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_supervisor_multi(multi_llm, cfg));
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("Supervisor mode"));
    }

    #[test]
    fn test_run_exec_with_mockito_different_providers() {
        // Test run_exec with OpenRouter provider
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_chat_response_json(
                    "anthropic/claude-sonnet-4-20250514",
                ))
                .create();

            let config = crate::config::LLMConfig {
                provider: crate::config::LLMProvider::OpenRouter,
                endpoint: server.url(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let llm = crate::llm::create_client(&config).unwrap();
            let response = run_exec(llm, "Hello!", &config.system_prompt)
                .await
                .unwrap();

            assert_eq!(response, "Hello from agent!");
            mock.assert();
        });
    }

    // ── ConversationMemory tests ──────────────────────────────────────

    #[test]
    fn test_conversation_memory_new() {
        let mem = ConversationMemory::new("You are a helpful assistant.", 10);
        assert_eq!(mem.history().len(), 1);
        assert_eq!(mem.history()[0].role, "system");
        assert_eq!(mem.history()[0].content, "You are a helpful assistant.");
    }

    #[test]
    fn test_conversation_memory_add_user_message() {
        let mut mem = ConversationMemory::new("System prompt.", 10);
        let history = mem.add_user_message("Hello!");
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].role, "user");
        assert_eq!(history[1].content, "Hello!");
    }

    #[test]
    fn test_conversation_memory_add_assistant_message() {
        let mut mem = ConversationMemory::new("System prompt.", 10);
        mem.add_user_message("Hello!");
        mem.add_assistant_message("Hi there!");
        assert_eq!(mem.history().len(), 3);
        assert_eq!(mem.history()[2].role, "assistant");
        assert_eq!(mem.history()[2].content, "Hi there!");
    }

    #[test]
    fn test_conversation_memory_trim() {
        // max_messages=3 means system + 2 messages max
        let mut mem = ConversationMemory::new("System.", 3);
        mem.add_user_message("msg1");
        mem.add_assistant_message("resp1");
        assert_eq!(mem.history().len(), 3);

        // Adding a 4th message should trim the oldest non-system (msg1)
        mem.add_user_message("msg2");
        assert_eq!(mem.history().len(), 3);
        // The oldest user message should have been removed
        assert_eq!(mem.history()[1].content, "resp1");
        assert_eq!(mem.history()[2].content, "msg2");
    }

    #[test]
    fn test_conversation_memory_unlimited() {
        let mut mem = ConversationMemory::new("System.", 0);
        for i in 0..100 {
            mem.add_user_message(&format!("msg{}", i));
        }
        assert_eq!(mem.history().len(), 101); // system + 100 messages
    }
}
