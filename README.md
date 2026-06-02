# 🐦‍⬛ RavenClaw — The Ultimate AI Agentic Worker

**Secure · Small · Efficient · Robust · Simple. Built in Rust.**

[![License](https://img.shields.io/badge/license-AGPLv3%20%2B%20Commercial-blue.svg)](LICENSING.md)
[![CI](https://github.com/egkristi/RavenClaw/actions/workflows/build.yml/badge.svg)](.github/workflows/build.yml)
[![Verification](https://img.shields.io/badge/verification-94%20checks-brightgreen)](VERIFICATION.md)
[![Binary](https://img.shields.io/badge/binary-~3MB-blue)]()
[![Status](https://img.shields.io/badge/status-pre--alpha-orange)](ROADMAP.md)

RavenClaw is a lightweight, secure Rust agent framework with multi-provider LLM
support. One static binary, zero runtime dependencies — no Python, no Node, no JVM.

> **Status: Pre-Alpha (v0.1.0).** The provider layer, deployment tooling, and the
> verification + supply-chain pipeline work today. The agent loop, tool-use, and
> swarm/supervisor modes are on the [roadmap](ROADMAP.md). This README marks
> ✅ built vs. 📋 planned — honestly. Trust is a feature; we don't inflate it.

| Footprint | Security | Providers | Deployment |
|---|---|---|---|
| **~3 MB binary** | **Memory-safe Rust** | **4 providers** | **Binary · Docker · K8s** |
| **0 runtime deps** | **Signed images + SBOM** | **Multi-model** | **94-check verification suite** |

---

## Vision

RavenClaw aims to be the **ultimate AI agentic assistant and worker** — and the
**preferred alternative** to the field: Nemoclaw, Hermes Agent, TrustClaw, ZeroClaw,
PicoClaw, NanoClaw, Claude Cowork, Manus, Perplexity Computer, Kimi Claw, and Vellum.

We don't aim to win by out-featuring them. We win by refusing to compromise on five
pillars at once:

- 🔒 **Secure** — memory-safe Rust (`unsafe` forbidden), fail-closed, no creds in config, verified supply chain.
- 🪶 **Small** — one static binary (~3 MB), distroless image, lean dependency tree.
- ⚡ **Efficient** — native performance, low memory, fast cold start, streaming everywhere.
- 🛡️ **Robust** — graceful degradation, provider fallback, deterministic config, verified across 4 deployment targets.
- ✨ **Simple** — one command to run, sensible defaults, no external services required for single-agent use.

See the **[ROADMAP](ROADMAP.md)** for how we get from here to there.

---

## Why RavenClaw?

### Small & efficient

- **~3 MB** stripped release binary (measured) — no interpreter, no runtime image baggage.
- **Single static binary** — no Python, no Node, no JVM, zero runtime dependencies.
- Native Rust with `lto` + `panic=abort`. Design targets (benchmarked toward v1.0 via the [verification suite](VERIFICATION.md)): **< 50 ms** cold start, **< 20 MB** RSS, **< 15 MB** binary across all targets.

### Secure & trustworthy

- **Memory-safe Rust** — whole classes of memory-corruption bugs eliminated at compile time.
- **No credentials in config** — environment variables and Kubernetes Secrets only.
- **Hardened containers** — distroless, non-root, read-only root filesystem, dropped capabilities, seccomp.
- **Verified supply chain** — multi-arch images signed with **Cosign**, **SBOM** (Syft) and build **provenance** attestation, plus **CodeQL**, **cargo-audit**, **cargo-deny**, **Trivy**, **Hadolint**, **Kubescape**, and **OSSF Scorecard** in CI.
- **TLS enforced** by default for non-local endpoints.
- *(Planned: deny-by-default tool policy, sandboxed tool execution, and a tamper-evident audit log — see roadmap v0.4. The `security.audit_log` config surface exists today but is not yet enforced.)*

### Multi-provider, multi-model

- **LiteLLM** — OpenAI-compatible proxy fronting 100+ models.
- **OpenAI** — native GPT-4o, o-series, and more.
- **OpenRouter** — unified API for many hosted models.
- **Ollama** — local, private, air-gapped models.
- **Multi-model mode** — configure several providers at once (basic round-robin today; intelligent routing + fallback on the roadmap).

### Verified across every target

- A **94-check verification suite** (`scripts/verify.sh`) spanning **8 modules** across **4 deployment targets** — local binary, Docker, cross-compiled Linux, and Kubernetes — including security and performance checks. Each module runs independently; results are logged.
- *Note:* these are **system/integration checks** (shell-orchestrated, requiring live services such as LiteLLM/Docker/kubectl). Rust-level unit tests are currently minimal and are being expanded — see [ROADMAP](ROADMAP.md).

---

## Quick Start

### 30 seconds to your first agent

```bash
# Build from source (requires Rust)
git clone https://github.com/egkristi/RavenClaw
cd RavenClaw
cargo build --release

# Run with any provider
export LITELLM_API_KEY="your-key"
export RAVENCLAW__LLM__ENDPOINT="http://localhost:4000"
./target/release/ravenclaw --mode single
```

> **Note:** Pre-built binaries are wired in CI and publish on tagged releases; none tagged yet. Build from source for now. See [ROADMAP.md](ROADMAP.md).

### Docker

```bash
docker run --rm -it \
  -e LITELLM_API_KEY="your-key" \
  -e RAVENCLAW__LLM__ENDPOINT="http://litellm:4000" \
  ghcr.io/egkristi/ravenclaw:latest
```

### Docker Compose (with LiteLLM)

```bash
docker compose up -d
docker compose logs -f ravenclaw
```

### Kubernetes

```bash
kubectl apply -f k8s/deployment.yaml
kubectl -n ravenclaw get pods
kubectl -n ravenclaw logs -l app.kubernetes.io/name=ravenclaw
```

> Single mode currently performs one request and exits. A long-running **server
> mode** with `/health` `/ready` `/metrics` is on the roadmap (v0.7); until then,
> prefer the `deployment-test.yaml`/Job-style manifest for k8s smoke tests.

## Configuration

### Environment variables

| Variable | Description | Default |
|---|---|---|
| `RAVENCLAW__LLM__PROVIDER` | Provider: litellm, openrouter, ollama, openai | `litellm` |
| `RAVENCLAW__LLM__ENDPOINT` | LLM endpoint URL | (provider-dependent) |
| `RAVENCLAW__LLM__MODEL` | Default model | `gpt-4o-mini` |
| `LITELLM_API_KEY` | API key for LiteLLM/OpenRouter/OpenAI | (required for cloud) |
| `RAVENCLAW__LLMS` | JSON array for multi-model config | — |
| `RAVENCLAW__RAVENFABRIC__ENDPOINT` | RavenFabric endpoint (optional) | — |
| `RAVENCLAW__SECURITY__REQUIRE_TLS` | Enforce TLS | `true` |
| `RAVENCLAW__RUNTIME__MAX_AGENTS` | Max concurrent agents | `10` |
| `RUST_LOG` | Log level | `info` |

### Single provider mode

**LiteLLM:**
```toml
[llm]
provider = "litellm"
endpoint = "http://litellm:4000"
model = "gpt-4o-mini"
timeout_secs = 30
```

**Ollama (local, no API key):**
```toml
[llm]
provider = "ollama"
endpoint = "http://localhost:11434"
model = "llama3.1"
timeout_secs = 60
```

**OpenRouter:**
```toml
[llm]
provider = "openrouter"
model = "anthropic/claude-sonnet-4-20250514"
```

**OpenAI:**
```toml
[llm]
provider = "openai"
model = "gpt-4o"
```

### Multi-model mode

Configure several providers at once (basic round-robin today):

```toml
[[llms]]
provider = "ollama"
endpoint = "http://ollama:11434"
model = "llama3.1"

[[llms]]
provider = "openrouter"
model = "anthropic/claude-sonnet-4-20250514"

[[llms]]
provider = "openai"
model = "gpt-4o"
```

### Full config example

```toml
[llm]
provider = "litellm"
endpoint = "http://litellm:4000"
model = "gpt-4o-mini"
timeout_secs = 30

# Optional: RavenFabric for swarm/supervisor coordination (integration on roadmap)
# [ravenfabric]
# endpoint = "http://ravenfabric:8080"
# remote_exec = true
# allowed_hosts = ["litellm", "ravenfabric"]

[security]
require_tls = true
token_lifetime_secs = 3600   # surface present; enforcement on roadmap (v0.4)
audit_log = true             # surface present; enforcement on roadmap (v0.4)

[runtime]
workdir = "/workspace"
max_agents = 10
health_interval_secs = 60
```

## Agent modes

| Mode | Status | Description |
|---|---|---|
| `single` | ✅ **Working** | Sends prompt to LLM, logs response (one-shot, no agent loop yet) |
| `single` (multi-model) | ✅ **Working** | Iterates all configured providers, logs each response |
| `swarm` | 📋 Planned | Currently a stub that warns and exits — see [ROADMAP.md](ROADMAP.md) |
| `supervisor` | 📋 Planned | Currently a stub that warns and exits — see [ROADMAP.md](ROADMAP.md) |

## Building from source

### Prerequisites

- [Rust](https://rustup.rs/) 1.86+
- For Linux cross-compilation on macOS: `brew install FiloSottile/musl-cross/musl-cross`

### Build for host

```bash
git clone https://github.com/egkristi/RavenClaw
cd RavenClaw

cargo build --release      # release build for current platform
cargo test                 # unit tests
./scripts/verify.sh        # full 94-check verification suite (needs LiteLLM/Docker/kubectl)
docker build -t ravenclaw:latest .
```

### Cross-compile for all architectures

```bash
rustup target add \
    x86_64-apple-darwin aarch64-apple-darwin \
    x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
    x86_64-unknown-linux-musl

./scripts/build.sh --all                                   # all targets
./scripts/build.sh --target aarch64-unknown-linux-gnu      # one target
```

### Multi-arch Docker image

```bash
docker buildx build \
    --platform linux/amd64,linux/arm64 \
    -t ghcr.io/egkristi/ravenclaw:latest \
    --push .
```

## Downloads

> **Note:** Pre-built binaries publish automatically on tagged releases (CI is wired
> for it); none tagged yet. Build from source meanwhile.

| Architecture | Target Triple |
|---|---|
| Apple Silicon (M1+) | `aarch64-apple-darwin` |
| Intel Mac | `x86_64-apple-darwin` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` |
| Linux x86_64 (glibc) | `x86_64-unknown-linux-gnu` |
| Linux x86_64 (musl/static) | `x86_64-unknown-linux-musl` |

Container images target both `linux/amd64` and `linux/arm64`.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    RavenClaw Agent                    │
│  ┌──────────────────────────────────────────────┐    │
│  │         Single Mode (✅ Working)               │    │
│  │    Sends prompt → LLM → logs response          │    │
│  │    (one-shot; agent loop on roadmap)           │    │
│  └─────────────────────┬────────────────────────┘    │
│                        │                              │
│  ┌─────────────────────┴─────────────────────────┐    │
│  │         LLM Provider Abstraction Layer         │    │
│  │  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐  │    │
│  │  │LiteLLM │ │ OpenAI │ │OpenRtr │ │ Ollama │  │    │
│  │  └────────┘ └────────┘ └────────┘ └────────┘  │    │
│  └─────────────────────┬─────────────────────────┘    │
│                        │                              │
│  ┌─────────────────────┴─────────────────────────┐    │
│  │              Security Layer                    │    │
│  │  TLS · env-only secrets · non-root · RBAC      │    │
│  └────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
         │                          │
         ▼                          ▼
┌─────────────────┐      ┌──────────────────────┐
│  RavenFabric    │      │   Deployment Targets  │
│  (planned)      │      │  Binary · Docker · K8s │
└─────────────────┘      └──────────────────────┘
```

### What's implemented vs. planned

| Component | Status | Details |
|---|---|---|
| Single agent (single + multi-model) | ✅ Working | Sends prompt(s), logs response(s) |
| LLM providers (4) | ✅ Working | LiteLLM, OpenAI, OpenRouter, Ollama (unified trait) |
| CLI & env-var overrides | ✅ Working | `--provider`, `--endpoint`, `--model`; layered TOML→env→flags |
| Config validation | ✅ Working | TLS enforcement, endpoint checks |
| Container & K8s security | ✅ Working | Distroless, non-root, read-only FS, dropped caps, seccomp, RBAC |
| CI/CD pipeline | ✅ Implemented | fmt + clippy + test, 5-target builds, multi-arch images, Cosign + SBOM + provenance + Trivy, crates.io publish, releases |
| Security scanning | ✅ Implemented | CodeQL, cargo-audit, cargo-deny, Trivy (FS + config), Hadolint, Kubescape, OSSF Scorecard |
| Verification suite | ✅ Working | 94 system/integration checks · 8 modules · 4 targets (`scripts/verify.sh`) |
| Rust unit tests | ⚠️ Minimal | 3 smoke-level tests; `mockito` unused — expanding (see roadmap) |
| Multi-model routing | ⚠️ Partial | `next_client()` round-robin exists but is not yet wired into a strategy |
| RavenFabric integration | ⚠️ Partial | Config + container binary present; runtime wiring pending |
| `--exec` one-shot mode | 📋 Planned | CLI arg parsed but not yet used |
| Swarm / Supervisor modes | 📋 Planned | Currently stubs |
| Agent loop / ReAct planning | 📋 Planned | One-shot send-and-exit today |
| Tool-use / function calling | 📋 Planned | The #1 capability gap |
| Streaming responses | 📋 Planned | `stream` not yet wired |
| Conversation memory | 📋 Planned | In-memory only, lost on exit |

## How RavenClaw intends to win

RavenClaw is positioned against the field — Nemoclaw, Hermes Agent, TrustClaw,
ZeroClaw, PicoClaw, NanoClaw, Claude Cowork, Manus, Perplexity Computer, Kimi Claw,
and Vellum — by category:

- **vs. cloud / hosted assistants** (Claude Cowork, Manus, Perplexity Computer, Kimi Claw): RavenClaw is **self-hostable, offline-capable, and source-available** under AGPLv3. Your data and tool calls never leave infrastructure you control — and there is no phone-home.
- **vs. minimal agent runtimes** (ZeroClaw, PicoClaw, NanoClaw, TrustClaw): RavenClaw matches their footprint while adding a real **security model** (memory-safe core, verified supply chain, and — on the roadmap — deny-by-default tool policy, sandboxing, and an audit log) plus **multi-provider** routing.
- **vs. SDK / platform plays** (Vellum, Hermes Agent, Nemoclaw): RavenClaw is a **single dependency-light binary**, not a service you rent or a framework you marry. Embed it, ship it, forget it.

| Our commitment | How we back it |
|---|---|
| Memory-safe core | Rust with `unsafe` forbidden |
| Tiny footprint | ~3 MB binary, distroless image, 0 runtime deps |
| Trustworthy releases | Cosign signing · SBOM · provenance · CodeQL · Trivy · OSSF Scorecard |
| Runs anywhere, privately | Self-hostable, air-gappable, no telemetry |
| Honest about status | ✅/📋 markers everywhere; benchmarks published, not asserted |

> Where we intend to lead — measurably, by v1.0: smallest footprint in class,
> sub-50 ms cold start, zero known CVEs, fully self-hostable, signed +
> SBOM-attested. These are targets we will benchmark and publish, not marketing.

## Roadmap

See **[ROADMAP.md](ROADMAP.md)** for the full phased plan. Near-term priorities:

**v0.2 — make the build honest & green:** commit `Cargo.lock` (unblocks `--locked`
in CI/Docker), fix the multi-arch Docker cross-linker, verify the downloaded
RavenFabric agent, decide/implement `--exec`, make swarm/supervisor fail loudly,
and expand Rust unit/integration coverage.

**v0.3 — a real agent:** the perceive→plan→act→observe loop, interactive REPL,
conversation memory, and streaming.

**v0.4 — tools & safety:** function-calling, built-in tools behind a deny-by-default
policy, sandboxing, and a tamper-evident audit log.

## License

RavenClaw uses a **dual-license model**:

- **AGPL-3.0-or-later** — open source core. Free for personal use, OSS projects, and commercial use up to 50 agents / $5M revenue.
- **Commercial** — for large commercial deployments or embedding without AGPL obligations.

See [LICENSING.md](LICENSING.md) for the full breakdown.

## Contributing

1. Fork the repo
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Commit changes (`git commit -am 'Add feature'`)
4. Push (`git push origin feature/my-feature`)
5. Open a Pull Request

All contributions require signing a Contributor License Agreement (CLA) — see [LICENSING.md](LICENSING.md#contributor-license-agreement-cla).

---

**RavenClaw** — Secure · Small · Efficient · Robust · Simple. Simply the best. 🐦‍⬛
