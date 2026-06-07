# 🐦‍⬛ RavenClaw Roadmap

**Date:** 2026-06-07  
**Version:** v0.5.3 (released 2026-06-06) — Native Anthropic Provider ✅  
**Previous Release:** v0.5.2 (2026-06-06) — MCP Client Integration ✅  
**Current Commit:** `3a7d516` — test: Add AnthropicClient unit tests (v0.5.3)

**Vision:** RavenClaw shall become the ultimate AI agentic assistant and worker —
the supreme, most trusted, and most capable autonomous agent. Simply the best.

**Core Principles** — every decision is measured against these five. If a feature
can't be added without breaking one, it doesn't ship in core.

| Pillar | What it means in practice |
|---|---|
| 🔒 **Secure** | Memory-safe Rust (`unsafe` forbidden). Fail-closed. No creds in config, TLS enforced, every tool call policy-gated and audited. Signed releases, SBOM, verified supply chain. |
| 🪶 **Small** | One static binary, distroless image, lean dependency tree. Target < 15 MB stripped, < 30 MB image. |
| ⚡ **Efficient** | Native performance, low idle memory (< 20 MB RSS), fast cold start (< 50 ms), streaming everywhere. |
| 🛡️ **Robust** | No `panic`/`unwrap` on hot paths. Retries with backoff, provider fallback, deterministic config, high coverage. |
| ✨ **Simple** | One command to run. Sensible defaults. Zero-config for common cases. No external services required for single-agent use. |

### Non-goals

- Not a heavyweight orchestration platform — RavenClaw stays a small worker; large-scale mesh coordination is delegated to **RavenFabric**.
- Not a UI/IDE — RavenClaw is a headless binary + library; frontends consume it.
- No telemetry phone-home, ever. Observability is opt-in and self-hosted.

---

## Current State

**Version:** 0.5.3 (released 2026-06-07) — v0.5 milestone complete.  
**Stats:** 9 source modules, ~8,900 LOC, 5 LLM providers (LiteLLM, OpenAI, OpenRouter, Ollama, Anthropic), 278+ unit tests + 94 verification tests, multi-arch CI with signed images + SBOM.

| Component | Status | Details |
|---|---|---|
| Single agent (single-provider) | ✅ Working | Sends one prompt, logs response, exits |
| Single agent (multi-model) | ✅ Working | Iterates all providers, logs each response |
| LLM providers (5) | ✅ Working | LiteLLM, OpenAI, OpenRouter, Ollama, **Anthropic** (unified trait) |
| CLI & env-var overrides | ✅ Working | `--provider`, `--endpoint`, `--model`, layered TOML→env→flags |
| Config validation | ✅ Working | TLS enforcement, endpoint checks |
| Container & K8s security | ✅ Working | Distroless, non-root, read-only FS, dropped caps, seccomp, RBAC |
| CI/CD pipeline | ✅ Implemented | fmt + clippy `-D warnings` + test, 5-target builds, multi-arch images, **Cosign + SBOM + provenance + Trivy**, crates.io publish, releases — cross-compilation deps installed for all targets |
| Security scanning | ✅ Implemented | CodeQL, cargo-audit, cargo-deny, cargo-outdated, cargo-udeps, Trivy (FS + config), Hadolint, Kubescape, OSSF Scorecard, dependency review — all SARIF results uploaded to GitHub Security tab |
| Verification suite | ✅ Working | 94 system/integration checks · 9 modules · 4 targets (`scripts/verify.sh`: local, Docker, Linux, K8s, security, performance, LLM-quality) — shell-orchestrated, requires live services |
| Multi-model routing | ✅ Working | `next_client()` round-robin + fallback chain with circuit breaker |
| RavenFabric integration | ⚠️ Partial | Config struct exists, agent binary baked into the image with checksum verification; runtime integration not wired |
| `--exec` one-shot mode | ✅ Working | Sends prompt to LLM, prints response to stdout; full test coverage |
| Swarm / Supervisor modes | ⚠️ Stub | Return clear error instead of silent exit 0 |
| Rust unit tests | ✅ Working | 278+ tests across all 9 modules; `mockito`-based HTTP tests for all 5 providers |
| Agent loop / ReAct planning | ✅ Working | perceive→plan→act→observe with max-iteration guard, `FINAL:` marker detection, configurable via `--max-iterations` |
| Tool-use / function calling | ✅ Working | Tool abstraction + registry + 4 built-in tools + **MCP tool discovery** + agent loop wiring |
| Deny-by-default policy | ✅ **Wired to agent loop** | `PolicyEngine` validates ALL tool calls before execution (commit 51e42b0) |
| Sandboxed execution | ✅ **Wired to agent loop** | `Sandbox` provides workdir jail for `shell_exec` (commit 51e42b0) |
| Audit log | ✅ **Wired to agent loop** | HMAC-SHA256 chained, tamper-evident, emits events for all tool calls (commit 51e42b0) |
| Streaming responses | ✅ Working | SSE streaming for LiteLLM, default non-streaming fallback for others |
| Conversation memory | ✅ Working | `ConversationMemory` struct with configurable max history, auto-trim |
| Interactive REPL | ✅ Working | `--repl` flag with stdin loop, streaming output, `/exit` `/reset` commands |
| System prompt / persona | ✅ Working | `LLMConfig.system_prompt` field, CLI `--system-prompt`, env var override |
| MCP client | ✅ Working | JSON-RPC 2.0 over stdio, tool discovery from external servers (v0.5.2) |
| Native Anthropic provider | ✅ Working | Direct Claude API with tool use, token tracking (v0.5.3) |
| Retry / fallback / circuit breaker | ✅ Working | Exponential backoff, token budgets, provider fallback chain (v0.5.1) |
| Pre-built binary releases | 📋 Wired, untagged | CI produces them on tag; none released yet |
| Structured function calling | ✅ Working | OpenAI Tools format for OpenAI/LiteLLM/OpenRouter/Anthropic |
| Multi-modal input | ⚠️ Partial | AnthropicClient has image support structure, not wired to CLI |

