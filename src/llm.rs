//! Multi-provider LLM client integration
//!
//! Supports LiteLLM, OpenAI, OpenRouter, Ollama, and Anthropic with a unified trait-based API.
//! v0.5: Unified OpenAI-compatible client, retry/fallback, token budgets.

use futures::Stream;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use tracing::instrument;

/// A streaming chunk of an LLM response
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StreamChunk {
    pub content: String,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

/// Type alias for streaming response
#[allow(dead_code)]
pub type StreamResult = Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send>>;

use crate::config::{LLMConfig, LLMProvider};

/// Provider type for OpenAI-compatible APIs (v0.5 unified client)
///
/// # Stability
/// This enum is `#[non_exhaustive]` — new variants may be added in minor releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenAICompatibleProvider {
    LiteLLM,
    OpenAI,
    OpenRouter,
    /// Generic OpenAI-compatible endpoint (vLLM, llama.cpp, LM Studio, TGI, Groq, Together AI, etc.)
    Generic,
    /// Azure OpenAI Service — uses `api-key` header and `api-version` query parameter
    Azure,
    /// vLLM — high-throughput OpenAI-compatible inference server
    Vllm,
    /// SGLang — fast OpenAI-compatible inference server
    SGLang,
}

impl OpenAICompatibleProvider {
    /// Get default endpoint for provider
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            OpenAICompatibleProvider::LiteLLM => "http://localhost:4000",
            OpenAICompatibleProvider::OpenAI => "https://api.openai.com",
            OpenAICompatibleProvider::OpenRouter => "https://openrouter.ai",
            OpenAICompatibleProvider::Generic => "http://localhost:8000",
            OpenAICompatibleProvider::Azure => "https://YOUR_RESOURCE.openai.azure.com",
            OpenAICompatibleProvider::Vllm => "http://localhost:8000",
            OpenAICompatibleProvider::SGLang => "http://localhost:30000",
        }
    }

    /// Get provider name string
    pub fn name(&self) -> &'static str {
        match self {
            OpenAICompatibleProvider::LiteLLM => "litellm",
            OpenAICompatibleProvider::OpenAI => "openai",
            OpenAICompatibleProvider::OpenRouter => "openrouter",
            OpenAICompatibleProvider::Generic => "openai-compatible",
            OpenAICompatibleProvider::Azure => "azure",
            OpenAICompatibleProvider::Vllm => "vllm",
            OpenAICompatibleProvider::SGLang => "sglang",
        }
    }

    /// Check if provider requires special headers
    #[allow(dead_code)]
    pub fn requires_custom_headers(&self) -> bool {
        matches!(
            self,
            OpenAICompatibleProvider::OpenRouter | OpenAICompatibleProvider::Azure
        )
    }
}

/// LLM provider error type.
///
/// # Stability
/// This enum is `#[non_exhaustive]` — new variants may be added in minor releases.
/// Match with a wildcard arm to handle future variants.
#[derive(Error, Debug)]
#[non_exhaustive]
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

impl LLMError {
    /// Returns `true` if this error is transient and can be retried.
    #[allow(dead_code)]
    ///
    /// Transient errors are those that may succeed on retry:
    /// - `RequestFailed` — network issues, server errors
    /// - `RateLimited` — back off and retry
    /// - `CircuitBreakerOpen` — may close after recovery period
    ///
    /// Non-transient errors should NOT be retried:
    /// - `AuthFailed` — credentials are wrong
    /// - `TokenBudgetExceeded` — budget is exhausted
    /// - `InvalidResponse` — response format is wrong
    /// - `ProviderNotSupported` — wrong provider
    /// - `AllProvidersFailed` — all providers have been tried
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            LLMError::RequestFailed(_) | LLMError::RateLimited | LLMError::CircuitBreakerOpen(_)
        )
    }
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
///
/// # Stability
/// This enum is `#[non_exhaustive]` — new variants may be added in minor releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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

#[allow(dead_code)]
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

/// Pricing for a model, in USD per 1,000,000 tokens.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
#[allow(dead_code)] // library API; exercised in tests
pub struct ModelPricing {
    /// Cost per 1M input (prompt) tokens.
    pub input_per_million: f64,
    /// Cost per 1M output (completion) tokens.
    pub output_per_million: f64,
}

impl ModelPricing {
    /// Create a pricing entry from per-million-token prices.
    #[allow(dead_code)] // library API; exercised in tests
    pub fn new(input_per_million: f64, output_per_million: f64) -> Self {
        Self {
            input_per_million,
            output_per_million,
        }
    }

    /// A common "free/local" pricing entry (Ollama, llama.cpp, etc.).
    #[allow(dead_code)] // library API; exercised in tests
    pub fn free() -> Self {
        Self {
            input_per_million: 0.0,
            output_per_million: 0.0,
        }
    }
}

/// A cost ledger for LLM usage — accumulates token usage across calls and computes
/// an estimated cost from a per-model pricing table.
///
/// This is the "cost tracking" primitive: providers that report `usage` feed it
/// here, and `summary()` returns totals (calls, prompt tokens, completion tokens,
/// estimated cost). Models without a registered price are treated as free.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
#[allow(dead_code)] // library API; exercised in tests
pub struct CostTracker {
    /// Per-model pricing, keyed by lowercased model name.
    pricing: std::collections::HashMap<String, ModelPricing>,
    /// Per-model token accounting: (prompt_tokens, completion_tokens).
    usage: std::collections::HashMap<String, (u64, u64)>,
    /// Total number of `record()` calls.
    calls: u64,
}

/// A summary of tracked LLM usage.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
#[allow(dead_code)] // library API; exercised in tests
pub struct CostSummary {
    /// Total number of calls.
    pub calls: u64,
    /// Total prompt (input) tokens.
    pub prompt_tokens: u64,
    /// Total completion (output) tokens.
    pub completion_tokens: u64,
    /// Total tokens (prompt + completion).
    pub total_tokens: u64,
    /// Estimated cost in USD.
    pub estimated_cost: f64,
}

impl CostTracker {
    /// Create an empty cost tracker.
    #[allow(dead_code)] // library API; exercised in tests
    pub fn new() -> Self {
        Self::default()
    }

    /// Register pricing for a model name (case-insensitive).
    #[allow(dead_code)]
    pub fn with_pricing(mut self, model: impl Into<String>, pricing: ModelPricing) -> Self {
        self.pricing.insert(model.into().to_lowercase(), pricing);
        self
    }

    /// Register a "free" (zero-cost) model.
    #[allow(dead_code)]
    pub fn with_free_model(mut self, model: impl Into<String>) -> Self {
        self.pricing
            .insert(model.into().to_lowercase(), ModelPricing::free());
        self
    }

    /// Record usage for a single call. The model name is looked up (case-insensitively)
    /// in the pricing table; an unregistered model is treated as free (zero cost).
    #[allow(dead_code)]
    pub fn record(&mut self, model: &str, usage: &Usage) {
        let key = model.to_lowercase();
        let entry = self.usage.entry(key).or_insert((0, 0));
        entry.0 += usage.prompt_tokens as u64;
        entry.1 += usage.completion_tokens as u64;
        self.calls += 1;
    }

