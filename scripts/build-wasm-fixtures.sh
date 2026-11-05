#!/usr/bin/env bash
# Build the WASM plugin host's guest fixtures for wasm32-wasip2:
#   mshell-crates/mshell-plugin-host/tests/fixtures/{hello,sdk,assistant}-guest
# The host's `loads_guest` tests load these `.wasm` files to drive the real
# host <-> guest contract (wit/world.wit). Each fixture is a standalone crate
# with its own committed Cargo.lock, so the build is `--locked`.
#
# The output goes to each fixture's own target/ (where the tests look) even
# when CARGO_TARGET_DIR is set.
#
# Needs the wasm target on the pinned toolchain:
#   rustup target add wasm32-wasip2
# Usage: ./scripts/build-wasm-fixtures.sh [repo-root]
set -euo pipefail
root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
fixtures="$root/mshell-crates/mshell-plugin-host/tests/fixtures"

for dir in "$fixtures"/*-guest; do
    echo "build-wasm-fixtures: $(basename "$dir")"
    cargo build --locked --release --target wasm32-wasip2 \
        --manifest-path "$dir/Cargo.toml" --target-dir "$dir/target"
done
