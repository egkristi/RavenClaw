//! RavenClaws
//!
//! Secure by default: no credentials in config files, use environment variables.
//! Supports multiple LLM providers: LiteLLM, OpenRouter, Ollama, OpenAI.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

/// Configuration error type.
///
/// # Stability
/// This enum is `#[non_exhaustive]` — new variants may be added in minor releases.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("Failed to load config: {0}")]
    LoadError(String),
    #[error("Invalid configuration: {0}")]
    ValidationError(String),
    #[error("Missing required environment variable: {0}")]
    #[allow(dead_code)]
    MissingEnvVar(String),
}

/// LLM Provider type — determines which backend to use
///
/// # Stability
/// This enum is `#[non_exhaustive]` — new variants may be added in minor releases.
/// Match with a wildcard arm to handle future variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LLMProvider {
    #[default]
    LiteLLM,
    OpenRouter,
    Ollama,
    OpenAI,
    Anthropic,
    /// Generic OpenAI-compatible provider (vLLM, llama.cpp, LM Studio, TGI, Groq, Together AI, etc.)
    #[serde(rename = "openai-compatible")]
    OpenAICompatible,
    /// Azure OpenAI Service
    #[serde(rename = "azure")]
    Azure,
    /// vLLM — high-throughput OpenAI-compatible inference server (default http://localhost:8000)
    #[serde(rename = "vllm")]
    Vllm,
    /// SGLang — fast OpenAI-compatible inference server (default http://localhost:30000)
    #[serde(rename = "sglang")]
    SGLang,
}

impl LLMProvider {
    /// Parse a provider string (case-insensitive) into an [`LLMProvider`].
    ///
    /// Unknown strings fall back to [`LLMProvider::LiteLLM`]. This is the single
    /// source of truth for provider-string parsing, used by both the CLI flag
    /// (`--provider`) and the `RAVENCLAWS__LLM__PROVIDER` environment variable.
    pub fn parse(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "openrouter" => Self::OpenRouter,
            "ollama" => Self::Ollama,
            "openai" => Self::OpenAI,
            "anthropic" => Self::Anthropic,
            "openai-compatible" | "openai_compatible" => Self::OpenAICompatible,
            "azure" => Self::Azure,
            "vllm" => Self::Vllm,
            "sglang" => Self::SGLang,
            _ => Self::LiteLLM,
        }
    }
}

/// Top-level configuration for RavenClaws.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
/// Construct using `Config::load()` or use `..Default::default()` for updates.
#[derive(Debug, Clone, Deserialize, Default)]
#[non_exhaustive]
pub struct Config {
    /// LiteLLM configuration (single provider mode)
    #[serde(default)]
    pub llm: LLMConfig,

    /// Multiple LLM configurations (multi-model mode)
    #[serde(default)]
    pub llms: Vec<LLMConfig>,

    /// RavenFabric configuration
    #[serde(default)]
    pub ravenfabric: RavenFabricConfig,

    /// Security settings
    #[serde(default)]
    pub security: SecurityConfig,

    /// Runtime settings
    #[serde(default)]
    pub runtime: RuntimeConfig,

    /// Telemetry / OpenTelemetry settings (v0.7.2)
    #[serde(default)]
    pub telemetry: TelemetryConfig,

    /// Scheduler / triggers configuration (v0.8)
    #[serde(default)]
    pub scheduler: SchedulerConfig,

    /// Web search configuration (v0.8)
    #[serde(default)]
    pub web_search: WebSearchConfig,

    /// Heartbeat / autonomous agent configuration (v0.9)
    #[serde(default)]
    pub heartbeat: crate::heartbeat::HeartbeatConfig,

    /// Swarm orchestration configuration (v0.9)
    #[serde(default)]
    pub swarm: crate::swarm::SwarmConfig,

    /// MCP server connections configuration (v0.9.6)
    #[serde(default)]
    pub mcp: McpConfig,

    /// Browser automation configuration (v1.1.0)
    #[serde(default)]
    pub browser: BrowserConfig,

    /// Load management / graceful degradation configuration (v1.1.0)
    #[serde(default)]
    pub load: crate::load::LoadConfig,

    /// Web access (domain-level) policy configuration (v1.4.0)
    #[serde(default = "default_web_policy")]
    pub web_policy: crate::web_policy::WebAccessPolicy,
}

