#!/usr/bin/env bash
# Everything that can be verified without a Mac.
set -euo pipefail
cd "$(dirname "$0")/.."
rustup target add aarch64-apple-darwin >/dev/null
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Type-checks the macOS code (pane-macos, the mac platform, FFI) without linking.
cargo check --workspace --target aarch64-apple-darwin
cargo clippy --workspace --target aarch64-apple-darwin -- -D warnings
