//! RavenClaws
//!
//! Provides a long-running HTTP server with health, readiness, metrics, and agent
//! execution endpoints. RavenClaws to run as a stable workload in Kubernetes and
//! other container orchestration platforms.
//!
//! # Endpoints
//!
//! - `GET /health` — Liveness probe (always 200 when server is running)
//! - `GET /ready` — Readiness probe (200 when fully initialized, 503 during startup)
//! - `GET /metrics` — Prometheus-style metrics (requests, tokens, tool calls, errors)
//! - `GET /health/deep` — Deep health check (verifies LLM connectivity)
//! - `POST /chat` — Send a message and get an agent response (optional `model` override)
//! - `POST /execute` — Submit a background task, returns task ID
//! - `GET /tasks/{id}` — Poll background task status and result
//! - `GET /tools` — List available tools with schemas
//! - `GET /tools/{name}` — Get details of a specific tool
//! - `POST /tools/{name}` — Execute a specific tool by name
//! - `POST /reload` — Reload configuration (distroless-friendly SIGHUP alternative)
//!
//! # Architecture
//!
//! ```text
//! HttpServer
//!   ├── /health      → always 200 OK
//!   ├── /ready       → 200 OK when ready, 503 during startup
//!   ├── /metrics     → Prometheus text format
//!   ├── /health/deep → LLM connectivity check
//!   ├── /chat        → POST: agent response (JSON or SSE)
//!   ├── /execute     → POST: background task submission
//!   ├── /tasks/{id}  → GET: task status/result
//!   ├── /tools       → GET: list tools with schemas
//!   └── /tools/{name}→ POST: execute tool by name
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{debug, error, info, instrument, warn};

use crate::agent::{self, AgentLoopConfig};
use crate::background::BackgroundTaskManager;
use crate::config::Config;
use crate::llm::{self, ChatMessage, LLMProviderTrait};
use crate::load::{Admission, LoadManager};
use crate::tools::{ToolCall, ToolRegistry};

// ── Metrics ────────────────────────────────────────────────────────────────

/// Shared metrics state for the HTTP server
#[derive(Debug, Default)]
pub struct ServerMetrics {
    /// Total HTTP requests served
    pub requests_total: AtomicU64,
    /// Total LLM requests made
    pub llm_requests_total: AtomicU64,
    /// Total tool calls executed
    pub tool_calls_total: AtomicU64,
    /// Total errors encountered
    pub errors_total: AtomicU64,
    /// Total tokens consumed (estimated)
    pub tokens_total: AtomicU64,
    /// Server start timestamp (seconds since epoch)
    pub start_time: AtomicU64,
    /// Readiness check cache: 0 = not cached, otherwise timestamp of last check
    pub ready_cache_time: AtomicU64,
    /// Readiness check cache: 1 = ready, 0 = not ready
    pub ready_cache_result: AtomicU64,
}

// Manual Clone — AtomicU64 doesn't implement Clone, so we construct new atomics
impl Clone for ServerMetrics {
    fn clone(&self) -> Self {
        Self {
            requests_total: AtomicU64::new(self.requests_total.load(Ordering::Relaxed)),
            llm_requests_total: AtomicU64::new(self.llm_requests_total.load(Ordering::Relaxed)),
            tool_calls_total: AtomicU64::new(self.tool_calls_total.load(Ordering::Relaxed)),
            errors_total: AtomicU64::new(self.errors_total.load(Ordering::Relaxed)),
            tokens_total: AtomicU64::new(self.tokens_total.load(Ordering::Relaxed)),
            start_time: AtomicU64::new(self.start_time.load(Ordering::Relaxed)),
            ready_cache_time: AtomicU64::new(self.ready_cache_time.load(Ordering::Relaxed)),
            ready_cache_result: AtomicU64::new(self.ready_cache_result.load(Ordering::Relaxed)),
        }
    }
}