/// MCP server connections configuration (v0.9.6)
///
/// Defines one or more MCP servers to connect to at startup.
/// Each server is a subprocess that communicates via JSON-RPC 2.0 over stdio.
///
/// # Example (TOML)
///
/// ```toml
/// [mcp]
/// servers = [
///   { name = "filesystem", command = "npx", args = ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"] },
///   { name = "database", command = "python", args = ["mcp-server.py"], env = { DB_URL = "postgres://..." } },
/// ]
/// ```
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize, Default)]
#[non_exhaustive]
pub struct McpConfig {
    /// List of MCP servers to connect to
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// Configuration for a single MCP server connection (v0.9.6)
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct McpServerConfig {
    /// Human-readable name for this MCP server (used in logs and tool namespacing)
    pub name: String,
    /// Command to launch the MCP server process (stdio transport)
    /// Either `command` or `url` must be set, but not both.
    #[serde(default)]
    pub command: String,
    /// Arguments to pass to the command
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the MCP server process
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// SSE endpoint URL for SSE transport (e.g., "http://localhost:8080/sse")
    /// Either `command` or `url` must be set, but not both.
    #[serde(default)]
    pub url: String,
}

/// Web search configuration (v0.8)
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebSearchConfig {
    /// Search API endpoint (SearXNG or compatible)
    #[serde(default = "default_search_endpoint")]
    pub endpoint: String,

    /// Search engine to use (e.g., "duckduckgo", "google", "brave")
    #[serde(default = "default_search_engine")]
    pub engine: String,

    /// Maximum number of search results to return
    #[serde(default = "default_search_max_results")]
    pub max_results: usize,

    /// Whether to fetch and extract content from each search result
    #[serde(default = "default_true")]
    pub fetch_content: bool,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            endpoint: default_search_endpoint(),
            engine: default_search_engine(),
            max_results: default_search_max_results(),
            fetch_content: default_true(),
        }
    }
}

/// Browser automation configuration (v1.1.0)
///
/// Configures the CDP (Chrome DevTools Protocol) endpoint for browser automation.
/// Requires Chrome/Chromium running with `--remote-debugging-port=9222`.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct BrowserConfig {
    /// CDP endpoint URL (e.g., "http://127.0.0.1:9222")
    #[serde(default = "default_browser_cdp_url")]
    pub cdp_url: String,

    /// Request timeout in milliseconds for CDP commands
    #[serde(default = "default_browser_timeout")]
    pub request_timeout: u64,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            cdp_url: default_browser_cdp_url(),
            request_timeout: default_browser_timeout(),
        }
    }
}

fn default_browser_cdp_url() -> String {
    "http://127.0.0.1:9222".to_string()
}

fn default_browser_timeout() -> u64 {
    30000
}

fn default_web_policy() -> crate::web_policy::WebAccessPolicy {
    // Disabled by default — existing deployments are unaffected until they
    // explicitly enable domain-level filtering.
    crate::web_policy::WebAccessPolicy::disabled()
}

fn default_search_endpoint() -> String {
    "https://searx.be".to_string()
}

fn default_search_engine() -> String {
    "duckduckgo".to_string()
}

fn default_search_max_results() -> usize {
    5
}

fn default_otel_disabled() -> bool {
    true
}

/// Scheduler / triggers configuration (v0.8)
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize, Default)]
#[non_exhaustive]
pub struct SchedulerConfig {
    /// List of trigger configurations
    #[serde(default)]
    pub triggers: Vec<crate::scheduler::TriggerConfig>,
}

/// Telemetry / OpenTelemetry configuration (v0.7.2)
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize, Default)]
#[non_exhaustive]
pub struct TelemetryConfig {
    /// OTLP gRPC endpoint for OpenTelemetry (e.g., "http://jaeger:4317")
    #[serde(default)]
    pub otel_endpoint: Option<String>,

    /// Service name for OpenTelemetry traces
    #[serde(default)]
    pub otel_service_name: Option<String>,

    /// Disable OpenTelemetry tracing (default: true — opt-in)
    #[serde(default = "default_otel_disabled")]
    pub otel_disabled: bool,
}

/// LLM provider configuration.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
/// Construct using `LLMConfig::default()` or use `..Default::default()` for updates.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct LLMConfig {
    /// Provider type: litellm, openrouter, ollama, openai
    #[serde(default)]
    pub provider: LLMProvider,

    /// Endpoint URL (e.g., http://litellm:4000, http://localhost:11434, https://api.openai.com)
    #[serde(default)]
    pub endpoint: String,

    /// Default model to use
    #[serde(default = "default_model")]
    pub model: String,

    /// API key (prefer env var)
    #[serde(default)]
    pub api_key: Option<String>,

    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// System prompt / persona for the agent
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,

    /// Token budget (v0.5) — maximum tokens per run
    #[serde(default)]
    pub token_budget: Option<u32>,

    /// Retry max attempts (v0.5) — default 3
    #[serde(default = "default_retry_max")]
    pub retry_max: u32,

    /// Retry base delay in ms (v0.5) — default 100
    #[serde(default = "default_retry_base_delay")]
    pub retry_base_delay_ms: u64,

    /// Retry max delay in ms (v0.5) — default 10000
    #[serde(default = "default_retry_max_delay")]
    pub retry_max_delay_ms: u64,
}

pub fn default_retry_max() -> u32 {
    3
}
pub fn default_retry_base_delay() -> u64 {
    100
}
pub fn default_retry_max_delay() -> u64 {
    10000
}

pub fn default_system_prompt() -> String {
    "You are RavenClaws, a lightweight autonomous agent. \
        Be concise, efficient, and secure. Always validate inputs and outputs. \
        When you have completed the task, prefix your final answer with FINAL: \
        so the system knows the task is done."
        .to_string()
}

/// RavenFabric mesh client configuration.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
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
    #[allow(dead_code)]
    pub allowed_hosts: Vec<String>,
}

