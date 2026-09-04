//! # RavenClaws
//!
//! Lightweight, secure Rust agent framework with multi-provider LLM support.

// Memory-safety pillar: no first-party `unsafe` code is permitted.
// `deny` (rather than `forbid`) so that third-party generated code (e.g. eframe's
// `include_modules!`) may lift the lint within its own module, while our own code
// still cannot use `unsafe`.
#![deny(unsafe_code)]
//!
//! RavenClaws is a single-binary agent runtime that supports:
//! - **Single agent mode** — one prompt, one response
//! - **Swarm mode** — multiple parallel agents with different personas
//! - **Supervisor mode** — task decomposition with sub-agent spawning
//! - **Heartbeat mode** — autonomous long-running agents
//! - **REPL mode** — interactive conversation
//! - **Server mode** — HTTP server with health/metrics endpoints
//! - **MCP server mode** — expose tools over stdio via MCP protocol
//!
//! ## Architecture
//!
//! The crate is organized into 20 modules:
//!
//! | Module | Purpose |
//! |---|---|
//! | [`agent`] | Agent implementations, agent loop, conversation memory |
//! | [`llm`] | LLM provider abstraction + 5 client implementations |
//! | [`config`] | Configuration structs, TOML/env loading, validation |
//! | [`tools`] | Tool abstraction, registry, 5 built-in tools |
//! | [`persistence`] | SQLite-backed conversation persistence with retention policies |
//! | [`plugins`] | WASM plugin system for extending RavenClaws without recompiling |
//! | [`policy`] | Deny-by-default policy engine |
//! | [`sandbox`] | Sandboxed execution (workdir jail, resource limits) |
//! | [`audit`] | Tamper-evident audit log (HMAC-SHA256 chained) |
//! | [`mcp`] | MCP client + server (JSON-RPC 2.0 over stdio + SSE) |
//! | [`swarm`] | Swarm orchestration, worker profiles, health monitoring |
//! | [`heartbeat`] | Autonomous heartbeat agent |
//! | [`background`] | Background task manager with disk persistence |
//! | [`scheduler`] | Scheduling & triggers (cron, webhook, file-watch) |
//! | [`server`] | HTTP server mode (health, readiness, metrics) |
//! | [`telemetry`] | OpenTelemetry tracing (OTLP gRPC/stdout) |
//! | [`ravenfabric`] | RavenFabric mesh client |
//! | [`eval`] | Eval harness with assertions and run traces |
//! | [`error`] | Unified error types |
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use ravenclaws::config::Config;
//! use ravenclaws::llm::{create_client, LLMProviderTrait};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Config::load(None)?;
//! let llm = create_client(&config.llm)?;
//! let response = llm.chat(vec![
//!     ravenclaws::llm::ChatMessage::new("user", "Hello!"),
//! ]).await?;
//! println!("{}", response.choices[0].message.content);
//! # Ok(())
//! # }
//! ```
//!
//! ## Security
//!
//! RavenClaws uses a deny-by-default security model:
//! - All tool calls are validated by [`PolicyEngine`] before execution
//! - Shell commands execute in a [`Sandbox`] with resource limits
//! - All operations are logged to a tamper-evident [`AuditLog`]
//! - API keys are zeroized on drop
//!
//! ## Feature Flags
//!
//! - `otel-grpc` (default) — OpenTelemetry tracing via OTLP gRPC exporter
//! - `otel-stdout` — OpenTelemetry tracing via stdout exporter
//!
//! ## Minimum Supported Rust Version (MSRV)
//!
//! Rust 1.88 or later. This crate uses edition 2021.
//!
//! ## Semver Guarantees
//!
//! RavenClaws follows semantic versioning. The public API consists of all items
//! documented in this module and re-exported below. Items marked `#[doc(hidden)]`
//! or in `__private` modules are not part of the public API and may change in
//! minor releases.
//!
//! All public enums and structs are `#[non_exhaustive]` — new variants/fields may
//! be added in minor releases. Match statements on enums must include a wildcard
//! arm, and struct literals must use `..` syntax.

pub mod agent;
pub mod audit;
pub mod background;
pub mod blueprint;
pub mod config;
pub mod error;
pub mod eval;
pub mod healing;
pub mod heartbeat;
pub mod integrations;
#[cfg(feature = "k8s")]
pub mod k8s;
pub mod llm;
pub mod load;
pub mod mcp;
pub mod patterns;
pub mod persistence;
#[cfg(feature = "plugins")]
pub mod plugins;
pub mod policy;
pub mod ravenfabric;
pub mod sandbox;
pub mod scheduler;
pub mod server;
pub mod swarm;
pub mod telemetry;
pub mod tools;
pub mod ui;
pub mod web_policy;

// ── Re-exports of commonly used types ──────────────────────────────────────

pub use agent::{
    delete_checkpoint, load_checkpoint, run_agent_loop, run_agent_loop_with_images,
    run_agent_loop_with_mcp, run_agent_loop_with_mcp_and_images,
    run_agent_loop_with_mcp_and_registry, run_agent_loop_with_registry, save_checkpoint,
    AgentLoopConfig, CheckpointState, ConversationMemory,
};
pub use audit::AuditLog;
pub use background::BackgroundTaskManager;
pub use blueprint::{AgentBlueprint, BlueprintError, BlueprintPersona};
pub use config::{
    Config, LLMConfig, LLMProvider, McpConfig, McpServerConfig, RuntimeConfig, SecurityConfig,
};
pub use error::RavenClawsError;
pub use eval::EvalRunner;
pub use healing::{HealingCircuitBreaker, HealingCircuitState, HealingConfig, SelfHealingEngine};
pub use heartbeat::HeartbeatAgent;
pub use integrations::{
    send_discord, send_email, send_matrix, send_signal, send_slack, send_sms, send_teams,
    send_telegram, IntegrationResult,
};
#[cfg(feature = "k8s")]
pub use k8s::{K8sManager, K8sManagerConfig};
pub use llm::{
    create_client, load_image, ChatMessage, ChatResponse, ContentPart, CostSummary, CostTracker,
    ImageUrlContent, LLMProviderTrait, LocalInference, LocalInferenceKind, LocalInferenceServer,
    ModelPricing, MultiModelManager, WarmupResult,
};
pub use load::{Admission, LoadConfig, LoadManager, LoadMetrics, RequestOutcome};
pub use mcp::{McpClient, McpClientManager, McpServer, McpSseServer};
pub use patterns::{
    run_debate, run_debate_multi, run_research_synthesize, run_research_synthesize_multi,
    run_review_loop, run_review_loop_multi, run_voting, run_voting_multi, PatternConfig,
};
pub use persistence::{
    ConversationStore, MemoryEntry, MemoryStore, MemoryTier, RetentionPolicy, StoredMessage,
    StoredSession, TieredMemory,
};
#[cfg(feature = "plugins")]
pub use plugins::{PluginError, PluginTool, WasmPlugin, WasmPluginManager};
pub use policy::PolicyEngine;
pub use ravenfabric::RavenFabricClient;
pub use sandbox::{Sandbox, SandboxSnapshot};
pub use scheduler::Scheduler;
pub use server::run_server;
pub use swarm::SwarmOrchestrator;
pub use telemetry::TelemetryGuard;
pub use tools::{BrowserTool, PageState, ToolCall, ToolImpl, ToolRegistry, ToolResult};
pub use ui::ChatEngine;
pub use web_policy::{extract_domain, RateLimiter, WebAccessPolicy, WebCategory};