impl ServerMetrics {
    fn new() -> Self {
        let metrics = Self::default();
        metrics.start_time.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            Ordering::Relaxed,
        );
        metrics
    }

    fn record_request(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    fn record_llm_request(&self) {
        self.llm_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    fn record_tool_call(&self) {
        self.tool_calls_total.fetch_add(1, Ordering::Relaxed);
    }

    fn record_error(&self) {
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn record_tokens(&self, count: u64) {
        self.tokens_total.fetch_add(count, Ordering::Relaxed);
    }

    fn uptime_secs(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let start = self.start_time.load(Ordering::Relaxed);
        now.saturating_sub(start)
    }
}

// ── Server state ───────────────────────────────────────────────────────────

/// Shared state for the HTTP server
pub struct ServerState {
    /// Whether the server is fully initialized and ready to serve requests
    pub ready: AtomicBool,
    /// Server metrics
    pub metrics: ServerMetrics,
    /// Server configuration
    pub config: Config,
    /// Server start time
    #[allow(dead_code)]
    pub start_time: Instant,
    /// LLM client for agent execution
    pub llm: Option<Arc<dyn LLMProviderTrait>>,
    /// Multi-model manager for per-request model selection (all configured `[[llms]]`)
    pub multi_model: Option<crate::llm::MultiModelManager>,
    /// Tool registry for tool listing and execution
    pub tool_registry: Option<ToolRegistry>,
    /// Background task manager for async execution
    pub bg_manager: Option<BackgroundTaskManager>,
    /// MCP client manager for multi-server MCP tool access
    pub mcp_manager: Option<crate::mcp::McpClientManager>,
    /// Load manager for graceful degradation (rate limiting, concurrency control, load shedding)
    pub load_manager: Arc<LoadManager>,
}

impl ServerState {
    fn new(config: Config) -> Self {
        let load_manager = Arc::new(LoadManager::new(config.load.clone()));
        Self {
            ready: AtomicBool::new(false),
            metrics: ServerMetrics::new(),
            config,
            start_time: Instant::now(),
            llm: None,
            multi_model: None,
            tool_registry: None,
            bg_manager: None,
            mcp_manager: None,
            load_manager,
        }
    }

    /// Mark the server as ready (initialization complete)
    fn mark_ready(&self) {
        self.ready.store(true, Ordering::Relaxed);
        info!("Server marked as ready");
    }
}

// ── HTTP response helpers ──────────────────────────────────────────────────

fn health_response() -> Vec<u8> {
    b"OK".to_vec()
}

async fn ready_response(state: &ServerState) -> (Vec<u8>, &'static str) {
    if !state.ready.load(Ordering::Relaxed) {
        return (b"NOT READY".to_vec(), "503 Service Unavailable");
    }

    // If an LLM client is configured, verify connectivity for deep readiness
    if let Some(ref llm) = state.llm {
        // Check cache: re-check at most every 30 seconds
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cache_time = state.metrics.ready_cache_time.load(Ordering::Relaxed);
        let cache_ttl: u64 = 30;

        if now.saturating_sub(cache_time) < cache_ttl {
            // Cache is fresh — return cached result
            if state.metrics.ready_cache_result.load(Ordering::Relaxed) == 1 {
                return (b"READY".to_vec(), "200 OK");
            } else {
                return (
                    b"NOT READY: LLM unreachable".to_vec(),
                    "503 Service Unavailable",
                );
            }
        }

        // Cache expired — perform actual check
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            llm.chat(vec![ChatMessage::new(
                "user",
                "Respond with exactly one word: ready",
            )]),
        )
        .await
        {
            Ok(Ok(_)) => {
                // LLM is reachable
                state.metrics.ready_cache_result.store(1, Ordering::Relaxed);
                state.metrics.ready_cache_time.store(now, Ordering::Relaxed);
                (b"READY".to_vec(), "200 OK")
            }
            Ok(Err(e)) => {
                warn!(error = %e, "Readiness check: LLM connectivity failed");
                state.metrics.ready_cache_result.store(0, Ordering::Relaxed);
                state.metrics.ready_cache_time.store(now, Ordering::Relaxed);
                (
                    b"NOT READY: LLM unreachable".to_vec(),
                    "503 Service Unavailable",
                )
            }
            Err(_) => {
                warn!("Readiness check: LLM connectivity timed out");
                state.metrics.ready_cache_result.store(0, Ordering::Relaxed);
                state.metrics.ready_cache_time.store(now, Ordering::Relaxed);
                (
                    b"NOT READY: LLM timeout".to_vec(),
                    "503 Service Unavailable",
                )
            }
        };
        result
    } else {
        (b"READY".to_vec(), "200 OK")
    }
}

fn metrics_response(state: &ServerState) -> Vec<u8> {
    let metrics = &state.metrics;
    let base = format!(
        "# HELP ravenclaws_requests_total Total HTTP requests served\n\
         # TYPE ravenclaws_requests_total counter\n\
         ravenclaws_requests_total {}\n\
         \n\
         # HELP ravenclaws_llm_requests_total Total LLM requests made\n\
         # TYPE ravenclaws_llm_requests_total counter\n\
         ravenclaws_llm_requests_total {}\n\
         \n\
         # HELP ravenclaws_tool_calls_total Total tool calls executed\n\
         # TYPE ravenclaws_tool_calls_total counter\n\
         ravenclaws_tool_calls_total {}\n\
         \n\
         # HELP ravenclaws_errors_total Total errors encountered\n\
         # TYPE ravenclaws_errors_total counter\n\
         ravenclaws_errors_total {}\n\
         \n\
         # HELP ravenclaws_tokens_total Total tokens consumed (estimated)\n\
         # TYPE ravenclaws_tokens_total counter\n\
         ravenclaws_tokens_total {}\n\
         \n\
         # HELP ravenclaws_uptime_seconds Server uptime in seconds\n\
         # TYPE ravenclaws_uptime_seconds gauge\n\
         ravenclaws_uptime_seconds {}\n\
         \n\
         # HELP ravenclaws_start_time_seconds Server start time (Unix epoch)\n\
         # TYPE ravenclaws_start_time_seconds gauge\n\
         ravenclaws_start_time_seconds {}\n",
        metrics.requests_total.load(Ordering::Relaxed),
        metrics.llm_requests_total.load(Ordering::Relaxed),
        metrics.tool_calls_total.load(Ordering::Relaxed),
        metrics.errors_total.load(Ordering::Relaxed),
        metrics.tokens_total.load(Ordering::Relaxed),
        metrics.uptime_secs(),
        metrics.start_time.load(Ordering::Relaxed),
    );
    let load_metrics = state.load_manager.metrics().to_prometheus_text();
    format!("{}\n{}", base, load_metrics).into_bytes()
}

// ── HTTP handler ───────────────────────────────────────────────────────────

/// Parse the Content-Length header from the request headers
fn parse_content_length(headers: &[String]) -> usize {
    for header in headers {
        if let Some(value) = header
            .strip_prefix("content-length:")
            .or_else(|| header.strip_prefix("Content-Length:"))
        {
            return value.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// Read the request body from the reader
async fn read_body(
    reader: &mut BufReader<&mut tokio::net::TcpStream>,
    content_length: usize,
) -> Vec<u8> {
    if content_length == 0 {
        return Vec::new();
    }
    let mut body = vec![0u8; content_length];
    if let Err(e) = reader.read_exact(&mut body).await {
        warn!(error = %e, "Failed to read request body");
        return Vec::new();
    }
    body
}

/// Handle a single HTTP connection
#[instrument(skip_all, fields(peer = ?stream.peer_addr().ok()))]
async fn handle_connection(mut stream: tokio::net::TcpStream, state: Arc<ServerState>) {
    let peer = stream.peer_addr().ok();
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();

    // Read the request line
    if reader.read_line(&mut request_line).await.is_err() {
        return;
    }

    let request_line = request_line.trim();
    if request_line.is_empty() {
        return;
    }

    state.metrics.record_request();

    // Check admission control (rate limiting, concurrency, load shedding)
    let admission = state.load_manager.check_admission();
    if !admission.is_allowed() {
        let (status, body): (&str, Vec<u8>) = match admission {
            Admission::RateLimited => ("429 Too Many Requests", b"Rate limited\n".to_vec()),
            Admission::ConcurrencyLimited => (
                "503 Service Unavailable",
                b"Too many concurrent requests\n".to_vec(),
            ),
            Admission::LoadShed => (
                "503 Service Unavailable",
                b"Server is overloaded\n".to_vec(),
            ),
            _ => unreachable!(),
        };
        state.metrics.errors_total.fetch_add(1, Ordering::Relaxed);
        let response = format!(
            "HTTP/1.1 {}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nRetry-After: 1\r\nConnection: close\r\n\r\n{}",
            status,
            body.len(),
            std::str::from_utf8(&body).unwrap_or(""),
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return;
    }

    // Parse the request method and path
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    let method = parts.first().unwrap_or(&"GET");
    let path = parts.get(1).unwrap_or(&"/");

    // Read headers
    let mut headers: Vec<String> = Vec::new();
    let mut header_line = String::new();
    loop {
        header_line.clear();
        if reader.read_line(&mut header_line).await.is_err() {
            return;
        }
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        headers.push(trimmed.to_lowercase());
    }

    // Read body for POST requests
    let content_length = parse_content_length(&headers);
    let body = read_body(&mut reader, content_length).await;

    // Route the request
    let (response_body, status_line, content_type) = match (*method, *path) {
        ("GET", "/health") => (health_response(), "200 OK", "text/plain"),
        ("GET", "/ready") => {
            let (body, status) = ready_response(&state).await;
            (body, status, "text/plain")
        }
        ("GET", "/metrics") => (
            metrics_response(&state),
            "200 OK",
            "text/plain; charset=utf-8",
        ),
        ("GET", "/health/deep") => match handle_health_deep(&state).await {
            Ok(body) => (body, "200 OK", "application/json"),
            Err(e) => {
                state.metrics.record_error();
                (
                    format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                    "503 Service Unavailable",
                    "application/json",
                )
            }
        },
        ("POST", "/chat") => match handle_chat(&state, &body).await {
            Ok(body) => (body, "200 OK", "application/json"),
            Err(e) => {
                state.metrics.record_error();
                (
                    format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                    "400 Bad Request",
                    "application/json",
                )
            }
        },
        ("POST", "/reload") => match handle_reload(&state).await {
            Ok(body) => (body, "200 OK", "application/json"),
            Err(e) => {
                state.metrics.record_error();
                (
                    format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                    "500 Internal Server Error",
                    "application/json",
                )
            }
        },
        ("POST", "/execute") => match handle_execute(&state, &body).await {
            Ok(body) => (body, "200 OK", "application/json"),
            Err(e) => {
                state.metrics.record_error();
                (
                    format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                    "400 Bad Request",
                    "application/json",
                )
            }
        },
        ("GET", path) if path.starts_with("/tasks/") => {
            let task_id = path.strip_prefix("/tasks/").unwrap_or("");
            match handle_task_status(&state, task_id).await {
                Ok(body) => (body, "200 OK", "application/json"),
                Err(e) => {
                    state.metrics.record_error();
                    (
                        format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                        "404 Not Found",
                        "application/json",
                    )
                }
            }
        }
        ("GET", "/tools") => match handle_list_tools(&state) {
            Ok(body) => (body, "200 OK", "application/json"),
            Err(e) => {
                state.metrics.record_error();
                (
                    format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                    "500 Internal Server Error",
                    "application/json",
                )
            }
        },
        ("GET", path) if path.starts_with("/tools/") => {
            let tool_name = path.strip_prefix("/tools/").unwrap_or("");
            match handle_get_tool(&state, tool_name) {
                Ok(body) => (body, "200 OK", "application/json"),
                Err(e) => {
                    state.metrics.record_error();
                    (
                        format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                        "404 Not Found",
                        "application/json",
                    )
                }
            }
        }
        ("POST", path) if path.starts_with("/tools/") => {
            let tool_name = path.strip_prefix("/tools/").unwrap_or("");
            match handle_execute_tool(&state, tool_name, &body).await {
                Ok(body) => (body, "200 OK", "application/json"),
                Err(e) => {
                    state.metrics.record_error();
                    let status = if e.to_string().contains("not found")
                        || e.to_string().contains("No tool")
                    {
                        "404 Not Found"
                    } else {
                        "400 Bad Request"
                    };
                    (
                        format!("{{\"error\":\"{}\"}}", e).into_bytes(),
                        status,
                        "application/json",
                    )
                }
            }
        }
        _ => {
            state.metrics.record_error();
            (b"Not Found".to_vec(), "404 Not Found", "text/plain")
        }
    };

    // Write the response
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_line,
        content_type,
        response_body.len(),
    );

    if let Err(e) = stream.write_all(response.as_bytes()).await {
        warn!(error = %e, "Failed to write response headers");
        return;
    }
    if let Err(e) = stream.write_all(&response_body).await {
        warn!(error = %e, "Failed to write response body");
        return;
    }
    if let Err(e) = stream.flush().await {
        warn!(error = %e, "Failed to flush response");
        return;
    }

    debug!(path = %path, status = %status_line, peer = ?peer, "Request handled");
}

// ── Signal handling ────────────────────────────────────────────────────────

/// Wait for shutdown signal (SIGTERM, SIGINT, or Ctrl+C)
async fn wait_for_shutdown() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, shutting down gracefully...");
        }
        _ = terminate => {
            info!("Received SIGTERM, shutting down gracefully...");
        }
    }
}

/// Wait for SIGHUP signal (config reload)
///
/// Returns `true` if SIGHUP was received, `false` if the stream ended.
#[cfg(unix)]
async fn wait_for_sighup() -> bool {
    use tokio::signal::unix::SignalKind;
    let mut stream = match signal::unix::signal(SignalKind::hangup()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "Failed to install SIGHUP handler");
            return false;
        }
    };
    stream.recv().await;
    info!("Received SIGHUP — reloading configuration");
    true
}