### ✅ v0.4.0 Released (2026-06-03)

All v0.4 blockers resolved and shipped:
- ✅ Security features wired to agent loop (commit `51e42b0`)
- ✅ Structured function calling (OpenAI Tools format)
- ✅ 274 unit tests + 94 verification tests
- ✅ CI/CD pipeline green (fmt, clippy, test, security scans)

**Known limitations (documented, not blockers):**
- k8s Deployment enters CrashLoopBackOff — server mode planned for v0.7
- SSE transport for MCP not yet implemented (stdio only in v0.5.2)
- Multi-modal input (images) — Anthropic client has stub, not wired to CLI

### 🔧 Critical Blockers (v0.5 Release)

These must be resolved before v0.5 can ship:

1. ~~**Code duplication across OpenAI-compatible clients**~~ ✅ Fixed v0.5.0 — unified `OpenAICompatibleClient`
2. ~~**No provider fallback/retry logic**~~ ✅ Fixed v0.5.1 — exponential backoff, circuit breaker
3. ~~**No token budget tracking**~~ ✅ Fixed v0.5.1 — `TokenBudget` struct with cost estimation
4. ~~**No MCP integration**~~ ✅ Fixed v0.5.2 — full MCP client with stdio transport
5. ~~**No native Anthropic provider**~~ ✅ Fixed v0.5.3 — direct Claude API with tool use

### ✅ Resolved (v0.1 → v0.5.3)

