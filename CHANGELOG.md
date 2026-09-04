# Changelog

All notable changes to RavenClaws are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [v1.8.0] — 2026-09-04

### Added
- **Skill bundles (`skill.yaml`)** — new `src/skills.rs` with `SkillManifest`,
  `SkillEntrypoint`, and `SkillSandbox`. A declarative YAML/JSON manifest bundling a
  script command, fixed args, env vars, timeout, and sandbox preferences
  (network/write), layered *over* the existing WASM plugin ABI and `shell_exec`
  sandbox path. `from_yaml()`/`from_json()`/`validate()`/`to_command()` (shell-quoted)
  round out the API. 10 new unit tests. `SkillManifest` re-exported.
- **Config parser fuzz/property coverage** — two deterministic 10k-input tests in
  `src/config.rs`: `test_llm_provider_parse_never_panics_on_adversarial_input` (hammers
  `LLMProvider::parse` with random strings) and
  `test_config_validation_never_panics_on_adversarial_input` (random provider/endpoint
  combos through `Config::validate`). Complements the existing `policy.rs` fuzz tests
  with no external fuzzer dependency (preserves the zero-extra-deps "Small" pillar).
- **Tiered memory — episodic / semantic / procedural** — `MemoryStore::remember()`
  persists memories into three tiers (`MemoryTier::Episodic`/`Semantic`/`Procedural`)
  with a dependency-free deterministic feature-hash embedding (FNV-1a hashing trick,
  L2-normalized, 64-dim). `recall()` filters by tier/scope; `recall_semantic()` ranks
  memories by cosine similarity with a minimum-similarity threshold. No ML runtime is
  pulled in — preserving the ~5 MB "Small" pillar. `MemoryTier`/`TieredMemory`
  re-exported. 8 new unit tests.
- **Per-request `model` override on `/chat`** — `MultiModelManager::find_by_model()`
  (case-insensitive, whitespace-trimming model lookup) plus an optional `"model"`
  field on the `POST /chat` request. When present, the server selects the matching
  configured `[[llms]]` profile for that single request, enabling cost-aware
  multi-model fleets to route per-task models over HTTP. 2 new unit tests.
- **`POST /swarm/synthesize` HTTP endpoint** — one-shot "fan out a prompt to N
  diverse models and synthesize a consensus" endpoint. Accepts `prompt`, optional
  `n_agents` (default 3), and an optional `models` list (empty = all configured
  `[[llms]]` profiles). Fans out to diverse research perspectives across the
  selected models and returns a single synthesized answer plus per-agent findings.
  4 new unit tests.
- **RavenFabricClient integration tests** — 5 mockito-backed HTTP round-trip tests
  covering health, agent listing, execute happy-path, policy-denied (HTTP 403), and
  broadcast fan-out. Closes the prior gap of zero integration coverage for the
  RavenFabric mesh client.

### Fixed
- **Flaky `test_write_and_read_file` / `test_write_file_append`** — both file
  tests shared the same `ravenclaws_test_{pid}` temp directory and each removed it
  on cleanup, causing an intermittent race. Each test now uses a distinct directory.

## [v1.6.0] — 2026-08-20

### Added
- **Cost tracking** — New `CostTracker`, `CostSummary`, and `ModelPricing` types in
  `src/llm.rs`. A per-model cost ledger that accumulates prompt/completion tokens
  across calls and estimates cost from a pricing table (USD per 1M tokens); unknown
  or local models are free. 4 unit tests. Library API.
- **Terminal UI (`tui` feature)** — New `src/ui.rs` with a shared `ChatEngine` plus a
  `ratatui` + `crossterm` terminal interface (`--tui` flag). Streaming transcript,
  input box, Esc/Ctrl-C to quit. Optional feature; default binary unchanged.
- **Graphical UI (`gui` feature)** — Native cross-platform window built with
  **egui/eframe** (`--gui` flag). Immediate-mode, memory-safe, no webview/Electron.
  Optional feature.
- **vLLM + SGLang first-class providers** — New `LLMProvider::Vllm` and
  `LLMProvider::SGLang` variants (and matching `OpenAICompatibleProvider`/
  `LocalInferenceKind`), so vLLM (`http://localhost:8000`) and SGLang
  (`http://localhost:30000`) are configured with `provider = "vllm"` / `"sglang"`
  instead of the generic `openai-compatible`. Local-inference discovery now
  classifies them by their well-known default ports.

## [v1.5.0] — 2026-08-20

### Added
- **Declarative agent blueprints** — New `src/blueprint.rs` with `AgentBlueprint`,
  `BlueprintPersona`, and `BlueprintError`. A TOML/JSON document describing persona +
  LLM + enabled tools + security + max iterations; `validate()` and `to_config()`
  layer it over a base `Config`. New `--blueprint` CLI flag (`RAVENCLAWS_BLUEPRINT`).
  8 unit tests + `examples/blueprints/researcher.toml`.
- **Complexity-based model routing** — `MultiModelManager::route_by_complexity()`
  with `classify_complexity()` and `ComplexityTier` in `src/llm.rs`. Deterministic
  heuristic (Trivial/Simple/Complex) routes prompts to the most appropriate
  provider/model, with round-robin/index-0 fallback. 4 new unit tests. Library API.
- **Windows CI targets** — `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`
  added to the `build-binaries` matrix in `.github/workflows/build.yml`.
- **Sandbox filesystem snapshots** — `Sandbox::snapshot()`/`restore()` + `SandboxSnapshot`
  in `src/sandbox.rs` for workdir-level capture/rollback. 3 unit tests.
- **Interactive installer** — `install.sh` with OS/arch detection, dev/prod/airgap
  posture presets, prebuilt download + source-build fallback, `--config-only`. New
  `installer` verification module (3 tests).
- **Network-egress operator approval** — `Decision::RequireApproval` +
  `NetworkPolicy::approve_unknown_hosts` for human-in-the-loop egress approval.
  3 unit tests.
- **Local inference discovery** — `LocalInference` + `LocalInferenceServer` +
  `LocalInferenceKind` in `src/llm.rs`. Probes Ollama (`/api/tags`) and
  OpenAI-compatible (`/v1/models`) local endpoints (llama.cpp, vLLM, LM Studio, TGI)
  for "device discovery". 4 unit tests. Library API.
- **CUA readiness gate** — `BrowserTool::check_availability()` probes the CDP
  endpoint for a reachable browser target, gating computer-use-agent workflows with
  fail-fast readiness. 2 unit tests.

### Fixed
- **`src/patterns.rs` compile error** — removed a stray closing brace in
  `run_research_synthesize()` and normalized indentation (`cargo fmt`); the crate
  compiles again (587+ tests pass).
- **Critical `wasmtime` CVEs** — upgraded `wasmtime` 28.0.1 → 47.0.3, clearing two
  CRITICAL sandbox-escape advisories (RUSTSEC-2026-0095/0096) and 10+ lower issues;
  fixed the `Memory::grow` error-conversion API change.
- **`h2` unbounded empty DATA frames** — upgraded `h2` 0.4.15 → 0.4.16
  (RUSTSEC-2026-0258).
- **Poisoned-mutex panics** — replaced 94 `std::sync::Mutex::lock().unwrap()` sites
  with `.lock().unwrap_or_else(|p| p.into_inner())` across 6 modules.
- **K8s RBAC least-privilege** — scoped the `ravenclaws` Role's `secrets` access to
  `resourceNames: ["ravenclaws-secrets"]` with `get` only.
- **Cross-platform shutdown signals** — `main.rs` now gates SIGTERM/SIGINT behind
  `#[cfg(unix)]` with a Windows Ctrl+C fallback (was the last Windows-blocking code).
- **Stale `SECURITY.md` version table** — updated to reflect 1.x support.

### Changed
- **K8s manifest documentation** — documented the `changeme` secret placeholder +
  rotation guidance, the in-cluster HTTP TLS rationale, and a NetworkPolicy
  CIDR-scoping hardening example.
- **Memory-safety lint** — added `#![forbid(unsafe_code)]` to both crate roots
  (no first-party `unsafe` exists).
- **WASM plugin feature gating** — `wasmtime` + `src/plugins.rs` moved behind an
  optional `plugins` cargo feature (matches the existing `k8s` pattern), keeping the
  dependency graph honest and the default build lean.
- **Threat model + security posture profiles** — added a full threat model
  (trust boundaries, threat-actor/mitigation matrix, residual risks) and three
  posture profiles (`dev`/`prod`/`airgap`) to `SECURITY.md`.
- **Deterministic fuzz-style test** — `test_policy_engine_never_panics_on_adversarial_input`
  hammers `PolicyEngine` with 10,000 adversarial inputs (fixed-seed LCG, no deps).

## [Unreleased]

## [v1.7.1] — 2026-08-22

