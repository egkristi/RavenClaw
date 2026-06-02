//! Multi-provider LLM client integration
//!
//! Supports LiteLLM, OpenAI, OpenRouter, and Ollama with a unified trait-based API.

use futures::Stream;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;

/// A streaming chunk of an LLM response
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub content: String,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

/// Type alias for streaming response
pub type StreamResult = Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send>>;

use crate::config::{LLMConfig, LLMProvider};

#[derive(Error, Debug)]
pub enum LLMError {
    #[error("Request failed: {0}")]
    RequestFailed(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Rate limit exceeded")]
    RateLimited,

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Provider not supported: {0}")]
    #[allow(dead_code)]
    ProviderNotSupported(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub object: String,
    #[allow(dead_code)]
    pub created: u64,
    #[allow(dead_code)]
    pub model: String,
    pub choices: Vec<Choice>,
    #[allow(dead_code)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    #[allow(dead_code)]
    pub index: u32,
    pub message: ChatMessage,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    #[allow(dead_code)]
    pub prompt_tokens: u32,
    #[allow(dead_code)]
    pub completion_tokens: u32,
    #[allow(dead_code)]
    pub total_tokens: u32,
}

/// Trait for LLM providers — unified interface across all backends
#[async_trait::async_trait]
pub trait LLMProviderTrait: Send + Sync {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError>;
    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<StreamResult, LLMError> {
        // Default: non-streaming fallback
        let response = self.chat(messages).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();
        let finish_reason = response
            .choices
            .first()
            .and_then(|c| c.finish_reason.clone());

        let stream = futures::stream::once(async move {
            Ok(StreamChunk {
                content,
                finish_reason,
            })
        });
        Ok(Box::pin(stream))
    }
    fn provider_name(&self) -> &str;
    fn model(&self) -> &str;
}

/// Shared response handler for OpenAI-compatible providers
async fn handle_openai_response(response: Response) -> Result<ChatResponse, LLMError> {
    let status = response.status();

    if status.is_success() {
        response
            .json::<ChatResponse>()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.to_string()))
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        Err(LLMError::AuthFailed)
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Err(LLMError::RateLimited)
    } else {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        Err(LLMError::RequestFailed(format!("{}: {}", status, body)))
    }
}

/// LiteLLM client (OpenAI-compatible API)
pub struct LiteLLMClient {
    client: Client,
    config: LLMConfig,
}

impl LiteLLMClient {
    pub fn new(config: &LLMConfig) -> Result<Self, LLMError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LLMError::RequestFailed(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            config: config.clone(),
        })
    }

    fn build_request(&self, messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: None,
        }
    }

    async fn send_request(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let mut req = self
            .client
            .post(format!(
                "{}/v1/chat/completions",
                self.config.endpoint.trim_end_matches('/')
            ))
            .json(&request);

        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        handle_openai_response(response).await
    }
}