1. ~~**`Cargo.lock` is git-ignored, but `--locked` is used in CI**~~ ✅ Fixed — lockfile committed
2. ~~**Dockerfile cross-compile fails (no cross-linker)**~~ ✅ Fixed — `gcc-aarch64-linux-gnu` + linker config
3. ~~**RavenFabric agent download unverified**~~ ✅ Fixed — SHA256SUMS verification
4. ~~**CI cross-compilation missing toolchain deps**~~ ✅ Fixed — `musl-tools`, `libc6-dev-arm64-cross`
5. ~~**`--exec` dead code**~~ ✅ Fixed — fully implemented with streaming
6. ~~**Client code duplicated 4×**~~ ✅ Fixed v0.5.0 — unified `OpenAICompatibleClient`
7. ~~**No conversation memory**~~ ✅ Fixed — `ConversationMemory` with auto-trim
8. ~~**No REPL mode**~~ ✅ Fixed — `--repl` with `/exit`, `/reset`
9. ~~**No agent loop**~~ ✅ Fixed — `run_agent_loop()` with max-iteration guard
10. ~~**No tool system**~~ ✅ Fixed — 4 built-in tools + registry + agent loop wiring
11. ~~**No security infrastructure**~~ ✅ Fixed — `PolicyEngine`, `Sandbox`, `AuditLog` implemented
12. ~~**No retry/fallback logic**~~ ✅ Fixed v0.5.1 — exponential backoff, circuit breaker, token budgets, fallback chains
13. ~~**No MCP integration**~~ ✅ Fixed v0.5.2 — full MCP client with stdio transport, tool discovery, execution
14. ~~**No native Anthropic provider**~~ ✅ Fixed v0.5.3 — direct Claude API with tool use support

---

## Architecture

### Current (v0.4)

```text
        ┌──────────┐
        │  main.rs │  CLI (clap) · JSON logging · mode dispatch
        └────┬─────┘
   ┌─────────┼──────────────────────────────────────┐
┌──┴───┐ ┌───┴────┐ ┌───┴─────┐ ┌───┴───┐ ┌────────┐
│agent │ │ config │ │  error  │ │ tools │ │policy  │
│ loop │ │        │ │         │ │       │ │audit   │
│ mem  │ │        │ │         │ │       │ │sandbox │
└──┬───┘ └────────┘ └─────────┘ └───────┘ └────────┘
   │
┌──┴───────────────────────────┐
│ llm  (LLMProviderTrait)       │
│  LiteLLM · OpenAI · OpenRouter│
│  · Ollama · MultiModelManager │
└───────────────────────────────┘

⚠️ Dotted lines = NOT wired: policy, audit, sandbox are infrastructure-only
```

### Target (v1.0)

```text
                    ┌──────────┐
                    │   CLI    │  single · serve · swarm · supervisor
                    └────┬─────┘
                  ┌──────┴───────┐
                  │  Agent Core  │  perceive → plan → act → observe (+ memory)
                  └──┬────┬───┬──┘
          ┌──────────┘    │   └──────────┐
     ┌────┴────┐    ┌─────┴────┐   ┌──────┴───────┐
     │  Tools  │    │ Providers│   │ Orchestration │
     │ policy✅│    │ routing+ │   │ swarm/superv. │
     │ sandbox✅│   │ fallback+│   │ + RavenFabric │
     │ audit  ✅│   │ budgets  │   │  (E2E remote) │
     └─────────┘    └──────────┘   └───────────────┘
          │
   ┌──────┴───────┐
   │ Observability│  metrics · tracing · health endpoint
   └──────────────┘

✅ = Infrastructure exists, needs wiring to agent loop (v0.4)
```

---

## Competitive Positioning

RavenClaw aims to be the **preferred alternative** to the current field — including
Cognition (Claude), Manus, Perplexity Comet, Kimi, Open Interpreter, and Vellum.

We don't win by out-featuring them. We win by refusing to compromise on all five
pillars at once. By category:

- **vs. cloud / hosted assistants** (Claude Cowork, Manus, Perplexity Computer, Kimi): RavenClaw is **self-hostable, offline-capable, and source-available** under AGPLv3. Your data and tool calls never leave infrastructure you control — no phone-home.
- **vs. minimal agent runtimes** (Open Interpreter, ZeroClaw, PicoClaw): RavenClaw matches their footprint while adding a real **security model** (deny-by-default tool policy, audit log, sandboxing) and **multi-provider** routing with fallback.
- **vs. SDK / platform plays** (Vellum, Hermes Agent): RavenClaw is a **single dependency-light binary**, not a service you rent or a framework you marry. Embed it, ship it, forget it.

The bar: anything the field can do, RavenClaw should do **smaller, safer, and
simpler** — or deliberately not at all.

> **Where RavenClaw must lead, measurably (v1.0):** memory-safe core with zero
> known CVEs, sub-15 MB binary, sub-50 ms cold start, fully self-hostable and
> air-gappable, signed + SBOM-attested supply chain. These are claims we will
> benchmark and publish — not marketing.