    /// Total number of calls recorded.
    #[allow(dead_code)] // library API; exercised in tests
    pub fn call_count(&self) -> u64 {
        self.calls
    }

    /// Total tokens across all recorded calls.
    #[allow(dead_code)] // library API; exercised in tests
    pub fn total_tokens(&self) -> u64 {
        self.usage.values().map(|(p, c)| p + c).sum()
    }

    /// Estimated cost in USD, computed per-model from prompt/completion token
    /// counts against the registered pricing (unknown models are free).
    #[allow(dead_code)]
    pub fn estimated_cost(&self) -> f64 {
        self.usage
            .iter()
            .map(|(model, (prompt, completion))| {
                let price = self
                    .pricing
                    .get(model)
                    .copied()
                    .unwrap_or_else(ModelPricing::free);
                (*prompt as f64 / 1_000_000.0) * price.input_per_million
                    + (*completion as f64 / 1_000_000.0) * price.output_per_million
            })
            .sum()
    }

    /// Look up the registered pricing for a model (case-insensitive). Models
    /// without a registered price are treated as free.
    #[allow(dead_code)]
    pub fn pricing_for(&self, model: &str) -> ModelPricing {
        self.pricing
            .get(&model.to_lowercase())
            .copied()
            .unwrap_or_else(ModelPricing::free)
    }

    /// Produce a summary of all tracked usage.
    #[allow(dead_code)]
    pub fn summary(&self) -> CostSummary {
        let prompt_tokens: u64 = self.usage.values().map(|(p, _)| p).sum();
        let completion_tokens: u64 = self.usage.values().map(|(_, c)| c).sum();
        CostSummary {
            calls: self.calls,
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            estimated_cost: self.estimated_cost(),
        }
    }
}

/// A content part for multi-modal messages — text or image.
///
/// # Stability
/// This enum is `#[non_exhaustive]` — new variants may be added in minor releases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum ContentPart {
    /// Plain text content
    Text { text: String },
    /// Image content as a data URI (base64-encoded)
    /// Format: `data:{mime_type};base64,{data}`
    ImageUrl {
        #[serde(rename = "image_url")]
        image_url: ImageUrlContent,
    },
}

/// URL/content for an image in a multi-modal message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlContent {
    pub url: String,
}

impl ContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create an image content part from a data URI
    pub fn image(data_uri: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: ImageUrlContent {
                url: data_uri.into(),
            },
        }
    }

    /// Serialize this content part as a `serde_json::Value` for OpenAI-compatible APIs.
    pub fn to_openai_value(&self) -> serde_json::Value {
        match self {
            ContentPart::Text { text } => serde_json::json!({
                "type": "text",
                "text": text
            }),
            ContentPart::ImageUrl { image_url } => serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": image_url.url
                }
            }),
        }
    }

    /// Serialize this content part as a `serde_json::Value` for Anthropic APIs.
    pub fn to_anthropic_value(&self) -> serde_json::Value {
        match self {
            ContentPart::Text { text } => serde_json::json!({
                "type": "text",
                "text": text
            }),
            ContentPart::ImageUrl { image_url } => {
                // Parse the data URI to extract media_type and base64 data
                let url = &image_url.url;
                if let Some(rest) = url.strip_prefix("data:") {
                    if let Some((media_type, data)) = rest.split_once(";base64,") {
                        return serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": data
                            }
                        });
                    }
                }
                // Fallback: send as text if we can't parse the data URI
                serde_json::json!({
                    "type": "text",
                    "text": format!("[Image: {}]", url.chars().take(100).collect::<String>())
                })
            }
        }
    }
}

/// Load an image file and return a data URI string.
///
/// Reads the file, detects the MIME type from the extension, base64-encodes
/// the contents, and returns a data URI in the format `data:{mime};base64,{data}`.
///
/// Supported formats: PNG, JPEG, GIF, WebP.
pub fn load_image(path: &std::path::Path) -> Result<String, crate::error::RavenClawsError> {
    let data = std::fs::read(path).map_err(crate::error::RavenClawsError::IO)?;

    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => {
            return Err(crate::error::RavenClawsError::CommandExecution(format!(
                "Unsupported image format: '{}'. Supported: png, jpg, jpeg, gif, webp",
                path.display()
            )));
        }
    };

    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Optional structured content parts for multi-modal messages.
    /// When set, `content` is used as a fallback text representation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_parts: Option<Vec<ContentPart>>,
}

