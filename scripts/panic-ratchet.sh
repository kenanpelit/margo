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
#     baseline in the same PR and say why in the commit message.
#   * count < baseline  → CI fails too, with a friendly message: ratchet the
#     baseline DOWN to the new count so the cleanup is locked in.
#
# Exclusions (must stay in sync with how the baseline was measured):
#   * target/, .git/
#   * integration-test trees (any path containing /tests/)
#   * benchmark trees (any path containing /benches/) — dev-only, never
#     shipped; a panic in a bench can't take down the desktop
#   * build.rs (compile-time, panicking is fine)
#   * the braced item introduced by each `#[cfg(test)]` attribute (the in-file
#     unit-test module or a test-only fn), tracked by brace depth so production
#     code that follows the module — even mid-file — is still counted. A
#     `#[cfg(test)]` on a non-braced item (e.g. `use super::*;`) skips just that
#     statement.
set -euo pipefail
cd "$(dirname "$0")/.."

baseline_file="scripts/panic-baseline.txt"

count=$(find . -name '*.rs' \
    -not -path './target/*' \
    -not -path './.git/*' \
    -not -path '*/tests/*' \
    -not -path '*/benches/*' \
    -not -name 'build.rs' \
    -print0 | sort -z | xargs -0 awk '
      FNR == 1 { in_test = 0; depth = 0; arming = 0 }
      # Inside a #[cfg(test)] braced item: skip and track depth to its close.
      in_test {
        depth += gsub(/{/, "{") - gsub(/}/, "}")
        if (depth <= 0) in_test = 0
        next
      }
      # After a #[cfg(test)] attribute, wait for the items opening brace (a `{`)
      # or bail on a `;` (a non-braced test-only statement like `use super::*;`).
      arming {
        if (/{/) {
          depth = gsub(/{/, "{") - gsub(/}/, "}")
          arming = 0
          if (depth > 0) in_test = 1
          next
        }
        if (/;/) { arming = 0 }
        next
      }
      /#\[cfg\(test\)\]/ { arming = 1; next }
      /\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(|todo!\(|unimplemented!\(/ { n++ }
      END { print n + 0 }
    ')

baseline=$(tr -d '[:space:]' < "$baseline_file")

echo "panic-prone calls (non-test): $count  (baseline: $baseline)"

if [ "$count" -gt "$baseline" ]; then
    echo "FAIL: count rose above the baseline ($count > $baseline)."
    echo "Convert the new .unwrap()/.expect()/panic!() call sites to graceful"
    echo "handling, or raise $baseline_file in this PR with a rationale."
    exit 1
elif [ "$count" -lt "$baseline" ]; then
    echo "FAIL (the good kind): count dropped below the baseline ($count < $baseline)."
    echo "Lock the cleanup in: set $baseline_file to $count."
    exit 1
fi

echo "OK: at baseline."