### RavenClaw vs. Field (v0.4)

| Capability | RavenClaw v0.4 | Cognition (Claude) | Manus | Open Interpreter |
|---|:---:|:---:|:---:|:---:|
| Agent loop | ✅ | ✅ | ✅ | ✅ |
| Tool calling | ✅ (primitive) | ✅ (structured) | ✅ | ✅ |
| **MCP client/server** | ❌ | ✅ | ✅ | ✅ |
| Sandboxed execution | ⚠️ (not wired) | ✅ | ✅ | ⚠️ Optional |
| **Security model** | ⚠️ (not wired) | ⚠️ | ⚠️ | ❌ |
| **Local-first / air-gapped** | ✅ (Ollama) | ❌ | ❌ | ✅ |
| **~3 MB binary** | ✅ | ❌ (cloud) | ❌ (cloud) | ❌ (Python) |
| **RavenFabric mesh** | ❌ (roadmap) | ❌ | ❌ | ❌ |
| **No telemetry** | ✅ | ❌ | ❌ | ✅ |
| Multi-modal input | ❌ | ✅ | ✅ | ⚠️ |
| Web search | ⚠️ (fetch only) | ✅ | ✅ | ✅ |
| Browser automation | ❌ | ✅ | ✅ | ⚠️ Plugins |
| Async background runs | ❌ | ✅ | ✅ | ❌ |
| Scheduling / triggers | ❌ | ✅ | ✅ | ❌ |
| Sub-agents / swarm | ❌ (stub) | ✅ | ✅ | ❌ |
| OAuth connectors | ❌ | ✅ | ✅ | ⚠️ Plugins |

**RavenClaw's Wedge:**
1. **Trust as a feature** — deny-by-default security, no telemetry, verifiable end-to-end
2. **Edge-deployable** — ~3 MB binary, runs on Raspberry Pi, air-gapped capable
3. **RavenFabric mesh** — E2E-encrypted remote execution across fleet (unique)

---

## Features Required to Become the Preferred Alternative

Being *preferred* is a two-step bar: first reach **parity** on the capabilities the
field now treats as table stakes, then **win decisively** on the five pillars where
the cloud incumbents structurally can't follow.

### Part 1 — Table stakes (reach parity)

| Capability | Why it's table stakes | In RavenClaw | Target |
|---|---|:--:|:--:|
| Agent loop (plan → act → observe) | Without it there is no "agent" | ✅ | v0.3 |
| Tool / function calling | The substrate for every action | ✅ (primitive) | v0.4 |
| **MCP — client *and* server** | Industry standard (Anthropic, OpenAI, Google, Microsoft, Salesforce) | ❌ | **v0.4** |
| Sandboxed execution | Native primitive in competitors | ⚠️ (not wired) | v0.4 |
| Persistent memory (vector recall) | Without it every session starts from zero | ⚠️ (in-memory only) | v0.3 → v0.9 |
| Web search + headless browser | Manus/Perplexity center on browse/summarize/fill-forms | ⚠️ (fetch only) | **v0.4** |
| File operations (read/write/edit) | Core to "worker" | ✅ | v0.4 |
| Sub-agents / swarm orchestration | Kimi runs 300 sub-agents / 4,000 steps | ❌ (stub) | v0.6 |
| Async / long-horizon background runs | Manus's killer feature (cloud background) | ❌ | **v0.7** |
| Scheduling / triggers (cron, webhook) | Proactive, set-and-forget operation | ❌ | **v0.7** |
| Streaming + intermediate results | First-class in Vellum; needed for interactive UX | ✅ | v0.3 |
| Multi-modal input (images, PDFs) | Manus/Kimi are multimodal; "worker" must read docs | ❌ | v0.5 |
| Connectors / integrations (OAuth) | Claude-style connectors; Manus's weakness | ❌ | v0.6 |
| Retries / provider fallback | Vellum: retry, fall back, fail early | ⚠️ (partial) | v0.5 |
| Human-in-the-loop approvals | Enterprises require guardrails + audit + HITL | ❌ | **v0.4** |

### Part 2 — Where RavenClaw wins (the "preferred" wedge)