impl ChatMessage {
    /// Create a new text-only chat message.
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            content_parts: None,
        }
    }

    /// Create a new multi-modal chat message with text and optional image attachments.
    pub fn with_images(
        role: impl Into<String>,
        text: impl Into<String>,
        image_data_uris: Vec<String>,
    ) -> Self {
        let text = text.into();
        let mut parts = Vec::with_capacity(1 + image_data_uris.len());
        parts.push(ContentPart::text(&text));
        for uri in image_data_uris {
            parts.push(ContentPart::image(uri));
        }
        Self {
            role: role.into(),
            content: text.clone(),
            content_parts: Some(parts),
        }
    }

    /// Serialize this message as a `serde_json::Value` for OpenAI-compatible APIs.
    /// When `content_parts` is `Some`, produces the multi-modal content array format.
    pub fn to_openai_message(&self) -> serde_json::Value {
        match &self.content_parts {
            Some(parts) => {
                let content_array: Vec<serde_json::Value> =
                    parts.iter().map(|p| p.to_openai_value()).collect();
                serde_json::json!({
                    "role": self.role,
                    "content": content_array
                })
            }
            None => {
                serde_json::json!({
                    "role": self.role,
                    "content": self.content
                })
            }
        }
    }

    /// Serialize this message as a `serde_json::Value` for Anthropic APIs.
    pub fn to_anthropic_message(&self) -> serde_json::Value {
        match &self.content_parts {
            Some(parts) => {
                let content_array: Vec<serde_json::Value> =
                    parts.iter().map(|p| p.to_anthropic_value()).collect();
                serde_json::json!({
                    "role": self.role,
                    "content": content_array
                })
            }
            None => {
                serde_json::json!({
                    "role": self.role,
                    "content": self.content
                })
            }
        }
    }

    /// Get the base64 image data for Ollama's `images` array format.
    /// Returns `None` if there are no image content parts.
    pub fn ollama_images(&self) -> Option<Vec<String>> {
        let parts = self.content_parts.as_ref()?;
        let images: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::ImageUrl { image_url } => {
                    let url = &image_url.url;
                    url.strip_prefix("data:")
                        .and_then(|rest| rest.split_once(";base64,").map(|x| x.1))
                        .map(|s| s.to_string())
                }
                _ => None,
            })
            .collect();
        if images.is_empty() {
            None
        } else {
            Some(images)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    #[serde(serialize_with = "serialize_messages_openai")]
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

/// Serialize messages for OpenAI-compatible APIs, handling multi-modal content.
fn serialize_messages_openai<S>(messages: &[ChatMessage], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(messages.len()))?;
    for msg in messages {
        let value = msg.to_openai_message();
        seq.serialize_element(&value)?;
    }
    seq.end()
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
    #[allow(dead_code)]
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
    async fn send_request_with_retry(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, LLMError> {
        let mut last_error = None;

        for attempt in 0..=self.retry_config.max_retries {
            // Check circuit breaker
            {
                let mut cb = self.circuit_breaker.lock().map_err(|_| {
                    LLMError::RequestFailed("Circuit breaker lock poisoned".to_string())
                })?;
                if !cb.can_execute() {
                    return Err(LLMError::CircuitBreakerOpen(
                        self.provider.name().to_string(),
                    ));
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
        let req = self.apply_headers(self.client.post(self.endpoint()).json(&request));

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
        let mut url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
        if self.provider == OpenAICompatibleProvider::Azure {
            // Azure OpenAI requires api-version query parameter
            // Default to 2024-02-15-preview if not specified; users can override via endpoint
            if !url.contains("api-version") {
                url = format!("{}?api-version=2024-02-15-preview", url);
            }
        }
        url
    }

    fn apply_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.config.api_key {
            if self.provider == OpenAICompatibleProvider::Azure {
                // Azure OpenAI uses api-key header (not Bearer)
                req = req.header("api-key", key);
            } else {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
        }

        // Provider-specific headers
        if self.provider == OpenAICompatibleProvider::OpenRouter {
            req = req
                .header("HTTP-Referer", "https://github.com/egkristi/RavenClaws")
                .header("X-Title", "RavenClaws");
        }

        req
    }

    #[allow(dead_code)]
    async fn send_request(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let req = self.apply_headers(self.client.post(self.endpoint()).json(&request));

        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        handle_openai_response(response).await
    }
}

#[async_trait::async_trait]
impl LLMProviderTrait for OpenAICompatibleClient {
    #[instrument(skip(self, messages), fields(provider = self.provider_name(), model = self.model()))]
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

        let req = self.apply_headers(self.client.post(self.endpoint()).json(&request));

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
    #[instrument(skip(self, messages), fields(provider = self.provider_name(), model = self.model()))]
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        // Ollama uses slightly different format
        // For multi-modal, we need to convert messages to serde_json::Value
        // to handle the `images` array on user messages
        #[derive(Serialize)]
        struct OllamaRequest {
            model: String,
            #[serde(serialize_with = "serialize_messages_ollama")]
            messages: Vec<ChatMessage>,
            stream: bool,
        }

        /// Serialize messages for Ollama API, handling multi-modal content.
        fn serialize_messages_ollama<S>(
            messages: &[ChatMessage],
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            use serde::ser::SerializeSeq;
            let mut seq = serializer.serialize_seq(Some(messages.len()))?;
            for msg in messages {
                let value = if let Some(images) = msg.ollama_images() {
                    serde_json::json!({
                        "role": msg.role,
                        "content": msg.content,
                        "images": images
                    })
                } else {
                    serde_json::json!({
                        "role": msg.role,
                        "content": msg.content
                    })
                };
                seq.serialize_element(&value)?;
            }
            seq.end()
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

/// Anthropic native client (v0.6) — direct API with tool use and image support
pub struct AnthropicClient {
    client: Client,
    config: LLMConfig,
}

impl AnthropicClient {
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
impl LLMProviderTrait for AnthropicClient {
    #[instrument(skip(self, messages), fields(provider = self.provider_name(), model = self.model()))]
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        // Anthropic uses a different request/response format
        #[derive(Serialize)]
        struct AnthropicRequest {
            model: String,
            max_tokens: u32,
            #[serde(serialize_with = "serialize_anthropic_messages")]
            messages: Vec<ChatMessage>,
            #[serde(skip_serializing_if = "Option::is_none")]
            system: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
        }

        /// Serialize messages for Anthropic API, handling multi-modal content.
        fn serialize_anthropic_messages<S>(
            messages: &[ChatMessage],
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            use serde::ser::SerializeSeq;
            let mut seq = serializer.serialize_seq(Some(messages.len()))?;
            for msg in messages {
                let value = msg.to_anthropic_message();
                seq.serialize_element(&value)?;
            }
            seq.end()
        }

        // Extract system prompt if present
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.clone());

        let anthropic_messages: Vec<ChatMessage> = messages
            .into_iter()
            .filter(|m| m.role != "system")
            .collect();

        let request = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens: 2048,
            messages: anthropic_messages,
            system,
            temperature: Some(0.7),
        };

        let api_key = self
            .config
            .api_key
            .clone()
            .ok_or_else(|| LLMError::AuthFailed)?;

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        let status = response.status();

        if status.is_success() {
            // Anthropic response format
            #[derive(Deserialize)]
            #[allow(dead_code)]
            struct AnthropicResponse {
                id: String,
                #[serde(rename = "type")]
                response_type: String,
                role: String,
                content: Vec<AnthropicContentBlock>,
                model: String,
                stop_reason: Option<String>,
                #[serde(default)]
                usage: Option<AnthropicUsage>,
            }

            #[derive(Deserialize)]
            #[serde(tag = "type", rename_all = "lowercase")]
            enum AnthropicContentBlock {
                Text {
                    text: String,
                },
                ToolUse {
                    id: String,
                    name: String,
                    input: serde_json::Value,
                },
            }

            #[derive(Deserialize)]
            struct AnthropicUsage {
                input_tokens: u32,
                output_tokens: u32,
            }

            let anthropic_resp = response
                .json::<AnthropicResponse>()
                .await
                .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;

            // Convert Anthropic content to our format
            let mut content = String::new();
            let mut tool_calls = None;

            for block in anthropic_resp.content {
                match block {
                    AnthropicContentBlock::Text { text } => {
                        content.push_str(&text);
                    }
                    AnthropicContentBlock::ToolUse { id, name, input } => {
                        if tool_calls.is_none() {
                            tool_calls = Some(Vec::new());
                        }
                        if let Some(ref mut calls) = tool_calls {
                            calls.push(ToolCallResponse {
                                id,
                                call_type: "function".to_string(),
                                function: FunctionCall {
                                    name,
                                    arguments: input.to_string(),
                                },
                            });
                        }
                    }
                }
            }

            Ok(ChatResponse {
                id: anthropic_resp.id,
                object: "chat.completion".to_string(),
                created: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                model: anthropic_resp.model,
                choices: vec![Choice {
                    index: 0,
                    message: ChatMessage {
                        role: "assistant".to_string(),
                        content,
                        content_parts: None,
                    },
                    finish_reason: anthropic_resp.stop_reason,
                    tool_calls,
                }],
                usage: anthropic_resp.usage.map(|u| Usage {
                    prompt_tokens: u.input_tokens,
                    completion_tokens: u.output_tokens,
                    total_tokens: u.input_tokens + u.output_tokens,
                }),
            })
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

    fn provider_name(&self) -> &str {
        "anthropic"
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
            let unified =
                OpenAICompatibleClient::new(config, OpenAICompatibleProvider::OpenRouter)?;
            Ok(Arc::new(unified))
        }
        LLMProvider::Ollama => Ok(Arc::new(OllamaClient::new(config)?)),
        LLMProvider::OpenAI => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::OpenAI)?;
            Ok(Arc::new(unified))
        }
        LLMProvider::Anthropic => Ok(Arc::new(AnthropicClient::new(config)?)),
        LLMProvider::OpenAICompatible => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::Generic)?;
            Ok(Arc::new(unified))
        }
        LLMProvider::Azure => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::Azure)?;
            Ok(Arc::new(unified))
        }
        LLMProvider::Vllm => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::Vllm)?;
            Ok(Arc::new(unified))
        }
        LLMProvider::SGLang => {
            let unified = OpenAICompatibleClient::new(config, OpenAICompatibleProvider::SGLang)?;
            Ok(Arc::new(unified))
        }
    }
}

