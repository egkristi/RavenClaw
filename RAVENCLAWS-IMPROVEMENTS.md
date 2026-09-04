# 🐦‍⬛ RavenClaws — Open Improvements

**Date:** 2026-09-04 *(re-verified — supersedes the 2026-08-13 pruned edition)*
**Upstream Version:** v1.7.1 — 1,239 tests (620 lib + 612 bin + 7 doc), 27 modules

> This document lists **only the genuinely open** improvement surface. Every
> previously-flagged item that has since shipped (audit mutex, Helm `appVersion` drift,
> cargo-udeps/outdated CI, `/reload` endpoint, threat model, web-access policy, K8s
> operator, memory store, connectors, cost tracking, `/metrics`, Windows CI, deterministic
> fuzzing, multi-modal input, WASM plugins, browser automation, advanced reasoning, durable
> execution) has been **removed** and is no longer tracked here.

---

## Executive Summary

RavenClaws is at **v1.7.1** — **1,239 tests** (620 lib + 612 bin + 7 doc), **27 modules**,
zero CVEs, ~5.2 MB binary, distroless non-root container. It delivers on all five pillars:
**Secure, Small, Efficient, Robust, Simple**.

The remaining improvement surface is almost entirely **ecosystem & strategic parity**:

| # | Gap | Category | Leverage |
|---|---|---|---|
| 1 | **OAuth connectors** (Google Drive, M365, Slack, GitHub, Notion) | Parity | ⭐⭐⭐ |
| 2 | **Memory tiers** — episodic / semantic (local embeddings) / procedural | Parity | ⭐⭐⭐ |
| 3 | **Enterprise tier** — RBAC, SSO/SAML, compliance presets | Commercial | ⭐⭐⭐ |
| 4 | **SDK ecosystem** (Python / TypeScript) | Ecosystem | ⭐⭐ |
| 5 | **RavenFabric `rf-*` binary features** | Strategic | ⭐⭐ |

---

## 🔴 High Priority — Open

### 1. No per-request `model` override on worker `/chat`

`server.rs handle_chat` accepts only `messages` / `stream` / `max_iterations`. The model is
fixed by the worker's own config, so an external orchestrator cannot push a per-task model
through the HTTP API. The `MultiModelManager::route_cheapest()` / `route_by_complexity()`
library APIs exist but are not exposed over HTTP.

**Recommendation:** add an optional `"model": "<name>"` field to the `/chat` request; if
present, select that LLM profile for the request.

### 2. No one-shot "team synthesis" HTTP endpoint

Swarm and supervisor modes exist, but there is no one-shot "fan out a prompt to N diverse
models and synthesize a consensus" HTTP endpoint (the `research-synthesize` mode is
CLI-only).

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
memory.

**Recommendation:** add local embeddings (no cloud dependency) and episodic/procedural
tiers on top of the existing SQLite store.

### 2. OAuth connectors

`integrations.rs` covers outbound messaging only. **OAuth-based connectors** (Google Drive,
M365, Slack, GitHub, Notion) are not implemented.

**Recommendation:** add OAuth2 client flow + per-service adapters behind a feature gate.

### 3. Skill bundle concept (`skill.yaml`)

The WASM plugin ABI (v1.0.1) shipped, but the higher-level "skill bundle" concept — a
`skill.yaml` manifest + scripts + sandboxed execution — is not implemented.

**Recommendation:** layer `skill.yaml` over the WASM plugin ABI.

### 4. Formal fuzzing harness

`policy.rs` has deterministic fuzz-style tests (10k-input `PolicyEngine` and
`InjectionDetector`), but there is no `cargo fuzz` / libFuzzer harness and no property
tests for the config/TOML parsers.

**Recommendation:** add a `fuzz/` crate with libFuzzer targets for `config` and `policy`
parsers.

### 5. SSH in container (debugging)

Not implemented. Optional convenience for debugging running agents.

### 6. Documentation stats sync

`AGENTS.md` ("547 tests, 25 modules"), `README.md` ("604 tests"), and
`website/public/index.html` ("452 tests, 18 modules") still reference stale figures versus
the actual **v1.7.1, 1,239 tests, 27 modules**. The `ROADMAP.md` competitive-parity table
may also be stale.

**Recommendation:** single-source test/module counts and update on each release.

---

## 🟢 Low Priority — Open

### 1. Native provider support

AWS **Bedrock**, Google **Gemini**, and **Vertex** are still reachable only via the LiteLLM
gateway — no native clients.

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

*Document re-verified 2026-09-04 against upstream RavenClaws v1.7.1 (1,239 tests, 27 modules).*
