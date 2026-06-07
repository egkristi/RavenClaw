# Changelog

All notable changes to RavenClaw are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — v0.6.0-dev

### Added — 2026-06-07

#### Swarm Mode (Single-Provider)
- Parallel execution of 3 agents with different personas (analytical, creative, pragmatic)
- Results collected and displayed with agent attribution
- Tokio task spawning for true parallelism
- `run_swarm()` function in `src/agent.rs`

#### Supervisor Mode (Single-Provider)
- Task decomposition into subtasks via LLM prompting
- Sub-agent spawning for each subtask
- Result aggregation and final synthesis
- Security integration (PolicyEngine, Sandbox, AuditLog)
- `run_supervisor()` and `run_subtask_agent()` functions in `src/agent.rs`

#### Swarm Mode (Multi-Model)
- Parallel agents across different LLM providers
- Provider/model attribution in results
- Cost control (capped at 3 agents)
- `run_swarm_multi()` function in `src/agent.rs`

#### Supervisor Mode (Multi-Model)
- Provider-aware task decomposition
- Round-robin supervisor LLM selection
- Subtask assignment to specific providers based on strengths
- `run_supervisor_multi()` function in `src/agent.rs`

### Changed
- Updated ROADMAP.md to reflect v0.6 implementation status
- Added 4 new tests for swarm/supervisor function existence
- Increased LOC from ~8,900 to ~9,400 (+500 for v0.6 features)

### Technical Details
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