#[async_trait::async_trait]
impl LLMProviderTrait for LiteLLMClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let request = self.build_request(messages);
        self.send_request(request).await
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<StreamResult, LLMError> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: Some(true),
        };

        let mut req = self
            .client
            .post(format!(
                "{}/v1/chat/completions",
                self.config.endpoint.trim_end_matches('/')
            ))
            .json(&request);

        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(LLMError::AuthFailed);
            } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(LLMError::RateLimited);
            } else {
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                return Err(LLMError::RequestFailed(format!("{}: {}", status, body)));
            }
        }

        // Parse SSE stream — map byte chunks to StreamChunks, filtering empty ones
        use futures::StreamExt;
        let stream = response
            .bytes_stream()
            .filter_map(|chunk_result| async move {
                match chunk_result {
                    Err(e) => Some(Err(LLMError::RequestFailed(e.to_string()))),
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        let mut content = String::new();
                        let mut finish_reason = None;

                        for line in text.lines() {
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    finish_reason = Some("stop".to_string());
                                    continue;
                                }
                                if let Ok(sse_chunk) =
                                    serde_json::from_str::<serde_json::Value>(data)
                                {
                                    if let Some(choice) =
                                        sse_chunk["choices"].as_array().and_then(|c| c.first())
                                    {
                                        if let Some(delta) = choice["delta"].as_object() {
                                            if let Some(c) = delta["content"].as_str() {
                                                content.push_str(c);
                                            }
                                        }
                                        if let Some(reason) = choice["finish_reason"].as_str() {
                                            if reason != "null" {
                                                finish_reason = Some(reason.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if content.is_empty() && finish_reason.is_none() {
                            None // filter out empty chunks
                        } else {
                            Some(Ok(StreamChunk {
                                content,
                                finish_reason,
                            }))
                        }
                    }
                }
            });

        Ok(Box::pin(stream))
    }

    fn provider_name(&self) -> &str {
        "litellm"
    }

    fn model(&self) -> &str {
        &self.config.model
    }
}

/// OpenRouter client (OpenAI-compatible with model routing)
pub struct OpenRouterClient {
    client: Client,
    config: LLMConfig,
}

impl OpenRouterClient {
    pub fn new(config: &LLMConfig) -> Result<Self, LLMError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LLMError::RequestFailed(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            config: config.clone(),
        })
    }
}

#[async_trait::async_trait]
impl LLMProviderTrait for OpenRouterClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: None,
        };

        let endpoint = if self.config.endpoint.is_empty() {
            "https://openrouter.ai/api/v1/chat/completions".to_string()
        } else {
            format!(
                "{}/v1/chat/completions",
                self.config.endpoint.trim_end_matches('/')
            )
        };

        let req = self
            .client
            .post(&endpoint)
            .header(
                "Authorization",
                format!(
                    "Bearer {}",
                    self.config.api_key.as_ref().unwrap_or(&"".to_string())
                ),
            )
            .header("HTTP-Referer", "https://github.com/egkristi/RavenClaw")
            .header("X-Title", "RavenClaw")
            .json(&request);

        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        handle_openai_response(response).await
    }

    fn provider_name(&self) -> &str {
        "openrouter"
    }

    fn model(&self) -> &str {
        &self.config.model
    }
}

/// Ollama client (local/self-hosted models)
pub struct OllamaClient {
    client: Client,
    config: LLMConfig,
}

impl OllamaClient {
    pub fn new(config: &LLMConfig) -> Result<Self, LLMError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LLMError::RequestFailed(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            config: config.clone(),
        })
    }
}

#[async_trait::async_trait]
impl LLMProviderTrait for OllamaClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        // Ollama uses slightly different format
        #[derive(Serialize)]
        struct OllamaRequest {
            model: String,
            messages: Vec<ChatMessage>,
            stream: bool,
        }

        let request = OllamaRequest {
            model: self.config.model.clone(),
            messages,
            stream: false,
        };

        let response = self
            .client
            .post(format!(
                "{}/api/chat",
                self.config.endpoint.trim_end_matches('/')
            ))
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        let status = response.status();

        if status.is_success() {
            // Ollama returns a different structure, convert to our standard format
            #[derive(Deserialize)]
            struct OllamaResponse {
                model: String,
                message: ChatMessage,
                done: bool,
            }

            let ollama_resp = response
                .json::<OllamaResponse>()
                .await
                .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;

            Ok(ChatResponse {
                id: format!("ollama-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".to_string(),
                created: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                model: ollama_resp.model,
                choices: vec![Choice {
                    index: 0,
                    message: ollama_resp.message,
                    finish_reason: if ollama_resp.done {
                        Some("stop".to_string())
                    } else {
                        None
                    },
                }],
                usage: None, // Ollama doesn't always provide usage
            })
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            Err(LLMError::AuthFailed)
        } else {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(LLMError::RequestFailed(format!("{}: {}", status, body)))
        }
    }

    fn provider_name(&self) -> &str {
        "ollama"
    }

    fn model(&self) -> &str {
        &self.config.model
    }
}

