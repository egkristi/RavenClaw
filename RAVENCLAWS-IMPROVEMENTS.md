# 🐦‍⬛ RavenClaws — Open Improvements

**Date:** 2026-09-04 *(re-verified — supersedes the 2026-08-13 pruned edition)*
**Upstream Version:** v1.7.1 — 1,281 tests (649 lib + 625 bin + 7 doc), 28 modules

> This document lists **only the genuinely open** improvement surface. Every
> previously-flagged item that has since shipped (audit mutex, Helm `appVersion` drift,
> cargo-udeps/outdated CI, `/reload` endpoint, threat model, web-access policy, K8s
> operator, memory store, connectors, cost tracking, `/metrics`, Windows CI, deterministic
> fuzzing, multi-modal input, WASM plugins, browser automation, advanced reasoning, durable
> execution, per-request `/chat` model override, `swarm/synthesize` endpoint, RavenFabric
> integration tests, documentation stats, memory tiers, skill bundles, and config-parser
> fuzzing) has been **removed** and is no longer tracked here.

---

## Executive Summary

RavenClaws is at **v1.7.1** — **1,281 tests** (649 lib + 625 bin + 7 doc), **28 modules**,
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

### 2. SSH in container (debugging)

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

### Long-term (3–12 months)
2. Native Bedrock / Gemini / Vertex providers (Low #1)
3. Python / TypeScript SDKs (Low #2)
4. RavenFabric `rf-*` binary features + Terraform/Ansible (Low #3)
5. Enterprise tier: RBAC, SSO/SAML, compliance presets (Low #4)
6. SSH-in-container debugging (Medium #2)

---

*Document re-verified 2026-09-04 against upstream RavenClaws v1.7.1 (1,281 tests, 28 modules).*
