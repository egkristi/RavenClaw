# Changelog

All notable changes to RavenClaw will be documented in this file.

## [Unreleased]

### Added
- `--exec` mode now fully wired — one-shot command execution with response printed to stdout
- Comprehensive Rust unit tests: 111 tests across all modules (was 3)
- `serial_test` crate for serializing env-dependent tests to prevent env var leakage
- `Config::load()` now safely handles `RAVENCLAW__LLMS` env var by saving/restoring it around serde deserialization
- Manual `Default` implementations for `RavenFabricConfig`, `SecurityConfig`, and `RuntimeConfig` matching serde defaults
- CLI `--version` now uses `env!("CARGO_PKG_VERSION")` instead of hardcoded string
- Test coverage for config validation, LLM client creation, error types, CLI argument parsing, and agent stubs
- 15 new `mockito`-based HTTP tests covering all 4 LLM providers (LiteLLM, OpenAI, OpenRouter, Ollama) with success, auth failure (401), rate limit (429), server error (500), and invalid JSON response paths
- 8 new config edge case tests: TLS disabled, TLS with CA, TLS with cert+key, multi-provider config, custom LiteLLM config, custom Ollama config, custom OpenAI config, custom OpenRouter config
- 4 new agent tests: multi-model stubs, `--exec` error propagation, agent type check
- 4 new error tests: async network error, IO error, debug formatting, Send+Sync trait bounds
- RavenFabric agent SHA256 checksum verification in Dockerfile
- Cross-compilation linkers (`gcc-aarch64-linux-gnu`, `gcc-x86_64-linux-gnu`) in Docker build stage
- Cargo target linker configuration for multi-arch Docker builds

### Fixed
- `--exec` dead code — CLI arg was parsed but never used; now sends prompt to LLM and prints response
- Swarm/supervisor stubs now return `Err(RavenClawError::CommandExecution(...))` instead of silently exiting 0
- All 4 LLM client constructors (`LiteLLMClient`, `OpenRouterClient`, `OllamaClient`, `OpenAIClient`) now return `Result<Self, LLMError>` instead of calling `.expect()`
- `create_client()` factory function propagates client construction errors via `?`
- Verification `check_llm_response_quality` now handles `--exec` mode output (stdout-based responses)
- `Cargo.lock` removed from `.gitignore` and committed for reproducible builds
- OpenRouter and OpenAI clients now respect `config.endpoint` when non-empty, falling back to hardcoded defaults (enables mockito testing)
- Docker multi-arch build: cross-linkers installed and cargo target linker configured per-platform

### Changed
- `RavenFabricConfig`, `SecurityConfig`, `RuntimeConfig` now use manual `Default` impls instead of `#[derive(Default)]` to ensure serde defaults match Rust defaults