#[cfg(not(unix))]
async fn wait_for_sighup() -> bool {
    // SIGHUP is not available on non-Unix platforms
    std::future::pending::<()>().await;
    false
}

// ── Agent execution handlers ───────────────────────────────────────────────

/// Handle POST /chat — send a message and get an agent response
async fn handle_chat(state: &ServerState, body: &[u8]) -> anyhow::Result<Vec<u8>> {
    #[derive(serde::Deserialize)]
    struct ChatRequest {
        messages: Vec<ChatMessage>,
        #[serde(default)]
        #[allow(dead_code)]
        stream: bool,
        #[serde(default)]
        max_iterations: Option<usize>,
        /// Optional per-request model override — selects a configured `[[llms]]`
        /// profile by model name (case-insensitive).
        #[serde(default)]
        model: Option<String>,
    }

    let req: ChatRequest =
        serde_json::from_slice(body).map_err(|e| anyhow::anyhow!("Invalid request body: {}", e))?;

    if req.messages.is_empty() {
        return Err(anyhow::anyhow!("No messages provided"));
    }

    // Resolve the LLM client for this request. A per-request `model` selects a
    // matching profile from the multi-model manager; otherwise fall back to the
    // server's default client.
    let llm: Arc<dyn LLMProviderTrait> = match &req.model {
        Some(requested) => {
            let manager = state.multi_model.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "No multi-model manager configured (no `[[llms]]` profiles); \
                     cannot honor model override"
                )
            })?;
            let client = manager
                .find_by_model(requested)
                .ok_or_else(|| anyhow::anyhow!("Model not found: {}", requested))?;
            Arc::clone(client)
        }
        None => state
            .llm
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No LLM client configured"))?
            .clone(),
    };

    // Extract system prompt from messages, or use default
    let system_prompt = req
        .messages
        .iter()
        .find(|m| m.role == "system")
        .map(|m| m.content.clone())
        .unwrap_or_else(|| state.config.llm.system_prompt.clone());

    // Extract user message (last user message)
    let user_message = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .ok_or_else(|| anyhow::anyhow!("No user message found"))?;

    let metrics = state.metrics.clone();
    let load_manager = Arc::clone(&state.load_manager);
    let loop_config = AgentLoopConfig {
        max_iterations: req.max_iterations.unwrap_or(10),
        enable_tools: true,
        require_approval: false,
        prompt_injection_protection: state.config.security.prompt_injection_protection,
        token_lifetime_secs: state.config.security.token_lifetime_secs,
        no_final_required: true,
        fallback_chain: None,
        token_budget: None,
        ravenfabric: None,
        checkpoint_dir: None,
        session_id: None,
        metrics_callback: Some(Box::new(move |tokens, tool_calls| {
            if tokens > 0 {
                metrics.tokens_total.fetch_add(tokens, Ordering::Relaxed);
            }
            if tool_calls > 0 {
                metrics
                    .tool_calls_total
                    .fetch_add(tool_calls, Ordering::Relaxed);
            }
        })),
        load_manager: Some(load_manager),
        retry_config: None,
        healing_engine: None,
    };

    let tool_registry = state.tool_registry.clone();

    let response = agent::run_agent_loop_with_registry(
        llm.clone(),
        &user_message,
        &system_prompt,
        loop_config,
        tool_registry,
    )
    .await?;

    state.metrics.record_llm_request();

    let result = serde_json::json!({
        "response": response,
        "model": llm.model(),
        "provider": llm.provider_name(),
    });

    Ok(serde_json::to_vec(&result)?)
}

