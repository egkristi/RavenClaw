//! Multi-provider LLM client integration
//!
//! Supports LiteLLM, OpenAI, OpenRouter, Ollama, and Anthropic with a unified trait-based API.
//! v0.5: Unified OpenAI-compatible client, retry/fallback, token budgets.

use futures::Stream;
use rand::Rng;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use uuid::Uuid;

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

/// Provider type for OpenAI-compatible APIs (v0.5 unified client)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAICompatibleProvider {
    LiteLLM,
    OpenAI,
    OpenRouter,
}

impl OpenAICompatibleProvider {
    /// Get default endpoint for provider
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            OpenAICompatibleProvider::LiteLLM => "http://localhost:4000",
            OpenAICompatibleProvider::OpenAI => "https://api.openai.com",
            OpenAICompatibleProvider::OpenRouter => "https://openrouter.ai",
        }
    }

    /// Get provider name string
    pub fn name(&self) -> &'static str {
        match self {
            OpenAICompatibleProvider::LiteLLM => "litellm",
            OpenAICompatibleProvider::OpenAI => "openai",
            OpenAICompatibleProvider::OpenRouter => "openrouter",
        }
    }

    /// Check if provider requires special headers
    pub fn requires_custom_headers(&self) -> bool {
        matches!(self, OpenAICompatibleProvider::OpenRouter)
    }
}

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

    #[error("Token budget exceeded")]
    TokenBudgetExceeded,

    #[error("All providers failed after retries")]
    AllProvidersFailed,

    #[error("Circuit breaker open for provider: {0}")]
    CircuitBreakerOpen(String),
}

/// Retry configuration for LLM requests (v0.5)
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries (default: 3)
    pub max_retries: u32,
    /// Base delay for exponential backoff in ms (default: 100)
    pub base_delay_ms: u64,
    /// Maximum delay in ms (default: 10000)
    pub max_delay_ms: u64,
    /// Jitter factor (0.0-1.0, default: 0.5)
    pub jitter: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 100,
            max_delay_ms: 10000,
            jitter: 0.5,
        }
    }
}

impl RetryConfig {
    /// Calculate delay with exponential backoff and jitter
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        use rand::Rng;
        let exp = 2u64.pow(attempt);
        let base = self.base_delay_ms * exp;
        let capped = base.min(self.max_delay_ms);
        let jitter_range = (capped as f64) * self.jitter;
        let jitter = rand::thread_rng().gen_range(-jitter_range..=jitter_range) as u64;
        let delay = capped.saturating_add(jitter).max(self.base_delay_ms);
        Duration::from_millis(delay)
    }
}

/// Circuit breaker state (v0.5)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Circuit breaker for provider resilience (v0.5)
#[derive(Debug)]
pub struct CircuitBreaker {
    pub state: CircuitState,
    pub failure_count: u32,
    pub last_failure_time: Option<std::time::Instant>,
    pub open_duration: Duration,
}

impl CircuitBreaker {
    pub fn new(open_duration_secs: u64) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure_time: None,
            open_duration: Duration::from_secs(open_duration_secs),
        }
    }

    pub fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitState::Closed;
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure_time = Some(std::time::Instant::now());
        if self.failure_count >= 5 {
            self.state = CircuitState::Open;
        }
    }

    pub fn can_execute(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(last) = self.last_failure_time {
                    if last.elapsed() >= self.open_duration {
                        self.state = CircuitState::HalfOpen;
                        return true;
                    }
                }
                false
            }
            CircuitState::HalfOpen => true,
        }
    }
}

/// Token budget tracker (v0.5)
#[derive(Debug, Clone)]
pub struct TokenBudget {
    /// Maximum tokens allowed
    pub max_tokens: u32,
    /// Tokens used so far
    pub used_tokens: u32,
    /// Cost per 1K tokens (USD)
    pub cost_per_1k: f64,
}

impl TokenBudget {
    pub fn new(max_tokens: u32, cost_per_1k: f64) -> Self {
        Self {
            max_tokens,
            used_tokens: 0,
            cost_per_1k,
        }
    }

