# Known Issues

This document tracks known problems in RavenClaw that are not yet resolved.
Items are ordered by severity/impact.

---

## 🚨 Critical

### k8s Deployment enters CrashLoopBackOff

**Problem:** The binary exits after processing one request, but the k8s Deployment
(`k8s/deployment.yaml`) expects a long-running process. The pod immediately enters
`CrashLoopBackOff`.

**Root cause:** RavenClaw currently has no server/daemon mode. It processes a single
request and exits. A persistent server mode is planned for v0.7.

**Workaround:** None yet. The k8s manifest cannot be used until server mode exists.

**Tracking:** ROADMAP.md v0.7 — Async / long-horizon background runs.

---

## 🔧 Build & CI

### Container Build workflow may fail at "Set up job"

**Problem:** The `Container Images` job in `.github/workflows/build.yml` may fail
intermittently at the "Set up job" step with an infrastructure error.

**Root cause:** GitHub Actions runner provisioning issue — not a code defect.
The workflow uses `docker/build-push-action@v6` with multi-arch (`linux/amd64`,
`linux/arm64`) and requires QEMU + Buildx setup.

**Frequency:** Intermittent — depends on GitHub Actions runner availability.

**Workaround:** Re-run the workflow manually. If it persists, investigate GitHub
Actions runner availability for the repository.

**Status:** ✅ Dockerfile now has cross-linkers (`gcc-aarch64-linux-gnu`,
`gcc-x86_64-linux-gnu`) and SHA256 checksum verification for RavenFabric agent.
CI `build.yml` now installs `musl-tools` and `gcc-aarch64-linux-gnu` for
cross-compilation targets.

### Security Scan workflow may fail

**Problem:** The `Security Scan` workflow (`.github/workflows/security-scan.yml`)
may fail due to:
- `cargo-outdated` exit code 1 when dependencies are outdated (non-blocking, informational)
- `cargo-udeps` detecting unused dependencies (non-blocking, informational)
- Trivy scanner finding MEDIUM severity issues in dependencies
- Kubescape threshold violations on K8s manifests

**Root cause:** These are informational scans configured with `continue-on-error: true`
or lenient thresholds. Failures are expected for some scans and do not block the pipeline.

**Status:** All scans are configured. CodeQL, cargo-audit, cargo-deny, Hadolint,
and OSSF Scorecard are blocking. Trivy and Kubescape may produce findings that
need periodic review.

---

## 🧪 Code Quality

### ~~`next_client()` round-robin method never called~~ ✅ Fixed

**Problem:** `MultiModelManager::next_client()` in `src/llm.rs` implements
round-robin load balancing across providers, but was never invoked anywhere in
the codebase.

**Fix:** Changed return type to `Option`, removed `#[allow(dead_code)]`, wired
into `run_single_multi()` in agent.rs. Added 2 new tests.

### ~~`handle_response()` code duplicated across providers~~ ✅ Fixed

**Problem:** The `handle_response()` method in each LLM client contained nearly
identical JSON parsing logic.

**Fix:** Extracted shared `handle_openai_response()` async function. Replaced
duplicated code in LiteLLM, OpenRouter, and OpenAI clients. Ollama kept its own
handler (different API format).

### Dead code: unused enum variants and struct fields

Several enum variants and struct fields are annotated with `#[allow(dead_code)]`
because they are defined for future use or serde deserialization but not yet
consumed:

- `ConfigError::MissingEnvVar` — defined but never constructed
- `RavenClawError::RavenFabric` / `RavenClawError::SecurityViolation` — future use
- `LLMError::ProviderNotSupported` — defined but never constructed
- Various serde-deserialized fields in `ChatResponse`, `Choice`, `Usage`
- `RavenFabricConfig` fields (`agent_id`, `remote_exec`, `allowed_hosts`)
- `SecurityConfig` fields (`token_lifetime_secs`, `audit_log`)
- `RuntimeConfig` fields (`workdir`, `max_agents`, `health_interval_secs`)

These should be cleaned up as features are implemented.

---

## ✅ Resolved Issues

### Linux cross-compilation builds fail (RESOLVED)

**Fix:** CI `build.yml` now installs `musl-tools` and `gcc-aarch64-linux-gnu`
before building cross-compilation targets. Dockerfile has cross-linkers configured
for multi-arch builds. SHA256 checksum verification added for RavenFabric agent download.

### ROADMAP.md v0.2 exit criteria (RESOLVED)

All v0.2 items are complete:
- ✅ `Cargo.lock` committed, `--locked` works everywhere
- ✅ Multi-arch Docker build fixed (cross-linkers installed)
- ✅ RavenFabric agent download verified with SHA256 checksum
- ✅ `--version` wired to `CARGO_PKG_VERSION`
- ✅ `.expect()` on HTTP client replaced with error propagation
- ✅ `--exec` one-shot mode implemented
- ✅ Swarm/supervisor stubs return clear errors
- ✅ Tests expanded to 149 across all modules with `mockito`
- ✅ `cargo fmt && cargo clippy -D warnings && cargo test` all green

---

## 🔮 Future Considerations

### No graceful shutdown / signal handling

The binary does not handle SIGTERM/SIGINT. When running in interactive mode,
Ctrl+C will abort immediately without cleanup.

### No configuration hot-reload

Changes to `ravenclaw.toml` require a restart. No file-watch mechanism exists.