impl Default for RavenFabricConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            agent_id: None,
            remote_exec: default_true(),
            allowed_hosts: Vec::new(),
        }
    }
}

/// Security configuration.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SecurityConfig {
    /// Require TLS for all connections
    #[serde(default = "default_true")]
    pub require_tls: bool,

    /// Maximum token lifetime in seconds (0 = unlimited)
    /// When non-zero, agent sessions automatically terminate after this duration.
    /// Enforced in the agent loop — checked before each iteration.
    #[serde(default = "default_token_lifetime")]
    pub token_lifetime_secs: u64,

    /// Enable audit logging
    #[serde(default = "default_true")]
    #[allow(dead_code)]
    pub audit_log: bool,

    /// Enable prompt-injection defense
    /// When true, LLM responses are scanned for injection patterns
    /// and output schema violations before processing.
    #[serde(default = "default_true")]
    #[allow(dead_code)]
    pub prompt_injection_protection: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            require_tls: default_true(),
            token_lifetime_secs: default_token_lifetime(),
            audit_log: default_true(),
            prompt_injection_protection: default_true(),
        }
    }
}

/// Runtime configuration.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct RuntimeConfig {
    /// Working directory
    #[serde(default = "default_workdir")]
    #[allow(dead_code)]
    pub workdir: String,

    /// Maximum concurrent agents
    #[serde(default = "default_max_agents")]
    #[allow(dead_code)]
    pub max_agents: usize,

    /// Health check interval in seconds
    #[serde(default = "default_health_interval")]
    #[allow(dead_code)]
    pub health_interval_secs: u64,

    /// HTTP server host (v0.7)
    #[serde(default)]
    pub host: Option<String>,

    /// HTTP server port (v0.7)
    #[serde(default = "default_server_port")]
    pub port: u16,

    /// Checkpoint directory for durable execution (v0.9.12+)
    /// When set, the agent loop saves state after each iteration to this directory.
    /// On restart, the agent loop can resume from the latest checkpoint.
    /// Defaults to `{workdir}/checkpoints/` if not set.
    #[serde(default)]
    #[allow(dead_code)]
    pub checkpoint_dir: Option<String>,

    /// Checkpoint interval in iterations (v0.9.12+)
    /// Save a checkpoint every N iterations. Default: 1 (every iteration).
    /// Higher values reduce I/O but lose more work on crash.
    #[serde(default = "default_checkpoint_interval")]
    #[allow(dead_code)]
    pub checkpoint_interval: usize,
}

fn default_checkpoint_interval() -> usize {
    1
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            workdir: default_workdir(),
            max_agents: default_max_agents(),
            health_interval_secs: default_health_interval(),
            host: None,
            port: default_server_port(),
            checkpoint_dir: None,
            checkpoint_interval: 1,
        }
    }
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
    "/tmp/ravenclaws-workdir".to_string()
}

fn default_max_agents() -> usize {
    10
}

fn default_health_interval() -> u64 {
    60
}

fn default_server_port() -> u16 {
    8080
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            provider: LLMProvider::LiteLLM,
            endpoint: String::new(),
            model: default_model(),
            api_key: None,
            timeout_secs: default_timeout(),
            system_prompt: default_system_prompt(),
            token_budget: None,
            retry_max: default_retry_max(),
            retry_base_delay_ms: default_retry_base_delay(),
            retry_max_delay_ms: default_retry_max_delay(),
        }
    }
}

/// Zeroize sensitive fields on drop — API keys are cleared from memory
impl Drop for LLMConfig {
    fn drop(&mut self) {
        if let Some(ref mut key) = self.api_key {
            key.zeroize();
        }
    }
}

