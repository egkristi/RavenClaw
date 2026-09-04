# 🐦‍⬛ RavenClaws — Improvement Recommendations (Pruned)

**Date:** 2026-08-13 *(re-pruned — supersedes the v1.3.0 audit edition)*
**Upstream Version:** v1.7.1 — 1,239 tests (620 lib + 612 bin + 7 doc), 27 modules

> **Prune note:** The previous edition of this document was written against upstream
> **v1.3.0** (552 tests, 25 modules) and repeated many recommendations that have since
> shipped. This edition removes every already-implemented recommendation and keeps only the
> **genuinely open** improvement surface, re-verified against the current `master` tree
> (v1.7.1, 27 modules, 1,239 passing tests).

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Already Implemented (Removed from Recommendations)](#already-implemented-removed-from-recommendations)
3. [🔴 High Priority — Open](#-high-priority--open)
4. [🟡 Medium Priority — Open](#-medium-priority--open)
5. [🟢 Low Priority — Open](#-low-priority--open)
6. [Prioritized Action Plan](#prioritized-action-plan)

---

## Executive Summary

RavenClaws is at **v1.7.1** — **1,239 tests** (620 lib + 612 bin + 7 doc), **27 modules**,
zero CVEs, ~5.2 MB binary, distroless non-root container. It delivers on all five pillars:
**Secure, Small, Efficient, Robust, Simple**.

The improvement surface has shifted almost entirely to **ecosystem & strategic parity**.
Nearly every "correctness / dead-code / hardening" item from earlier audits is now shipped
(see [§2](#already-implemented-removed-from-recommendations)). The remaining work is:

| # | Gap | Category | Leverage |
|---|---|---|---|
| 1 | **OAuth connectors** (Google Drive, M365, Slack, GitHub, Notion) | Parity | ⭐⭐⭐ |
| 2 | **Memory tiers** — episodic / semantic (local embeddings) / procedural | Parity (OpenClaw/Manus) | ⭐⭐⭐ |
| 3 | **Enterprise tier** — RBAC, SSO/SAML, compliance presets | Commercial | ⭐⭐⭐ |
| 4 | **SDK ecosystem** (Python / TypeScript) | Ecosystem | ⭐⭐ |
| 5 | **RavenFabric `rf-*` binary features** | Strategic | ⭐⭐ |

---

## Already Implemented (Removed from Recommendations)

These items were flagged in prior audits and are **now shipped**. They are listed here for
traceability only — no further action is required.

| Prior finding | Shipped in |
|---|---|
| Audit log mutex `unwrap()` | v0.9.3 — `lock_entries()` helper |
| ~60 `#[allow(dead_code)]` annotations | v0.9.8 — infrastructure wiring release |
| Helm `appVersion` drift (was `0.7.2`) | now `1.7.1` — matches `Cargo.toml` |
| cargo-udeps / cargo-outdated red CI | `continue-on-error: true` (informational) |
| Config hot-reload unusable in distroless | `POST /reload` HTTP endpoint in `server.rs` |
| Threat model + posture profiles | `SECURITY.md` — Threat Model + Posture Profiles |
| Domain-based web-access policy | `src/web_policy.rs` — category allow/block + `RateLimiter` |
| K8s operator / programmatic pod lifecycle | `src/k8s.rs` (feature `k8s`) |
| Memory store + session search + auto-title | `src/persistence.rs` — `MemoryStore`, `search_conversations`, `auto_title` |
| Connectors — outbound messaging | `src/integrations.rs` — Slack/Discord/Teams/Signal/Matrix/Telegram/Email/SMS |
| Cost tracking + cheapest routing | `src/llm.rs` — `CostTracker`, `cost_per_1k`, `route_cheapest` |
| `/metrics` endpoint | `GET /metrics` in `server.rs` |
| Windows CI targets | `build.yml` — `x86_64` + `aarch64` `pc-windows-msvc` |
| Fuzzing (deterministic) | `src/policy.rs` — two 10k-input deterministic fuzz tests |
| Multi-modal input / WASM plugins / browser automation / advanced reasoning / durable execution | v1.0.x–v1.3.0 |

---

## 🔴 High Priority — Open

### 1. No per-request `model` override on worker `/chat`

`server.rs handle_chat` accepts only `messages` / `stream` / `max_iterations`. The model is
fixed by the worker's own config, so an external orchestrator cannot push a per-task model
through the HTTP API. The `MultiModelManager::route_cheapest()` / `route_by_complexity()`
library APIs exist but are not exposed over HTTP.

**Recommendation:** add an optional `"model": "<name>"` field to the `/chat` request; if
present, select that LLM profile for the request. This makes RavenClaws a first-class
citizen for cost-aware multi-model fleets.

### 2. No one-shot "team synthesis" HTTP endpoint

Swarm and supervisor modes exist, but there is no one-shot "fan out a prompt to N diverse
models and synthesize a consensus" HTTP endpoint (the `research-synthesize` mode is CLI-only).

**Recommendation:** expose a `swarm/synthesize` endpoint taking a prompt + `n_agents` + an
optional model list and returning a single synthesized answer.

### 3. RavenFabricClient integration coverage

`src/ravenfabric.rs` has unit tests but no true integration tests (happy-path round-trip,
policy-deny, timeout). The eval harness covers tool calls and the agent loop, not mesh
communication.

**Recommendation:** add at least 3 integration tests: agent→relay→agent round-trip;
policy-denied command returns a structured error; unreachable relay returns a timeout error.

---

## 🟡 Medium Priority — Open

### 1. Memory tiers — episodic / semantic / procedural

`persistence.rs` provides `MemoryStore` (key-value + scoping) and conversation search, but
there are no **semantic embeddings**, no **episodic** recall, and no **procedural** skill
memory. This is the largest remaining parity gap with OpenClaw/Manus.

**Recommendation:** add local embeddings (no cloud dependency) and episodic/procedural
tiers on top of the existing SQLite store.

### 2. OAuth connectors

`integrations.rs` covers outbound messaging, but **OAuth-based connectors** (Google Drive,
M365, Slack, GitHub, Notion) are not implemented. This is a "table-stakes" parity gap.

**Recommendation:** add OAuth2 client flow + per-service adapters behind a feature gate.

### 3. Skill bundle concept (`skill.yaml`)

The WASM plugin ABI (v1.0.1) shipped, but the higher-level "skill bundle" concept — a
`skill.yaml` manifest + scripts + sandboxed execution — is not implemented.

**Recommendation:** layer `skill.yaml` over the WASM plugin ABI.

### 4. Formal fuzzing harness

`policy.rs` has deterministic fuzz tests, but there is no `cargo fuzz` / libFuzzer harness
and no property tests for the config/TOML parsers.

**Recommendation:** add a `fuzz/` crate with libFuzzer targets for `config` and `policy`
parsers.

### 5. SSH in container (debugging)

Not implemented. Optional convenience for debugging running agents.

### 6. Documentation stats sync

`AGENTS.md`, `README.md`, and `website/public/index.html` still reference stale figures
(e.g. "547 tests / 25 modules", "452 tests / 18 modules") versus the actual **v1.7.1,
1,239 tests, 27 modules**. The `ROADMAP.md` competitive-parity table may also be stale.

**Recommendation:** single-source test/module counts and update on each release.

---

## 🟢 Low Priority — Open

### 1. Native provider support

AWS **Bedrock**, Google **Gemini**, and **Vertex** are still reachable only via the LiteLLM
gateway — no native clients. Consider adding direct Bedrock support for AWS-native
deployments.

### 2. SDKs

No Python or TypeScript SDKs exist. The library crate (`ravenclaws` on crates.io) is
Rust-only.

### 3. RavenFabric `rf-*` binary features

The mesh client library exists, but the following operator-facing binary features remain
unimplemented: `rf-relay --metrics-listen`, structured policy validation, `--rate-limit` /
`--burst`, relay HA (`--peer`), `rf audit verify`, policy versioning/rollback, multi-agent
identity management, `rf cp` / `rf sync`, `rf shell`, `rf skill generate`, `rf-dashboard`,
Terraform provider, Ansible collection.

### 4. Enterprise tier (commercial)

- RBAC + multi-tenant isolation
- SSO / SAML
- Compliance presets & reporting (SOC2, ISO 27001, HIPAA, GDPR, PCI-DSS)
- Multi-level audit logging (off/basic/detailed/debug; JSON/CEF/LEEF/Syslog)
- Air-gap / offline licensing
- Output artifacts & reporting

### 5. Competitive gap — features competitors have

| Feature | RavenClaws | OpenClaw | Manus |
|---|---|---|---|
| OAuth connectors | ❌ | ❌ | ✅ |
| Native Telegram bot | ⚠️ outbound only | ✅ | ❌ |
| SSH in container | ❌ | ✅ | ❌ |

**RavenClaws advantages remain strong:** ~265x less memory and ~228x less CPU than
OpenClaw, distroless non-root container, edge-deployable on RPi5, no telemetry.

---

## Prioritized Action Plan

### Short-term (next 2–4 weeks)
1. Add per-request `model` override to `/chat` (High #1)
2. Add `swarm/synthesize` HTTP endpoint (High #2)
3. Add RavenFabricClient integration tests (High #3)
4. Sync stale test/module counts in `AGENTS.md` / `README` / website (Medium #6)

### Medium-term (1–3 months)
5. Memory tiers — episodic / semantic (local embeddings) / procedural (Medium #1)
6. OAuth connector framework + Google Drive / GitHub (Medium #2)
7. `skill.yaml` bundle concept over the WASM plugin ABI (Medium #3)
8. `cargo fuzz` harness for `config` / `policy` parsers (Medium #4)

### Long-term (3–12 months)
9. Native Bedrock / Gemini / Vertex providers (Low #1)
10. Python / TypeScript SDKs (Low #2)
11. RavenFabric `rf-*` binary features + Terraform/Ansible (Low #3)
12. Enterprise tier: RBAC, SSO/SAML, compliance presets (Low #4)
13. SSH-in-container debugging (Medium #5)

---

*Document re-pruned 2026-08-13 against upstream RavenClaws v1.7.1 (1,239 tests, 27 modules).*