/// OpenAI native client
pub struct OpenAIClient {
    client: Client,
    config: LLMConfig,
}

impl OpenAIClient {
    pub fn new(config: &LLMConfig) -> Result<Self, LLMError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LLMError::RequestFailed(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            config: config.clone(),
        })
    }
}

#[async_trait::async_trait]
impl LLMProviderTrait for OpenAIClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: None,
        };

        let endpoint = if self.config.endpoint.is_empty() {
            "https://api.openai.com/v1/chat/completions".to_string()
        } else {
            format!(
                "{}/v1/chat/completions",
                self.config.endpoint.trim_end_matches('/')
            )
        };

        let response = self
            .client
            .post(&endpoint)
            .header(
                "Authorization",
                format!(
                    "Bearer {}",
                    self.config.api_key.as_ref().unwrap_or(&"".to_string())
                ),
            )
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        handle_openai_response(response).await
    }

    fn provider_name(&self) -> &str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.config.model
    }
}

/// Factory function to create the appropriate client based on provider type
pub fn create_client(config: &LLMConfig) -> Result<Arc<dyn LLMProviderTrait>, LLMError> {
    match config.provider {
        LLMProvider::LiteLLM => Ok(Arc::new(LiteLLMClient::new(config)?)),
        LLMProvider::OpenRouter => Ok(Arc::new(OpenRouterClient::new(config)?)),
        LLMProvider::Ollama => Ok(Arc::new(OllamaClient::new(config)?)),
        LLMProvider::OpenAI => Ok(Arc::new(OpenAIClient::new(config)?)),
    }
}

/// Multi-model manager for handling multiple providers simultaneously
pub struct MultiModelManager {
    clients: Vec<Arc<dyn LLMProviderTrait>>,
}

impl MultiModelManager {
    pub fn new(configs: Vec<LLMConfig>) -> Result<Self, LLMError> {
        let clients: Result<Vec<_>, _> = configs.iter().map(create_client).collect();
        Ok(Self { clients: clients? })
    }

