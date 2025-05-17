#!/bin/bash

set -e

# Build
cargo build -p hyperliquid_rust_sdk

# Check formatting
cargo fmt -- --check

# Run Clippy
cargo clippy -p hyperliquid_rust_sdk -- -D warnings

# Run tests
cargo test -p hyperliquid_rust_sdk

echo "CI checks passed successfully."