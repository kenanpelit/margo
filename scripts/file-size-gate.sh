#!/usr/bin/env bash
# file-size-gate.sh — keep god-files from silently regrowing.
#
#   scripts/file-size-gate.sh [BASE]
#
# scripts/file-size-limits.txt lists `<path> <max-lines>` pairs (blank lines
# and `#` comments ignored). The gate fails when a listed file has more lines
# than its limit. margo/src/state.rs is the reason this exists: it was split
# from ~6800 lines once, quietly grew back to ~3000, and was split again.
# The four biggest mshell files are gated too; new code belongs in the
# `state/*.rs`, `ipc/*.rs`, `settings/*.rs`, `schema/config/*.rs` or
# `frame/*.rs` module that owns its subsystem.
#
# Raising a limit is allowed, but only on purpose: every commit in BASE..HEAD
# that raises a number in the limits file must carry a trailer naming why,
#
#   FILE-SIZE-RAISE: state.rs needs the new Foo field group until #123 lands
#
# (same escape hatch as scripts/panic-baseline-guard.sh). Lowering a limit
# needs nothing. BASE defaults to $FILE_SIZE_GATE_BASE, then
# $PANIC_RATCHET_BASE, then the merge-base with origin/main; an unknown BASE
# (e.g. the all-zero "before" SHA of a new branch push) falls back to HEAD~1.
set -euo pipefail
cd "$(dirname "$0")/.."

limits="scripts/file-size-limits.txt"
trailer="FILE-SIZE-RAISE"

# Read a limits file on stdin; print its "<path> <limit>" lines.
parse() { sed -e 's/#.*//' | awk 'NF == 2 { print $1, $2 }'; }

bad=0
while read -r path limit; do
    if [ ! -f "$path" ]; then
        echo "FAIL: $path is listed in $limits but does not exist."
        bad=1
        continue
    fi
    n=$(wc -l < "$path")
    if [ "$n" -gt "$limit" ]; then
        echo "FAIL: $path has $n lines (limit $limit). Move code into the module"
        echo "      that owns it instead of growing this file; if it truly"
        echo "      belongs here, raise the limit in $limits in a commit"
        echo "      carrying a '$trailer: <reason>' trailer."
        bad=1
    else
        echo "ok   $path: $n / $limit lines"
    fi
done < <(parse < "$limits")

# Limit raises must be justified.
base="${1:-${FILE_SIZE_GATE_BASE:-${PANIC_RATCHET_BASE:-}}}"
if [ -z "$base" ] && git rev-parse -q --verify origin/main >/dev/null; then
    base=$(git merge-base HEAD origin/main)
fi
if [ -z "$base" ] || ! git rev-parse -q --verify "$base^{commit}" >/dev/null 2>&1; then
    if git rev-parse -q --verify HEAD~1 >/dev/null; then
        base=HEAD~1
    else
        exit "$bad"
    fi
fi

limit_in() { # commit path -> limit for path at commit (empty if absent)
    git show "$1:$limits" 2>/dev/null | parse | awk -v p="$2" '$1 == p { print $2 }' || true
}

for c in $(git rev-list --reverse --no-merges "$base..HEAD" -- "$limits"); do
    while read -r path new; do
        old=$(limit_in "$c^" "$path")
        [ -n "$old" ] || continue
        if [ "$new" -gt "$old" ]; then
            reason=$(git log -1 --format="%(trailers:key=$trailer,valueonly)" "$c" | tr -d '[:space:]')
            subject=$(git log -1 --format='%h %s' "$c")
            if [ -z "$reason" ]; then
                echo "FAIL: $subject raises the $path limit $old -> $new without a"
                echo "      '$trailer: <reason>' trailer."
                bad=1
            else
                echo "note: $subject raises the $path limit $old -> $new ($trailer given)."
            fi
        fi
    done < <(git show "$c:$limits" 2>/dev/null | parse)
done
exit "$bad"