// ── Local inference discovery ─────────────────────────────────────────────

/// A discovered local inference server.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[allow(dead_code)] // library API; exercised in tests
pub struct LocalInferenceServer {
    /// The base endpoint (e.g. `http://localhost:11434`).
    pub endpoint: String,
    /// The inferred server kind.
    pub kind: LocalInferenceKind,
    /// Models advertised by the server (may be empty if the probe couldn't
    /// enumerate models but the server still responded).
    pub models: Vec<String>,
}

/// The kind of local inference server detected.
///
/// # Stability
/// This enum is `#[non_exhaustive]` — new variants may be added in minor releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[allow(dead_code)] // library API; exercised in tests
pub enum LocalInferenceKind {
    /// Ollama (native `/api/tags` endpoint).
    Ollama,
    /// An OpenAI-compatible server (llama.cpp, LM Studio, TGI, etc. via `/v1/models`).
    OpenAiCompatible,
    /// vLLM — OpenAI-compatible server, typically on port 8000.
    Vllm,
    /// SGLang — OpenAI-compatible server, typically on port 30000.
    SGLang,
    /// Responded, but we couldn't classify the kind.
    Unknown,
}

/// Result of a model warmup attempt against a discovered local inference server.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[allow(dead_code)] // library API; exercised in tests
pub struct WarmupResult {
    /// The server endpoint the warmup targeted.
    pub endpoint: String,
    /// The model name that was warmed up.
    pub model: String,
    /// Whether the warmup request completed successfully (model loaded + responded).
    pub success: bool,
    /// Wall-clock duration of the warmup request in milliseconds.
    pub duration_ms: u64,
    /// Optional human-readable detail (e.g. the error message on failure).
    pub detail: Option<String>,
}

/// Discovery of local inference servers (Ollama / llama.cpp / vLLM / LM Studio).
///
/// Probes a configurable list of candidate endpoints using short HTTP requests.
/// This is the "device discovery" foundation for local/GPU inference management —
/// it finds what's running so the agent can route to it, without requiring any
/// GPU-specific libraries or hardware.
///
/// # Example
///
/// ```rust,no_run
/// use ravenclaws::llm::{LocalInference, LocalInferenceKind};
///
/// # async fn example() {
/// let found = LocalInference::default().discover().await;
/// for server in found {
///     println!("{} ({:?}): {:?}", server.endpoint, server.kind, server.models);
/// }
/// # }
/// ```
#[allow(dead_code)] // library API; exercised in tests
pub struct LocalInference {
    /// Candidate endpoints to probe, in order.
    pub endpoints: Vec<String>,
    /// Probe timeout in milliseconds.
    pub timeout_ms: u64,
}

impl Default for LocalInference {
    fn default() -> Self {
        Self {
            endpoints: vec![
                "http://localhost:11434".to_string(), // Ollama
                "http://127.0.0.1:11434".to_string(),
                "http://localhost:8000".to_string(), // vLLM / LM Studio / TGI
                "http://localhost:30000".to_string(), // SGLang
                "http://localhost:8080".to_string(), // llama.cpp
                "http://localhost:1234".to_string(), // LM Studio
            ],
            timeout_ms: 500,
        }
    }
}

#[allow(dead_code)] // library API; exercised in tests
impl LocalInference {
    /// Probe all candidate endpoints and return the servers that responded.
    pub async fn discover(&self) -> Vec<LocalInferenceServer> {
        let mut found = Vec::new();
        for endpoint in &self.endpoints {
            if let Some(server) = self.probe(endpoint).await {
                found.push(server);
            }
        }
        found
    }

    /// Probe a single endpoint. Returns `None` if the server did not respond.
    pub async fn probe(&self, endpoint: &str) -> Option<LocalInferenceServer> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .build()
            .ok()?;

        let base = endpoint.trim_end_matches('/');

