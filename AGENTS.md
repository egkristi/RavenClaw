# 🐦‍⬛ RavenClaw — AI Agent Instructions

This file contains structured instructions for AI coding agents working on the RavenClaw codebase. It defines the project's architecture, conventions, common tasks, and guardrails.

---

## Project Overview

RavenClaw is a **lightweight, secure Rust agent framework** with multi-provider LLM support. It runs as a single binary with zero runtime dependencies.

- **Language:** Rust (edition 2021)
- **Version:** 0.1.0 (pre-release / developer preview)
- **License:** AGPL-3.0-or-later + Commercial
- **Repository:** https://github.com/egkristi/RavenClaw
- **Build:** `cargo build --release` (~3MB stripped binary, ~7ms startup)

### Architecture (5 modules)

```
src/
├── main.rs      — CLI entry point (clap), config loading, mode dispatch
├── agent.rs     — Agent implementations (single, swarm stubs, supervisor stubs, REPL, ConversationMemory)
├── llm.rs       — LLM provider abstraction (trait + 4 clients + multi-model manager + streaming)
├── config.rs    — Config structs, TOML/env loading, validation
└── error.rs     — Unified error types
```

### Current State

| Feature | Status |
|---|---|
| Single agent mode | ✅ Working — sends prompt, logs response |
| Multi-provider (LiteLLM, OpenAI, OpenRouter, Ollama) | ✅ Working |
| Multi-model manager | ✅ Working — iterates all configured providers, round-robin routing |
| CLI with env-var overrides | ✅ Working |
| OpenAI-compatible API support | ✅ Working — any `/v1/chat/completions` endpoint |
| Container security (non-root, read-only FS, dropped caps) | ✅ Working |
| Verification suite (157 tests, 5 modules, 0 warnings) | ✅ Working |
| `--exec` mode | ✅ Working — one-shot command execution with response to stdout |
| Streaming responses | ✅ Working — SSE streaming for LiteLLM, default fallback for others |
| Conversation memory | ✅ Working — `ConversationMemory` struct with configurable max history |
| Interactive REPL | ✅ Working — `--repl` flag with stdin loop, streaming output |
| System prompt / persona | ✅ Working — `LLMConfig.system_prompt`, CLI `--system-prompt`, env var |
| Swarm mode | ❌ Stub — returns error "not yet implemented" |
| Supervisor mode | ❌ Stub — returns error "not yet implemented" |
| Tool-use / function calling | ❌ Not implemented |
| Agent loop / ReAct planning | ❌ Not implemented — one-shot send-and-exit |
| RavenFabric integration | Partial — config struct exists, binary included in container, runtime wiring pending |
| GitHub Actions CI/CD | ✅ Implemented — fmt + clippy + test, 5-target builds, multi-arch images, Cosign + SBOM + provenance + Trivy, crates.io publish, releases |
| Security scanning | ✅ Implemented — CodeQL, cargo-audit, cargo-deny, cargo-outdated, cargo-udeps, Trivy (FS + config), Hadolint, Kubescape, OSSF Scorecard, dependency review |
| Pre-built binaries / releases | 📋 Wired, untagged — CI produces them on tag; none released yet |

---

## Documentation Conventions

### CHANGELOG.md — Tracking Implemented Features & Fixes

All implemented features, resolved issues, and completed changes **must** be documented in `CHANGELOG.md` with the following format:

```markdown
## [Unreleased]

### Added
- New feature description (#issue-number)

### Fixed
- Bug fix description (#issue-number)

### Changed
- Modification to existing feature (#issue-number)

### Removed
- Feature or code removal (#issue-number)
```

**When to update:** Every time a PR is merged or a feature branch is completed. Never commit a feature without a corresponding CHANGELOG entry.

### ISSUES.md — Tracking Known Problems

All identified bugs, technical debt, and known limitations **must** be documented in `ISSUES.md` with severity labels:

```markdown
## Critical
- **`--exec` mode is dead code** — CLI arg parsed but never used in `main.rs`. [No issue]

## High
- **Swarm/Supervisor stubs exit silently** — `--mode swarm` and `--mode supervisor` warn "not yet implemented" and exit 0 instead of returning an error. [No issue]

## Medium
- **Only 2 Rust unit tests** — `cargo test` runs in <1s and tests almost nothing. [No issue]
```

**Severity levels:**
- **Critical** — Blocks a release or causes incorrect behavior
- **High** — Significant missing functionality or risk
- **Medium** — Important but non-blocking
- **Low** — Nice-to-have improvements

### ROADMAP.md — Tracking Planned Features