| Differentiator | Why it beats the field | Pillars | Phase |
|---|---|:--:|:--:|
| **Local-first / self-hosted / air-gapped** | Manus is cloud-only; Comet's "Local" is a browser, not a worker. RavenClaw runs fully offline with Ollama. | Secure · Simple | ✅ core |
| **Security model: deny-by-default + sandbox + audit** | Field bolts security on; we ship it in core. | Secure | ⚠️ v0.4 (wire it) |
| **~3 MB single binary, edge/embeddable** | No cloud agent runs on a Raspberry Pi. | Small · Efficient | ✅ |
| **Provider-agnostic + cost-aware routing** | Not locked to one model vendor. | Efficient · Robust | v0.5 |
| **RavenFabric mesh: E2E-encrypted remote exec** | Unique — competitors are single-host or single-cloud. | Robust | v0.6 |
| **No telemetry · signed + SBOM** | Trust as a feature, verifiable end-to-end. | Secure | ✅ |
| **Open core + commercial** | No lock-in, vs. proprietary cloud. | Simple | ✅ |

### Part 3 — The five that move the needle most

1. **MCP client + server (v0.4)** — instant access to entire tool ecosystem. Single highest-leverage feature.
2. **Wire security model (v0.4)** — PolicyEngine + Sandbox + AuditLog invoked on every tool call. Core value proposition.
3. **Local-first privacy + security** — the wedge no cloud agent can copy.
4. **Async / background + scheduling (v0.7)** — matches Manus's "assign-and-walk-away".
5. **RavenFabric distributed execution (v0.6)** — the capability *no competitor has*.

---

## Phased Plan

Versions are capability milestones, not dates. Each must keep all five pillars green.

### v0.2 — Foundations: make the build honest and green 🔧

- [x] **Commit `Cargo.lock`** (remove from `.gitignore`) so `--locked` works in CI/Docker/publish.
- [x] **Fix multi-arch Docker build** — install cross-linkers (`gcc-aarch64-linux-gnu`) + set the cargo target linker.
- [x] **Verify the RavenFabric agent download** against a published checksum / Cosign signature.
- [x] **Single source of version truth** — wire clap `--version` to `env!("CARGO_PKG_VERSION")`.
- [x] **Replace `.expect()` on HTTP client construction** with error propagation (no abort path under `panic = "abort"`).
- [x] **Decide `--exec`**: implement one-shot mode (preferred, see v0.3) or remove the flag.
- [x] **Make swarm/supervisor fail loudly** — return a clear error instead of `exit 0` until implemented.
- [x] **Expand tests** — use `mockito` to exercise request/response/error paths for every provider; cover config parsing and the multi-model manager.
- [x] **README status-honesty.**

**Exit criteria:** `cargo fmt && cargo clippy -D warnings && cargo test` green; `docker buildx` produces working `amd64`+`arm64` images; fresh clone builds with `--locked`.

### v0.3 — A real agent 🧠

- [x] **Agent loop**: perceive → plan → act → observe, with max-iteration guard and cancellation.
- [x] **`--exec "<task>"`** one-shot mode — sends prompt to LLM, prints response to stdout.
- [x] **Interactive REPL** (stdin) — continuous conversation mode.
- [x] **Conversation memory** — context across turns; configurable window (last N turns or token budget); session save/restore.
- [x] **Streaming responses** end to end (`stream = true`) across the trait and all clients.
- [x] **System-prompt / persona** configuration.
- [x] **Robust errors** — typed retries, timeouts, graceful provider failure. All error paths covered with `thiserror` + `anyhow`; 26 error tests across 7 variants.

**Exit criteria:** `ravenclaw --exec "summarize this repo"` performs a real multi-step task and returns a result.

### v0.4 — Tools and safety 🧰🔒 **(CURRENT)**

Agency with guardrails — the security differentiator.

