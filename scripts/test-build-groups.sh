#!/usr/bin/env bash
# Tests for scripts/build-groups.sh. Works on a copy of the PKGBUILD in a temp
# dir; touches nothing in the real checkout.
#   ./scripts/test-build-groups.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
check() { # name expected actual
    if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (expected '$2', got '$3')"; fail=1; fi
}
fresh() { cp "$root/PKGBUILD" "$tmp/"; }
run() { set +e; "$here/build-groups.sh" "$@" >/dev/null 2>&1; echo $?; set -e; }

fresh
check "the checked-in PKGBUILD matches" 0 "$(run --check "$tmp")"

fresh; sed -i 's/ -p mcal -p mtm$/ -p mcal/' "$tmp/PKGBUILD"
check "a crate missing from a PKGBUILD group fails" 1 "$(run --check "$tmp")"

fresh; sed -i 's/ -p mcal -p mtm$/ -p mcal -p mtm -p mpicker/; s/-p mshell -p mshellctl -p mshellshare/-p mshell -p mshellctl -p mshellshare -p mtm/' "$tmp/PKGBUILD"
check "a crate moved to another PKGBUILD group fails" 1 "$(run --check "$tmp")"

check "an unknown group fails" 1 "$(run run nope true)"
check "run without a command is a usage error" 2 "$(run run mtune)"
check "each without a command is a usage error" 2 "$(run each)"
check "the names" "compositor shell mtune" "$("$here/build-groups.sh" names | paste -sd' ')"
# printf prints each argument on its own line: one word per argument proves
# the crate list reaches the command already split.
check "run passes -p <crate> as separate arguments" "x|-p|mtune" \
    "$("$here/build-groups.sh" run mtune printf '%s\n' x 2>/dev/null | paste -sd'|')"
check "each runs every group" 3 "$("$here/build-groups.sh" each true 2>&1 | grep -c '^build-groups: \[')"
check "each stops at the first failing group" 1 "$("$here/build-groups.sh" each false 2>&1 | grep -c '^build-groups: \[')"

exit "$fail"