### Fixed
- **Provider-string parsing** — `RAVENCLAWS__LLM__PROVIDER=azure` (and
  `openai-compatible`) silently fell back to LiteLLM because the env override in
  `Config::load()` was missing those cases. Added `LLMProvider::parse()` as the
  single source of truth for provider-string parsing (case-insensitive, all 9
  providers, LiteLLM fallback) and wired both the env override and the
  `--provider` CLI flag to it. 1 new unit test.

## [v1.7.0] — 2026-08-21

### Added
- **Cost-aware routing** — `MultiModelManager::route_cheapest(&CostTracker)` +
  `CostTracker::pricing_for()` in `src/llm.rs`. Ranks clients by registered
  per-model pricing (unregistered/free models first), complementing
  `route_by_complexity` for cost/quality policy. 3 new unit tests. Library API.
- **Local inference model selection** — `LocalInference::select_model()`
  (capability-scoring heuristic by parameter count + model family) and
  `warmup_all()` in `src/llm.rs`. 3 new unit tests. Library API.
- **Prompt-injection fuzz coverage** — `test_injection_detector_never_panics_on_adversarial_input`
  hammers `InjectionDetector` with 10,000 adversarial inputs (fixed-seed LCG +
  curated attack fragments), complementing the existing `PolicyEngine` fuzz test.
- **Local inference model warmup (provisioning)** — `LocalInference::warmup(endpoint, model)` +
  `WarmupResult` in `src/llm.rs`. Sends a minimal single-token chat request (Ollama
  `/api/chat`, falling back to `/v1/chat/completions` for vLLM/SGLang/llama.cpp/LM
  Studio) to force model load and verify serving readiness. Turns passive device
  discovery into a verified, ready-to-use backend. 3 new unit tests. Library API.
- **CUA observe primitive** — `BrowserTool::page_state()` + `PageState`
  (title/url/visible-text snapshot) and a new `"observe"` browser action in
  `src/tools.rs` — the "perceive" primitive of a perceive → plan → act → observe
  computer-use-agent loop. 1 new unit test. `PageState`/`BrowserTool` re-exported.