- [x] **Tool / function-calling abstraction** (provider-agnostic schema + registry).
- [x] **Built-in tools**: shell exec, file read/write, web fetch — each behind a capability flag.
- [x] **Tool wiring into agent loop** — `run_agent_loop` detects `TOOL_CALL:` / `ARGS:` patterns, executes tools, injects results as `OBSERVATION:`.
- [x] **Deny-by-default policy** (command / path / host allow-lists), à la RavenFabric's RPCPolicy.
- [x] **Sandboxed execution** (workdir jail, resource limits, timeouts).
- [x] **Audit log** — structured, HMAC-chained, tamper-evident trail of every tool call.
- [x] **Wire security to agent loop** — `PolicyEngine` validates all tool calls; `Sandbox` executes `shell_exec`; `AuditLog` emits events. **COMMIT: 51e42b0**
- [ ] **Structured function calling** — OpenAI Tools format for OpenAI/LiteLLM/OpenRouter; native JSON instead of pattern-matching. **(BLOCKER)**
- [ ] **MCP — client *and* server** — consume any Model Context Protocol tool/server, and expose RavenClaw itself as an MCP server. The industry tool standard (Anthropic, OpenAI, Google, Microsoft, Salesforce). **(HIGHEST LEVERAGE)**
- [ ] **Human-in-the-loop approvals** — configurable approval gates for sensitive tool calls (allow / deny / ask).
- [ ] **Web search + headless browser tool** — search, navigate, extract, and fill forms (beyond simple web fetch).
- [ ] **Wire `zeroize`** for secret material; automatic secret/PII redaction in logs.
- [ ] **Honor `token_lifetime_secs`** for any issued credentials.
- [ ] **Prompt-injection defense** — instruction-boundary enforcement, output schema validation.

**Exit criteria:** an agent runs tools, but only those allowed by policy, with a complete audit log. Security features actively invoked, not just present.

### v0.5 — Providers and routing 🔀 **(COMPLETE 2026-06-07)**

**Primary objective:** Eliminate code duplication and add production-grade resilience.

- [x] **Unified OpenAI-Compatible Client** ✅ v0.5.0
  - Merge LiteLLM, OpenAI, OpenRouter into `OpenAICompatibleClient` with provider enum
  - Provider-specific defaults: endpoint, headers (OpenRouter needs `HTTP-Referer`, `X-Title`)
  - Keep Ollama separate (different API format)
  - **Impact:** ~400 LOC reduction, single maintenance path

- [x] **Retry & Fallback Chain** ✅ v0.5.1
  - Exponential backoff with jitter (base 100ms, max 10s, 3 retries)
  - Fallback chain: primary → secondary → tertiary (configurable order)
  - Circuit breaker: open after 5 consecutive failures, half-open after 30s
  - **Exit criteria:** `ravenclaw --exec "task"` with fallback to Ollama when cloud providers fail

- [x] **Token Budget & Cost Tracking** ✅ v0.5.1
  - `--token-budget <N>` CLI flag and `RAVENCLAW_TOKEN_BUDGET` env var
  - Track tokens per request using `usage` field in responses
  - Cost estimation table (per-provider, per-model pricing)
  - Auto-downgrade: switch to cheaper model when 80% of budget consumed
  - **Exit criteria:** Agent stops before exceeding budget, logs cost estimate

- [x] **MCP Client Integration** (highest leverage) ✅ v0.5.2
  - MCP client: connect to external MCP servers (filesystem, database, API tools)
  - Tool discovery and registration from MCP servers
  - Protocol: JSON-RPC over stdio or SSE
  - **Exit criteria:** Can use MCP-provided tools alongside built-in tools

