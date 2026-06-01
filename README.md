# RavenClaw 🐦‍⬛

**Lightweight, secure Rust agent framework with LiteLLM + RavenFabric**

Built for efficiency, security, and easy deployment.

## Features

- ⚡ **Fast** — Native Rust, optimized for performance
- 🔒 **Secure by default** — No credentials in config, TLS required, minimal permissions
- 🚀 **LiteLLM integration** — Unified API for 100+ LLM providers
- 🕸️ **RavenFabric ready** — Swarm coordination and remote command execution
- 📦 **Easy deployment** — Binary, Docker, Kubernetes (Helm ready)
- 🎯 **Minimal footprint** — Distroless container, <20MB binary

## Quick Start

### Binary

```bash
# Download release
curl -LO https://github.com/egkristi/RavenClaw/releases/latest/download/ravenclaw
chmod +x ravenclaw

# Run with environment variables
export LITELLM_API_KEY="your-key"
export RAVENCLAW__LLM__ENDPOINT="http://localhost:4000"
./ravenclaw --mode single
```

### Docker

```bash
docker run --rm -it \
  -e LITELLM_API_KEY="your-key" \
  -e RAVENCLAW__LLM__ENDPOINT="http://litellm:4000" \
  ghcr.io/egkristi/ravenclaw:latest
```

### Docker Compose (Development)

```bash
# Start RavenClaw + LiteLLM
docker-compose up -d

# View logs
docker-compose logs -f ravenclaw
```

### Kubernetes

```bash
# Deploy to cluster
kubectl apply -f k8s/deployment.yaml

# Check status
kubectl -n ravenclaw get pods
kubectl -n ravenclaw logs -l app.kubernetes.io/name=ravenclaw
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|---|---|---|
| `LITELLM_API_KEY` | LiteLLM API key | (required) |
| `RAVENCLAW__LLM__ENDPOINT` | LiteLLM endpoint | (required) |
| `RAVENCLAW__LLM__MODEL` | Default model | `gpt-4o-mini` |
| `RAVENCLAW__RAVENFABRIC__ENDPOINT` | RavenFabric endpoint | - |
| `RAVENCLAW__SECURITY__REQUIRE_TLS` | Enforce TLS | `true` |
| `RAVENCLAW__RUNTIME__MAX_AGENTS` | Max concurrent agents | `10` |
| `RUST_LOG` | Log level | `info` |

### Config File (TOML)

```toml
[llm]
endpoint = "http://litellm:4000"
model = "gpt-4o-mini"
timeout_secs = 30

[ravenfabric]
endpoint = "http://ravenfabric:8080"
remote_exec = true
allowed_hosts = ["litellm", "ravenfabric"]

[security]
require_tls = true
token_lifetime_secs = 3600
audit_log = true

[runtime]
workdir = "/workspace"
max_agents = 10
health_interval_secs = 60
```

## Agent Modes

| Mode | Description |
|---|---|
| `single` | Standalone autonomous agent |
| `swarm` | Multiple coordinated agents |
| `supervisor` | Orchestrator for sub-agents |

## Security

- ✅ No credentials in config files (use env vars or K8s Secrets)
- ✅ TLS required for production endpoints
- ✅ Read-only root filesystem (container)
- ✅ Non-root user (container)
- ✅ Dropped capabilities (container)
- ✅ Audit logging enabled by default
- ✅ Token lifetime limits

## Building from Source

```bash
# Clone
git clone https://github.com/egkristi/RavenClaw
cd RavenClaw

# Build release
cargo build --release

# Run tests
cargo test

# Build Docker image
docker build -t ravenclaw:latest .
```

## Architecture

```
┌─────────────────┐     ┌─────────────────┐
│   RavenClaw     │────▶│   LiteLLM       │
│   Agent         │     │   (LLM Proxy)   │
└────────┬────────┘     └─────────────────┘
         │
         │ RavenFabric
         ▼
┌─────────────────┐
│   Swarm         │
│   Coordination  │
└─────────────────┘
```

## Roadmap

- [ ] RavenFabric integration (remote exec)
- [ ] Swarm mode implementation
- [ ] Supervisor mode
- [ ] Helm chart
- [ ] Prometheus metrics
- [ ] OpenTelemetry tracing
- [ ] Plugin system

## License

MIT — See [LICENSE](LICENSE)

## Contributing

1. Fork the repo
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Commit changes (`git commit -am 'Add feature'`)
4. Push (`git push origin feature/my-feature`)
5. Open a Pull Request

---

**RavenClaw** — Secure, efficient, fast, lightweight. 🐦‍⬛