### Fixed
- **Flaky test (#23)** — `integrations::tests::test_send_sms_requires_recipient`
  raced against sibling tests mutating the same process-global `TWILIO_*` env vars.
  Annotated all 10 env-var-mutating integration tests with `serial_test`'s
  `#[serial]`. Verified 8× full runs (612/612) + single-threaded run.
- **K8s verification test image-pull** — `test-k8s.sh` built `ravenclaws-verify:${TIMESTAMP}`
  but the manifest referenced a hardcoded tag, guaranteeing `ImagePullBackOff`; the
  image was also never loaded into a separate cluster runtime. Added a deterministic
  `K8S_TEST_IMAGE` tag, `ensure_k8s_test_image()` (podman-first) and
  `load_image_into_cluster()` (kind/minikube/k3d) helpers.
- **Code scanning: 107 open alerts remediated** — Resolved every alert in the GitHub
  Security tab (code-scanning):
  - **CVE-2026-48504 / GHSA-w9wp-h8wv-79jx** — Upgraded the OpenTelemetry crate
    family `0.28` → `0.32` (`opentelemetry_sdk` → `0.32.1`), which fixes the
    unbounded W3C Baggage-propagation allocation. Also set
    `opentelemetry-otlp` `default-features = false` so the unused HTTP exporter
    (and a second `reqwest 0.13` copy) is no longer compiled into the binary.
  - **77 Scorecard `Pinned-Dependencies`** — Pinned every GitHub Action in
    `build.yml`, `container.yml`, and `security-scan.yml` to a full commit SHA
    (plus the `kubeconform` `docker://` image to a content digest).
  - **4 Scorecard `Token-Permissions`** — Added top-level
    `permissions: contents: read` (least privilege) to all three workflows.
  - **Trivy Kubernetes misconfigs** — Removed `secrets` from the RBAC `Role`
    (KSV-0113; the API key is injected via `secretKeyRef`, needing no RBAC read),
    pinned the image tag off `:latest` (KSV-0013), and added a pod-level
    `securityContext` with `runAsNonRoot` + `seccompProfile: RuntimeDefault` to the
    test manifest (KSV-0118/0104/0030/0012).
  - **`Dependency-Update-Tool`** — Added `.github/dependabot.yml` (Cargo, GitHub
    Actions, and Docker ecosystem updates on a weekly cadence).
  - **4 shell scripts** — Restructured the `curl | python3 -c` JSON parsing in the
    LiteLLM/vLLM/llama.cpp test scripts to fetch-then-parse, clearing the Scorecard
    `downloadThenRun` heuristic.
  - Dismissed 16 false-positive / process-level alerts with documented reasons
    (7 CodeQL `rust/unused-variable` where the variable is used inside a `tracing`
    macro; 2 Trivy `KSV-0125` where `ghcr.io` — GitHub's own registry — is not in
    Trivy's default allowlist; 1 intentional `hostNetwork` in the test manifest;
    6 Scorecard repo-admin/process checks).
  - Verified: `cargo build --locked`, `cargo clippy -D warnings`, `cargo fmt
    --check`, and 600 unit tests + 6 doc-tests all pass.

## [v1.4.0] — 2026-08-13

### Added
- **Web access policy (domain-level)** — New `src/web_policy.rs` module with `WebAccessPolicy`, `WebCategory`, `RateLimiter`, and `extract_domain()`. Category-based allowlist/blocklist/permission rules for web-facing tools (`web_fetch`, `web_search`, `browser`), complementary to the resource-level `PolicyEngine`. Wired into `WebFetchTool` and `WebSearchTool` (they consult the policy before any network request), and into `ToolRegistry::with_config()`. New `web_policy` config section (`[web_policy]`) in `config.rs`, disabled by default so existing deployments are unaffected. 11 new unit tests. Re-exported from the library crate.
- **Long-term memory + conversation search + auto-title** — Extended `src/persistence.rs` with a `MemoryStore` (scoped key-value persistence with upsert semantics), `search_conversations()` (keyword search across titles, system prompts, and message content), and auto-titling (`set_title`/`get_title`/`auto_title`). Added a `title` column to the `sessions` table and a `title` field to `StoredSession`. 16 new unit tests. Re-exported from the library crate.
- **Kubernetes operator support (feature `k8s`)** — New `src/k8s.rs` module with `K8sManager` and `K8sManagerConfig` for programmatic pod lifecycle management. Provisions/disposes RavenClaws worker pods via the Kubernetes API (list worker pods, get ready count, create/delete pods, delete by role, ensure ClusterIP service, fleet stats). Labels, image, namespace, and secret references are all parameterized. Gated behind the optional `k8s` cargo feature so the default ~5 MB binary is unaffected. New `RavenClawsError::K8s` variant. 4 new unit tests. Re-exported from the library crate.
- **Messaging & connector integrations** — New `src/integrations.rs` module with env-var-gated outbound notifications for Slack, Discord, Microsoft Teams, Signal (signald), Matrix, Telegram, Email (Mailgun), and SMS (Twilio). Each `send_*` function returns a structured `IntegrationResult` (`sent`/`disabled`/`error`) and gracefully degrades to `disabled` when credentials are absent. Re-uses the existing `reqwest` dependency (no new deps). 10 new unit tests. Re-exported from the library crate.

### Fixed
- **Stale Helm `appVersion`** — `charts/ravenclaws/Chart.yaml` `appVersion` bumped from `0.7.2` to `1.3.0` to match the released binary.
- **K8s init-container non-root violation** — Removed the root-running `chown-workspace` init container from `k8s/deployment.yaml`. It was redundant with the pod-level `fsGroup: 65532` and caused `CreateContainerConfigError` ("container's runAsUser breaks non-root policy"). Verified the pod now starts and serves the HTTP endpoints.
- **Pre-existing clippy warnings** — Removed a redundant `&` in `format!` (`src/eval.rs`) and switched a map iteration to `.values()` (`src/mcp.rs`) so `clippy -D warnings` is fully clean.
- **`ROADMAP.md` duplication** — Removed ~2,048 lines of accidentally duplicated checklist content (2762 → ~752 lines).

### Changed
- **Module count** — 22 → 25 source modules (`web_policy`, `integrations`, `k8s`).

### Removed
*(none)*

## [v1.3.0] — 2026-07-02

### Added
- **Advanced Reasoning: Tree-of-Thought** — New `run_tree_of_thought()` and `run_tree_of_thought_multi()` patterns in `src/patterns.rs`. Explores multiple parallel reasoning paths simultaneously, generates N candidate thoughts at each step, evaluates each thought's promise via confidence scoring, prunes to top-K branches, and continues exploring the most promising paths. Configurable via `--tot-branches` (default: 3), `--tot-depth` (default: 3), `--tot-top-k` (default: 2). Accessible via `--mode tree-of-thought`. Multi-model variant uses round-robin provider assignment per branch. Self-healing circuit breaker integration. RavenFabric broadcast on completion. 547 unit tests pass, clippy clean.
- **Advanced Reasoning: Self-Reflection** — New `run_self_reflection()` and `run_self_reflection_multi()` patterns in `src/patterns.rs`. Agent generates an initial solution, then reflects on it to identify gaps, errors, or weaknesses, and produces an improved version. Iterates for configurable reflection rounds. Two-phase process: (1) critical review identifying gaps, logical flaws, edge cases, and improvement opportunities; (2) improvement pass that addresses all feedback. Configurable via `--reflection-rounds` (default: 2). Multi-model variant uses alternating providers for reflection vs improvement roles. Accessible via `--mode self-reflection`. Self-healing circuit breaker integration. RavenFabric broadcast on completion. 547 unit tests pass, clippy clean.

### Fixed
*(none)*

### Changed
- **PatternConfig extended** — Added `tot_branches`, `tot_depth`, `tot_top_k`, and `reflection_rounds` fields to support advanced reasoning patterns. All existing patterns updated to pass through the new fields.

### Removed
*(none)*

## [v1.2.0] — 2026-07-02

### Added
- **asciinema demo script** — New `scripts/demo.sh` showcases 11 RavenClaws features in an automated terminal demo: version/help, binary profile, configuration, source modules, test suite, one-shot exec mode, HTTP server mode, MCP server mode, Docker & K8s deployment, website, and verification suite. Recorded demo available at https://asciinema.org/a/Qd8hCw6TVdrnNYLE. New `docs/guides/demo.md` documents how to record and share demos.
- **Agent loop retry with exponential backoff** — New `call_llm_with_retry()` helper wraps LLM calls with configurable retry logic. Retries on transient errors (RequestFailed, RateLimited, CircuitBreakerOpen) with exponential backoff (100ms base, doubled each attempt, configurable jitter). Non-transient errors (AuthFailed, TokenBudgetExceeded, InvalidResponse) are NOT retried. Checkpoints are preserved during retries — only deleted on permanent failure. New `retry_config` field on `AgentLoopConfig` (default: `None` — no retry, uses fallback chain directly). 5 new unit tests covering config defaults, custom config, delay calculation, delay capping, and jitter. 512 unit tests pass, clippy clean.
- **Self-healing engine** — New `src/healing.rs` module with `SelfHealingEngine`, `HealingCircuitBreaker`, `FailureRecord`, and `HealingConfig`. Provides automatic detection, retry, and recovery of failed agents. Circuit breaker with Closed/Open/HalfOpen states, configurable threshold (default: 5) and recovery timeout (default: 30s). Exponential backoff with jitter for retries (base 1000ms, max 30000ms, 30% jitter). Per-agent failure tracking with dead worker detection. `is_transient()` classification on `LLMError` and `RavenClawsError`. New `AgentFailed(String)` and `HealingError(String)` error variants. 22 unit tests (20 healing + 2 error), 552 total tests pass, clippy clean.

### Fixed
*(none)*

### Changed
*(none)*

### Removed
*(none)*

## [v1.1.0] — 2026-07-02

### Added
- **Multi-modal input support** — New `ContentPart` enum (`Text`, `ImageUrl`) enables attaching images alongside text in chat messages. Added `load_image()` utility that reads image files (PNG, JPEG, GIF, WebP) and produces data URIs. New `--image` / `-I` CLI flag accepts image file paths. Multi-modal serialization for all 5 providers: OpenAI-compatible APIs use `[{type, image_url, image_url: {url}}]`, Anthropic uses `[{type, image, source: {type, media_type, data}}]`, Ollama uses `images: [base64, ...]` array on messages. Agent loop integration via `run_agent_loop_with_images()` and `run_agent_loop_with_mcp_and_images()`. `ConversationMemory::add_user_message_with_images()` for persistent multi-turn conversations with images. Library exports include `ContentPart`, `ImageUrlContent`, `load_image`. 478 unit tests pass, clippy clean.
- **Browser automation tool** — New `BrowserTool` built-in tool that controls a browser via Chrome DevTools Protocol (CDP). Supports 10 actions: `navigate` (go to URL), `click` (click element by CSS selector), `type` (type text into element), `screenshot` (capture page content), `extract` (extract text from element), `get_html` (get page/element HTML), `get_text` (get visible page text), `scroll` (scroll down/up/to_bottom/to_top), `wait` (pause execution), `evaluate` (run JavaScript). Connects to Chrome/Chromium running with `--remote-debugging-port=9222`. New `BrowserConfig` struct in `config.rs` with `cdp_url` and `request_timeout` fields. New `ToolCategory::Browser` variant. Registered in all 3 registry methods (`with_default_tools`, `with_web_search_config`, `with_config`). 15 unit tests covering definition, config, registry, argument validation, and action dispatch. 500 unit tests pass, clippy clean.
- **Graceful degradation under load** — New `src/load.rs` module with `LoadManager`, `TokenBucket` rate limiter, `ErrorTracker` sliding window, and `LoadConfig` configuration. Combines rate limiting, concurrency control (semaphore-based), and load shedding (queue depth + error rate thresholds) into a single admission control API. Wired to HTTP server (`ServerState.load_manager`) — admission checks in `handle_connection` return 429/503 on overload. Wired to agent loop (`AgentLoopConfig.load_manager`) — admission checks before LLM calls with outcome recording. Load metrics exposed via `/metrics` endpoint in Prometheus format. New `LoadConfig` struct in `config.rs` with 7 configurable fields. 12 unit tests covering token bucket, error tracker, admission control, metrics, and Prometheus formatting. 507 unit tests pass, clippy clean.

## [v1.0.1] — 2026-06-02

### Added
- **WASM plugin system** — New `src/plugins.rs` module with `WasmPlugin`, `WasmPluginManager`, and `PluginTool` types. Load `.wasm` binaries at runtime via Plugin ABI v1 (7 required exports). Supports tool enumeration, execution, and registration into `ToolRegistry`. 11 unit tests covering load, tools, execute, manager, unload, version mismatch, and missing export scenarios. Uses `wasmtime` 28 with `cranelift` and `runtime` features. (rpi5-feedback)

### Fixed
- **`/execute` returns empty result without `no_final_required`** — Changed `no_final_required` default from `false` to `true` in `background.rs` task execution config. The `/execute` endpoint now correctly returns the agent's response even when the model doesn't emit a `FINAL:` marker, matching the behavior of the `/chat` endpoint. (#39, rpi5-feedback)
- **RavenFabric health check URL builder error with WebSocket endpoints** — Added `http_url()` helper method in `ravenfabric.rs` that converts `ws://` to `http://` and `wss://` to `https://` for REST API calls. Applied to `health()`, `list_agents()`, and `execute()` methods. (#42, rpi5-feedback)
- **`GET /tools/{name}` returns 404 instead of falling through to catch-all** — Added dedicated `GET /tools/{name}` route handler (`handle_get_tool`) that returns tool details by name. Also improved error status mapping in `POST /tools/{name}` to return 404 (not 400) when a tool is not found. (rpi5-feedback)

### Added
- **`POST /reload` endpoint for distroless-friendly config reload** — New `/reload` HTTP endpoint provides the same config reload functionality as SIGHUP but works in distroless containers that lack a shell or `kill` binary. Accepts `POST` requests and reloads configuration from the original config path. (rpi5-feedback)
- **`--no-final-required` is now the default** — Changed `AgentLoopConfig::default()` to set `no_final_required: true`. The agent loop now treats any non-tool-call response as completion without requiring a `FINAL:` marker. Added `--require-final` CLI flag (and `RAVENCLAWS_REQUIRE_FINAL` env var) for users who want the old behavior. This is the #1 usability fix — most models don't emit `FINAL:`. (rpi5-feedback)
- **`Dockerfile.slim` — Debian-based alternative for MCP client support** — New `Dockerfile.slim` using `debian:stable-slim` as the runtime base image. Includes `nodejs` and `npm` for MCP client connections (via `npx`), `curl` for HTTP endpoint testing, and a shell for debugging. Runs as non-root user (UID 65532). Build with `docker buildx build -f Dockerfile.slim ...`. (rpi5-feedback)
- **SQLite conversation persistence** — New `src/persistence.rs` module with `ConversationStore` for SQLite-backed conversation history. Supports session CRUD, message storage with token counts, and configurable retention policies (time-based, count-based, token-budget-based, unlimited). `import_memory()` bridges `ConversationMemory` to SQLite. 15 unit tests. (rpi5-feedback)

## [0.9.6] — 2026-06-02

### Added
- **Full Agent Execution API (6 new endpoints)** — `server.rs` now exposes `POST /chat` (chat completion), `POST /execute` (async task execution with tools), `GET /tasks/{id}` (poll task status), `GET /tools` (list registered tools), `POST /tools/{name}` (execute a specific tool), and `GET /health/deep` (detailed health with uptime, request count, LLM provider, tools registered). All endpoints return structured JSON responses.
- **SIGHUP configuration reload** — `server.rs` now listens for `SIGHUP` on Unix systems. On receipt, re-reads the config file and logs the result. Full hot-reload of LLM clients and tool registries deferred to v0.9.8.
- **MCP TOML configuration section** — New `[mcp]` config section with `[[mcp.servers]]` array for declaring MCP server processes. Each server has `name`, `command`, `args`, and `env` fields. Config structs `McpConfig` and `McpServerConfig` added to `config.rs` and re-exported from `lib.rs`.
- **Swarm profiles shorthand syntax** — `[swarm.profiles]` now accepts both the standard `[[swarm.profiles]]` array-of-tables and a compact map shorthand (`name = "persona"`). Custom `deserialize_profiles` deserializer handles both formats transparently.
- **Tool call assertions for eval harness** — Two new assertion variants: `tool_called("name")` and `tool_not_called("name")`. These check the run trace's tool call list rather than the response text. `check_assertions()` now accepts an optional `&RunTrace` parameter. Includes 5 unit tests covering pass/fail for both variants and the no-trace edge case.
- **Server mode documentation** — New `docs/guides/server-mode.md` guide and `website/public/docs/server-mode.html` page covering all 9 endpoints, configuration, Docker/K8s/systemd deployment, SIGHUP reload, graceful shutdown, and cURL/Python/Node.js examples. Added to sitemap, docs index, sidebar nav, and footer.
- **`ToolRegistry` now implements `Clone`** — Added `#[derive(Clone)]` to `ToolRegistry` so it can be shared across threads (e.g., cloned into `ServerState`).

### Fixed
- **Config test initializers** — All 17 `Config` struct initializations in test code updated to include the new `mcp` field.
- **Eval test calls updated** — All `check_single_assertion` test calls updated to pass the new `Option<&RunTrace>` parameter.

## [v0.9.7] — 2026-06-02

### Added
- **Multi-MCP-client support** — New `McpClientManager` struct in `mcp.rs` that holds multiple `McpClient` instances. `from_config()` creates clients from TOML config `[mcp]` section. `register_all_tools()` registers tools from all connected servers into a `ToolRegistry`. Wired into both `--exec` mode and `--serve` (HTTP server) mode. `#[allow(dead_code)]` removed from `McpConfig` and `McpServerConfig`. Re-exported from `lib.rs`.
- **Readiness LLM connectivity check** — `ready_response()` in `server.rs` now optionally verifies LLM connectivity before returning 200. When an LLM client is configured, sends a lightweight probe and returns `503 Service Unavailable` with descriptive message if the LLM is unreachable or times out (5s timeout). Falls back to simple boolean check when no LLM is configured.

## [v0.9.9] — 2026-06-28

### Added
- **ProviderFallbackChain wired into agent loop** — Both `run_agent_loop_with_registry` and `run_agent_loop_with_mcp_and_registry` now use `ProviderFallbackChain` on primary LLM failure. Configs are cloned out of mutex to avoid holding `MutexGuard` across `.await`. Token usage recorded from fallback responses.
- **TokenBudget wired into agent loop** — Checked before every LLM call in both agent loop variants. If remaining tokens < 100, returns `SecurityViolation` error with audit log entry. Token usage recorded after each response.
- **RavenFabricClient wired into agent loop** — `health()` called after each LLM response in both agent loop variants. Wired into all `run_single`, `run_swarm`, `run_supervisor` variants (both single and multi-model).
- **AgentMessageBus wired into swarm** — Created and shared across sub-orchestrators. `send()` and `format_for_prompt()` used in swarm execution.
- **SwarmHealthMonitor wired into swarm** — `check_health()` called during swarm execution. Dead agents detected and logged.

### Changed
- **`#[allow(dead_code)]` removed from wired components** — `TokenBudget` struct + impl, `TokenBudgetExceeded` variant, `RavenFabricClient` struct + impl, `ExecuteResponse`, `RemoteAgent`, `RavenFabricConfig` fields (`agent_id`, `remote_exec`), `RavenClawsError::RavenFabric` variant. `ProviderFallbackChain` gets `#[derive(Debug)]` and `pub configs` field.
- **`AgentLoopConfig` extended** — 3 new optional fields: `fallback_chain`, `token_budget`, `ravenfabric`. All initializers across `agent.rs`, `background.rs`, `server.rs`, `heartbeat.rs`, and `examples/agent_loop.rs` updated.
- **`main.rs` updated** — RavenFabricClient init moved before `--exec` block. Fallback chain and token budget created from config in `--exec` mode.
- **ROADMAP.md updated** — v0.9.8 exit criteria corrected: 5/5 infra components wired ✅, 7 production hardening items deferred to v0.9.9. v0.9.9 scope expanded to include Tier 4 (production hardening). Strategic positioning refined.

### Fixed
- **Default workdir changed from `/workspace` to `/tmp/ravenclaws-workdir`** — The previous default `/workspace` caused `Permission denied (os error 13)` on distroless containers without an explicit volume mount. The new default uses `/tmp` which is writable on distroless. K8s deployments that explicitly set `workdir = "/workspace"` with an `emptyDir` volume are unaffected. (#rpi5-feedback)

### Removed
*(none)*

## [v1.0.0] — 2026-07-02

### Added
- **`--mcp-sse-server` CLI flag** — New `--mcp-sse-server` flag (env: `RAVENCLAWS_MCP_SSE_SERVER`) runs RavenClaws as an MCP SSE (Server-Sent Events) transport server. Supports `--mcp-sse-host` (default `0.0.0.0`) and `--mcp-sse-port` (default `8081`) flags. Wired into `main.rs` dispatch with graceful shutdown via `ShutdownFlag`. (#mcp-sse-wiring)
- **SSE transport for MCP client config** — `McpServerConfig` now supports a `url` field for SSE transport. When `url` is non-empty, `McpClientManager::from_config()` creates an SSE transport instead of stdio. Both `command` (stdio) and `url` (SSE) fields are supported, but not both simultaneously. (#mcp-sse-wiring)
- **MCP SSE integration tests** — New `scripts/lib/test-mcp.sh` with 5 test scenarios: stdio server tools/list, SSE server endpoint + tools/list + tools/call, SSE server health check + 404 handling, SSE client CLI flag verification, and multiple concurrent SSE clients. (#mcp-sse-tests)
- **SSE transport documentation** — Added SSE transport sections to `docs/guides/mcp-integration.md` covering: transport types comparison table, SSE client configuration, SSE server mode (`--mcp-sse-server`), SSE IDE integration (OpenClaw, Claude Desktop, VS Code), and SSE multi-agent workflows. (#mcp-sse-docs)
- **Website SSE transport docs** — Updated `website/public/docs/mcp-integration.html` with transport types table, SSE client config, SSE server endpoint table, IDE integration examples, and "New in v0.9.16" sidebar section. (#mcp-sse-docs)

### Changed
- **`scripts/verify.sh` MODULES array** — Added `mcp` test module for MCP integration tests (stdio + SSE). (#mcp-sse-tests)
- **`McpServerConfig` extended with `url` field** — New `url: String` field (default empty) enables SSE transport. Validation ensures only one of `command` or `url` is set. (#mcp-sse-wiring)
- **`#[allow(dead_code)]` removed from SSE components** — `McpTransportConfig::Sse` variant, `McpSseServer` struct and impl, and `McpClientManager::from_config()` SSE branch all unwired — now fully wired and active. (#mcp-sse-wiring)
- **`lib.rs` re-exports updated** — `McpSseServer` added to public API re-exports. Module description updated to "JSON-RPC 2.0 over stdio + SSE". (#mcp-sse-wiring)
- **ROADMAP.md updated for v0.9.16** — SSE MCP items marked complete. v1.0 exit criteria updated. (#roadmap-update)
- **Version bumped to 1.0.0** — Stable release. All v1.0 exit criteria met. (#v1.0-release)

### Fixed
*(none)*

### Removed
*(none)*

## [v0.9.10] — 2026-07-02

### Added
- **Community health files** — `SECURITY.md` (vulnerability reporting policy, supported versions, security features, supply chain security, hardening roadmap), `CONTRIBUTING.md` (development setup, coding conventions, testing instructions, PR process), `CODE_OF_CONDUCT.md` (Contributor Covenant v2.0 with enforcement guidelines), `SUPPORT.md` (documentation links, community channels, commercial support), `.github/FUNDING.yml` (GitHub Sponsors).
- **Issue and PR templates** — `.github/ISSUE_TEMPLATE/bug_report.md` (structured bug report with environment, logs, reproduction steps), `.github/ISSUE_TEMPLATE/feature_request.md` (feature request with pillar alignment checklist), `.github/ISSUE_TEMPLATE/config.yml` (contact links for discussions, docs, security), `.github/PULL_REQUEST_TEMPLATE.md` (PR template with type selection, checklist for fmt/clippy/test/docs/CHANGELOG/ROADMAP/ISSUES/binary size).
- **Graceful shutdown for HeartbeatAgent** — Added `impl Drop for HeartbeatAgent` that calls `persist_state()` on drop. State is now saved on graceful shutdown (SIGTERM/SIGINT) without requiring a signal handler. Ensures heartbeat state survives pod termination.
- **Init container chown in K8s deployment** — Added `initContainers` section to `k8s/deployment.yaml` with `busybox:1.36.1` running `chown -R 65532:65532 /workspace` as root before the main container starts. Ensures workspace is writable by the non-root UID 65532 even with `readOnlyRootFilesystem: true`.
- **`--exec` mode documentation** — Added comprehensive `--exec` mode section to `docs/guides/getting-started.md` covering: how `--exec` works, model compatibility table (`FINAL:` marker, structured tool calls, `--no-final-required`), `--no-final-required` flag usage, `--verbose` flag for debugging, and exit codes.
- **Migration docs v0.9.1 → v0.9.2** — Added migration section to `docs/guides/migration.md` documenting: `AgentMessageBus`, `MessageType` enum, `SwarmHealthMonitor`, `WorkerHealthStatus`, `SwarmOrchestrator::new_with_bus()`, and new `[swarm]` config fields (`communication_enabled`, `health_monitoring_enabled`, `heartbeat_interval_secs`, `max_missed_beats`, `replacement_timeout_secs`).
- **Graceful shutdown for all modes** — Added `ShutdownFlag` struct (wraps `Arc<AtomicBool>`) and `ShutdownGuard` struct (logs on drop) in `main.rs`. SIGTERM/SIGINT handler sets the flag. All mode dispatches (single/swarm/supervisor/orchestrate) use `tokio::select!` with `wait_for_shutdown()`. Heartbeat mode checks flag during sleep at 1s granularity.
- **UPX container compression** — Added UPX v5.2.0 installation in Docker builder stage. Both `ravenclaws` and `ravenfabric-agent` binaries compressed with `upx --best --lzma`. Added `INCLUDE_RAVENFABRIC` build arg (default `true`) for conditional RavenFabric agent binary inclusion. Image reduced from ~50 MB to ~25 MB.
- **K8s NetworkPolicy** — Added `ravenclaws-default-deny` NetworkPolicy to `k8s/deployment.yaml` with `podSelector: {}`, `policyTypes: [Ingress, Egress]`, ingress deny all, egress allow DNS (udp 53), HTTPS (tcp 443), HTTP (tcp 80).
- **K8s Secret reference documentation** — Added "Kubernetes Deployment" section to `docs/guides/getting-started.md` with Secret reference table (Key, Description, Example Value columns), `secretKeyRef` YAML examples, and NetworkPolicy explanation.

### Changed
- **ROADMAP.md updated for v0.9.10** — Version bumped from v0.9.9 to v0.9.10. All 4 remaining v0.9.11+ items (container image size, NetworkPolicy, Secret docs, graceful shutdown) marked as completed in v0.9.10. Status tables updated across all sections.
- **AGENTS.md version reference updated** — Version changed from v0.9.7 to v0.9.10.
- **ISSUES.md updated for v0.9.10** — 6 resolved items moved from open to resolved sections. Added ✅ v0.9.10 milestone table.

### Fixed
*(none)*

### Removed
*(none)*

## [0.9.5] — 2026-06-27

### Added
- **ToolCallDetector — text-based tool call detection fallback** — New `ToolCallDetector` struct with 5 regex patterns for detecting tool calls in natural language text. Acts as a fallback when LLMs don't emit structured `tool_calls`. Includes 11 unit tests covering all patterns, deduplication, and edge cases.
- **Tool execution debug logging** — Enhanced `ToolRegistry::execute()` with `debug!`-level logging of tool arguments and output length. Helps diagnose silent tool failures.
- **`ToolRegistry::with_config()` constructor** — New method that accepts `&Config` to create a registry with configured web search endpoint. Replaces hardcoded SearXNG URL.
- **`run_agent_loop_with_registry()` and `run_agent_loop_with_mcp_and_registry()`** — New public API functions that accept an optional pre-configured `ToolRegistry`, allowing callers to pass custom tool configurations (e.g., configured web search endpoint).
- **`--exec` mode uses configured tool registry** — The `--exec` one-shot mode now passes a `ToolRegistry` built from config (via `with_config()`) to the agent loop, so web search endpoint and other tool settings from config are respected.

### Fixed
- **`WebSearchConfig` dead_code warnings removed** — Removed `#[allow(dead_code)]` from `WebSearchConfig` struct and `web_search` field in `Config`. Both are now wired through `ToolRegistry::with_config()`.
- **`ToolCallDetector` and `DetectorPattern` dead_code warnings suppressed** — Added `#[allow(dead_code)]` to these types since they are a fallback mechanism used only in tests until wired into the agent loop.

## [0.9.4] — 2026-06-27

### Added
- **`--no-final-required` CLI flag** — New `AgentLoopConfig.no_final_required` field and `--no-final-required` CLI flag. When set, any non-tool-call LLM response is treated as completion (no `FINAL:` marker required). Wired to both `run_agent_loop` and `run_agent_loop_with_mcp`. Useful for models that don't reliably emit `FINAL:` convention.
- **Agent loop response logging** — Added `debug!` log after each LLM response in both agent loops, showing response length and first 500 characters of content. Helps diagnose silent failures and model behavior.
- **Default system prompt `FINAL:` instructions** — Updated `default_system_prompt()` in `config.rs` to include `FINAL:` usage instructions, guiding models to signal task completion.
- **`agent_count` serde alias** — Added `#[serde(alias = "agent_count")]` to `SwarmConfig.max_workers` for backward compatibility with configs using the old field name.

### Fixed
- **Heartbeat goal error message** — Improved error message when no goal is provided for heartbeat mode. Now includes a concrete example: `--heartbeat-goal "Monitor system health and report anomalies"`.

## [0.9.3] — 2026-06-27

### Added
- **rpi5 deployment feedback fixes** — Fixed 6 of 12 issues from real-world Raspberry Pi 5 deployment: added `#[serde(alias = "flat")]` to `SwarmTopology::Star` for backward compatibility, fixed all swarm/heartbeat/telemetry/server docs field names (42 incorrect occurrences across 6 files), made OpenTelemetry opt-in by default (`otel_disabled` default changed to `true`). Remaining items tracked in ISSUES.md and ROADMAP.md v1.0 hardening checklist.
- **Provider strategy documented** — Added comprehensive Provider Strategy section to ROADMAP.md covering: generic `openai-compatible` provider (unlocks vLLM, llama.cpp, LM Studio, TGI, Groq, Together AI, Fireworks, DeepInfra, and any custom OpenAI-compatible endpoint), vLLM docs/verification tests, llama.cpp docs/verification tests, and Azure OpenAI adapter. Deferred native AWS Bedrock and Gemini/Vertex (reachable via LiteLLM/OpenRouter). Tool-calling fidelity documented per backend.
- **Library API (`src/lib.rs`)** — Added `[lib]` section to `Cargo.toml` and created `src/lib.rs` with public re-exports of all 18 modules. RavenClaws is now usable as both a binary and a library crate (`ravenclaws`).
- **Performance benchmarks** — Verified v1.0 targets: 5.2 MB stripped binary, 5.2 ms cold start. Both well under v1.0 targets (< 15 MB, < 50 ms).
- **API stability guarantees** — `#[non_exhaustive]` added to all public enums (`RavenClawsError`, `ConfigError`, `LLMError`, `ToolError`, `LLMProvider`, `OpenAICompatibleProvider`, `CircuitState`, `ToolCategory`) and public structs (`Config`, `LLMConfig`, `SecurityConfig`, `RuntimeConfig`, `RavenFabricConfig`, `TelemetryConfig`, `SchedulerConfig`, `WebSearchConfig`). Doc comments added to all public types with stability notes.
- **Documentation guides** (`docs/guides/`) — Created comprehensive guides: getting-started, configuration reference, swarm-mode, mcp-integration, and heartbeat-mode. Each guide includes setup instructions, configuration examples, best practices, and use cases.
- **Runnable examples** (`examples/`) — Created 5 runnable Rust examples: `basic_chat` (minimal library usage), `agent_loop` (full agent loop with tools), `swarm` (multi-agent orchestration), `mcp_client` (MCP server connection), `heartbeat` (autonomous agent). Each example is documented with usage instructions.
- **README FAQ section** — Added comprehensive FAQ covering common questions: differentiation, API keys, offline use, security model, mode differences, library usage, upgrade paths, deployment targets, and licensing.
- **Library API re-exports** (`src/lib.rs`) — Added `pub use` re-exports for commonly used types. Added MSRV note (Rust 1.86+), semver guarantees section, and feature flag documentation.

### Removed
- **Deprecated LLM client types** — `LiteLLMClient`, `OpenRouterClient`, `OpenAIClient` (deprecated since v0.5.0) removed. Use `OpenAICompatibleClient` with the appropriate `OpenAICompatibleProvider` variant instead.
- **Legacy `execute_tool_call` function** — Deprecated since v0.4, replaced by `execute_tool_call_with_security` with full PolicyEngine/Sandbox/AuditLog integration.
- **Unused `run_exec_stream` function** — Streaming exec functionality is handled by the agent loop internally.
- **Unused `futures::StreamExt` import** — No longer needed after `run_exec_stream` removal.

### Changed
- **ROADMAP.md** — Updated for v1.0 scope: v0.10 features deferred to post-1.0. v1.0 now focuses on hardening + docs + API stability. Added completed items for deprecated type removal, dead code elimination, library API establishment, performance benchmarks, zero CVEs, API stability, and complete docs. Updated stats to 18 modules, 452 tests. Updated progress to 14/38 v1.0 items.
- **AGENTS.md** — Updated architecture diagram to include `lib.rs`, `eval.rs`, `ravenfabric.rs`. Updated module responsibilities table. Updated build stats (5.2 MB, 5 ms). Updated tool count to 5 built-in tools.
- **README.md** — Updated binary size references from ~3.4 MB to ~5.2 MB. Updated status to v0.9.3. Updated test count to 452. Added library crate mention. Added library usage guide with code example and module table. Updated verification badge to 114 checks. Added library crate badge. Added documentation section with links to all guides. Updated library code example to use new `pub use` re-exports.
- **VERIFICATION.md** — Updated module count from 16 to 18. Updated stale test counts (311→452). Removed duplicate `cargo test` line.
- **deny.toml** — Fixed typo in license exception name (`RavenClawss` → `RavenClaws`).

### Fixed
- **Cargo.toml** — Added missing `homepage`, `documentation`, `readme`, and `exclude` fields for crates.io publication readiness.
- **Dockerfile** — Pinned base image digests for reproducible builds (`rust:1.86-slim-bookworm` and `gcr.io/distroless/cc-debian12:nonroot` now use `@sha256:...` digests).

## [0.9.2] — 2026-06-25

### Added
- **Inter-agent communication bus** (`src/swarm.rs`) — Swarm agents can now share information and coordinate via a shared message bus.
  - `AgentMessage` struct with UUID, sender, recipient, message type, content, timestamp, and metadata
  - `MessageType` enum: Information, Question, Result, Error, Coordination, Generic
  - `AgentMessageBus` with send, receive, filter, and broadcast capabilities
  - `SwarmOrchestrator::new_with_bus()` for shared bus across sub-orchestrators
  - Task prompts enriched with message bus context for informed decision-making
  - Results broadcast back to the bus for peer awareness
  - CLI flags: `--swarm-communication` (env: `RAVENCLAW_SWARM_COMMUNICATION`)
  - 14 unit tests covering all message bus operations
- **Swarm health & telemetry** (`src/swarm.rs`) — Production-grade health monitoring for swarm agents with heartbeat tracking, dead-agent detection, and aggregate metrics.
  - `SwarmHealthMonitor` — tracks per-worker heartbeats, detects degraded/unhealthy/dead agents, and identifies replacement candidates
  - `WorkerHealthStatus` — four-state health model: Healthy, Degraded, Unhealthy, Dead
  - `WorkerTelemetry` — per-worker metrics: tasks completed/failed, error count, avg duration, messages sent/received, iteration count
  - `SwarmMetrics` — aggregate swarm health: total/healthy/degraded/unhealthy/dead workers, task throughput, worker utilization, error rate, communication latency
  - Heartbeat protocol with configurable interval (default: 5s), max missed beats (default: 3), and replacement timeout (default: 30s)
  - Health monitoring integrated into `execute_with_profile()` and `recursive_supervise_impl()` — workers auto-register, heartbeats update on task completion, failures are tracked
  - Health monitor shared across sub-orchestrators via `Arc<RwLock<>>` for recursive supervision
  - Periodic health check logging in supervisor loop (every 3 iterations)
  - Public accessors: `health_metrics()` and `worker_telemetry()` on `SwarmOrchestrator`
  - CLI flag: `--swarm-health-monitoring` (env: `RAVENCLAW_SWARM_HEALTH_MONITORING`)
  - 22 unit tests covering all health monitoring operations
  - 452 total unit tests (0 regressions)

## [0.9.1] — 2026-06-23

### Added
- **Self-provisioning of sub-agents** (`src/swarm.rs`) — RavenClaws dynamically spawns new agent instances based on task decomposition. Supervisor mode becomes recursive: supervisors spawn sub-supervisors, creating task decomposition trees of arbitrary depth.
  - `SwarmOrchestrator` — core orchestrator with recursive supervision, task analysis, role assignment, and result aggregation
  - `WorkerProfile` — declarative profile with persona, allowed_tools, provider/model overrides, resource limits, and delegation capability
  - `SwarmTopology` — four topologies: Star, Mesh, Hierarchical, Hybrid
  - `SwarmConfig` — configurable max_depth (default: 3), max_workers (default: 100), dynamic_role_assignment, profiles
  - 5 built-in worker profiles: researcher, creative, executor, reviewer, supervisor
  - Recursive supervision via `Box::pin` to avoid Rust's recursive async fn limitation
  - LLM-based dynamic role assignment (`analyze_task_roles`) with fallback to default roles
  - CLI flags: `--swarm-topology`, `--swarm-max-depth`, `--swarm-max-workers`, `--swarm-dynamic-roles`, `--swarm-profiles`
  - Config section: `[swarm]` in `ravenclaws.toml`
  - Mode dispatch: `--mode orchestrate` for both single-provider and multi-model paths
  - `MultiModelManager` made `Clone` for sub-orchestrator spawning
  - 17 unit tests covering all profiles, config serde, orchestrator construction, depth limits, task analysis fallback
  - 416 total unit tests (0 regressions)

## [0.9.0] — 2026-06-22

### Added
- **`token_lifetime_secs` enforcement** — `SecurityConfig.token_lifetime_secs` is now honored at runtime. When set to a non-zero value, agent sessions automatically terminate after the configured duration, enforcing credential/session expiry.
  - `AgentLoopConfig.token_lifetime_secs` — new field (default: 0 = unlimited)
  - Wired into both `run_agent_loop` and `run_agent_loop_with_mcp` — checked before each iteration
  - Session start time tracked via `std::time::Instant`
  - On expiry: returns `RavenClawsError::SecurityViolation` with elapsed time details
  - Audit log records `SecurityViolation` event with elapsed time, limit, and iteration
  - Removed `#[allow(dead_code)]` from `config.rs` `SecurityConfig.token_lifetime_secs`
  - 393 total unit tests (0 regressions)
- **Autonomous heartbeat agent** (`src/heartbeat.rs`) — persistent background loop that operates without human supervision, with configurable tick interval, progress assessment, planning, and execution.
  - `HeartbeatConfig` — config struct with goal, tick_interval_secs, max_iterations_per_tick, workdir, max_ticks, enable_tools
  - `HeartbeatState` — persisted state (id, goal, tick, progress, assessments, plans, results) with JSON serialization
  - `HeartbeatAgent` — full implementation with assess→plan→act→persist→sleep loop
  - State persistence to `workdir/heartbeat-<id>.json` — survives restarts and resumes from last checkpoint
  - LLM-driven goal completion detection (responds to `GOAL_COMPLETE` / `[DONE]` markers)
  - Agent loop integration for tool execution during each tick
  - CLI flags: `--heartbeat`, `--heartbeat-goal`, `--heartbeat-tick-interval`, `--heartbeat-max-ticks`, `--heartbeat-session`
  - Config section: `[heartbeat]` in `ravenclaws.toml`
  - 8 unit tests covering config defaults, state lifecycle, serialization, and prompt building
  - 401 total unit tests (0 regressions)
- **Long-horizon task persistence** — task state survives restarts; agent resumes from last checkpoint with full context.
  - `HeartbeatState` persisted to `workdir/heartbeat-<id>.json` after every tick
  - `HeartbeatAgent::new()` auto-resumes from saved state on restart
  - `BackgroundTaskManager` persists all tasks as individual JSON files in `<workdir>/tasks/`
  - `--task-resume` flag re-executes incomplete tasks on startup
  - 401 total unit tests (0 regressions)

## [0.8.0] — 2026-06-22

### Added
- **Prompt-injection defense** (`src/policy.rs`) — two-layer LLM output security that detects and blocks prompt-injection attempts before they reach the agent loop
  - `InjectionDetector` — scans LLM responses for 50+ known injection/jailbreak patterns (instruction override, system prompt extraction, DAN jailbreak, token smuggling, meta-instruction attacks)
  - `InjectionVerdict` — `Clean` or `Suspicious(reason)` result type
  - Instruction-boundary enforcement — detects attempts to ignore/disregard/override system instructions
  - Output schema validation — validates JSON in tool call arguments, detects unbalanced code blocks, enforces maximum response length (100KB)
  - Wired into both `run_agent_loop` and `run_agent_loop_with_mcp` — checks every LLM response before processing
  - `SecurityConfig.prompt_injection_protection` — enable/disable via config (default: enabled)
  - `AgentLoopConfig.prompt_injection_protection` — per-invocation control
  - `AuditEventType::SecurityViolation` — new audit event type for injection detection
  - All violations are logged to audit log with reason, iteration, and content preview
  - 390 total unit tests (0 regressions)
- **`zeroize` for secret material** — API keys in `LLMConfig` and HMAC secret key in `AuditLog` are zeroized on drop, preventing secret leakage from memory dumps
  - `use zeroize::Zeroize` in `config.rs` and `audit.rs`
  - `impl Drop for LLMConfig` — zeroizes `api_key` field
  - `impl Drop for AuditLog` — zeroizes `key` field
  - Replaced `atty` dependency with `std::io::IsTerminal` (Rust 1.70+ stable)
  - 390 total unit tests (0 regressions)
- **Web search + content extraction tool** (`src/tools.rs`) — search the web and extract readable content from results
  - `WebSearchTool` with SearXNG JSON API and DuckDuckGo HTML backends
  - `WebSearchConfig` in `config.rs` — configurable endpoint, engine, max_results, fetch_content
  - `html_to_text()` — strips HTML tags, extracts title, normalizes whitespace, decodes HTML entities
  - `strip_html_tags()`, `extract_href()`, `urlencoding()` helper functions
  - `ToolRegistry::with_web_search_config()` — configurable web search registration
  - 20 new unit tests covering tool definition, config, HTML extraction, URL encoding, error handling
  - 390 total unit tests (+20, 0 regressions)
- **Eval harness** (`src/eval.rs`) — golden-task evaluation framework with run inspection
  - `EvalConfig`/`EvalTask`/`EvalRunner` — TOML-based eval suite configuration with 7 assertion types (contains, not_contains, exact, regex, non_empty, min_length, max_length)
  - `RunTrace` — full step-by-step trace of agent runs including LLM calls and tool calls
  - `EvalReport` — human-readable text and machine-readable JSON output formats
  - `--eval <path>` CLI flag — run an eval suite from a TOML config file
  - `--eval-json` CLI flag — output eval results as JSON
  - 24 Rust unit tests covering all assertion types, config parsing, report formatting, and error handling
  - Sample eval configs in `tests/eval/basic-suite.toml` and `tests/eval/security-suite.toml`
  - 20 verification tests in `scripts/lib/test-eval.sh` registered in `verify.sh` as `--eval` module
  - 353 total unit tests (+24, 0 regressions) → 390 (+37 for web search + scheduling + background)
- **Scheduling & triggers** (`src/scheduler.rs`) — cron, webhook, and file-watch activation for proactive 24/7 agents
  - `TriggerConfig` — configurable trigger with name, prompt, system_prompt, and trigger type
  - `TriggerType` enum — `Cron { expression }`, `Webhook { secret }`, `Watch { path, events, debounce_secs }`
  - `Scheduler` — manages trigger lifecycle with `start()`/`stop()` methods
  - Cron triggers — parses cron expressions via `cron` crate, sleeps until next scheduled time
  - Webhook triggers — TCP listener on configurable port (default 9090), JSON-RPC style POST handler
  - Watch triggers — filesystem monitoring via `notify` crate with debouncing and event filtering
  - `--scheduler` CLI flag — runs scheduler mode with all configured triggers
  - `--webhook-port` CLI flag — override webhook listener port (env: `RAVENCLAW_WEBHOOK_PORT`)
  - All triggers submit tasks to `BackgroundTaskManager` for execution
  - 17 new unit tests covering config serialization, cron parsing, scheduler lifecycle, webhook response format, and all trigger types
  - 353 total unit tests (+17, 0 regressions)
- **Async / long-horizon background runs** (`src/background.rs`) — assign-and-walk-away background task execution with disk persistence and resumability across restarts
  - `BackgroundTaskManager` — manages task lifecycle with in-memory index + JSON file persistence
  - `BackgroundTask` — full task struct with id, prompt, status, result, error, timestamps, provider/model metadata
  - `TaskStatus` — Pending → Running → Completed / Failed / Cancelled lifecycle
  - `--background` CLI flag — submit a task and return immediately (prints task ID to stdout)
  - `--task-status <id>` — check status and full details of a specific task
  - `--task-list` — list all tasks with status, creation time, and prompt preview
  - `--task-cancel <id>` — cancel a pending or running task
  - `--task-resume` — on startup, find and re-execute any incomplete tasks from disk
  - Tasks stored as individual JSON files in `<workdir>/tasks/` directory
  - 8 new unit tests covering creation, submission, status transitions, cancellation, listing, persistence across restarts, and error handling
  - 319 total unit tests (+8, 0 regressions)
- **Helm chart** (`charts/ravenclaws/`) — official Helm chart for deploying RavenClaws on Kubernetes
  - 11 configurable Kubernetes resources: ServiceAccount, ConfigMap, Secret, Deployment, Service, Ingress, RBAC (Role + RoleBinding), PersistentVolumeClaim, NetworkPolicy, PodDisruptionBudget, ServiceMonitor
  - Full values.yaml with sensible defaults matching existing `k8s/deployment.yaml`
  - Optional OpenTelemetry and RavenFabric configuration in ConfigMap
  - Prometheus ServiceMonitor support for metrics scraping
  - Helm chart validated with `helm lint` (0 failures)
- **Maintenance Cycle Workflow** in `AGENTS.md` — structured 7-phase SOP for every maintenance cycle: check CI, fix issues, verify on Orbstack, update docs, commit & push, verify CI after push, release if milestone reached.

## [0.7.2] — 2026-06-20

### Added
- **OpenTelemetry tracing** (`src/telemetry.rs`) — opt-in distributed tracing with OTLP exporter
  - `TelemetryConfig` with `--otel-endpoint`, `--otel-service-name`, `--otel-disabled` CLI flags
  - `TelemetryGuard` — flushes and shuts down OTel exporter on drop
  - gRPC OTLP exporter (default) and stdout exporter fallback
  - Feature-gated: `otel-grpc` (default), `otel-stdout` (optional)
  - `#[instrument]` spans on agent loop, HTTP server, tool execution, and LLM provider calls
  - 4 new unit tests covering config, disabled mode, guard drop, and custom settings

### Changed
- **Cargo.toml** — added `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `opentelemetry-stdout`, `tracing-opentelemetry` dependencies
- **Features** — `default = ["otel-grpc"]`, `otel-grpc = ["opentelemetry-otlp"]`, `otel-stdout = ["opentelemetry-stdout"]`
- **Config** — `Config.telemetry` field added with `TelemetryConfig` struct
- **311 unit tests** (+4 for telemetry, +0 regressions)

## [0.7.1] — 2026-02-06

### Added
- **HTTP Server Mode** (`src/server.rs`) — long-running server with health, readiness, and metrics endpoints
  - `GET /health` — liveness probe (always 200 OK)
  - `GET /ready` — readiness probe (200 OK when ready, 503 during startup)
  - `GET /metrics` — Prometheus-style metrics (requests, tokens, tool calls, errors, uptime)
  - `--serve` CLI flag to run in HTTP server mode
  - `--server-host` / `--server-port` CLI overrides
  - `runtime.host` / `runtime.port` config fields (default: `0.0.0.0:8080`)
  - Graceful shutdown on SIGTERM/SIGINT
  - 9 new unit tests covering health, readiness, metrics, uptime, HTTP responses, 404 handling

### Changed
- **k8s deployment** — switched from `--mode single` to `--serve` mode; probes now use HTTP `/health` and `/ready` endpoints instead of `--version` exec

## [0.7.0] — 2026-02-05

### Added
- **MCP Server** (`src/mcp.rs`) — expose RavenClaws's built-in tools over stdio via the Model Context Protocol
  - `McpServer` struct with `run()`, `handle_request()`, `handle_initialize()`, `handle_tools_list()`, `handle_tools_call()`
  - Supports `initialize`, `notifications/initialized`, `tools/list`, `tools/call` MCP methods
  - All tool calls policy-checked via `PolicyEngine` and logged to `AuditLog`
  - `--mcp-server` CLI flag to run in MCP server mode
  - 7 new unit tests covering initialization, tool listing, tool execution, error handling

### Changed
- **ROADMAP.md** — updated to v0.7.0 (MCP Server + Observability Foundations); MCP Server marked complete
- **Config** — `RuntimeConfig` now has `host` (Option<String>) and `port` (u16) fields; `Config` derives `Default`
- **Test count**: 291 → 307 (+7 MCP Server + 9 HTTP Server tests)

### Planned
- Agent communication — structured message passing; conflict resolution across agents (v0.6.2)
- OpenTelemetry tracing (v0.7)
- Prometheus metrics integration (v0.7)
- Human-in-the-loop approvals (v0.7)

## [v0.6.1] — 2026-06-19

### Added
- **RavenFabric client module** (`src/ravenfabric.rs`) — full HTTP client for RavenFabric REST API
  - `RavenFabricClient` struct with `new()`, `health()`, `list_agents()`, `execute()`, `broadcast()` methods
  - `ExecuteRequest` / `ExecuteResponse` / `RemoteAgent` types with serde serialization
  - `RavenFabricError` enum with `NotConfigured`, `ConnectionFailed`, `RequestFailed` variants
  - 12 unit tests covering: no-endpoint, with-endpoint, disabled config, error display, connection refused (3), serialization (2), deserialization (3)
- **RavenFabric wiring into all agent modes** — client initialized in `main.rs` from config, passed to all 6 agent mode functions (`run_single`, `run_swarm`, `run_supervisor`, `run_single_multi`, `run_swarm_multi`, `run_supervisor_multi`)
- **RavenFabric status logging** — each agent mode logs whether RavenFabric remote execution is available on startup

### Fixed
- **aarch64 build hanging in CI** — Cross-compilation step (`apt-get install gcc-aarch64-linux-gnu`) kept hanging indefinitely on x86_64 GitHub Actions runners. Switched to native `ubuntu-24.04-arm` runner for aarch64 builds, eliminating the need for cross-compilation entirely. This is faster and more reliable.
- **Duplicate "Fixed" section in CHANGELOG.md** — Removed duplicate entry for aarch64 build fix.

## [v0.6.0] — 2026-06-18

### Added
- **Swarm Mode (Single-Provider)** — parallel execution of 3 agents with different personas (analytical, creative, pragmatic); results collected with agent attribution; tokio task spawning for true parallelism
- **Supervisor Mode (Single-Provider)** — task decomposition into subtasks via LLM prompting; sub-agent spawning; result aggregation and final synthesis; security integration (PolicyEngine, Sandbox, AuditLog)
- **Swarm Mode (Multi-Model)** — parallel agents across different LLM providers; provider/model attribution; cost control (capped at 3 agents)
- **Supervisor Mode (Multi-Model)** — provider-aware task decomposition; round-robin supervisor LLM selection; subtask assignment to specific providers based on strengths
- **Git hooks system** — pre-commit and pre-push hooks for automated verification
  - `.githooks/pre-commit` — fast checks: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --locked`, binary size check, secrets scan
  - `.githooks/pre-push` — comprehensive checks: full pre-commit + release build + binary integrity + Docker build + security scan
  - `.githooks/setup.sh` — install/check/remove hooks with `git config core.hooksPath`
- **CI/CD hardening** — `DEBIAN_FRONTEND=noninteractive`, `-o Dpkg::Options::=--force-confdef`, `timeout-minutes: 20`, and retry logic (3 attempts) for cross-compilation dependency install
- **Node.js 24 migration** — `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` in all 3 workflow files
- **CodeQL v4 migration** — all `github/codeql-action/*` updated from `@v3` to `@v4`

### Fixed
- **Build Fixes After Upstream Merge (2026-06-02)** — 13+ compilation errors across 6 files resolved:
  - Merge artifacts in `src/main.rs` (duplicate `system_prompt`, stray brace, missing `warn` import, missing `LLMProvider::Anthropic` match arm)
  - Type/lifetime issues in `src/agent.rs` (`&str`/`String` mismatch, `tokio::spawn` lifetime, missing `.clone()` on `Arc`)
  - Formatting/borrow issues in `src/llm.rs`, `src/mcp.rs` (double borrow, moved field)
  - Missing config fields in `src/config.rs` (47+ test constructors updated)
  - MCP test assertion fix (`protocol_version` → `protocolVersion`)
  - Retry disabled in 7 error-path mockito tests
- **Exec mode test** — fixed `check_llm_response_quality` in `scripts/lib/common.sh` to detect agent loop progress instead of non-existent log message
- **apt-get hanging in CI** — `x86_64-unknown-linux-musl` build was getting stuck indefinitely; added `DEBIAN_FRONTEND=noninteractive` and `timeout-minutes`
- **aarch64-unknown-linux-gnu build timeout** — Build & Release #68 failed; added retry loop (3 attempts) and extended timeout to 20 minutes
- **22 pre-existing clippy dead_code warnings** — resolved by replacing deprecated struct usage in tests and adding `#[allow(dead_code)]` to intentionally unused types

### Changed
- Updated ROADMAP.md, ISSUES.md, README.md, AGENTS.md for v0.6 implementation status
- Increased LOC from ~8,900 to ~9,400 (+500 for v0.6 features)
- All 277+ unit tests passing across 9 source modules
- Binary size: ~3.4 MB (arm64 macOS release build)
- All modes use `FINAL:` marker detection for completion
- Supervisor modes support up to 15 iterations for complex task decomposition
- Subtask agents run with 5-iteration limit each
- Full security wiring (policy, sandbox, audit) preserved in supervisor mode

---

## [v0.5.3] — 2026-06-07

### Added
- Native Anthropic provider (`AnthropicClient`) with direct Claude API support
- Tool use support for Anthropic (native function calling format)
- Token tracking for Anthropic responses
- Unit tests for AnthropicClient (94 verification tests total)

### Changed
- Updated ROADMAP.md to mark v0.5.3 as complete

---

## [v0.5.2] — 2026-06-06

### Added
- MCP (Model Context Protocol) client with stdio transport
- MCP tool discovery and registration into ToolRegistry
- `run_agent_loop_with_mcp()` for MCP-integrated agent execution
- CLI flags: `--mcp-command`, `--mcp-args`, `--mcp-env`

### Changed
- Agent loop now supports both built-in and MCP-discovered tools

---

## [v0.5.1] — 2026-06-06

### Added
- Retry logic with exponential backoff and jitter
- Provider fallback chain with circuit breaker
- Token budget tracking (`TokenBudget` struct)
- Cost estimation for multi-provider runs

### Changed
- `LLMConfig` now includes `retry_max`, `retry_base_delay_ms`, `token_budget`

---

## [v0.5.0] — 2026-06-06

### Added
- Unified `OpenAICompatibleClient` for OpenAI, OpenRouter, Ollama, LiteLLM
- Eliminated code duplication across 4 provider clients
- Structured function calling (OpenAI Tools format)

### Changed
- Refactored `src/llm.rs` to use trait-based architecture
- All providers now implement `LLMProviderTrait`

---

## [v0.4.0] — 2026-06-03

### Added
- Security features wired to agent loop (commit `51e42b0`)
- `PolicyEngine` with deny-by-default tool validation
- `Sandbox` for shell execution isolation
- `AuditLog` with HMAC-SHA256 tamper-evident chaining
- Tool abstraction with registry and 4 built-in tools

### Changed
- Agent loop now enforces policy checks before tool execution
- All tool calls are audited with event types

---

## [v0.3.0] — 2026-05-28

### Added
- Agent loop with ReAct pattern (perceive→plan→act→observe)
- `--exec` one-shot mode with streaming
- Interactive REPL (`--repl`) with `/exit`, `/reset` commands
- Conversation memory with configurable max history
- System prompt / persona support

---

## [v0.2.0] — 2026-05-20

### Added
- RavenFabric verification (SHA256 checksums)
- Version wiring from Cargo.toml
- Cross-compilation fixes for ARM64 Linux
- Error propagation improvements

---

## [v0.1.0] — 2026-05-15

### Added
- Initial release
- Single agent mode (single-provider and multi-model)
- 5 LLM providers: LiteLLM, OpenAI, OpenRouter, Ollama, Anthropic (stub)
- CLI with config file and env-var overrides
- Container and Kubernetes manifests with security hardening
- CI/CD pipeline with security scanning

---

[Unreleased]: https://github.com/egkristi/RavenClaws/compare/v1.7.1...HEAD
[v1.7.1]: https://github.com/egkristi/RavenClaws/compare/v1.7.0...v1.7.1
[v1.7.0]: https://github.com/egkristi/RavenClaws/compare/v0.8.0...v1.7.0
[0.8.0]: https://github.com/egkristi/RavenClaws/compare/v0.7.2...v0.8.0
[0.7.2]: https://github.com/egkristi/RavenClaws/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/egkristi/RavenClaws/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/egkristi/RavenClaws/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/egkristi/RavenClaws/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/egkristi/RavenClaws/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/egkristi/RavenClaws/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/egkristi/RavenClaws/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/egkristi/RavenClaws/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/egkristi/RavenClaws/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/egkristi/RavenClaws/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/egkristi/RavenClaws/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/egkristi/RavenClaws/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/egkristi/RavenClaws/releases/tag/v0.1.0