- [x] **Native Anthropic Provider** ✅ v0.5.3
  - Direct Anthropic API client (not via OpenRouter)
  - Support for tool use (Anthropic's native function calling)
  - Image input support (stubbed for future multi-modal expansion)
  - Full test coverage (4 unit tests + integration via factory)

- [ ] **Multi-modal Input** ⚠️ **PARTIAL** — AnthropicClient has image support structure, not wired to CLI
  - Image attachments in `ChatMessage` (base64 or URL)
  - PDF/text document ingestion
  - Provider-specific encoding (OpenAI vision, Anthropic images)

- [ ] **Skill / Plugin System** (foundations) — **MOVED TO v0.6**
  - Portable capability bundles: `skill.yaml` + scripts + resources
  - Progressive disclosure: skills advertise capabilities, agent selects
  - Sandboxed skill execution (reuse `Sandbox`)

**Exit criteria:** ✅ COMPLETE
1. [x] Single run transparently fails over between providers
2. [x] Respects token budget
3. [x] Can consume MCP-provided tools
4. [x] Code coverage ≥80% on routing/fallback logic (274+ tests across 9 modules)

### v0.6 — Swarm, supervisor, and RavenFabric 🕸️

- [ ] **Supervisor mode** — task decomposition, sub-agent spawning, result aggregation, quality checks.
- [ ] **Swarm mode** — coordinated agents with a shared blackboard/state; per-subtask model selection.
- [ ] **RavenFabric integration** — secure E2E remote command execution + mesh coordination (the headline capability).
- [ ] **Agent communication** — structured message passing; conflict resolution across agents.
- [ ] **Connectors / integrations** — OAuth connectors for Google Drive, M365, Slack, GitHub, Notion (acts as the user, not a shared service account).
- [ ] **Skill / Plugin System** (foundations) — **MOVED FROM v0.5**
  - Portable capability bundles: `skill.yaml` + scripts + resources
  - Progressive disclosure: skills advertise capabilities, agent selects
  - Sandboxed skill execution (reuse `Sandbox`)

**Exit criteria:** a supervisor decomposes a task across ≥3 sub-agents over RavenFabric and aggregates results.

### v0.7 — Observability and ops 📈

- [ ] **Long-running server mode** with a real HTTP `/health` `/ready` `/metrics` endpoint (fixes the k8s CrashLoop).
- [ ] **Prometheus metrics** (requests, tokens, tool calls, errors, latencies).
- [ ] **OpenTelemetry tracing** (opt-in, self-hosted collector, correlation IDs).
- [ ] **Graceful shutdown**, signal handling, `health_interval_secs` honored.
- [ ] **Helm chart**; systemd unit; optional self-update with rollback.
- [ ] **Async / long-horizon background runs** — assign-and-walk-away background execution, resumable across restarts (matches Manus's headline UX).
- [ ] **Scheduling & triggers** — cron, webhook, and file-watch activation for proactive 24/7 agents.
- [ ] **Eval harness + run inspection** — golden-task evals, assertions on intermediate steps, and replayable run traces.

**Exit criteria:** RavenClaw runs as a stable long-lived workload with green probes and exported metrics.

### v0.8 — Enterprise and compliance 🏢 *(commercial-licensed)*

Maps to the commercial tier in [LICENSING.md](LICENSING.md).

- [ ] **RBAC + multi-tenant isolation** (separate workspaces, secrets, quotas).
- [ ] **SSO / SAML.**
- [ ] **SecurityPolicy** — immutable rules, blast-radius limits.
- [ ] **Multi-level audit logging** — levels (`off`/`basic`/`detailed`/`debug`), formats (JSON/CEF/LEEF/Syslog), shipping sinks, integrity chaining.
- [ ] **Compliance presets & reporting** (SOC2, ISO 27001, HIPAA, GDPR, PCI-DSS).
- [ ] **Air-gap / offline licensing**; runtime feature-flag gating.
- [ ] **Output artifacts & reporting** — generate documents, spreadsheets, slides, and sites via the skill system (v0.5); underpins compliance and executive reporting.

### v0.9 — Hardening, ecosystem, advanced reasoning 💎

- [ ] **Threat model + external security review.**
- [ ] **Fuzzing** (`cargo fuzz`) + property tests for config/policy parsers.
- [ ] **Skill/plugin marketplace + WASM sandboxing** for third-party extensions (core MCP ships in v0.4, the skill system in v0.5).
- [ ] **SDKs** (Python/TS) and a documentation site.
- [ ] **Advanced reasoning** — tree-of-thought, self-reflection, uncertainty estimation / ask-for-help.
- [ ] **Memory tiers** — episodic, semantic (local embeddings), procedural.

### v1.0 — Simply the best 🏆

- [ ] **API stability** guarantees + semver discipline.
- [ ] **All performance targets met** and benchmarked against the field (published).
- [ ] **Complete docs**, examples, migration guides.
- [ ] **Reproducible builds.**

---

## Testing Strategy

- **Unit:** every module; provider request/response/error paths via `mockito`.
- **Integration:** end-to-end agent runs against a stubbed provider and a local Ollama.
- **Policy/security:** table-driven allow/deny tests; fuzzing on policy + config parsing.
- **CI gates:** `fmt`, `clippy -D warnings`, `test`, Trivy (CRITICAL/HIGH fail), SBOM per release.
- **Coverage goal:** ≥ 80% line coverage by v1.0; no `unwrap`/`expect` on non-test hot paths.

**Current coverage:** 274 unit tests across 8 modules + 94 verification tests across 4 deployment targets.

---

## Performance Targets (v1.0)

| Metric | Target | Current |
|---|---|---|
| Stripped binary size | < 15 MB | ~3 MB ✅ |
| Container image size | < 30 MB | ~50 MB ⚠️ |
| Cold start (single mode) | < 50 ms | ~7 ms ✅ |
| Idle memory (server mode) | < 20 MB RSS | N/A (no server) |
| Provider failover decision | < 5 ms | N/A (no fallback) |
| Tool-call audit write | non-blocking, < 1 ms enqueue | N/A (not wired) |

---

## Security Hardening (by version)

| Version | Hardening added |
|---|---|
| 0.1 | Memory-safe Rust, TLS check, no creds in config, distroless, signed images, SBOM, Trivy. |
| 0.2 | Verified supply chain for downloaded binaries (SHA256 checksum); no panic/abort on client init; cross-compilation deps in CI. |
| 0.4 | Deny-by-default tool policy, sandboxed execution, audit log, secret zeroization, prompt-injection defense. **(Infrastructure complete, needs wiring)** |
| 0.6 | E2E-encrypted remote exec via RavenFabric. |
| 0.8 | RBAC, SecurityPolicy with blast-radius limits, compliance reporting. |
| 0.9 | External security review, fuzzing, published threat model. |

---

## Design Decisions

- **Rust, `unsafe` forbidden** — memory safety and small static binaries are foundational to "secure + small."
- **OpenAI-compatible core** — most providers speak it; one client shape covers LiteLLM/OpenAI/OpenRouter, with Ollama as the documented exception.
- **AGPLv3 + Commercial dual license** — keeps the core open, closes the SaaS loophole, funds development. See [LICENSING.md](LICENSING.md).
- **Delegate heavy orchestration to RavenFabric** — RavenClaw stays a small worker; the mesh/remote-exec substrate is a separate, specialized system.
- **No phone-home** — observability is opt-in and self-hosted; trust is a feature.

---

## Technical Debt

Concrete items carried from the current codebase:

1. **Security infrastructure not wired** — `PolicyEngine`, `Sandbox`, `AuditLog` are complete but never invoked. *(v0.4 blocker)*
2. **Pattern-matching tool calls** — Fragile `TOOL_CALL:` / `ARGS:` parsing instead of structured JSON. *(v0.4 blocker)*
3. **No MCP integration** — Reinventing tools instead of using industry standard. *(v0.4 highest-leverage)*
4. **k8s Deployment runs a program that exits immediately** → needs server mode (v0.7) or a Job manifest meanwhile.
5. **Client duplication** across LiteLLM/OpenAI/OpenRouter (`handle_response` ×4). *(v0.5)*
6. **Dead/unwired code:** `rustls` + `zeroize` deps unused; `security`/`ravenfabric` config fields not honored. *(v0.5)*
7. **No graceful shutdown** — SIGTERM/SIGINT not handled; no audit log flush on exit. *(v0.5)*
8. **No config hot-reload** — Changes require restart. *(v0.6)*
9. **Container image ~50 MB** — Target is < 30 MB. *(v0.5)*
10. **cargo-udeps findings** — Unused dependencies detected. *(periodic review)*
11. **cargo-outdated findings** — Dependencies behind latest. *(periodic review)*

---

## How You Can Help

- **Contributors:** pick an unchecked item and open a PR (CLA required — see [LICENSING.md](LICENSING.md#contributor-license-agreement-cla)).
- **Security researchers:** audit the code and report responsibly. *(A `SECURITY.md` policy is planned for v0.2.)*
- **Users:** file issues for missing features or rough edges.
- **Enterprise:** ask about commercial licensing and priority features.

---

*Secure. Small. Efficient. Robust. Simple. — Simply the best.* 🐦‍⬛