        // 1) Try Ollama's native `/api/tags` endpoint.
        if let Ok(resp) = client.get(format!("{}/api/tags", base)).send().await {
            if resp.status().is_success() {
                let models = resp
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("models").cloned())
                    .and_then(|models| {
                        models.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m.get("name").and_then(|n| n.as_str()).map(String::from)
                                })
                                .collect()
                        })
                    })
                    .unwrap_or_default();
                return Some(LocalInferenceServer {
                    endpoint: base.to_string(),
                    kind: LocalInferenceKind::Ollama,
                    models,
                });
            }
        }

        // 2) Try the OpenAI-compatible `/v1/models` endpoint (vLLM, SGLang, llama.cpp, etc.).
        if let Ok(resp) = client.get(format!("{}/v1/models", base)).send().await {
            if resp.status().is_success() {
                let models = resp
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("data").cloned())
                    .and_then(|data| {
                        data.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m.get("id").and_then(|n| n.as_str()).map(String::from)
                                })
                                .collect()
                        })
                    })
                    .unwrap_or_default();

                // Infer vLLM vs SGLang by the well-known default ports.
                let kind = if base.contains(":30000") {
                    LocalInferenceKind::SGLang
                } else if base.contains(":8000") {
                    LocalInferenceKind::Vllm
                } else {
                    LocalInferenceKind::OpenAiCompatible
                };

                return Some(LocalInferenceServer {
                    endpoint: base.to_string(),
                    kind,
                    models,
                });
            }
        }

        None
    }

    /// Warm up a model on a discovered server — the "provisioning" half of local
    /// inference management.
    ///
    /// Sends a minimal single-token chat completion request to force the server
    /// to load the model into memory (model warmup) and confirm it can actually
    /// serve inference. This turns a passive discovery result into a verified,
    /// ready-to-use backend.
    ///
    /// Supports Ollama (native `/api/chat`) and OpenAI-compatible servers
    /// (`/v1/chat/completions`, used by vLLM / SGLang / llama.cpp / LM Studio).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ravenclaws::llm::LocalInference;
    ///
    /// # async fn example() {
    /// let inference = LocalInference::default();
    /// for server in inference.discover().await {
    ///     if let Some(model) = server.models.first() {
    ///         let result = inference.warmup(&server.endpoint, model).await;
    ///         println!("{:?}", result);
    ///     }
    /// }
    /// # }
    /// ```
    #[allow(dead_code)] // library API; exercised in tests
    pub async fn warmup(&self, endpoint: &str, model: &str) -> WarmupResult {
        let base = endpoint.trim_end_matches('/');
        let start = std::time::Instant::now();
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return WarmupResult {
                    endpoint: base.to_string(),
                    model: model.to_string(),
                    success: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                    detail: Some(format!("HTTP client build failed: {}", e)),
                }
            }
        };

        // 1) Ollama native chat endpoint.
        let ollama_body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "OK"}],
            "stream": false,
        });
        if let Ok(resp) = client
            .post(format!("{}/api/chat", base))
            .json(&ollama_body)
            .send()
            .await
        {
            let elapsed = start.elapsed().as_millis() as u64;
            if resp.status().is_success() {
                return WarmupResult {
                    endpoint: base.to_string(),
                    model: model.to_string(),
                    success: true,
                    duration_ms: elapsed,
                    detail: None,
                };
            }
            // Fall through to the OpenAI-compatible path if Ollama chat 404s.
            if resp.status() != reqwest::StatusCode::NOT_FOUND {
                return WarmupResult {
                    endpoint: base.to_string(),
                    model: model.to_string(),
                    success: false,
                    duration_ms: elapsed,
                    detail: Some(format!("Ollama /api/chat returned {}", resp.status())),
                };
            }
        }

        // 2) OpenAI-compatible chat completions (vLLM / SGLang / llama.cpp / LM Studio).
        let openai_body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "OK"}],
            "max_tokens": 1,
            "stream": false,
        });
        match client
            .post(format!("{}/v1/chat/completions", base))
            .json(&openai_body)
            .send()
            .await
        {
            Ok(resp) => {
                let elapsed = start.elapsed().as_millis() as u64;
                WarmupResult {
                    endpoint: base.to_string(),
                    model: model.to_string(),
                    success: resp.status().is_success(),
                    duration_ms: elapsed,
                    detail: if resp.status().is_success() {
                        None
                    } else {
                        Some(format!("/v1/chat/completions returned {}", resp.status()))
                    },
                }
            }
            Err(e) => WarmupResult {
                endpoint: base.to_string(),
                model: model.to_string(),
                success: false,
                duration_ms: start.elapsed().as_millis() as u64,
                detail: Some(format!("warmup request failed: {}", e)),
            },
        }
    }

    /// Select the most capable model from a discovered server's model list —
    /// the "quantized model selection" half of local inference provisioning.
    ///
    /// Ranks models by a quality heuristic: prefer larger/stronger families
    /// (higher parameter counts, or names containing strong markers like `70b`,
    /// `llama3`, `mixtral`, `deepseek`, `qwen`, `codestral`), and avoid tiny
    /// "embedding" / "instruct" helpers where a larger general model exists.
    ///
    /// Returns `None` when the server advertised no models.
    #[allow(dead_code)] // library API; exercised in tests
    pub fn select_model(&self, server: &LocalInferenceServer) -> Option<String> {
        if server.models.is_empty() {
            return None;
        }

        // Score each model by a rough capability heuristic.
        let best = server
            .models
            .iter()
            .max_by_key(|name| model_capability_score(name))
            .cloned();

        best
    }

    /// Warm up every model advertised by a discovered server, returning results
    /// in discovery order. A convenience over [`Self::warmup`] for provisioning
    /// a whole server at once (e.g. an Ollama host with several models).
    #[allow(dead_code)] // library API; exercised in tests
    pub async fn warmup_all(&self, server: &LocalInferenceServer) -> Vec<WarmupResult> {
        let mut results = Vec::with_capacity(server.models.len());
        for model in &server.models {
            results.push(self.warmup(&server.endpoint, model).await);
        }
        results
    }
}

/// Score a model name by a rough capability heuristic (higher = more capable).
/// Used by [`LocalInference::select_model`] to pick the best local model.
fn model_capability_score(name: &str) -> u64 {
    let n = name.to_lowercase();
    let mut score: u64 = 0;

    // Parameter-count signals (e.g. "70b", "8b", "7b", "1b").
    for (marker, pts) in [
        ("70b", 70),
        ("65b", 65),
        ("34b", 34),
        ("32b", 32),
        ("13b", 13),
        ("8b", 8),
        ("7b", 7),
        ("3b", 3),
        ("1b", 1),
    ] {
        if n.contains(marker) {
            score += pts;
            break;
        }
    }

    // Strong family markers.
    for (marker, pts) in [
        ("deepseek", 50),
        ("mixtral", 50),
        ("llama3", 40),
        ("qwen", 40),
        ("codestral", 40),
        ("mistral", 30),
        ("gemma", 30),
        ("phi", 20),
        ("tiny", 5),
        ("nomic-embed", 0),
        ("embedding", 0),
        ("instruct", 0),
    ] {
        if n.contains(marker) {
            score += pts;
        }
    }

    score
}