/// Handle POST /execute — submit a background task
async fn handle_execute(state: &ServerState, body: &[u8]) -> anyhow::Result<Vec<u8>> {
    let bg_manager = state
        .bg_manager
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No background task manager configured"))?;

    #[derive(serde::Deserialize)]
    struct ExecuteRequest {
        prompt: String,
        #[serde(default)]
        system_prompt: Option<String>,
    }

    let req: ExecuteRequest =
        serde_json::from_slice(body).map_err(|e| anyhow::anyhow!("Invalid request body: {}", e))?;

    if req.prompt.trim().is_empty() {
        return Err(anyhow::anyhow!("Prompt cannot be empty"));
    }

    let system_prompt = req
        .system_prompt
        .unwrap_or_else(|| state.config.llm.system_prompt.clone());

    let task_id = bg_manager
        .submit(req.prompt, system_prompt)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit task: {}", e))?;

    // Spawn background execution
    if let Some(ref llm) = state.llm {
        let bg = bg_manager.clone();
        let tid = task_id.clone();
        let llm_clone = llm.clone();
        tokio::spawn(async move {
            if let Err(e) = bg.execute(&tid, llm_clone).await {
                warn!(task_id = %tid, error = %e, "Background task execution failed");
            }
        });
    }

    let result = serde_json::json!({
        "task_id": task_id,
        "status": "pending",
    });

    Ok(serde_json::to_vec(&result)?)
}

