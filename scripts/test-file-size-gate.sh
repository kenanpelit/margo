#!/usr/bin/env bash
# Tests for scripts/file-size-gate.sh. Builds throwaway git repos in a temp
# dir (with a copy of the gate); touches nothing in the real checkout.
#   ./scripts/test-file-size-gate.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
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
    cp "$here/file-size-gate.sh" "$here/file-size-gate.py" scripts/
    seq 10 > big.rs
    echo "big.rs 12" > scripts/file-size-limits.txt
    git add . && git commit -qm "base"
}
set_limit() { # value message
    echo "big.rs $1" > scripts/file-size-limits.txt
    git commit -qam "$2"
}
run_gate() { set +e; ./scripts/file-size-gate.sh "$1" >/dev/null 2>&1; echo $?; set -e; }

new_repo; base=$(git rev-parse HEAD)
check "under the limit passes" 0 "$(run_gate "$base")"

seq 13 > big.rs && git commit -qam "grow"
check "over the limit fails" 1 "$(run_gate "$base")"

set_limit 20 "chore: raise it"
check "raise without trailer fails" 1 "$(run_gate "$base")"

new_repo; base=$(git rev-parse HEAD)
seq 13 > big.rs
echo "big.rs 20" > scripts/file-size-limits.txt
git commit -qam "grow" -m "FILE-SIZE-RAISE: needed for the test"
check "raise with trailer passes" 0 "$(run_gate "$base")"

new_repo; base=$(git rev-parse HEAD)
set_limit 11 "chore: lower it"
check "lowering needs no trailer" 0 "$(run_gate "$base")"

new_repo; base=$(git rev-parse HEAD)
echo "missing.rs 5" >> scripts/file-size-limits.txt && git commit -qam "list a ghost"
check "listed file that does not exist fails" 1 "$(run_gate "$base")"

exit "$fail"