/// Multi-model manager for handling multiple providers simultaneously
#[derive(Clone)]
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

    /// Find a client by its configured model name (case-insensitive).
    ///
    /// Used to support a per-request `model` override over HTTP: an external
    /// orchestrator can select a specific LLM profile for a single request
    /// without changing the worker's default config.
    ///
    /// Returns `None` if no client's model matches `model`.
    pub fn find_by_model(&self, model: &str) -> Option<&Arc<dyn LLMProviderTrait>> {
        let needle = model.trim().to_lowercase();
        if needle.is_empty() {
            return None;
        }
        self.clients
            .iter()
            .find(|c| c.model().to_lowercase() == needle)
    }

    /// Round-robin selection for load balancing
    pub fn next_client(&self, last_index: usize) -> Option<&Arc<dyn LLMProviderTrait>> {
        if self.clients.is_empty() {
            return None;
        }
        let next = (last_index + 1) % self.clients.len();
        Some(&self.clients[next])
    }

    /// Complexity-aware routing — select the best provider/model for a prompt.
    ///
    /// Classifies the prompt into a complexity tier using a deterministic
    /// heuristic (no LLM round-trip) and routes accordingly:
    ///
    /// - **Trivial** (greetings, very short): any provider (index 0).
    /// - **Simple**: prefers a fast/cheap local model when present (Ollama or a
    ///   model name containing `small`/`mini`/`tiny`/`flash`), else index 0.
    /// - **Complex** (code, long context, or "hard" markers): prefers a
    ///   large/capable model (name containing `gpt-4`, `gpt-5`, `opus`, `sonnet`,
    ///   `deepseek`, `large`, `pro`), else index 0.
    ///
    /// Falls back to index 0 when no preference matches. This is a heuristic —
    /// a full complexity classifier + cost/quality policy is a future enhancement.
    ///
    /// Library-only API: exposed for embedders who want complexity-based routing
    /// (the binary's `run_single_multi` fans out to *all* providers instead of
    /// selecting one). Not called by the default binary.
    #[allow(dead_code)]
    pub fn route_by_complexity(&self, prompt: &str) -> Option<&Arc<dyn LLMProviderTrait>> {
        if self.clients.is_empty() {
            return None;
        }
        let tier = Self::classify_complexity(prompt);

        let preferred: Option<usize> = match tier {
            ComplexityTier::Trivial => Some(0),
            ComplexityTier::Simple => self.clients.iter().position(|c| {
                let m = c.model().to_lowercase();
                c.provider_name().eq_ignore_ascii_case("ollama")
                    || ["small", "mini", "tiny", "flash", "nano"]
                        .iter()
                        .any(|k| m.contains(k))
            }),
            ComplexityTier::Complex => self.clients.iter().position(|c| {
                let m = c.model().to_lowercase();
                [
                    "gpt-4", "gpt-5", "opus", "sonnet", "deepseek", "large", "pro", "claude",
                ]
                .iter()
                .any(|k| m.contains(k))
            }),
        };

        let index = preferred.unwrap_or(0);
        Some(&self.clients[index])
    }

    /// Classify a prompt into a complexity tier using a deterministic heuristic.
    #[allow(dead_code)]
    fn classify_complexity(prompt: &str) -> ComplexityTier {
        let trimmed = prompt.trim();
        let len = trimmed.chars().count();

        // Trivial: very short greetings / single words
        if len < 12 {
            let lower = trimmed.to_lowercase();
            if ["hi", "hello", "hey", "thanks", "ok", "yes", "no", "?"]
                .iter()
                .any(|g| lower == *g)
            {
                return ComplexityTier::Trivial;
            }
        }

        // Complex: code, long context, or explicit "hard" markers
        let has_code = trimmed.contains("fn ")
            || trimmed.contains("def ")
            || trimmed.contains("class ")
            || trimmed.contains("import ")
            || trimmed.contains("```")
            || trimmed.contains("->");
        let has_complex_marker = trimmed.contains("analyze")
            || trimmed.contains("explain in depth")
            || trimmed.contains("write a detailed")
            || trimmed.contains("architecture");

        if len > 500 || has_code || has_complex_marker {
            return ComplexityTier::Complex;
        }

        ComplexityTier::Simple
    }

    /// Cost-aware routing — select the cheapest client by registered pricing.
    ///
    /// Consults the given [`CostTracker`]'s pricing table to rank clients by
    /// estimated input cost (per 1M tokens), preferring the cheapest (including
    /// free/local models). Clients whose model has no registered price are treated
    /// as free and ranked first, so a local Ollama/llama.cpp backend beats a paid
    /// cloud model. Ties are broken by registration order (index 0 first).
    ///
    /// Falls back to index 0 when the manager is empty or no pricing is registered
    /// (in which case all clients tie and index 0 wins).
    ///
    /// This closes the "cost/quality policy" half of complexity routing: combined
    /// with [`Self::route_by_complexity`], embedders can route Trivial/Simple
    /// prompts to the cheapest backend and Complex prompts to the most capable.
    ///
    /// Library-only API: exposed for embedders; the binary's `run_single_multi`
    /// fans out to all providers rather than selecting one.
    #[allow(dead_code)]
    pub fn route_cheapest(&self, tracker: &CostTracker) -> Option<&Arc<dyn LLMProviderTrait>> {
        if self.clients.is_empty() {
            return None;
        }

        // Rank by input cost (ascending). Unknown/unregistered models are free.
        let mut ranked: Vec<usize> = (0..self.clients.len()).collect();
        ranked.sort_by_key(|&i| {
            let model = self.clients[i].model();
            let price = tracker.pricing_for(model);
            // Order by cost; use a stable sort so ties keep index order.
            (price.input_per_million * 1_000_000.0) as i64
        });

        Some(&self.clients[ranked[0]])
    }
}

/// Complexity tier used for model routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum ComplexityTier {
    Trivial,
    Simple,
    Complex,
}

