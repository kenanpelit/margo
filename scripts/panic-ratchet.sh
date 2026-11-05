#!/usr/bin/env bash
# panic-ratchet.sh — keep the panic-prone call count from growing.
#
# Counts `.unwrap()` / `.expect(` / `panic!(` / `unreachable!(` / `todo!(` /
# `unimplemented!(` in non-test Rust code and compares against the committed
# baseline (scripts/panic-baseline.txt).
# In a compositor a panic kills the whole desktop (and in mshell, the bar),
# so the count may only go DOWN over time:
#
#   * count > baseline  → CI fails. Convert the new call sites to graceful
#     handling (Result + log/degrade), or — when a new unwrap is genuinely
#     justified (provably infallible, startup-only assert) — raise the
#     baseline in the same PR, in a commit carrying a
#     `PANIC-BASELINE-RAISE: <reason>` trailer. scripts/panic-baseline-guard.sh
#     (run at the end of this script) fails any commit since the base
#     ($PANIC_RATCHET_BASE, else the merge-base with origin/main) that raises
#     the baseline without that trailer.
#   * count < baseline  → CI fails too, with a friendly message: ratchet the
#     baseline DOWN to the new count so the cleanup is locked in.
#
# Exclusions (must stay in sync with how the baseline was measured):
#   * target/, .git/
#   * integration-test trees (any path containing /tests/)
#   * benchmark trees (any path containing /benches/) — dev-only, never
#     shipped; a panic in a bench can't take down the desktop
#   * build.rs (compile-time, panicking is fine)
#   * the item introduced by each `#[cfg(test)]` attribute (the in-file
#     unit-test module or a test-only fn — scripts/lib/skip-cfg-test.awk), tracked by brace depth so production
#     code that follows the module — even mid-file — is still counted. A
#     `#[cfg(test)]` on a non-braced item (e.g. `use super::*;`) skips just that
#     statement.
#   * the body of each `#[test]` / `#[tokio::test]` fn. A test module can also
#     be gated at its *declaration* site (`#[cfg(test)] mod snapshot_tests;`),
#     in which case the separate file has no `#[cfg(test)]` of its own and its
#     test bodies were being counted as production code. Skipping test fns
#     covers that case wherever the gate lives.
set -euo pipefail
cd "$(dirname "$0")/.."

baseline_file="scripts/panic-baseline.txt"

# Whole-file test modules gated at their *declaration* site
# (`#[cfg(test)] mod render_tests;`). The file itself carries no
# `#[cfg(test)]`, so the per-file scan below can only recognise its `#[test]`
# fns — the scaffolding around them (fixtures, render helpers) would count as
# production code. Panicking is the correct behaviour in a test helper, so
# exclude these files wholesale, the same as any other test tree.
gated_test_files=$(find . -name '*.rs' \
    -not -path './target/*' \
    -not -path './.git/*' \
    -not -path './vendor/*' \
    -not -path './fuzz/*' \
    -not -path './mplugin-sdk/*' \
    -print0 | xargs -0 awk '
      /#\[cfg\(test\)\]/ { pending = 1; next }
      pending && /^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+[[:space:]]*;/ {
        name = $0
        sub(/^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]+/, "", name)
        sub(/[[:space:]]*;.*$/, "", name)
        dir = FILENAME
        sub(/\/[^\/]*$/, "", dir)
        print dir "/" name ".rs"
        pending = 0
        next
      }
      { pending = 0 }
    ')

rs_files=$(find . -name '*.rs' \
    -not -path './target/*' \
    -not -path './.git/*' \
    -not -path './vendor/*' \
    -not -path './fuzz/*' \
    -not -path './mplugin-sdk/*' \
    -not -path '*/tests/*' \
    -not -path '*/benches/*' \
    -not -name 'build.rs' \
    -print | sort)

if [ -n "$gated_test_files" ]; then
    rs_files=$(printf '%s\n' "$rs_files" | grep -vxF "$gated_test_files" || true)
fi

# The test-item skipping lives in scripts/lib/skip-cfg-test.awk, shared with
# design-lint.sh so both gates agree on what is test code.
skip_cfg_test=$(< scripts/lib/skip-cfg-test.awk)
count=$(printf '%s\n' "$rs_files" | tr '\n' '\0' | xargs -0 awk "$skip_cfg_test"'
      /\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(|todo!\(|unimplemented!\(/ { n += gsub(/\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(|todo!\(|unimplemented!\(/, "&") }
      END { print n + 0 }
    ')

baseline=$(< "$baseline_file")
# Canonical decimal, bounded so shell integer comparisons cannot overflow.
if [[ ! "$baseline" =~ ^(0|[1-9][0-9]{0,8})$ ]]; then
    echo "FAIL: $baseline_file must contain one decimal integer (0..999999999)."
    exit 1
fi

echo "panic-prone calls (non-test): $count  (baseline: $baseline)"

if [ "$count" -gt "$baseline" ]; then
    echo "FAIL: count rose above the baseline ($count > $baseline)."
    echo "Convert the new .unwrap()/.expect()/panic!() call sites to graceful"
    echo "handling, or raise $baseline_file in this PR in a commit with a"
    echo "'PANIC-BASELINE-RAISE: <reason>' trailer."
    exit 1
elif [ "$count" -lt "$baseline" ]; then
    echo "FAIL (the good kind): count dropped below the baseline ($count < $baseline)."
    echo "Lock the cleanup in: set $baseline_file to $count."
    exit 1
fi

echo "OK: at baseline."

# The number only goes up on purpose: every raise needs a reason trailer.
./scripts/panic-baseline-guard.sh