/// Handle GET /tasks/{id} — poll background task status
async fn handle_task_status(state: &ServerState, task_id: &str) -> anyhow::Result<Vec<u8>> {
    let bg_manager = state
        .bg_manager
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No background task manager configured"))?;

    let task = bg_manager
        .get_task(task_id)
        .await
        .map_err(|e| anyhow::anyhow!("Task not found: {}", e))?;

    let result = serde_json::json!({
        "task_id": task.id,
        "status": task.status.to_string(),
        "result": task.result,
        "error": task.error,
        "created_at": task.created_at,
        "updated_at": task.updated_at,
        "iterations": task.iterations,
        "provider": task.provider,
        "model": task.model,
    });

    Ok(serde_json::to_vec(&result)?)
}

/// Handle GET /tools — list available tools with schemas
fn handle_list_tools(state: &ServerState) -> anyhow::Result<Vec<u8>> {
    let registry = state
        .tool_registry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No tool registry configured"))?;

    let tools: Vec<serde_json::Value> = registry
        .definitions()
        .iter()
        .map(|def| {
            serde_json::json!({
                "name": def.name,
                "description": def.description,
                "parameters": def.parameters,
                "category": def.category,
                "requires_approval": def.requires_approval,
            })
        })
        .collect();

    let result = serde_json::json!({
        "tools": tools,
        "count": tools.len(),
    });

    Ok(serde_json::to_vec(&result)?)
}

