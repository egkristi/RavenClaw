# RavenClaw — Multi-stage build for minimal production image
# Stage 1: Builder
FROM rust:1.82-slim-bookworm AS builder

WORKDIR /app

# Install dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock* ./
COPY src/ ./src/

# Build optimized release
RUN cargo build --release --locked

# Stage 2: Runtime (minimal)
FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /app

# Copy binary from builder
COPY --from=builder --chown=nonroot:nonroot /app/target/release/ravenclaw .

# Security: run as non-root, read-only filesystem
USER nonroot
READONLY_FILESYSTEM

# Environment variables (set via K8s/Docker)
ENV RAVENCLAW_CONFIG=/config/ravenclaw.toml
ENV RUST_LOG=info

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/app/ravenclaw", "--version"]

ENTRYPOINT ["/app/ravenclaw"]
CMD ["--mode", "single"]