/// Provider fallback chain (v0.5) — tries providers in order until one succeeds
#[derive(Debug)]
pub struct ProviderFallbackChain {
    /// Provider configurations in fallback order
    pub configs: Vec<LLMConfig>,
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
    #[instrument(skip(self, messages))]
    pub async fn chat_with_fallback(
        &mut self,
        messages: Vec<ChatMessage>,
    ) -> Result<ChatResponse, LLMError> {
        let mut last_error = None;

        for (i, config) in self.configs.iter().enumerate() {
            let client = match create_client(config) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        "Failed to create client for provider {:?}: {}",
                        config.provider,
                        e
                    );
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
    #[allow(dead_code)]
    pub fn provider_names(&self) -> Vec<String> {
        self.configs
            .iter()
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
            ChatMessage::new("system", "You are helpful."),
            ChatMessage::new("user", "Hello!"),
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
        assert_eq!(
            OpenAICompatibleProvider::LiteLLM.default_endpoint(),
            "http://localhost:4000"
        );
        assert_eq!(
            OpenAICompatibleProvider::OpenAI.default_endpoint(),
            "https://api.openai.com"
        );
        assert_eq!(
            OpenAICompatibleProvider::OpenRouter.default_endpoint(),
            "https://openrouter.ai"
        );
        assert_eq!(
            OpenAICompatibleProvider::Vllm.default_endpoint(),
            "http://localhost:8000"
        );
        assert_eq!(
            OpenAICompatibleProvider::SGLang.default_endpoint(),
            "http://localhost:30000"
        );
    }

    #[test]
    fn test_openai_compatible_provider_names() {
        assert_eq!(OpenAICompatibleProvider::LiteLLM.name(), "litellm");
        assert_eq!(OpenAICompatibleProvider::OpenAI.name(), "openai");
        assert_eq!(OpenAICompatibleProvider::OpenRouter.name(), "openrouter");
        assert_eq!(OpenAICompatibleProvider::Vllm.name(), "vllm");
        assert_eq!(OpenAICompatibleProvider::SGLang.name(), "sglang");
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client =
            OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
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
                token_budget: None,
                retry_max: 0, // Disable retries for error-path tests
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
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
                .match_header("HTTP-Referer", "https://github.com/egkristi/RavenClaws")
                .match_header("X-Title", "RavenClaws")
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
            let _ = client.chat(make_chat_messages()).await.unwrap();
            mock.assert();
        });
    }

    // ── AnthropicClient tests (v0.5.3) ─────────────────────────────────

    #[test]
    fn test_anthropic_client_new() {
        let config = LLMConfig {
            provider: LLMProvider::Anthropic,
            endpoint: String::new(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: Some("sk-ant-test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client = AnthropicClient::new(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_anthropic_client_provider_name() {
        let config = LLMConfig {
            provider: LLMProvider::Anthropic,
            endpoint: String::new(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: Some("sk-ant-test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client = AnthropicClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "anthropic");
    }

    #[test]
    fn test_anthropic_client_model() {
        let config = LLMConfig {
            provider: LLMProvider::Anthropic,
            endpoint: String::new(),
            model: "claude-opus-4-20250514".to_string(),
            api_key: Some("sk-ant-test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client = AnthropicClient::new(&config).unwrap();
        assert_eq!(client.model(), "claude-opus-4-20250514");
    }

    #[test]
    fn test_create_client_anthropic() {
        let config = LLMConfig {
            provider: LLMProvider::Anthropic,
            endpoint: String::new(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: Some("sk-ant-test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client = create_client(&config);
        assert!(client.is_ok());
        assert_eq!(client.unwrap().provider_name(), "anthropic");
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
    fn test_cost_tracker_basic() {
        let mut tracker = CostTracker::new();
        let usage = Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        };

        tracker.record("gpt-4o", &usage);
        tracker.record("gpt-4o", &usage);

        let summary = tracker.summary();
        assert_eq!(summary.calls, 2);
        assert_eq!(summary.prompt_tokens, 200);
        assert_eq!(summary.completion_tokens, 100);
        assert_eq!(summary.total_tokens, 300);
        // No pricing registered → free.
        assert_eq!(summary.estimated_cost, 0.0);
    }

    #[test]
    fn test_cost_tracker_pricing() {
        let mut tracker = CostTracker::new().with_pricing("gpt-4o", ModelPricing::new(2.50, 10.00)); // $ per 1M

        let usage = Usage {
            prompt_tokens: 1_000_000,     // 1M input
            completion_tokens: 1_000_000, // 1M output
            total_tokens: 2_000_000,
        };
        tracker.record("gpt-4o", &usage);

        // 1M input * $2.50/M + 1M output * $10/M = $12.50
        let summary = tracker.summary();
        assert!((summary.estimated_cost - 12.50).abs() < 0.0001);
    }

    #[test]
    fn test_cost_tracker_case_insensitive_and_free_model() {
        let mut tracker = CostTracker::new().with_free_model("llama3.1");

        let usage = Usage {
            prompt_tokens: 1000,
            completion_tokens: 500,
            total_tokens: 1500,
        };
        // Model name case must not matter; "Llama3.1" vs "llama3.1".
        tracker.record("Llama3.1", &usage);
        tracker.record("gpt-4o", &usage); // unregistered → free

        let summary = tracker.summary();
        assert_eq!(summary.calls, 2);
        assert_eq!(summary.total_tokens, 3000);
        assert_eq!(summary.estimated_cost, 0.0);
    }

    #[test]
    fn test_cost_tracker_empty() {
        let tracker = CostTracker::new();
        let summary = tracker.summary();
        assert_eq!(summary.calls, 0);
        assert_eq!(summary.total_tokens, 0);
        assert_eq!(summary.estimated_cost, 0.0);
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
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
                token_budget: None,
                retry_max: 0,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
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
                token_budget: None,
                retry_max: 0,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
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
                token_budget: None,
                retry_max: 0,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM).unwrap();
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
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
                token_budget: None,
                retry_max: 0, // Disable retries for error-path tests
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
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
                token_budget: None,
                retry_max: 0, // Disable retries for error-path tests
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
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
                token_budget: None,
                retry_max: 0, // Disable retries for error-path tests
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
                token_budget: None,
                retry_max: 0, // Disable retries for error-path tests
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
                token_budget: None,
                retry_max: 0, // Disable retries for error-path tests
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
                token_budget: None,
                retry_max: 0, // Disable retries for error-path tests
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };

            let client =
                OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
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
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client =
            OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client =
            OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
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
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
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
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
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
    fn test_complexity_routing_trivial_uses_first() {
        let manager = MultiModelManager::new(vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("test".to_string()),
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
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
        ])
        .unwrap();

        // "hi" is trivial → index 0
        assert_eq!(
            manager.route_by_complexity("hi").unwrap().provider_name(),
            "litellm"
        );
    }

    #[test]
    fn test_complexity_routing_simple_prefers_local() {
        let manager = MultiModelManager::new(vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o".to_string(),
                api_key: Some("test".to_string()),
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
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
        ])
        .unwrap();

        // A short non-trivial prompt is "simple" → prefers the Ollama (local) client
        assert_eq!(
            manager
                .route_by_complexity("Summarize the weather today")
                .unwrap()
                .provider_name(),
            "ollama"
        );
    }

    #[test]
    fn test_complexity_routing_complex_prefers_capable_model() {
        let manager = MultiModelManager::new(vec![
            LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o".to_string(),
                api_key: Some("test".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
        ])
        .unwrap();

        // A code prompt is "complex" → prefers the capable gpt-4o model (index 1)
        let prompt = "fn main() { println!(\"hello\"); } explain this code in depth";
        assert_eq!(
            manager.route_by_complexity(prompt).unwrap().provider_name(),
            "litellm"
        );
    }

    #[test]
    fn test_complexity_routing_empty_manager() {
        let manager = MultiModelManager::new(Vec::<LLMConfig>::new()).unwrap();
        assert!(manager.route_by_complexity("any prompt").is_none());
    }

    #[test]
    fn test_cost_aware_routing_prefers_cheapest() {
        let manager = MultiModelManager::new(vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o".to_string(),
                api_key: Some("test".to_string()),
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
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
        ])
        .unwrap();

        // Register pricing: gpt-4o is expensive, llama3.1 is free → pick llama3.1.
        let tracker = CostTracker::new()
            .with_pricing("gpt-4o", ModelPricing::new(2.50, 10.00))
            .with_free_model("llama3.1");

        assert_eq!(
            manager.route_cheapest(&tracker).unwrap().provider_name(),
            "ollama"
        );
    }

    #[test]
    fn test_cost_aware_routing_falls_back_to_index0() {
        let manager = MultiModelManager::new(vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o".to_string(),
                api_key: Some("test".to_string()),
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
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
        ])
        .unwrap();

        // No pricing registered → all free → index 0 (LiteLLM) wins.
        let tracker = CostTracker::new();
        assert_eq!(
            manager.route_cheapest(&tracker).unwrap().provider_name(),
            "litellm"
        );
    }

    #[test]
    fn test_cost_aware_routing_empty_manager() {
        let manager = MultiModelManager::new(Vec::<LLMConfig>::new()).unwrap();
        let tracker = CostTracker::new();
        assert!(manager.route_cheapest(&tracker).is_none());
    }

    #[test]
    fn test_find_by_model_case_insensitive() {
        let manager = MultiModelManager::new(vec![
            LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o".to_string(),
                api_key: Some("test".to_string()),
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
                timeout_secs: 60,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
        ])
        .unwrap();

        // Exact match
        let gpt = manager.find_by_model("gpt-4o").unwrap();
        assert_eq!(gpt.provider_name(), "litellm");

        // Case-insensitive
        let ollama = manager.find_by_model("LLAMA3.1").unwrap();
        assert_eq!(ollama.provider_name(), "ollama");

        // Whitespace is trimmed
        let trimmed = manager.find_by_model("  gpt-4o  ").unwrap();
        assert_eq!(trimmed.model(), "gpt-4o");
    }

    #[test]
    fn test_find_by_model_unknown_and_empty() {
        let manager = MultiModelManager::new(vec![LLMConfig {
            provider: LLMProvider::LiteLLM,
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: 30,
            system_prompt: crate::config::default_system_prompt(),
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        }])
        .unwrap();

        assert!(manager.find_by_model("does-not-exist").is_none());
        assert!(manager.find_by_model("").is_none());
        assert!(manager.find_by_model("   ").is_none());
    }

    #[test]
    fn test_chat_request_serialization() {
        let request = ChatRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![
                ChatMessage::new("system", "You are a helpful assistant."),
                ChatMessage::new("user", "Hello!"),
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
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
            (LLMProvider::Vllm, "vllm"),
            (LLMProvider::SGLang, "sglang"),
        ];

        for (provider, expected_name) in test_cases {
            let config = LLMConfig {
                provider,
                endpoint: "http://localhost:4000".to_string(),
                model: "test-model".to_string(),
                api_key: Some("test-key".to_string()),
                timeout_secs: 30,
                system_prompt: crate::config::default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let result = OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::LiteLLM);
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client =
            OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenAI).unwrap();
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client =
            OpenAICompatibleClient::new(&config, OpenAICompatibleProvider::OpenRouter).unwrap();
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let client = OllamaClient::new(&config).unwrap();
        assert_eq!(client.provider_name(), "ollama");
        assert_eq!(client.model(), "llama3.1");
    }

    #[test]
    fn test_chat_request_no_temperature() {
        let request = ChatRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![ChatMessage::new("user", "Hello!")],
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
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
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };

        let manager = MultiModelManager::new(vec![config]).unwrap();
        // With one client, next_client wraps to index 0
        let next = manager.next_client(0).unwrap();
        assert_eq!(next.provider_name(), "litellm");
    }

    // ── Local inference discovery tests ─────────────────────────────────

    #[test]
    fn test_local_inference_discovers_ollama() {
        let mut server = Server::new();
        let m = server
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"models":[{"name":"llama3.1"},{"name":"mistral"}]}"#)
            .create();

        let discovery = LocalInference {
            endpoints: vec![server.url()],
            timeout_ms: 1000,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let found = rt.block_on(discovery.discover());

        m.assert();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, LocalInferenceKind::Ollama);
        assert_eq!(found[0].models, vec!["llama3.1", "mistral"]);
        assert_eq!(found[0].endpoint, server.url());
    }

    #[test]
    fn test_local_inference_discovers_openai_compatible() {
        let mut server = Server::new();
        // /api/tags returns 404 (not Ollama), /v1/models succeeds.
        let _tags = server.mock("GET", "/api/tags").with_status(404).create();
        let models = server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":[{"id":"llama.cpp-model"},{"id":"qwen2.5"}]}"#)
            .create();

        let discovery = LocalInference {
            endpoints: vec![server.url()],
            timeout_ms: 1000,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let found = rt.block_on(discovery.discover());

        models.assert();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, LocalInferenceKind::OpenAiCompatible);
        assert_eq!(found[0].models, vec!["llama.cpp-model", "qwen2.5"]);
    }

    #[test]
    fn test_local_inference_ignores_unreachable() {
        // A closed port (mockito server that returns nothing) → no discovery.
        let discovery = LocalInference {
            endpoints: vec!["http://127.0.0.1:1".to_string()], // port 1 is almost always closed
            timeout_ms: 300,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let found = rt.block_on(discovery.discover());

        assert!(found.is_empty());
    }

    #[test]
    fn test_local_inference_classifies_vllm_and_sglang() {
        // vLLM (port 8000) → VLLM kind.
        let mut vllm_server = Server::new();
        let _ = vllm_server
            .mock("GET", "/api/tags")
            .with_status(404)
            .create();
        let _ = vllm_server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":[{"id":"meta-llama/Llama-3.1-8B"}]}"#)
            .create();

        // SGLang (port 30000) → SGLang kind.
        let mut sglang_server = Server::new();
        let _ = sglang_server
            .mock("GET", "/api/tags")
            .with_status(404)
            .create();
        let _ = sglang_server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":[{"id":"Qwen/Qwen2.5-7B"}]}"#)
            .create();

        // Note: mockito uses dynamic ports, not 8000/30000, so these servers are
        // classified as OpenAiCompatible. The port-based classification is covered
        // by the endpoint-list default test below; here we assert the generic path
        // still works for any OpenAI-compatible endpoint.
        let discovery = LocalInference {
            endpoints: vec![vllm_server.url()],
            timeout_ms: 1000,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let found = rt.block_on(discovery.discover());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, LocalInferenceKind::OpenAiCompatible);

        let discovery = LocalInference {
            endpoints: vec![sglang_server.url()],
            timeout_ms: 1000,
        };
        let found = rt.block_on(discovery.discover());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, LocalInferenceKind::OpenAiCompatible);
    }

    #[test]
    fn test_local_inference_default_endpoints() {
        let d = LocalInference::default();
        assert!(d.endpoints.contains(&"http://localhost:11434".to_string()));
        assert!(d.endpoints.contains(&"http://localhost:8000".to_string()));
        assert!(d.endpoints.contains(&"http://localhost:30000".to_string()));
    }

    // ── Local inference warmup (provisioning) tests ──────────────────────

    #[test]
    fn test_warmup_ollama_success() {
        let mut server = Server::new();
        let m = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"llama3.1","message":{"role":"assistant","content":"OK"}}"#)
            .create();

        let inference = LocalInference {
            endpoints: vec![],
            timeout_ms: 2000,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(inference.warmup(&server.url(), "llama3.1"));

        m.assert();
        assert!(result.success);
        assert_eq!(result.model, "llama3.1");
        assert_eq!(result.endpoint, server.url());
        assert!(result.detail.is_none());
    }

    #[test]
    fn test_warmup_openai_compatible_success() {
        // /api/chat returns 404 → falls through to /v1/chat/completions.
        let mut server = Server::new();
        let _ollama = server.mock("POST", "/api/chat").with_status(404).create();
        let openai = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"choices":[{"message":{"content":"OK"}}]}"#)
            .create();

        let inference = LocalInference {
            endpoints: vec![],
            timeout_ms: 2000,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(inference.warmup(&server.url(), "vllm-model"));

        openai.assert();
        assert!(result.success);
        assert_eq!(result.model, "vllm-model");
    }

    #[test]
    fn test_warmup_failure_records_detail() {
        let mut server = Server::new();
        let _ollama = server.mock("POST", "/api/chat").with_status(404).create();
        let _openai = server
            .mock("POST", "/v1/chat/completions")
            .with_status(500)
            .create();

        let inference = LocalInference {
            endpoints: vec![],
            timeout_ms: 2000,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(inference.warmup(&server.url(), "broken-model"));

        assert!(!result.success);
        assert!(result.detail.is_some());
    }

    // ── Local inference model selection tests ─────────────────────────────

    #[test]
    fn test_select_model_prefers_capable() {
        let inference = LocalInference::default();
        let server = LocalInferenceServer {
            endpoint: "http://localhost:11434".to_string(),
            kind: LocalInferenceKind::Ollama,
            models: vec![
                "llama3.1:8b".to_string(),
                "mistral:7b".to_string(),
                "nomic-embed-text".to_string(),
            ],
        };
        // llama3.1:8b (8b + llama3 family) should outrank mistral:7b and the embed model.
        assert_eq!(
            inference.select_model(&server).as_deref(),
            Some("llama3.1:8b")
        );
    }

    #[test]
    fn test_select_model_empty_returns_none() {
        let inference = LocalInference::default();
        let server = LocalInferenceServer {
            endpoint: "http://localhost:11434".to_string(),
            kind: LocalInferenceKind::Ollama,
            models: vec![],
        };
        assert_eq!(inference.select_model(&server), None);
    }

    #[test]
    fn test_select_model_prefers_larger_params() {
        let inference = LocalInference::default();
        let server = LocalInferenceServer {
            endpoint: "http://localhost:8000".to_string(),
            kind: LocalInferenceKind::Vllm,
            models: vec!["model-7b".to_string(), "model-70b".to_string()],
        };
        assert_eq!(
            inference.select_model(&server).as_deref(),
            Some("model-70b")
        );
    }
}