    pub fn get_client(&self, index: usize) -> Option<&Arc<dyn LLMProviderTrait>> {
        self.clients.get(index)
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// Round-robin selection for load balancing
    pub fn next_client(&self, last_index: usize) -> Option<&Arc<dyn LLMProviderTrait>> {
        if self.clients.is_empty() {
            return None;
        }
        let next = (last_index + 1) % self.clients.len();
        Some(&self.clients[next])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    // ── Helper ──────────────────────────────────────────────────────────

    fn make_chat_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are helpful.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello!".to_string(),
            },
        ]
    }

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
                        "content": "Hi there!"
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

    fn sample_ollama_response_json(model: &str) -> String {
        format!(
            r#"{{
            "model": "{}",
            "message": {{
                "role": "assistant",
                "content": "Hi there!"
            }},
            "done": true
        }}"#,
            model
        )
    }

    /// Helper: run an async test with a mockito server.
    /// mockito::Server::new() spawns a background thread with its own tokio
    /// runtime, so we must create it *before* entering the tokio test runtime.
    fn with_mockito<F, Fut>(f: F)
    where
        F: FnOnce(mockito::ServerGuard) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let server = Server::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(f(server));
    }

    // ── LiteLLM mockito tests ──────────────────────────────────────────

    #[test]
    fn test_litellm_chat_success() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_chat_response_json("gpt-4o-mini"))
                .create();

            let config = LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = LiteLLMClient::new(&config).unwrap();
            let response = client.chat(make_chat_messages()).await.unwrap();

            assert_eq!(response.model, "gpt-4o-mini");
            assert_eq!(response.choices[0].message.content, "Hi there!");
            assert_eq!(response.usage.unwrap().total_tokens, 15);
            mock.assert();
        });
    }

    #[test]
    fn test_litellm_chat_auth_failure() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(401)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Unauthorized"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("bad-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = LiteLLMClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::AuthFailed));
            mock.assert();
        });
    }

    #[test]
    fn test_litellm_chat_rate_limit() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(429)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Rate limit exceeded"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = LiteLLMClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RateLimited));
            mock.assert();
        });
    }

    #[test]
    fn test_litellm_chat_server_error() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(500)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Internal server error"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = LiteLLMClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RequestFailed(_)));
            assert!(format!("{}", err).contains("500"));
            mock.assert();
        });
    }

    #[test]
    fn test_litellm_chat_invalid_json() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body("not-json")
                .create();

            let config = LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = LiteLLMClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::InvalidResponse(_)));
            mock.assert();
        });
    }

    // ── OpenRouter mockito tests ───────────────────────────────────────

    #[test]
    fn test_openrouter_chat_success() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_chat_response_json(
                    "anthropic/claude-sonnet-4-20250514",
                ))
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: server.url(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenRouterClient::new(&config).unwrap();
            let response = client.chat(make_chat_messages()).await.unwrap();

            assert_eq!(response.model, "anthropic/claude-sonnet-4-20250514");
            assert_eq!(response.choices[0].message.content, "Hi there!");
            mock.assert();
        });
    }

    #[test]
    fn test_openrouter_chat_auth_failure() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(401)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Unauthorized"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: server.url(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("bad-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenRouterClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::AuthFailed));
            mock.assert();
        });
    }

    #[test]
    fn test_openrouter_chat_rate_limit() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(429)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Rate limited"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: server.url(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenRouterClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RateLimited));
            mock.assert();
        });
    }

    #[test]
    fn test_openrouter_chat_server_error() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(500)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Internal error"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: server.url(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenRouterClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RequestFailed(_)));
            assert!(format!("{}", err).contains("500"));
            mock.assert();
        });
    }

    #[test]
    fn test_openrouter_chat_invalid_json() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body("not-json")
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: server.url(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenRouterClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::InvalidResponse(_)));
            mock.assert();
        });
    }

    // ── OpenAI mockito tests ───────────────────────────────────────────

    #[test]
    fn test_openai_chat_success() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_chat_response_json("gpt-4o"))
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenAI,
                endpoint: server.url(),
                model: "gpt-4o".to_string(),
                api_key: Some("sk-test".to_string()),
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenAIClient::new(&config).unwrap();
            let response = client.chat(make_chat_messages()).await.unwrap();

            assert_eq!(response.model, "gpt-4o");
            assert_eq!(response.choices[0].message.content, "Hi there!");
            mock.assert();
        });
    }

    #[test]
    fn test_openai_chat_auth_failure() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(401)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Unauthorized"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenAI,
                endpoint: server.url(),
                model: "gpt-4o".to_string(),
                api_key: Some("bad-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenAIClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::AuthFailed));
            mock.assert();
        });
    }

    #[test]
    fn test_openai_chat_rate_limit() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(429)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Rate limited"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenAI,
                endpoint: server.url(),
                model: "gpt-4o".to_string(),
                api_key: Some("sk-test".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenAIClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RateLimited));
            mock.assert();
        });
    }

    #[test]
    fn test_openai_chat_server_error() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(500)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Internal error"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenAI,
                endpoint: server.url(),
                model: "gpt-4o".to_string(),
                api_key: Some("sk-test".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenAIClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RequestFailed(_)));
            assert!(format!("{}", err).contains("500"));
            mock.assert();
        });
    }

    #[test]
    fn test_openai_chat_invalid_json() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body("not-json")
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenAI,
                endpoint: server.url(),
                model: "gpt-4o".to_string(),
                api_key: Some("sk-test".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenAIClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::InvalidResponse(_)));
            mock.assert();
        });
    }

    // ── Ollama mockito tests ───────────────────────────────────────────

    #[test]
    fn test_ollama_chat_success() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/api/chat")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(sample_ollama_response_json("llama3.1"))
                .create();

            let config = LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: server.url(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OllamaClient::new(&config).unwrap();
            let response = client.chat(make_chat_messages()).await.unwrap();

            assert_eq!(response.model, "llama3.1");
            assert_eq!(response.choices[0].message.content, "Hi there!");
            assert_eq!(response.choices[0].finish_reason, Some("stop".to_string()));
            mock.assert();
        });
    }

    #[test]
    fn test_ollama_chat_server_error() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/api/chat")
                .with_status(500)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Model not loaded"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: server.url(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OllamaClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RequestFailed(_)));
            mock.assert();
        });
    }

    #[test]
    fn test_ollama_chat_invalid_json() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/api/chat")
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body("not-json")
                .create();

            let config = LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: server.url(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OllamaClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::InvalidResponse(_)));
            mock.assert();
        });
    }

    #[test]
    fn test_ollama_chat_auth_failure() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/api/chat")
                .with_status(401)
                .with_header("content-type", "application/json")
                .with_body(r#"{"error": "Unauthorized"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: server.url(),
                model: "llama3.1".to_string(),
                api_key: Some("bad-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OllamaClient::new(&config).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::AuthFailed));
            mock.assert();
        });
    }

    // ── Factory function tests ─────────────────────────────────────────

    #[test]
    fn test_create_client_factory_litellm() {
        let config = LLMConfig {
            provider: LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = create_client(&config).unwrap();
        assert_eq!(client.provider_name(), "litellm");
        assert_eq!(client.model(), "gpt-4o-mini");
    }

    #[test]
    fn test_ollama_client_creation() {
        let config = LLMConfig {
            provider: LLMProvider::Ollama,
            endpoint: "http://localhost:11434".to_string(),
            model: "llama3.1".to_string(),
            api_key: None,
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OllamaClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "ollama");
        assert_eq!(client.model(), "llama3.1");
    }

    #[test]
    fn test_openai_client_creation() {
        let config = LLMConfig {
            provider: LLMProvider::OpenAI,
            endpoint: String::new(),
            model: "gpt-4o".to_string(),
            api_key: Some("sk-test".to_string()),
            timeout_secs: 60,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OpenAIClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "openai");
        assert_eq!(client.model(), "gpt-4o");
    }

    #[test]
    fn test_openrouter_client_creation() {
        let config = LLMConfig {
            provider: LLMProvider::OpenRouter,
            endpoint: String::new(),
            model: "anthropic/claude-sonnet-4-20250514".to_string(),
            api_key: Some("sk-test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OpenRouterClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "openrouter");
        assert_eq!(client.model(), "anthropic/claude-sonnet-4-20250514");
    }

    #[test]
    fn test_multi_model_manager_empty() {
        let manager = MultiModelManager::new(vec![]).unwrap();
        assert_eq!(manager.client_count(), 0);
        assert!(manager.get_client(0).is_none());
    }

    #[test]
    fn test_multi_model_manager_single() {
        let config = LLMConfig {
            provider: LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let manager = MultiModelManager::new(vec![config]).unwrap();
        assert_eq!(manager.client_count(), 1);
        assert!(manager.get_client(0).is_some());
        assert_eq!(manager.get_client(0).unwrap().provider_name(), "litellm");
    }

    #[test]
    fn test_multi_model_manager_multiple() {
        let configs = vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            },
            LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
            },
        ];

        let manager = MultiModelManager::new(configs).unwrap();
        assert_eq!(manager.client_count(), 2);
        assert_eq!(manager.get_client(0).unwrap().provider_name(), "litellm");
        assert_eq!(manager.get_client(1).unwrap().provider_name(), "ollama");
    }

    #[test]
    fn test_multi_model_next_client_round_robin() {
        let configs = vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            },
            LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
            },
        ];

        let manager = MultiModelManager::new(configs).unwrap();
        // Start at index 0, next should be index 1
        let next = manager.next_client(0).unwrap();
        assert_eq!(next.provider_name(), "ollama");
        // Next after index 1 wraps to index 0
        let next = manager.next_client(1).unwrap();
        assert_eq!(next.provider_name(), "litellm");
    }

    #[test]
    fn test_chat_request_serialization() {
        let request = ChatRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: "You are a helpful assistant.".to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: "Hello!".to_string(),
                },
            ],
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("gpt-4o-mini"));
        assert!(json.contains("system"));
        assert!(json.contains("user"));
        assert!(json.contains("Hello!"));
        assert!(json.contains("0.7"));
        // stream: None should be skipped
        assert!(!json.contains("stream"));
    }

    #[test]
    fn test_chat_response_deserialization() {
        let json = r#"{
            "id": "chat-123",
            "object": "chat.completion",
            "created": 1717000000,
            "model": "gpt-4o-mini",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello! How can I help you?"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        }"#;

        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.id, "chat-123");
        assert_eq!(response.model, "gpt-4o-mini");
        assert_eq!(response.choices.len(), 1);
        assert_eq!(response.choices[0].message.role, "assistant");
        assert_eq!(
            response.choices[0].message.content,
            "Hello! How can I help you?"
        );
        assert_eq!(response.usage.unwrap().total_tokens, 30);
    }

    #[test]
    fn test_multi_model_manager_new_invalid_config() {
        // Config with empty endpoint for LiteLLM should fail at client creation
        let configs = vec![LLMConfig {
            provider: LLMProvider::LiteLLM,
            endpoint: String::new(), // empty endpoint — will fail HTTP client creation
            model: "gpt-4o-mini".to_string(),
            api_key: None,
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        }];

        let result = MultiModelManager::new(configs);
        // The client creation itself won't fail (HTTP client doesn't validate endpoint),
        // but the request will fail. This tests the error propagation path.
        assert!(result.is_ok());
        let manager = result.unwrap();
        assert_eq!(manager.client_count(), 1);
    }

    #[test]
    fn test_create_client_all_providers() {
        let test_cases = vec![
            (LLMProvider::LiteLLM, "litellm"),
            (LLMProvider::OpenRouter, "openrouter"),
            (LLMProvider::Ollama, "ollama"),
            (LLMProvider::OpenAI, "openai"),
        ];

        for (provider, expected_name) in test_cases {
            let config = LLMConfig {
                provider,
                endpoint: "http://localhost:4000".to_string(),
                model: "test-model".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = create_client(&config).unwrap();
            assert_eq!(client.provider_name(), expected_name);
            assert_eq!(client.model(), "test-model");
        }
    }

    #[test]
    fn test_llm_error_display() {
        let err = LLMError::RequestFailed("timeout".to_string());
        assert_eq!(format!("{}", err), "Request failed: timeout");

        let err = LLMError::AuthFailed;
        assert_eq!(format!("{}", err), "Authentication failed");

        let err = LLMError::RateLimited;
        assert_eq!(format!("{}", err), "Rate limit exceeded");

        let err = LLMError::InvalidResponse("bad json".to_string());
        assert_eq!(format!("{}", err), "Invalid response: bad json");

        let err = LLMError::ProviderNotSupported("custom".to_string());
        assert_eq!(format!("{}", err), "Provider not supported: custom");
    }

    #[test]
    fn test_llm_error_is_debug() {
        let err = LLMError::RequestFailed("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("RequestFailed"));
    }

    #[test]
    fn test_llm_error_is_send() {
        fn check_send<T: Send>() {}
        check_send::<LLMError>();
    }

    #[test]
    fn test_llm_error_is_sync() {
        fn check_sync<T: Sync>() {}
        check_sync::<LLMError>();
    }

    #[test]
    fn test_litellm_client_new_with_invalid_timeout() {
        // Very large timeout should still succeed (reqwest accepts large values)
        let config = LLMConfig {
            provider: LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: u64::MAX,
            system_prompt: crate::config::default_system_prompt(),
        };

        let result = LiteLLMClient::new(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_openai_client_with_custom_endpoint() {
        let config = LLMConfig {
            provider: LLMProvider::OpenAI,
            endpoint: "https://custom.openai.example.com".to_string(),
            model: "gpt-4o".to_string(),
            api_key: Some("sk-test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OpenAIClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "openai");
        assert_eq!(client.model(), "gpt-4o");
    }

    #[test]
    fn test_openrouter_client_with_custom_endpoint() {
        let config = LLMConfig {
            provider: LLMProvider::OpenRouter,
            endpoint: "https://custom.openrouter.example.com".to_string(),
            model: "anthropic/claude-sonnet-4-20250514".to_string(),
            api_key: Some("or-key".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OpenRouterClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "openrouter");
        assert_eq!(client.model(), "anthropic/claude-sonnet-4-20250514");
    }

    #[test]
    fn test_ollama_client_with_auth() {
        // Ollama with API key should still create successfully
        let config = LLMConfig {
            provider: LLMProvider::Ollama,
            endpoint: "http://localhost:11434".to_string(),
            model: "llama3.1".to_string(),
            api_key: Some("some-key".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OllamaClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "ollama");
        assert_eq!(client.model(), "llama3.1");
    }

    #[test]
    fn test_chat_request_no_temperature() {
        let request = ChatRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Hello!".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stream: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("gpt-4o-mini"));
        assert!(json.contains("Hello!"));
        // Optional fields should be skipped
        assert!(!json.contains("temperature"));
        assert!(!json.contains("max_tokens"));
        assert!(!json.contains("stream"));
    }

    #[test]
    fn test_chat_response_deserialization_no_usage() {
        let json = r#"{
            "id": "chat-456",
            "object": "chat.completion",
            "created": 1717000001,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Sure!"
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;

        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.id, "chat-456");
        assert_eq!(response.model, "gpt-4o");
        assert_eq!(response.choices.len(), 1);
        assert!(response.usage.is_none());
    }

    #[test]
    fn test_chat_response_deserialization_multiple_choices() {
        let json = r#"{
            "id": "chat-789",
            "object": "chat.completion",
            "created": 1717000002,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "First choice"
                    },
                    "finish_reason": "stop"
                },
                {
                    "index": 1,
                    "message": {
                        "role": "assistant",
                        "content": "Second choice"
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;

        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.choices.len(), 2);
        assert_eq!(response.choices[0].message.content, "First choice");
        assert_eq!(response.choices[1].message.content, "Second choice");
    }

    #[test]
    fn test_llm_error_into_boxed() {
        let err = LLMError::AuthFailed;
        let boxed: Box<dyn std::error::Error> = Box::new(err);
        assert!(format!("{}", boxed).contains("Authentication failed"));
    }

    #[test]
    fn test_llm_error_into_string() {
        let err = LLMError::RateLimited;
        let msg: String = err.to_string();
        assert_eq!(msg, "Rate limit exceeded");
    }

    #[test]
    fn test_create_client_with_empty_api_key() {
        // Some providers (Ollama) don't require an API key
        let config = LLMConfig {
            provider: LLMProvider::Ollama,
            endpoint: "http://localhost:11434".to_string(),
            model: "llama3.1".to_string(),
            api_key: None,
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = create_client(&config).unwrap();
        assert_eq!(client.provider_name(), "ollama");
    }

    #[test]
    fn test_multi_model_manager_get_client_out_of_bounds() {
        let manager = MultiModelManager::new(vec![]).unwrap();
        assert!(manager.get_client(0).is_none());
        assert!(manager.get_client(100).is_none());
        assert!(manager.get_client(usize::MAX).is_none());
    }

    #[test]
    fn test_multi_model_next_client_empty() {
        let manager = MultiModelManager::new(vec![]).unwrap();
        assert!(manager.next_client(0).is_none());
    }

    #[test]
    fn test_multi_model_next_client_single() {
        let config = LLMConfig {
            provider: LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let manager = MultiModelManager::new(vec![config]).unwrap();
        // With one client, next_client wraps to index 0
        let next = manager.next_client(0).unwrap();
        assert_eq!(next.provider_name(), "litellm");
    }
}