All planned features and feature requests **must** be documented as checklists in `ROADMAP.md` with priority labels:

```markdown
### Priority: High
- [ ] **Tool-use (function calling)** — The #1 missing piece. Agent must call tools, not just chat.

### Priority: Medium
- [ ] **Streaming responses** — Real-time token-by-token output for interactive use
```

**Priority levels:**
- **High** — Required for next release or core functionality
- **Medium** — Important but can wait for a later release
- **Low** — Nice-to-have, no fixed timeline

**When a feature is completed:** Move it from ROADMAP.md to CHANGELOG.md under the appropriate `[Unreleased]` section header (`### Added`, `### Fixed`, etc.). Never leave a completed feature in ROADMAP.md — it must be migrated to CHANGELOG.md to keep both documents accurate.

### Test Coverage Mandate

**All critical functionality MUST have automated tests.** Specifically:

- **Every new feature** must include both:
  1. Rust unit tests in the relevant `src/*.rs` module (via `#[cfg(test)] mod tests`)
  2. Shell verification tests in `scripts/lib/` (for integration-level coverage)
- **Every bug fix** must include a test that reproduces the bug and verifies the fix
- **Every config field** must have a test that exercises it
- **Every LLM provider** must have a test in `test-llm-quality.sh`
- **Every deployment target** must have tests in the corresponding `test-*.sh` module

If a feature cannot be tested (e.g., hardware-dependent), document the reason in the test file.

---

## Code Conventions

### Rust Style

- **Formatting:** Standard `rustfmt` (no custom config)
- **Linting:** Standard `clippy`
- **Naming:** Snake_case for functions/variables, CamelCase for types/enums, SCREAMING_SNAKE_CASE for constants
- **Error handling:** `thiserror` for library errors, `anyhow` for application-level errors
- **Async:** `tokio` runtime with `async-trait` for provider abstraction
- **Logging:** `tracing` with JSON format — use `info!`, `warn!`, `error!` consistently
- **Unimplemented features:** Use `warn!("...not yet implemented")` + return `Ok(())` — do not panic or exit with error

### Module Responsibilities

| Module | Owns | Does NOT own |
|---|---|---|
| `main.rs` | CLI parsing, config loading, mode dispatch | Agent logic, LLM calls, config structs |
| `agent.rs` | Agent run functions (single, swarm, supervisor) | LLM client creation, config parsing |
| `llm.rs` | `LLMProviderTrait`, client implementations, `MultiModelManager` | Agent logic, config structs |
| `config.rs` | `Config`, `LLMConfig`, validation, env loading | Agent logic, HTTP requests |
| `error.rs` | `RavenClawError` enum, `Result<T>` alias | Everything else |

### Adding a New LLM Provider

1. Add a variant to `LLMProvider` enum in `config.rs`
2. Create a client struct in `llm.rs` implementing `LLMProviderTrait`
3. Add the client to the `create_client()` factory function
4. Add the provider mapping in `main.rs` CLI override section
5. Add test config in `tests/config/`
6. Add verification tests in `scripts/lib/test-llm-quality.sh`
7. Add CHANGELOG.md entry under "Added"
8. Mark the corresponding ROADMAP.md checklist item as complete

---

## Verification System

The verification suite lives in `scripts/` and is **separate from `cargo test`** (which only has 2 unit tests).

### Structure

```
scripts/
├── verify.sh              — Main orchestrator
└── lib/
    ├── common.sh          — Shared library (colors, paths, logging, test runner)
    ├── test-litellm.sh    — LiteLLM connectivity (4 tests)
    ├── test-local.sh      — macOS binary (12 tests)
    ├── test-docker.sh     — Docker container (10 tests)
    ├── test-linux.sh      — Linux cross-compile (6 tests)
    ├── test-k8s.sh        — Kubernetes (13 tests)
    ├── test-security.sh   — Binary integrity (8 tests)
    ├── test-performance.sh— Benchmarks (5 benchmarks)
    └── test-llm-quality.sh— LLM response quality (36 tests)
```

### Running Tests

```bash
./scripts/verify.sh --all          # Full suite (94 tests)
./scripts/verify.sh --quick        # Smoke test (litellm + local + security)
./scripts/verify.sh --litellm      # Single module
./scripts/verify.sh --build        # Build + all tests
```

### Test Runner Functions

- `run_test "name" command args...` — Runs command, captures output to `target/verification-results/`, logs PASS/FAIL
- `run_test_verbose "name" command args...` — Same but shows full output on failure
- `check_llm_response_quality log_file model_name` — Checks that LLM response is non-empty

