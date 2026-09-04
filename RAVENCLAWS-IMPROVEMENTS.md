# 🐦‍⬛ RavenClaws — Open Improvements

**Date:** 2026-09-04 *(re-verified — supersedes the 2026-08-13 pruned edition)*
**Upstream Version:** v1.7.1 — 1,261 tests (631 lib + 623 bin + 7 doc), 27 modules

> This document lists **only the genuinely open** improvement surface. Every
> previously-flagged item that has since shipped (audit mutex, Helm `appVersion` drift,
> cargo-udeps/outdated CI, `/reload` endpoint, threat model, web-access policy, K8s
> operator, memory store, connectors, cost tracking, `/metrics`, Windows CI, deterministic
> fuzzing, multi-modal input, WASM plugins, browser automation, advanced reasoning, durable
> execution, per-request `/chat` model override, `swarm/synthesize` endpoint, RavenFabric
> integration tests, documentation stats, and memory tiers) has been **removed** and is no
> longer tracked here.

---

## Executive Summary

RavenClaws is at **v1.7.1** — **1,261 tests** (631 lib + 623 bin + 7 doc), **27 modules**,
zero CVEs, ~5.2 MB binary, distroless non-root container. It delivers on all five pillars:
**Secure, Small, Efficient, Robust, Simple**.

The remaining improvement surface is almost entirely **ecosystem & strategic parity**:

| # | Gap | Category | Leverage |
|---|---|---|---|
| 1 | **OAuth connectors** (Google Drive, M365, Slack, GitHub, Notion) | Parity | ⭐⭐⭐ |
| 2 | **Enterprise tier** — RBAC, SSO/SAML, compliance presets | Commercial | ⭐⭐⭐ |
| 3 | **SDK ecosystem** (Python / TypeScript) | Ecosystem | ⭐⭐ |
| 4 | **RavenFabric `rf-*` binary features** | Strategic | ⭐⭐ |

---

## 🟡 Medium Priority — Open

### 1. OAuth connectors

`integrations.rs` covers outbound messaging only. **OAuth-based connectors** (Google Drive,
M365, Slack, GitHub, Notion) are not implemented.

**Recommendation:** add OAuth2 client flow + per-service adapters behind a feature gate.

### 2. Skill bundle concept (`skill.yaml`)

The WASM plugin ABI (v1.0.1) shipped, but the higher-level "skill bundle" concept — a
`skill.yaml` manifest + scripts + sandboxed execution — is not implemented.

**Recommendation:** layer `skill.yaml` over the WASM plugin ABI.

### 3. Formal fuzzing harness

`policy.rs` has deterministic fuzz-style tests (10k-input `PolicyEngine` and
`InjectionDetector`), but there is no `cargo fuzz` / libFuzzer harness and no property
tests for the config/TOML parsers.

**Recommendation:** add a `fuzz/` crate with libFuzzer targets for `config` and `policy`
parsers.

### 4. SSH in container (debugging)

Not implemented. Optional convenience for debugging running agents.

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

### Medium-term (1–3 months)
1. OAuth connector framework + Google Drive / GitHub (Medium #1)
2. `skill.yaml` bundle concept over the WASM plugin ABI (Medium #2)
3. `cargo fuzz` harness for `config` / `policy` parsers (Medium #3)

### Long-term (3–12 months)
4. Native Bedrock / Gemini / Vertex providers (Low #1)
5. Python / TypeScript SDKs (Low #2)
6. RavenFabric `rf-*` binary features + Terraform/Ansible (Low #3)
7. Enterprise tier: RBAC, SSO/SAML, compliance presets (Low #4)
8. SSH-in-container debugging (Medium #4)

---

*Document re-verified 2026-09-04 against upstream RavenClaws v1.7.1 (1,261 tests, 27 modules).*