    pub fn remaining(&self) -> u32 {
        self.max_tokens.saturating_sub(self.used_tokens)
    }

    pub fn can_spend(&self, tokens: u32) -> bool {
        self.remaining() >= tokens
    }

    pub fn record_usage(&mut self, tokens: u32) {
        self.used_tokens = self.used_tokens.saturating_add(tokens);
    }

    pub fn estimated_cost(&self) -> f64 {
        (self.used_tokens as f64 / 1000.0) * self.cost_per_1k
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
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
pub struct ToolCallResponse {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    #[allow(dead_code)]
    pub index: u32,
    pub message: ChatMessage,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
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

/// Unified OpenAI-compatible client (v0.5)
/// Replaces separate LiteLLM, OpenAI, and OpenRouter clients
pub struct OpenAICompatibleClient {
    client: Client,
    config: LLMConfig,
    provider: OpenAICompatibleProvider,
    retry_config: RetryConfig,
    circuit_breaker: std::sync::Mutex<CircuitBreaker>,
}

impl OpenAICompatibleClient {
    pub fn new(config: &LLMConfig, provider: OpenAICompatibleProvider) -> Result<Self, LLMError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LLMError::RequestFailed(format!("Failed to create HTTP client: {}", e)))?;

        let retry_config = RetryConfig {
            max_retries: config.retry_max,
            base_delay_ms: config.retry_base_delay_ms,
            max_delay_ms: config.retry_max_delay_ms,
            jitter: 0.5,
        };

        Ok(Self {
            client,
            config: config.clone(),
            provider,
            retry_config,
            circuit_breaker: std::sync::Mutex::new(CircuitBreaker::new(30)),
        })
    }

    /// Send request with retry logic (v0.5)
    async fn send_request_with_retry(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let mut last_error = None;

        for attempt in 0..=self.retry_config.max_retries {
            // Check circuit breaker
            {
                let mut cb = self.circuit_breaker.lock().map_err(|_| {
                    LLMError::RequestFailed("Circuit breaker lock poisoned".to_string())
                })?;
                if !cb.can_execute() {
                    return Err(LLMError::CircuitBreakerOpen(self.provider.name().to_string()));
                }
            }

            let result = self.send_request_inner(request.clone()).await;

            match result {
                Ok(response) => {
                    // Record success in circuit breaker
                    {
                        let mut cb = self.circuit_breaker.lock().map_err(|_| {
                            LLMError::RequestFailed("Circuit breaker lock poisoned".to_string())
                        })?;
                        cb.record_success();
                    }
                    return Ok(response);
                }
                Err(e) => {
                    // Record failure
                    {
                        let mut cb = self.circuit_breaker.lock().map_err(|_| {
                            LLMError::RequestFailed("Circuit breaker lock poisoned".to_string())
                        })?;
                        cb.record_failure();
                    }

                    last_error = Some(e);

                    // Don't retry on auth failures
                    if matches!(last_error, Some(LLMError::AuthFailed)) {
                        return Err(last_error.unwrap());
                    }

                    // Wait before retry (if not last attempt)
                    if attempt < self.retry_config.max_retries {
                        let delay = self.retry_config.delay_for_attempt(attempt);
                        sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or(LLMError::AllProvidersFailed))
    }

    /// Inner send request (no retry)
    async fn send_request_inner(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let req = self.apply_headers(self.client.post(&self.endpoint()).json(&request));

        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        handle_openai_response(response).await
    }

    fn build_request(&self, messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: None,
            tools: None,
            tool_choice: None,
        }
    }

    fn endpoint(&self) -> String {
        let base = if self.config.endpoint.is_empty() {
            self.provider.default_endpoint()
        } else {
            &self.config.endpoint
        };
        format!("{}/v1/chat/completions", base.trim_end_matches('/'))
    }

    fn apply_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        // OpenRouter-specific headers
        if self.provider.requires_custom_headers() {
            req = req
                .header("HTTP-Referer", "https://github.com/egkristi/RavenClaw")
                .header("X-Title", "RavenClaw");
        }

        req
    }

    async fn send_request(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let req = self.apply_headers(self.client.post(&self.endpoint()).json(&request));

        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        handle_openai_response(response).await
    }
}

#[async_trait::async_trait]
impl LLMProviderTrait for OpenAICompatibleClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let request = self.build_request(messages);
        self.send_request_with_retry(request).await
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<StreamResult, LLMError> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: Some(true),
            tools: None,
            tool_choice: None,
        };

