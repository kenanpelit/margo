#!/usr/bin/env bash
# panic-baseline-guard.sh — the panic baseline may only go up on purpose.
#
#   scripts/panic-baseline-guard.sh [BASE]
#
# Walks every commit in BASE..HEAD that changed scripts/panic-baseline.txt.
# A commit that RAISES the number must carry a trailer naming why:
#
#   PANIC-BASELINE-RAISE: startup-only asserts in the new parser
#
# Lowering it needs nothing. BASE defaults to $PANIC_RATCHET_BASE, then to the
# merge-base with origin/main; an unknown BASE (e.g. the all-zero "before" SHA
# of a new branch push) falls back to HEAD~1. Called by panic-ratchet.sh.
set -euo pipefail

file="scripts/panic-baseline.txt"
trailer="PANIC-BASELINE-RAISE"

base="${1:-${PANIC_RATCHET_BASE:-}}"
if [ -z "$base" ] && git rev-parse -q --verify origin/main >/dev/null; then
    base=$(git merge-base HEAD origin/main)
fi
if [ -z "$base" ] || ! git rev-parse -q --verify "$base^{commit}" >/dev/null 2>&1; then
    if git rev-parse -q --verify HEAD~1 >/dev/null; then
        base=HEAD~1
    else
        echo "panic-baseline-guard: no base commit to compare against; skipping."
        exit 0
    fi
fi

value_at() { # commit -> baseline number at that commit (empty if absent)
    git show "$1:$file" 2>/dev/null | tr -d '[:space:]' || true
}

bad=0
for c in $(git rev-list --reverse --no-merges "$base..HEAD" -- "$file"); do
    new=$(value_at "$c")
    old=$(value_at "$c^")
    [ -n "$new" ] && [ -n "$old" ] || continue
    if [ "$new" -gt "$old" ]; then
        reason=$(git log -1 --format="%(trailers:key=$trailer,valueonly)" "$c" | tr -d '[:space:]')
        subject=$(git log -1 --format='%h %s' "$c")
        if [ -z "$reason" ]; then
            echo "FAIL: $subject raises the panic baseline $old -> $new without a"
            echo "      '$trailer: <reason>' trailer. Convert the new call sites to"
            echo "      graceful handling, or add the trailer explaining why they are fine."
            bad=1
        else
            echo "note: $subject raises the panic baseline $old -> $new ($trailer given)."
        fi
    fi
done
exit "$bad"
