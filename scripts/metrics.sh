#!/usr/bin/env bash
# metrics.sh — print the project metrics that docs otherwise hand-carry.
#
# road_map.md and CLAUDE.md quote a Rust line count, a crate count, a panic
# baseline and a test count. Hand-maintained numbers drift the moment anyone
# touches the tree; this script is the single source of truth so those figures
# can be regenerated instead of guessed.
#
# Output is a plain `key\tvalue` table (stable keys), so it can be diffed,
# grepped, or spliced into a doc by a later tool. No dependencies beyond cargo
# and the coreutils already assumed by the other scripts here. For the
# per-crate table, see scripts/crate-table.sh.
#
# Exclusions match panic-ratchet.sh: target/, .git/, integration-test trees,
# benchmark trees, and generated dirs.
set -euo pipefail
cd "$(dirname "$0")/.."

find_rs() {
    find . -name '*.rs' \
        -not -path './target/*' \
        -not -path './.git/*' \
        -not -path '*/tests/*' \
        -not -path '*/benches/*' \
        -not -name 'build.rs' \
        -print0
}

# Rust source lines (production, matching the panic-ratchet corpus).
rust_loc=$(find_rs | xargs -0 cat | wc -l | tr -d '[:space:]')

# Workspace member crates, as cargo sees them. Counting every Cargo.toml with
# a [package] section over-counts: it also finds the excluded mplugin-sdk, the
# plugin-host test fixtures and the vendored wayle crates. The member IDs carry
# no quotes, so counting quoted strings in the array is enough (no jq needed).
crates=$(cargo metadata --no-deps --format-version 1 --offline 2>/dev/null \
    | grep -o '"workspace_members":\[[^]]*\]' \
    | grep -o '"[^"]*"' | tail -n +2 | wc -l | tr -d '[:space:]')

# Unit tests: #[test] + #[tokio::test] attributes across the tree.
tests=$(find_rs | xargs -0 grep -hcE '^\s*#\[(test|tokio::test)\]' 2>/dev/null \
    | awk '{s+=$1} END {print s+0}')

# Unsafe surface: `unsafe {` blocks + `unsafe fn` definitions (production).
unsafe_count=$(find_rs | xargs -0 grep -hoE 'unsafe (\{|fn )' 2>/dev/null | wc -l | tr -d '[:space:]')

# Panic-prone call baseline (the committed ratchet floor).
panic_baseline=$(tr -d '[:space:]' < scripts/panic-baseline.txt)

printf '%s\t%s\n' \
    rust_loc         "$rust_loc" \
    member_crates    "$crates" \
    unit_tests       "$tests" \
    unsafe_sites     "$unsafe_count" \
    panic_baseline   "$panic_baseline"