        let req = self.apply_headers(self.client.post(&self.endpoint()).json(&request));

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
                            None
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
        self.provider.name()
    }

    fn model(&self) -> &str {
        &self.config.model
    }
}

/// LiteLLM client (OpenAI-compatible API) — DEPRECATED in v0.5, kept for backward compatibility
#[deprecated(since = "0.5.0", note = "Use OpenAICompatibleClient with OpenAICompatibleProvider::LiteLLM instead")]
pub struct LiteLLMClient {
    client: Client,
    config: LLMConfig,
}

#[allow(deprecated)]
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
}

#[allow(deprecated)]
#[async_trait::async_trait]
impl LLMProviderTrait for LiteLLMClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: None,
            tools: None,
            tool_choice: None,
        };

        let req = self
            .client
            .post(format!(
                "{}/v1/chat/completions",
                self.config.endpoint.trim_end_matches('/')
            ))
            .json(&request);

        let req = if let Some(ref key) = self.config.api_key {
            req.header("Authorization", format!("Bearer {}", key))
        } else {
            req
        };

        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        handle_openai_response(response).await
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<StreamResult, LLMError> {
        // Delegate to unified implementation
        let unified = OpenAICompatibleClient::new(&self.config, OpenAICompatibleProvider::LiteLLM)?;
        unified.chat_stream(messages).await
    }

    fn provider_name(&self) -> &str {
        "litellm"
    }

    fn model(&self) -> &str {
        &self.config.model
    }
}

/// OpenRouter client (OpenAI-compatible with model routing) — DEPRECATED in v0.5
#[deprecated(since = "0.5.0", note = "Use OpenAICompatibleClient with OpenAICompatibleProvider::OpenRouter instead")]
pub struct OpenRouterClient {
    client: Client,
    config: LLMConfig,
}

#[allow(deprecated)]
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

#[allow(deprecated)]
#[async_trait::async_trait]
impl LLMProviderTrait for OpenRouterClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let unified = OpenAICompatibleClient::new(&self.config, OpenAICompatibleProvider::OpenRouter)?;
        unified.chat(messages).await
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<StreamResult, LLMError> {
        let unified = OpenAICompatibleClient::new(&self.config, OpenAICompatibleProvider::OpenRouter)?;
        unified.chat_stream(messages).await
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
                    tool_calls: None,
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

/// OpenAI native client — DEPRECATED in v0.5
#[deprecated(since = "0.5.0", note = "Use OpenAICompatibleClient with OpenAICompatibleProvider::OpenAI instead")]
pub struct OpenAIClient {
    client: Client,
    config: LLMConfig,
}

#[allow(deprecated)]
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

#[allow(deprecated)]
#[async_trait::async_trait]
impl LLMProviderTrait for OpenAIClient {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let unified = OpenAICompatibleClient::new(&self.config, OpenAICompatibleProvider::OpenAI)?;
        unified.chat(messages).await
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<StreamResult, LLMError> {
        let unified = OpenAICompatibleClient::new(&self.config, OpenAICompatibleProvider::OpenAI)?;
        unified.chat_stream(messages).await
    }

    fn provider_name(&self) -> &str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.config.model
    }
}

/// Factory function to create the appropriate client based on provider type (v0.5 unified)
pub fn create_client(config: &LLMConfig) -> Result<Arc<dyn LLMProviderTrait>, LLMError> {
    match config.provider {
        LLMProvider::LiteLLM => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::LiteLLM)?;
            Ok(Arc::new(unified))
        }
        LLMProvider::OpenRouter => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::OpenRouter)?;
            Ok(Arc::new(unified))
        }
        LLMProvider::Ollama => Ok(Arc::new(OllamaClient::new(config)?)),
        LLMProvider::OpenAI => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::OpenAI)?;
            Ok(Arc::new(unified))
        }
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