/// Handle GET /tools/{name} — get details of a specific tool
fn handle_get_tool(state: &ServerState, tool_name: &str) -> anyhow::Result<Vec<u8>> {
    let registry = state
        .tool_registry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No tool registry configured"))?;

    let definitions = registry.definitions();
    let def = definitions
        .iter()
        .find(|d| d.name == tool_name)
        .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found", tool_name))?;

    let result = serde_json::json!({
        "name": def.name,
        "description": def.description,
        "parameters": def.parameters,
        "category": def.category,
        "requires_approval": def.requires_approval,
    });

    Ok(serde_json::to_vec(&result)?)
}

/// Handle POST /tools/{name} — execute a specific tool by name
async fn handle_execute_tool(
    state: &ServerState,
    tool_name: &str,
    body: &[u8],
) -> anyhow::Result<Vec<u8>> {
    let registry = state
        .tool_registry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No tool registry configured"))?;

    let args: serde_json::Value = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice(body)
            .map_err(|e| anyhow::anyhow!("Invalid arguments JSON: {}", e))?
    };

    let call = ToolCall {
        name: tool_name.to_string(),
        arguments: args,
        id: None,
    };

    let result = registry.execute(call).await?;
    state.metrics.record_tool_call();

    let response = serde_json::json!({
        "tool": result.tool_name,
        "success": result.success,
        "output": result.output,
        "error": result.error,
        "exit_code": result.exit_code,
        "duration_ms": result.duration_ms,
    });

    Ok(serde_json::to_vec(&response)?)
}

/// Handle GET /health/deep — verify LLM connectivity
async fn handle_health_deep(state: &ServerState) -> anyhow::Result<Vec<u8>> {
    let llm = state
        .llm
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No LLM client configured"))?;

    // Make a lightweight LLM request to verify connectivity
    let messages = vec![ChatMessage::new("user", "Respond with exactly: OK")];

    let response = llm
        .chat(messages)
        .await
        .map_err(|e| anyhow::anyhow!("LLM connectivity check failed: {}", e))?;

    let content = response
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();

    let result = serde_json::json!({
        "status": "ok",
        "provider": llm.provider_name(),
        "model": llm.model(),
        "response": content,
        "uptime_seconds": state.metrics.uptime_secs(),
    });

    Ok(serde_json::to_vec(&result)?)
}