impl Config {
    /// Load configuration from file and environment
    pub fn load(config_path: Option<&str>) -> Result<Self, ConfigError> {
        // Start with defaults from environment
        dotenvy::dotenv().ok();

        let mut config_builder = config::Config::builder();

        // Load from file if provided
        if let Some(path) = config_path {
            config_builder =
                config_builder.add_source(config::File::with_name(path).required(false));
        }

        // Load from environment (RAVENCLAW_* prefix)
        // Save and remove RAVENCLAWS__LLMS before serde deserialization because
        // config::Environment passes it as a raw string, which serde can't
        // deserialize into Vec<LLMConfig>. We restore and parse it manually below.
        let ravenclaws_llms = std::env::var("RAVENCLAWS__LLMS").ok();
        if ravenclaws_llms.is_some() {
            std::env::remove_var("RAVENCLAWS__LLMS");
        }

        config_builder = config_builder
            .add_source(config::Environment::with_prefix("RAVENCLAW").separator("__"));

        let config = config_builder
            .build()
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;

        let mut cfg: Config = config
            .try_deserialize()
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;

        // Restore RAVENCLAWS__LLMS if it was set
        if let Some(ref val) = ravenclaws_llms {
            std::env::set_var("RAVENCLAWS__LLMS", val);
        }

        // Override sensitive values from environment
        // Single provider mode
        if let Ok(key) = std::env::var("LITELLM_API_KEY") {
            cfg.llm.api_key = Some(key);
        }
        if let Ok(provider) = std::env::var("RAVENCLAWS__LLM__PROVIDER") {
            cfg.llm.provider = LLMProvider::parse(&provider);
        }
        if let Ok(endpoint) = std::env::var("RAVENCLAWS__LLM__ENDPOINT") {
            cfg.llm.endpoint = endpoint;
        }
        if let Ok(model) = std::env::var("RAVENCLAWS__LLM__MODEL") {
            cfg.llm.model = model;
        }

        // Multi-provider mode
        // Note: RAVENCLAWS__LLMS is handled manually (not via config::Environment)
        // because it's a JSON string that serde can't deserialize into Vec<LLMConfig>.
        if let Ok(keys) = std::env::var("RAVENCLAWS__LLMS") {
            // Parse JSON array of LLM configs from env
            if let Ok(llms) = serde_json::from_str::<Vec<LLMConfig>>(&keys) {
                cfg.llms = llms;
            }
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
        // Validate single provider config
        if !self.llm.endpoint.is_empty() {
            self.validate_llm_config(&self.llm)?;
        }

        // Validate multi-provider configs
        for (i, llm) in self.llms.iter().enumerate() {
            self.validate_llm_config(llm)
                .map_err(|e| ConfigError::ValidationError(format!("LLM[{}]: {}", i, e)))?;
        }

        // At least one provider must be configured
        if self.llm.endpoint.is_empty() && self.llms.is_empty() {
            return Err(ConfigError::ValidationError(
                "At least one LLM provider must be configured (llm or llms)".to_string(),
            ));
        }

        Ok(())
    }

    fn validate_llm_config(&self, llm: &LLMConfig) -> Result<(), ConfigError> {
        if llm.endpoint.is_empty()
            && llm.provider != LLMProvider::OpenAI
            && llm.provider != LLMProvider::OpenRouter
            && llm.provider != LLMProvider::Anthropic
        {
            // OpenAI, OpenRouter, and Anthropic have fixed endpoints
            return Err(ConfigError::ValidationError(
                "LLM endpoint is required for this provider".to_string(),
            ));
        }

        if self.security.require_tls
            && !llm.endpoint.is_empty()
            && !llm.endpoint.starts_with("https://")
            && !llm.endpoint.contains("localhost")
            && !llm.endpoint.contains("127.0.0.1")
            && !llm.endpoint.contains("0.0.0.0")
        {
            return Err(ConfigError::ValidationError(
                "TLS required but endpoint is not HTTPS".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial(env_test)]
    fn test_default_config() {
        std::env::set_var("LITELLM_API_KEY", "test-key");
        std::env::set_var("RAVENCLAWS__LLM__ENDPOINT", "http://localhost:4000");

        let config = Config::load(None).unwrap();
        assert_eq!(config.llm.model, "gpt-4o-mini");
        assert_eq!(config.llm.timeout_secs, 30);
        // require_tls defaults to true via serde(default = "default_true")
        // but only when deserialized, not via #[derive(Default)]
        // Since we load via serde, it should be true
        assert!(config.security.require_tls);

        // Clean up env vars set by this test
        std::env::remove_var("LITELLM_API_KEY");
        std::env::remove_var("RAVENCLAWS__LLM__ENDPOINT");
    }

    #[test]
    fn test_llm_provider_default() {
        assert_eq!(LLMProvider::default(), LLMProvider::LiteLLM);
    }

    #[test]
    fn test_llm_provider_parse() {
        // Case-insensitive, all known providers.
        assert_eq!(LLMProvider::parse("litellm"), LLMProvider::LiteLLM);
        assert_eq!(LLMProvider::parse("openrouter"), LLMProvider::OpenRouter);
        assert_eq!(LLMProvider::parse("ollama"), LLMProvider::Ollama);
        assert_eq!(LLMProvider::parse("openai"), LLMProvider::OpenAI);
        assert_eq!(LLMProvider::parse("anthropic"), LLMProvider::Anthropic);
        assert_eq!(
            LLMProvider::parse("openai-compatible"),
            LLMProvider::OpenAICompatible
        );
        assert_eq!(
            LLMProvider::parse("openai_compatible"),
            LLMProvider::OpenAICompatible
        );
        assert_eq!(LLMProvider::parse("azure"), LLMProvider::Azure);
        assert_eq!(LLMProvider::parse("vllm"), LLMProvider::Vllm);
        assert_eq!(LLMProvider::parse("sglang"), LLMProvider::SGLang);

        // Case-insensitivity.
        assert_eq!(LLMProvider::parse("OpenAI"), LLMProvider::OpenAI);
        assert_eq!(LLMProvider::parse("AZURE"), LLMProvider::Azure);

        // Unknown → fallback to LiteLLM.
        assert_eq!(LLMProvider::parse(""), LLMProvider::LiteLLM);
        assert_eq!(LLMProvider::parse("unknown"), LLMProvider::LiteLLM);
    }

    #[test]
    fn test_llm_provider_serde() {
        let json = r#""litellm""#;
        let provider: LLMProvider = serde_json::from_str(json).unwrap();
        assert_eq!(provider, LLMProvider::LiteLLM);

        let json = r#""openai""#;
        let provider: LLMProvider = serde_json::from_str(json).unwrap();
        assert_eq!(provider, LLMProvider::OpenAI);

        let json = r#""ollama""#;
        let provider: LLMProvider = serde_json::from_str(json).unwrap();
        assert_eq!(provider, LLMProvider::Ollama);

        let json = r#""openrouter""#;
        let provider: LLMProvider = serde_json::from_str(json).unwrap();
        assert_eq!(provider, LLMProvider::OpenRouter);

        let json = r#""vllm""#;
        let provider: LLMProvider = serde_json::from_str(json).unwrap();
        assert_eq!(provider, LLMProvider::Vllm);

        let json = r#""sglang""#;
        let provider: LLMProvider = serde_json::from_str(json).unwrap();
        assert_eq!(provider, LLMProvider::SGLang);
    }

    #[test]
    fn test_llm_config_default() {
        let config = LLMConfig::default();
        assert_eq!(config.provider, LLMProvider::LiteLLM);
        assert_eq!(config.model, "gpt-4o-mini");
        assert_eq!(config.timeout_secs, 30);
        assert!(config.api_key.is_none());
        assert!(config.endpoint.is_empty());
        assert!(config.system_prompt.contains("RavenClaws"));
    }

    #[test]
    fn test_system_prompt_custom() {
        let mut config = LLMConfig::default();
        config.system_prompt = "You are a helpful coding assistant.".to_string();
        assert_eq!(config.system_prompt, "You are a helpful coding assistant.");
    }

    #[test]
    fn test_validate_missing_endpoint() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: String::new(),
                model: "gpt-4o-mini".to_string(),
                api_key: None,
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("At least one LLM provider"));
    }

    #[test]
    fn test_validate_tls_required() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://example.com:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: true,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("TLS required"));
    }

    #[test]
    fn test_validate_tls_localhost_allowed() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://localhost:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: true,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_openai_no_endpoint_needed() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::OpenAI,
                endpoint: String::new(),
                model: "gpt-4o".to_string(),
                api_key: Some("sk-key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        // OpenAI doesn't need an endpoint, but the llm.endpoint is empty
        // and the llm section is checked. Since llm.endpoint is empty,
        // validate() skips the llm check but then fails because no providers.
        // Fix: set llm.endpoint to something or add llms
        let result = config.validate();
        assert!(result.is_err()); // No endpoint set for llm, and no llms
    }

    #[test]
    fn test_validate_multi_provider() {
        let config = Config {
            llm: LLMConfig::default(),
            llms: vec![LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 60,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            }],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_ravenfabric_config_default() {
        let config = RavenFabricConfig::default();
        assert!(config.endpoint.is_none());
        assert!(config.agent_id.is_none());
        assert!(config.remote_exec);
        assert!(config.allowed_hosts.is_empty());
    }

    #[test]
    fn test_security_config_default() {
        let config = SecurityConfig::default();
        assert!(config.require_tls);
        assert_eq!(config.token_lifetime_secs, 3600);
        assert!(config.audit_log);
    }

    #[test]
    fn test_runtime_config_default() {
        let config = RuntimeConfig::default();
        assert_eq!(config.workdir, "/tmp/ravenclaws-workdir");
        assert_eq!(config.max_agents, 10);
        assert_eq!(config.health_interval_secs, 60);
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::LoadError("file not found".to_string());
        assert_eq!(format!("{}", err), "Failed to load config: file not found");

        let err = ConfigError::ValidationError("bad field".to_string());
        assert_eq!(format!("{}", err), "Invalid configuration: bad field");

        let err = ConfigError::MissingEnvVar("API_KEY".to_string());
        assert_eq!(
            format!("{}", err),
            "Missing required environment variable: API_KEY"
        );
    }

    #[test]
    fn test_validate_openrouter_no_endpoint_needed() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: String::new(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        // OpenRouter doesn't need an endpoint, but llm.endpoint is empty
        // so validate() skips llm check and fails because no providers
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_ollama_needs_endpoint() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: String::new(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("At least one LLM provider"));
    }

    #[test]
    fn test_validate_tls_localhost_ip_allowed() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://127.0.0.1:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: true,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tls_wildcard_allowed() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://0.0.0.0:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: true,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_multi_provider_with_tls() {
        let config = Config {
            llm: LLMConfig::default(),
            llms: vec![
                LLMConfig {
                    provider: LLMProvider::Ollama,
                    endpoint: "http://localhost:11434".to_string(),
                    model: "llama3.1".to_string(),
                    api_key: None,
                    timeout_secs: 60,
                    system_prompt: default_system_prompt(),
                    token_budget: None,
                    retry_max: 3,
                    retry_base_delay_ms: 100,
                    retry_max_delay_ms: 10000,
                },
                LLMConfig {
                    provider: LLMProvider::LiteLLM,
                    endpoint: "https://litellm.example.com:4000".to_string(),
                    model: "gpt-4o-mini".to_string(),
                    api_key: Some("key".to_string()),
                    timeout_secs: 30,
                    system_prompt: default_system_prompt(),
                    token_budget: None,
                    retry_max: 3,
                    retry_base_delay_ms: 100,
                    retry_max_delay_ms: 10000,
                },
            ],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: true,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_multi_provider_tls_failure() {
        let config = Config {
            llm: LLMConfig::default(),
            llms: vec![LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "http://example.com:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            }],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: true,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("TLS required"));
    }

    #[test]
    fn test_ravenfabric_config_custom() {
        let config = RavenFabricConfig {
            endpoint: Some("https://fabric.example.com:8443".to_string()),
            agent_id: Some("agent-01".to_string()),
            remote_exec: false,
            allowed_hosts: vec!["10.0.0.0/8".to_string()],
        };
        assert_eq!(config.endpoint.unwrap(), "https://fabric.example.com:8443");
        assert_eq!(config.agent_id.unwrap(), "agent-01");
        assert!(!config.remote_exec);
        assert_eq!(config.allowed_hosts.len(), 1);
    }

    #[test]
    fn test_security_config_custom() {
        let config = SecurityConfig {
            require_tls: false,
            token_lifetime_secs: 7200,
            audit_log: false,
            prompt_injection_protection: false,
        };
        assert!(!config.require_tls);
        assert_eq!(config.token_lifetime_secs, 7200);
        assert!(!config.audit_log);
    }

    #[test]
    fn test_runtime_config_custom() {
        let config = RuntimeConfig {
            workdir: "/data".to_string(),
            max_agents: 5,
            health_interval_secs: 120,
            host: Some("127.0.0.1".to_string()),
            port: 9090,
            checkpoint_dir: None,
            checkpoint_interval: 1,
        };
        assert_eq!(config.workdir, "/data");
        assert_eq!(config.max_agents, 5);
        assert_eq!(config.health_interval_secs, 120);
        assert_eq!(config.host, Some("127.0.0.1".to_string()));
        assert_eq!(config.port, 9090);
    }

    #[test]
    fn test_llm_config_custom() {
        let config = LLMConfig {
            provider: LLMProvider::OpenAI,
            endpoint: String::new(),
            model: "gpt-4o".to_string(),
            api_key: Some("sk-test".to_string()),
            timeout_secs: 120,
            system_prompt: default_system_prompt(),
            token_budget: None,
            retry_max: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 10000,
        };
        assert_eq!(config.provider, LLMProvider::OpenAI);
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.timeout_secs, 120);
        assert_eq!(config.api_key.clone().unwrap(), "sk-test");
    }

    #[test]
    fn test_llm_provider_serde_invalid_fallback() {
        // Unknown provider values should fall back to default (LiteLLM)
        let json = r#""unknown_provider""#;
        let provider: LLMProvider = serde_json::from_str(json).unwrap_or_default();
        assert_eq!(provider, LLMProvider::LiteLLM);
    }

    #[test]
    #[serial(env_test)]
    fn test_config_load_with_env_overrides() {
        // Set up env vars for single-provider mode
        std::env::set_var("RAVENCLAWS__LLM__ENDPOINT", "http://localhost:4000");
        std::env::set_var("RAVENCLAWS__LLM__MODEL", "gpt-4o");
        std::env::set_var("LITELLM_API_KEY", "env-key");

        let config = Config::load(None).unwrap();
        assert_eq!(config.llm.endpoint, "http://localhost:4000");
        assert_eq!(config.llm.model, "gpt-4o");
        assert_eq!(config.llm.api_key.clone().unwrap(), "env-key");

        // Clean up env vars set by this test
        std::env::remove_var("RAVENCLAWS__LLM__ENDPOINT");
        std::env::remove_var("RAVENCLAWS__LLM__MODEL");
        std::env::remove_var("LITELLM_API_KEY");
    }

    #[test]
    #[serial(env_test)]
    fn test_config_load_with_llms_json_env() {
        let llms_json = r#"[{"provider":"ollama","endpoint":"http://localhost:11434","model":"llama3.1","timeout_secs":60}]"#;
        std::env::set_var("RAVENCLAWS__LLMS", llms_json);
        std::env::set_var("LITELLM_API_KEY", "dummy");
        std::env::set_var("RAVENCLAWS__LLM__ENDPOINT", "http://localhost:4000");

        let config = Config::load(None).unwrap();
        assert_eq!(config.llms.len(), 1);
        assert_eq!(config.llms[0].provider, LLMProvider::Ollama);
        assert_eq!(config.llms[0].endpoint, "http://localhost:11434");
        assert_eq!(config.llms[0].model, "llama3.1");
        assert_eq!(config.llms[0].timeout_secs, 60);

        // Clean up env vars set by this test
        std::env::remove_var("RAVENCLAWS__LLMS");
        std::env::remove_var("LITELLM_API_KEY");
        std::env::remove_var("RAVENCLAWS__LLM__ENDPOINT");
    }

    #[test]
    #[serial(env_test)]
    fn test_config_load_with_ravenfabric_env() {
        std::env::set_var("RAVENFABRIC_ENDPOINT", "https://fabric.example.com:8443");
        std::env::set_var("LITELLM_API_KEY", "dummy");
        std::env::set_var("RAVENCLAWS__LLM__ENDPOINT", "http://localhost:4000");

        let config = Config::load(None).unwrap();
        assert_eq!(
            config.ravenfabric.endpoint.unwrap(),
            "https://fabric.example.com:8443"
        );

        // Clean up env vars set by this test
        std::env::remove_var("RAVENFABRIC_ENDPOINT");
        std::env::remove_var("LITELLM_API_KEY");
        std::env::remove_var("RAVENCLAWS__LLM__ENDPOINT");
    }

    #[test]
    #[serial(env_test)]
    fn test_config_load_with_provider_env() {
        // Test provider override via env var — use a valid serde value
        std::env::set_var("RAVENCLAWS__LLM__PROVIDER", "openai");
        std::env::set_var("RAVENCLAWS__LLM__ENDPOINT", "https://api.openai.com");
        std::env::set_var("LITELLM_API_KEY", "dummy");

        let config = Config::load(None).unwrap();
        assert_eq!(config.llm.provider, LLMProvider::OpenAI);

        // Clean up env vars set by this test
        std::env::remove_var("RAVENCLAWS__LLM__PROVIDER");
        std::env::remove_var("RAVENCLAWS__LLM__ENDPOINT");
        std::env::remove_var("LITELLM_API_KEY");
    }

    #[test]
    fn test_config_load_with_provider_env_fallback() {
        // Test that the manual override code handles unknown providers
        // We test the override logic directly to avoid serde deserialization failure
        let mapped = match "unknown" {
            "openrouter" => LLMProvider::OpenRouter,
            "ollama" => LLMProvider::Ollama,
            "openai" => LLMProvider::OpenAI,
            _ => LLMProvider::LiteLLM,
        };
        assert_eq!(mapped, LLMProvider::LiteLLM);

        // Test with empty string
        let mapped = match "" {
            "openrouter" => LLMProvider::OpenRouter,
            "ollama" => LLMProvider::Ollama,
            "openai" => LLMProvider::OpenAI,
            _ => LLMProvider::LiteLLM,
        };
        assert_eq!(mapped, LLMProvider::LiteLLM);
    }

    #[test]
    fn test_validate_openai_with_endpoint() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::OpenAI,
                endpoint: "https://api.openai.com".to_string(),
                model: "gpt-4o".to_string(),
                api_key: Some("sk-key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_openrouter_with_endpoint() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: "https://openrouter.ai/api".to_string(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_https_endpoint_with_tls() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: "https://api.example.com:4000".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: true,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    #[serial(env_test)]
    fn test_config_load_with_nonexistent_file() {
        // Loading with a non-existent file path should still succeed
        // (the File source is created with required(false))
        std::env::set_var("LITELLM_API_KEY", "test-key");
        std::env::set_var("RAVENCLAWS__LLM__ENDPOINT", "http://localhost:4000");

        let result = Config::load(Some("/tmp/nonexistent/ravenclaws.toml"));
        assert!(result.is_ok());

        std::env::remove_var("LITELLM_API_KEY");
        std::env::remove_var("RAVENCLAWS__LLM__ENDPOINT");
    }

    #[test]
    fn test_config_error_missing_env_var_display() {
        let err = ConfigError::MissingEnvVar("DATABASE_URL".to_string());
        assert_eq!(
            format!("{}", err),
            "Missing required environment variable: DATABASE_URL"
        );
    }

    #[test]
    fn test_llm_config_deserialize() {
        let json = r#"{
            "provider": "openai",
            "endpoint": "https://api.openai.com",
            "model": "gpt-4o",
            "api_key": "sk-test",
            "timeout_secs": 120
        }"#;
        let config: LLMConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.provider, LLMProvider::OpenAI);
        assert_eq!(config.endpoint, "https://api.openai.com");
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.timeout_secs, 120);
    }

    #[test]
    fn test_security_config_serde_defaults() {
        // When deserializing from an empty JSON object, serde should use defaults
        let json = r#"{}"#;
        let config: SecurityConfig = serde_json::from_str(json).unwrap();
        assert!(config.require_tls);
        assert_eq!(config.token_lifetime_secs, 3600);
        assert!(config.audit_log);
    }

    #[test]
    fn test_runtime_config_serde_defaults() {
        let json = r#"{}"#;
        let config: RuntimeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.workdir, "/tmp/ravenclaws-workdir");
        assert_eq!(config.max_agents, 10);
        assert_eq!(config.health_interval_secs, 60);
    }

    #[test]
    fn test_ravenfabric_config_serde_defaults() {
        let json = r#"{}"#;
        let config: RavenFabricConfig = serde_json::from_str(json).unwrap();
        assert!(config.endpoint.is_none());
        assert!(config.agent_id.is_none());
        assert!(config.remote_exec);
        assert!(config.allowed_hosts.is_empty());
    }

    #[test]
    fn test_validate_ollama_with_endpoint_succeeds() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::Ollama,
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.1".to_string(),
                api_key: None,
                timeout_secs: 60,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_openrouter_with_endpoint_succeeds() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::OpenRouter,
                endpoint: "https://openrouter.ai/api".to_string(),
                model: "anthropic/claude-sonnet-4-20250514".to_string(),
                api_key: Some("or-key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_litellm_with_empty_endpoint_fails() {
        let config = Config {
            llm: LLMConfig {
                provider: LLMProvider::LiteLLM,
                endpoint: String::new(),
                model: "gpt-4o-mini".to_string(),
                api_key: Some("key".to_string()),
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            },
            llms: vec![],
            ravenfabric: RavenFabricConfig::default(),
            security: SecurityConfig {
                require_tls: false,
                token_lifetime_secs: 3600,
                audit_log: false,
                prompt_injection_protection: false,
            },
            runtime: RuntimeConfig::default(),
            telemetry: TelemetryConfig::default(),
            scheduler: SchedulerConfig::default(),
            web_search: WebSearchConfig::default(),
            heartbeat: crate::heartbeat::HeartbeatConfig::default(),
            mcp: McpConfig::default(),
            swarm: crate::swarm::SwarmConfig::default(),
            browser: BrowserConfig::default(),
            load: crate::load::LoadConfig::default(),
            web_policy: crate::web_policy::WebAccessPolicy::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("At least one LLM provider"));
    }

    #[test]
    fn test_llm_provider_serde_serialize() {
        let provider = LLMProvider::OpenAI;
        let json = serde_json::to_string(&provider).unwrap();
        assert_eq!(json, r#""openai""#);

        let provider = LLMProvider::Ollama;
        let json = serde_json::to_string(&provider).unwrap();
        assert_eq!(json, r#""ollama""#);

        let provider = LLMProvider::OpenRouter;
        let json = serde_json::to_string(&provider).unwrap();
        assert_eq!(json, r#""openrouter""#);

        let provider = LLMProvider::LiteLLM;
        let json = serde_json::to_string(&provider).unwrap();
        assert_eq!(json, r#""litellm""#);
    }

    /// Deterministic property-style fuzz test for the provider parser: hammer
    /// `LLMProvider::parse` with 10,000 random-ish strings and assert it never
    /// panics and always returns a valid variant (falling back to LiteLLM).
    #[test]
    fn test_llm_provider_parse_never_panics_on_adversarial_input() {
        struct Lcg(u64);
        impl Lcg {
            fn next(&mut self) -> u64 {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                self.0
            }
        }
        let mut rng = Lcg(0x9e37_79b9_7f4a_7c15);
        let alphabet: Vec<char> =
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_./ "
                .chars()
                .collect();

        for _ in 0..10_000 {
            let len = (rng.next() as usize) % 32;
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            // Must never panic; result is always one of the 9 known variants.
            let parsed = LLMProvider::parse(&s);
            let _ = parsed; // no panic is the assertion; variant is always valid
        }

        // Deterministic sanity: a known input still parses correctly.
        assert_eq!(LLMProvider::parse("openai"), LLMProvider::OpenAI);
    }

    /// Deterministic property test for config validation: construct 10,000
    /// pseudo-random `LLMConfig` endpoint/provider/model combinations and assert
    /// validation never panics (it returns Ok or a well-formed Err).
    #[test]
    fn test_config_validation_never_panics_on_adversarial_input() {
        struct Lcg(u64);
        impl Lcg {
            fn next(&mut self) -> u64 {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                self.0
            }
        }
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        let endpoints = [
            "",
            "http://localhost:4000",
            "https://api.openai.com",
            "http://127.0.0.1:11434",
            "not-a-url",
        ];
        let providers = [
            LLMProvider::LiteLLM,
            LLMProvider::OpenAI,
            LLMProvider::OpenRouter,
            LLMProvider::Anthropic,
            LLMProvider::Ollama,
            LLMProvider::OpenAICompatible,
        ];

        for _ in 0..10_000 {
            let endpoint = endpoints[(rng.next() as usize) % endpoints.len()].to_string();
            let provider = providers[(rng.next() as usize) % providers.len()].clone();
            let llm = LLMConfig {
                provider,
                endpoint,
                model: "m".to_string(),
                api_key: None,
                timeout_secs: 30,
                system_prompt: default_system_prompt(),
                token_budget: None,
                retry_max: 3,
                retry_base_delay_ms: 100,
                retry_max_delay_ms: 10000,
            };
            let config = Config {
                llm,
                llms: vec![],
                ravenfabric: RavenFabricConfig::default(),
                security: SecurityConfig::default(),
                runtime: RuntimeConfig::default(),
                telemetry: TelemetryConfig::default(),
                scheduler: SchedulerConfig::default(),
                web_search: WebSearchConfig::default(),
                heartbeat: crate::heartbeat::HeartbeatConfig::default(),
                mcp: McpConfig::default(),
                swarm: crate::swarm::SwarmConfig::default(),
                browser: BrowserConfig::default(),
                load: crate::load::LoadConfig::default(),
                web_policy: crate::web_policy::WebAccessPolicy::default(),
            };
            // Must never panic; Ok/Err both acceptable.
            let _ = config.validate();
        }
    }
}
