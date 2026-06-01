#!/bin/bash
# RavenClaw Build Script
# Builds optimized release binary and Docker image

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

echo "🐦‍⬛ RavenClaw Build"
echo "==================="

# Build Rust binary
echo "Building Rust binary..."
cargo build --release --locked

# Show binary size
BINARY_SIZE=$(du -h target/release/ravenclaw | cut -f1)
echo "✅ Binary size: $BINARY_SIZE"

# Build Docker image (optional)
if [[ "${BUILD_DOCKER:-false}" == "true" ]]; then
    echo "Building Docker image..."
    docker build -t ravenclaw:latest .
    echo "✅ Docker image built"
fi

# Run tests (optional)
if [[ "${RUN_TESTS:-false}" == "true" ]]; then
    echo "Running tests..."
    cargo test --locked
    echo "✅ Tests passed"
fi

echo "Build complete!"