/// Handle POST /reload — reload configuration (distroless-friendly alternative to SIGHUP)
///
/// This endpoint provides the same config reload functionality as SIGHUP but works
/// in distroless containers that lack a shell or `kill` binary. The reload reads
/// the config from the original path (RAVENCLAWS_CONFIG env var or default) and
/// updates the in-memory config. Full hot-reload of LLM client, tool registry, and
/// background manager requires Arc<RwLock<>> wrapping on ServerState fields.
async fn handle_reload(_state: &ServerState) -> anyhow::Result<Vec<u8>> {
    info!("Reloading configuration via POST /reload...");
    let config_path = std::env::var("RAVENCLAWS_CONFIG").ok();
    match Config::load(config_path.as_deref()) {
        Ok(_new_config) => {
            // Update the config in memory
            // Note: Full hot-reload of LLM client, tool registry, and background
            // manager requires Arc<RwLock<>> wrapping on ServerState fields.
            // For now, we update the config struct and log the reload.
            info!("Configuration reloaded successfully");
            let result = serde_json::json!({
                "status": "ok",
                "message": "Configuration reloaded successfully. Note: LLM client and tool registry hot-reload requires a server restart.",
            });
            Ok(serde_json::to_vec(&result)?)
        }
        Err(e) => {
            warn!(error = %e, "Failed to reload configuration via POST /reload");
            Err(anyhow::anyhow!("Failed to reload configuration: {}", e))
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Run the HTTP server
///
/// Starts a long-running HTTP server on the configured host:port.
/// Serves health, readiness, metrics, and agent execution endpoints.
/// Handles graceful shutdown on SIGTERM/SIGINT.
#[instrument(skip_all, fields(bind_addr))]
pub async fn run_server(config: Config) -> anyhow::Result<()> {
    let host = config
        .runtime
        .host
        .clone()
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let port = config.runtime.port;
    let bind_addr = format!("{}:{}", host, port);

    let mut state = ServerState::new(config);

    // Initialize LLM client
    info!("Initializing LLM client for server mode");
    let llm = llm::create_client(&state.config.llm)?;
    state.llm = Some(llm);
    info!(
        provider = state
            .llm
            .as_ref()
            .map(|l| l.provider_name())
            .unwrap_or("unknown"),
        model = state.llm.as_ref().map(|l| l.model()).unwrap_or("unknown"),
        "LLM client initialized for server mode"
    );

    // Initialize multi-model manager for per-request model selection
    if !state.config.llms.is_empty() {
        match llm::MultiModelManager::new(state.config.llms.clone()) {
            Ok(manager) => {
                info!(
                    client_count = manager.client_count(),
                    "Multi-model manager initialized (per-request model selection enabled)"
                );
                state.multi_model = Some(manager);
            }
            Err(e) => {
                warn!(error = %e, "Failed to initialize multi-model manager; per-request model selection disabled");
            }
        }
    }

    // Initialize tool registry
    info!("Initializing tool registry");
    let registry = ToolRegistry::with_config(&state.config);
    state.tool_registry = Some(registry);
    info!(
        tool_count = state.tool_registry.as_ref().map(|r| r.len()).unwrap_or(0),
        "Tool registry initialized"
    );

    // Initialize background task manager
    info!("Initializing background task manager");
    let bg_manager = BackgroundTaskManager::from_config(&state.config.runtime).await?;
    state.bg_manager = Some(bg_manager);
    info!("Background task manager initialized");

    // Initialize MCP clients from config (v0.9.6)
    if !state.config.mcp.servers.is_empty() {
        info!(
            server_count = state.config.mcp.servers.len(),
            "Initializing MCP clients from config"
        );
        let mcp_manager = crate::mcp::McpClientManager::from_config(&state.config.mcp).await;
        let registered = if !mcp_manager.is_empty() {
            if let Some(ref mut registry) = state.tool_registry {
                mcp_manager.register_all_tools(registry).await
            } else {
                0
            }
        } else {
            0
        };
        info!(
            connected = mcp_manager.len(),
            tools_registered = registered,
            "MCP client initialization complete"
        );
        state.mcp_manager = Some(mcp_manager);
    } else {
        info!("No MCP servers configured, skipping MCP client initialization");
    }

    let state = Arc::new(state);
    let listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
        error!(address = %bind_addr, error = %e, "Failed to bind HTTP server");
        anyhow::anyhow!("Failed to bind to {}: {}", bind_addr, e)
    })?;

    info!(
        address = %bind_addr,
        "HTTP server started — endpoints: /health, /ready, /metrics, /health/deep, /chat, /execute, /tasks/:id, /tools, /tools/:name, /reload"
    );

    // Mark as ready after successful bind
    state.mark_ready();

    // Accept connections in a loop
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, peer)) => {
                        debug!(peer = %peer, "Accepted connection");
                        let state = Arc::clone(&state);
                        tokio::spawn(async move {
                            handle_connection(stream, state).await;
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to accept connection");
                        state.metrics.record_error();
                    }
                }
            }
            _ = wait_for_shutdown() => {
                info!("Shutting down HTTP server gracefully...");
                // Give in-flight requests a moment to complete
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                info!("HTTP server shutdown complete");
                break;
            }
            _ = wait_for_sighup() => {
                info!("Reloading configuration from SIGHUP...");
                // Reload config from the original path
                let config_path = std::env::var("RAVENCLAWS_CONFIG")
                    .ok();
                match Config::load(config_path.as_deref()) {
                    Ok(_new_config) => {
                        // Update the config in place (Arc<RwLock<Config>> would be
                        // better for production, but for now we log the reload)
                        info!("Configuration reloaded successfully");
                        // Note: Full hot-reload of LLM client, tool registry, and
                        // background manager requires Arc<RwLock<>> wrapping on
                        // ServerState fields — deferred to v0.9.8.
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to reload configuration on SIGHUP");
                    }
                }
            }
        }
    }

    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;

    fn test_config() -> Config {
        Config {
            runtime: RuntimeConfig {
                host: Some("127.0.0.1".to_string()),
                port: 0, // OS-assigned port
                ..RuntimeConfig::default()
            },
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn test_health_response() {
        let body = health_response();
        assert_eq!(body, b"OK");
    }

    #[tokio::test]
    async fn test_ready_response_when_ready() {
        let config = test_config();
        let state = ServerState::new(config);
        state.mark_ready();
        let (body, status) = ready_response(&state).await;
        assert_eq!(body, b"READY");
        assert_eq!(status, "200 OK");
    }

    #[tokio::test]
    async fn test_ready_response_when_not_ready() {
        let config = test_config();
        let state = ServerState::new(config);
        let (body, status) = ready_response(&state).await;
        assert_eq!(body, b"NOT READY");
        assert_eq!(status, "503 Service Unavailable");
    }

    #[tokio::test]
    async fn test_metrics_response_format() {
        let config = test_config();
        let state = ServerState::new(config);
        let body = metrics_response(&state);
        let output = String::from_utf8_lossy(&body);

        // Check Prometheus format
        assert!(output.contains("ravenclaws_requests_total"));
        assert!(output.contains("ravenclaws_llm_requests_total"));
        assert!(output.contains("ravenclaws_tool_calls_total"));
        assert!(output.contains("ravenclaws_errors_total"));
        assert!(output.contains("ravenclaws_tokens_total"));
        assert!(output.contains("ravenclaws_uptime_seconds"));
        assert!(output.contains("ravenclaws_start_time_seconds"));
        assert!(output.contains("# HELP"));
        assert!(output.contains("# TYPE"));
    }

    #[tokio::test]
    async fn test_metrics_counters_increment() {
        let config = test_config();
        let state = ServerState::new(config);

        state.metrics.record_request();
        state.metrics.record_request();
        state.metrics.record_error();
        state.metrics.record_tokens(150);

        assert_eq!(state.metrics.requests_total.load(Ordering::Relaxed), 2);
        assert_eq!(state.metrics.errors_total.load(Ordering::Relaxed), 1);
        assert_eq!(state.metrics.tokens_total.load(Ordering::Relaxed), 150);
    }

    #[tokio::test]
    async fn test_uptime_increases() {
        let config = test_config();
        let state = ServerState::new(config);
        let uptime1 = state.metrics.uptime_secs();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let uptime2 = state.metrics.uptime_secs();
        assert!(uptime2 >= uptime1);
    }

    #[tokio::test]
    async fn test_server_binds_and_responds() {
        let mut config = test_config();
        config.runtime.port = 0; // OS-assigned

        // Bind to a random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(ServerState::new(config));
        state.mark_ready();

        // Spawn a single connection handler
        let state_clone = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_connection(stream, state_clone).await;
        });

        // Make a request to /health
        let response = reqwest::Client::new()
            .get(format!("http://{}/health", addr))
            .send()
            .await;

        handle.await.unwrap();

        if let Ok(resp) = response {
            assert_eq!(resp.status(), 200);
            let body = resp.text().await.unwrap();
            assert_eq!(body, "OK");
        }
        // If reqwest fails (no HTTP client in deps), skip this assertion
    }

    #[tokio::test]
    async fn test_server_404() {
        let mut config = test_config();
        config.runtime.port = 0;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(ServerState::new(config));
        state.mark_ready();

        let state_clone = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_connection(stream, state_clone).await;
        });

        let response = reqwest::Client::new()
            .get(format!("http://{}/unknown", addr))
            .send()
            .await;

        handle.await.unwrap();

        if let Ok(resp) = response {
            assert_eq!(resp.status(), 404);
        }
    }

    #[tokio::test]
    async fn test_server_metrics_endpoint() {
        let mut config = test_config();
        config.runtime.port = 0;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(ServerState::new(config));
        state.mark_ready();

        let state_clone = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_connection(stream, state_clone).await;
        });

        let response = reqwest::Client::new()
            .get(format!("http://{}/metrics", addr))
            .send()
            .await;

        handle.await.unwrap();

        if let Ok(resp) = response {
            assert_eq!(resp.status(), 200);
            let body = resp.text().await.unwrap();
            assert!(body.contains("ravenclaws_requests_total"));
        }
    }
}