/// Provider fallback chain (v0.5) — tries providers in order until one succeeds
pub struct ProviderFallbackChain {
    configs: Vec<LLMConfig>,
    token_budget: Option<TokenBudget>,
}

impl ProviderFallbackChain {
    pub fn new(configs: Vec<LLMConfig>) -> Self {
        Self {
            configs,
            token_budget: None,
        }
    }

    pub fn with_token_budget(mut self, budget: TokenBudget) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Execute with fallback — tries each provider in order until success
    pub async fn chat_with_fallback(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let mut last_error = None;

        for (i, config) in self.configs.iter().enumerate() {
            let client = match create_client(config) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to create client for provider {}: {}", config.provider.clone().into(), e);
                    last_error = Some(e);
                    continue;
                }
            };

            // Check token budget before making request
            if let Some(ref budget) = self.token_budget {
                // Estimate ~500 tokens for typical request
                if !budget.can_spend(500) {
                    return Err(LLMError::TokenBudgetExceeded);
                }
            }

            match client.chat(messages.clone()).await {
                Ok(response) => {
                    // Record token usage if available
                    if let Some(ref mut budget) = self.token_budget {
                        if let Some(usage) = &response.usage {
                            budget.record_usage(usage.total_tokens);
                        }
                    }
                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!("Provider {} failed: {}", i, e);
                    last_error = Some(e);
                    // Continue to next provider
                }
            }
        }

        Err(last_error.unwrap_or(LLMError::AllProvidersFailed))
    }

    /// Get provider names in chain
    pub fn provider_names(&self) -> Vec<String> {
        self.configs.iter()
            .map(|c| format!("{:?}", c.provider))
            .collect()
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

    // ── OpenAICompatibleClient tests (v0.5 unified) ─────────────────────

    #[test]
    fn test_openai_compatible_provider_defaults() {
        assert_eq!(OpenAICompatibleProvider::LiteLLM.default_endpoint(), "http://localhost:4000");
        assert_eq!(OpenAICompatibleProvider::OpenAI.default_endpoint(), "https://api.openai.com");
        assert_eq!(OpenAICompatibleProvider::OpenRouter.default_endpoint(), "https://openrouter.ai");
    }

    #[test]
    fn test_openai_compatible_provider_names() {
        assert_eq!(OpenAICompatibleProvider::LiteLLM.name(), "litellm");
        assert_eq!(OpenAICompatibleProvider::OpenAI.name(), "openai");
        assert_eq!(OpenAICompatibleProvider::OpenRouter.name(), "openrouter");
    }

    #[test]
    fn test_openai_compatible_requires_custom_headers() {
        assert!(!OpenAICompatibleProvider::LiteLLM.requires_custom_headers());
        assert!(OpenAICompatibleProvider::OpenRouter.requires_custom_headers());
        assert!(!OpenAICompatibleProvider::OpenAI.requires_custom_headers());
    }

    #[test]
    fn test_openai_compatible_client_new() {
        let config = LLMConfig {
            provider: LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test-key".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM);
        assert!(client.is_ok());
        assert_eq!(client.unwrap().provider_name(), "litellm");
    }

    #[test]
    fn test_openai_compatible_client_endpoint() {
        // Test with custom endpoint
        let config = LLMConfig {
            provider: LLMProvider::OpenAI,
            endpoint: "https://custom.api.example.com".to_string(),
            model: "gpt-4o".to_string(),
            api_key: Some("test-key".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
        };

        let client = OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
        // Endpoint is private, but we can verify provider name
        assert_eq!(client.provider_name(), "openai");
    }

    #[test]
    fn test_openai_compatible_client_chat_success() {
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

            let client = OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
            let response = client.chat(make_chat_messages()).await.unwrap();

            assert_eq!(response.model, "gpt-4o-mini");
            assert_eq!(response.choices[0].message.content, "Hi there!");
            mock.assert();
        });
    }

    #[test]
    fn test_openai_compatible_client_auth_failure() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(401)
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

            let client = OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::AuthFailed));
            mock.assert();
        });
    }

    #[test]
    fn test_openai_compatible_client_rate_limit() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .with_status(429)
                .with_body(r#"{"error": "Rate limited"}"#)
                .create();

            let config = LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: server.url(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
            let err = client.chat(make_chat_messages()).await.unwrap_err();

            assert!(matches!(err, LLMError::RateLimited));
            mock.assert();
        });
    }

    #[test]
    fn test_openrouter_client_uses_custom_headers() {
        with_mockito(|mut server| async move {
            let mock = server
                .mock("POST", "/v1/chat/completions")
                .match_header("HTTP-Referer", "https://github.com/egkristi/RavenClaw")
                .match_header("X-Title", "RavenClaw")
                .with_status(200)
                .with_body(sample_chat_response_json("claude-sonnet-4"))
                .create();

            let config = LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: server.url(),
                model: "claude-sonnet-4".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
            };

            let client = OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
            let _ = client.chat(make_chat_messages()).await.unwrap();
            mock.assert();
        });
    }

    // ── Retry & Circuit Breaker tests (v0.5) ───────────────────────────

    #[test]
    fn test_retry_config_delay_calculation() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 100,
            max_delay_ms: 10000,
            jitter: 0.0, // No jitter for predictable testing
        };

        // Exponential backoff: 100, 200, 400, 800...
        assert_eq!(config.delay_for_attempt(0).as_millis(), 100);
        assert_eq!(config.delay_for_attempt(1).as_millis(), 200);
        assert_eq!(config.delay_for_attempt(2).as_millis(), 400);
    }

    #[test]
    fn test_retry_config_max_delay_cap() {
        let config = RetryConfig {
            max_retries: 10,
            base_delay_ms: 100,
            max_delay_ms: 1000,
            jitter: 0.0,
        };

        // Should cap at max_delay_ms
        assert!(config.delay_for_attempt(10).as_millis() <= 1000);
    }

    #[test]
    fn test_circuit_breaker_state_transitions() {
        let mut cb = CircuitBreaker::new(30);

        // Initially closed
        assert_eq!(cb.state, CircuitState::Closed);
        assert!(cb.can_execute());

        // Record 5 failures → should open
        for _ in 0..5 {
            cb.record_failure();
        }
        assert_eq!(cb.state, CircuitState::Open);
        assert!(!cb.can_execute());
    }

    #[test]
    fn test_circuit_breaker_success_resets() {
        let mut cb = CircuitBreaker::new(30);

        // Record 3 failures
        for _ in 0..3 {
            cb.record_failure();
        }
        assert_eq!(cb.failure_count, 3);

        // Record success → should reset
        cb.record_success();
        assert_eq!(cb.failure_count, 0);
        assert_eq!(cb.state, CircuitState::Closed);
    }

    #[test]
    fn test_token_budget_tracking() {
        let mut budget = TokenBudget::new(1000, 0.002); // $0.002 per 1K tokens

        assert_eq!(budget.remaining(), 1000);
        assert!(budget.can_spend(500));

        budget.record_usage(300);
        assert_eq!(budget.remaining(), 700);
        assert!(budget.can_spend(500));

        budget.record_usage(500);
        assert_eq!(budget.remaining(), 200);
        assert!(!budget.can_spend(500));

        // Estimated cost: 800 tokens / 1000 * $0.002 = $0.0016
        assert!((budget.estimated_cost() - 0.0016).abs() < 0.0001);
    }

    #[test]
    fn test_provider_fallback_chain_creation() {
        let configs = vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o".to_string(),
                api_key: Some("key1".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
        ];

        let chain = ProviderFallbackChain::new(configs);
        assert_eq!(chain.provider_names(), vec!["LiteLLM", "Ollama"]);
    }

    // ── LiteLLM mockito tests (legacy, deprecated) ─────────────────────

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
            tools: None,
            tool_choice: None,
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
            tools: None,
            tool_choice: None,
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