### Common Pitfalls

- **Test names with `/`** — Creates subdirectories in results. Use spaces or hyphens instead.
- **`bash -c` quoting** — Use escaped double quotes `\"` inside `bash -c` strings. Avoid nested single quotes.
- **macOS `file` command** — Doesn't say "stripped" for stripped binaries. Check for "Mach-O" instead.
- **Distroless containers** — No shell, no `cat`, no `id`. Use `docker image inspect` for user checks.
- **K8s `runAsNonRoot`** — Requires numeric UID. Use `runAsUser: 65532` instead.
- **ConfigMap jsonpath** — Keys with dots (e.g., `ravenclaw.toml`) are unreliable. Use `go-template='{{index .data "key"}}'`.

---

## Deployment Targets

| Target | Binary Location | How to Build |
|---|---|---|
| macOS (aarch64) | `target/release/ravenclaw` | `cargo build --release` |
| macOS (x86_64) | `target/x86_64-apple-darwin/release/ravenclaw` | `cargo build --release --target x86_64-apple-darwin` |
| Linux ARM64 | `target/aarch64-unknown-linux-gnu/release/ravenclaw` | Cross-compile or Docker build |
| Linux x86_64 | `target/x86_64-unknown-linux-gnu/release/ravenclaw` | Cross-compile or Docker build |
| Docker | `ghcr.io/egkristi/ravenclaw:latest` | `docker build -t ravenclaw:latest .` |
| Kubernetes | `k8s/deployment.yaml` | `kubectl apply -f k8s/deployment.yaml` |

### Docker

- Multi-stage build: `rust:1.86-slim-bookworm` → `gcr.io/distroless/cc-debian12:nonroot`
- User: `nonroot` (UID 65532)
- No shell, no package manager, minimal attack surface
- HEALTHCHECK runs `--version`

### Kubernetes

- Production: `k8s/deployment.yaml` — in-cluster LiteLLM, full RBAC, Secrets
- Testing: `k8s/deployment-test.yaml` — hostNetwork for local LiteLLM, no Secrets

---

## Common Tasks

### Fixing `--exec` Mode

The `--exec` CLI flag is parsed in `main.rs` but never used. To fix:
1. In `main.rs`, after config loading, check `if let Some(prompt) = args.exec`
2. Pass the prompt as the user message content instead of the hardcoded "Ready. Awaiting instructions."
3. The agent should send the exec prompt and print the response to stdout (not just log it)

### Adding a New Test Module

1. Create `scripts/lib/test-myfeature.sh` with a function `test_myfeature()`
2. Source `common.sh` at the top
3. Add to `MODULES` array in `scripts/verify.sh`: `"myfeature:test-myfeature.sh:test_myfeature:My Feature Description"`
4. Use `run_test` and `check_llm_response_quality` from common.sh

### Adding a New Config Field

1. Add field to the appropriate struct in `config.rs`
2. Add a `#[serde(default = "default_fn")]` attribute with a default function
3. Add validation in `Config::validate()` if needed
4. Add env var mapping in `Config::load()` if needed
5. Update test configs in `tests/config/`

---

## Guardrails

### Do NOT

- **Do not** add Python, Node.js, or other runtime dependencies — the binary must be self-contained
- **Do not** hardcode API keys, tokens, or credentials anywhere in the codebase
- **Do not** remove the `strip = true` or `panic = "abort"` from release profile
- **Do not** use `unsafe` code unless absolutely necessary and documented
- **Do not** add large dependencies (>100KB) without evaluating alternatives
- **Do not** change the distroless base image without security review
- **Do not** remove the `readOnlyRootFilesystem: true` or `capabilities.drop: ["ALL"]` from K8s manifests
- **Do not** make swarm/supervisor modes exit with error — keep the stub pattern until implemented

### Do

- **Do** use `tracing` for all logging (not `println!` or `eprintln!`)
- **Do** add tests for every new feature (both Rust unit tests and shell verification tests)
- **Do** update CHANGELOG.md for every implemented feature, fix, or change
- **Do** update ISSUES.md when discovering new bugs or limitations
- **Do** update ROADMAP.md when starting or completing roadmap items
- **Do** update VERIFICATION.md when adding or changing verification tests
- **Do** update README.md when adding user-facing features
- **Do** keep the binary under 5MB — if it grows, investigate alternatives
- **Do** use env vars for all secrets — never config files

---

## Release Process

When all TODOs and features for a version milestone are completed, GitHub Actions for the final commit are green, all tests pass, test coverage is good, and everything else is ready — follow this release process.

### Phase 1: Pre-Release Checks

