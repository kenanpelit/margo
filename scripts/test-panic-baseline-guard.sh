#!/usr/bin/env bash
# Tests for scripts/panic-baseline-guard.sh. Builds throwaway git repos in a
# temp dir; touches nothing in the real checkout.
#   ./scripts/test-panic-baseline-guard.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
guard="$here/panic-baseline-guard.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
check() { # name expected-exit actual-exit
    if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (expected exit $2, got $3)"; fail=1; fi
}

new_repo() {
    rm -rf "$tmp/r" && mkdir -p "$tmp/r/scripts" && cd "$tmp/r"
    git init -q -b main
    git config user.email t@t && git config user.name t
    echo 100 > scripts/panic-baseline.txt
    git add . && git commit -qm "base"
}
commit_baseline() { # value message
    echo "$1" > scripts/panic-baseline.txt
    git commit -qam "$2"
}
run_guard() { set +e; "$guard" "$1" >/dev/null 2>&1; echo $?; set -e; }

new_repo; base=$(git rev-parse HEAD)
commit_baseline 120 "chore: raise it"
check "raise without trailer fails" 1 "$(run_guard "$base")"

new_repo; base=$(git rev-parse HEAD)
commit_baseline 120 "chore: raise it

PANIC-BASELINE-RAISE: startup-only asserts in the new parser"
check "raise with trailer passes" 0 "$(run_guard "$base")"

new_repo; base=$(git rev-parse HEAD)
commit_baseline 120 "chore: raise it

PANIC-BASELINE-RAISE:"
check "raise with an empty reason fails" 1 "$(run_guard "$base")"

new_repo; base=$(git rev-parse HEAD)
commit_baseline 90 "fix: fewer unwraps"
check "lowering needs no trailer" 0 "$(run_guard "$base")"

new_repo; base=$(git rev-parse HEAD)
commit_baseline 90 "fix: fewer unwraps"
commit_baseline 95 "feat: sneak some back"
check "raise after a drop is still a raise" 1 "$(run_guard "$base")"

new_repo; base=$(git rev-parse HEAD)
echo x > other && git add other && git commit -qm "unrelated"
check "no baseline change passes" 0 "$(run_guard "$base")"

new_repo
check "unknown base falls back without failing" 0 "$(run_guard 0000000000000000000000000000000000000000)"

exit "$fail"