Before bumping the version, verify everything is clean:

```bash
# 1. Full Rust test suite — all 149+ tests must pass
cargo test --locked 2>&1

# 2. Clippy must be clean (no warnings, no errors)
cargo clippy --locked --all-targets -- -D warnings 2>&1

# 3. Formatting must be clean
cargo fmt --check 2>&1

# 4. Full verification suite (build + all test modules)
./scripts/verify.sh --build 2>&1

# 5. Check test coverage is adequate (no untested critical paths)
#    - Every config field has a test
#    - Every LLM provider has a test
#    - Every error variant has a test
#    - Every CLI argument has a test
#    - Every deployment target has verification tests
```

If any of these fail, **do not proceed** — fix the issue first.

### Phase 2: Version Bump

Update the version in `Cargo.toml`:

```bash
# For a patch release (bug fixes only):
sed -i '' 's/^version = "0\.1\.0"/version = "0.1.1"/' Cargo.toml

# For a minor release (new features, backward-compatible):
sed -i '' 's/^version = "0\.1\.0"/version = "0.2.0"/' Cargo.toml

# For a major release (breaking changes):
sed -i '' 's/^version = "0\.1\.0"/version = "1.0.0"/' Cargo.toml
```

Update the version reference in `AGENTS.md` itself (Project Overview section) and any other docs that reference the version.

### Phase 3: Update Changelog

Move all `[Unreleased]` entries into a new `[vX.Y.Z]` section:

```markdown
## [v0.1.0] - 2026-06-02

### Added
- (all added features from Unreleased)

### Fixed
- (all fixes from Unreleased)

### Changed
- (all changes from Unreleased)
```

Then clear the `[Unreleased]` section headers (keep the structure, remove old entries).

### Phase 4: Commit & Tag

```bash
# Commit the version bump and changelog update
git add -A
git commit -m "Release v0.1.0"

# Tag the release (signed tag preferred)
git tag -s v0.1.0 -m "RavenClaw v0.1.0"

# Push everything (triggers CI/CD pipelines)
git push --follow-tags
```

Pushing the tag triggers the following GitHub Actions workflows:

| Workflow | Trigger | What it does |
|---|---|---|
| `build.yml` | Tag push `v*` | `check` (fmt+clippy+test) → `build-binaries` (5 targets) → `containers` (multi-arch, multi-registry, sign, SBOM, Trivy) → `publish-cratesio` → `release` (GitHub Release with assets) |
| `container.yml` | Tag push `v*` | Builds + pushes multi-arch containers, signs with Cosign, generates attestation, Trivy scan, SBOM |
| `security-scan.yml` | Tag push `v*` | CodeQL, cargo-audit, cargo-deny, cargo-outdated, cargo-udeps, OSSF Scorecard |

### Phase 5: Monitor CI/CD

After pushing the tag, **monitor all GitHub Actions workflows to completion**:

1. Go to https://github.com/egkristi/RavenClaw/actions
2. Verify the `check` job passes (fmt + clippy + test)
3. Verify all 5 `build-binaries` matrix jobs succeed:
   - `x86_64-unknown-linux-gnu`
   - `aarch64-unknown-linux-gnu`
   - `x86_64-unknown-linux-musl`
   - `x86_64-apple-darwin`
   - `aarch64-apple-darwin`
4. Verify `containers` job builds and pushes multi-arch images to both GHCR and Docker Hub
5. Verify Cosign signing completes
6. Verify artifact attestation is generated
7. Verify Trivy vulnerability scan passes (exit code 0)
8. Verify SBOM is generated and uploaded
9. Verify `publish-cratesio` publishes to crates.io
10. Verify `release` job creates the GitHub Release with all binary assets attached

If **any job fails**, investigate and fix before proceeding. Do not create a new tag until the issue is resolved.

### Phase 6: Post-Release Binary & Container Verification

After all CI/CD pipelines pass, run a full battery of tests against the **newly released** binaries and container images:

```bash
# ── 6a. Pull and verify the released container image ──────────
docker pull ghcr.io/egkristi/ravenclaw:v0.1.0
docker run --rm ghcr.io/egkristi/ravenclaw:v0.1.0 --version

# ── 6b. Run full verification suite against the release ───────
# (This tests local binary, Docker container, K8s manifests, etc.)
./scripts/verify.sh --build

# ── 6c. Verify binary integrity ───────────────────────────────
# Download a released binary and check its SHA256
# (Replace URL with actual release asset URL)
curl -sL https://github.com/egkristi/RavenClaw/releases/download/v0.1.0/ravenclaw-x86_64-apple-darwin.tar.gz \
  -o /tmp/ravenclaw-release.tar.gz
tar xzf /tmp/ravenclaw-release.tar.gz -C /tmp/
./scripts/verify.sh --security

# ── 6d. Verify container security properties ──────────────────
docker image inspect ghcr.io/egkristi/ravenclaw:v0.1.0 --format '{{.Config.User}}'
# Must output "65532" (nonroot user)

# ── 6e. Verify multi-arch support ─────────────────────────────
docker buildx imagetools inspect ghcr.io/egkristi/ravenclaw:v0.1.0
# Must show both linux/amd64 and linux/arm64 manifests

# ── 6f. Verify container signature ────────────────────────────
cosign verify ghcr.io/egkristi/ravenclaw:v0.1.0 \
  --certificate-identity-regexp "https://github.com/egkristi/RavenClaw" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com"

# ── 6g. Verify SBOM exists ────────────────────────────────────
# Download SBOM from GitHub Release assets or regenerate:
syft ghcr.io/egkristi/ravenclaw:v0.1.0 -o spdx-json=sbom-verify.spdx.json
```

### Phase 7: Final Validation

```bash
# ── 7a. Verify crates.io package ──────────────────────────────
cargo search ravenclaw
# Should show the new version

# ── 7b. Verify GitHub Release ─────────────────────────────────
# Check https://github.com/egkristi/RavenClaw/releases
# Must have:
#   - Release notes (auto-generated from changelog)
#   - 5 binary archives (.tar.gz/.zip) with SHA256 checksums
#   - SBOM artifact
#   - Container image signatures

# ── 7c. Verify the binary works end-to-end ────────────────────
/tmp/ravenclaw --version
/tmp/ravenclaw --help
echo "Hello" | /tmp/ravenclaw --exec "Say hello back" 2>&1 || true
```

### Release Abort Criteria

If any of the following occur during the release process, **abort immediately** and do not create/push a tag:

1. Any Rust test fails (`cargo test --locked`)
2. Clippy or fmt check fails
3. Any verification suite module fails (`./scripts/verify.sh --all`)
4. Any CI job in the build workflow fails
5. Trivy scan finds CRITICAL or HIGH vulnerabilities (exit code 1)
6. Cosign signing fails
7. Container image does not run as nonroot user (UID 65532)
8. Multi-arch manifest is missing either linux/amd64 or linux/arm64
9. Binary SHA256 checksums do not match after download
10. GitHub Release is missing required assets

**If aborting:** Delete the tag locally and remotely, fix the issue, then restart from Phase 1:

```bash
git tag -d v0.1.0
git push --delete origin v0.1.0
```

### Release Checklist Template

Copy this into a new issue or comment when starting a release:

```markdown
## Release vX.Y.Z Checklist

### Phase 1: Pre-Release
- [ ] `cargo test --locked` — all tests pass
- [ ] `cargo clippy --locked --all-targets -- -D warnings` — clean
- [ ] `cargo fmt --check` — clean
- [ ] `./scripts/verify.sh --build` — full suite passes
- [ ] Test coverage is adequate

### Phase 2: Version Bump
- [ ] Version updated in `Cargo.toml`
- [ ] Version references updated in docs

### Phase 3: Changelog
- [ ] `[Unreleased]` entries moved to `[vX.Y.Z]` section
- [ ] Release date added

### Phase 4: Commit & Tag
- [ ] Commit pushed: `git push`
- [ ] Tag pushed: `git push --follow-tags`

### Phase 5: CI/CD Monitoring
- [ ] `check` job passes
- [ ] All 5 `build-binaries` jobs succeed
- [ ] `containers` job succeeds (multi-arch, multi-registry)
- [ ] Cosign signing completes
- [ ] Artifact attestation generated
- [ ] Trivy scan passes (no CRITICAL/HIGH)
- [ ] SBOM generated and uploaded
- [ ] `publish-cratesio` succeeds
- [ ] GitHub Release created with all assets

### Phase 6: Post-Release Verification
- [ ] Container image pulls and runs
- [ ] `./scripts/verify.sh --build` passes against release
- [ ] Binary SHA256 checksums verified
- [ ] Container runs as nonroot (UID 65532)
- [ ] Multi-arch manifest verified (amd64 + arm64)
- [ ] Cosign signature verified
- [ ] SBOM verified

### Phase 7: Final Validation
- [ ] crates.io package published
- [ ] GitHub Release has all assets
- [ ] Binary works end-to-end
```

---

*RavenClaw — Small. Sleek. Secure. Supreme.* 🐦‍⬛
