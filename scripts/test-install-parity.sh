#!/usr/bin/env bash
# Tests for scripts/install-parity.sh. Works on copies of PKGBUILD and
# install.sh in a temp dir; touches nothing in the real checkout.
#   ./scripts/test-install-parity.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
check() { # name expected-exit actual-exit
    if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (expected exit $2, got $3)"; fail=1; fi
}
fresh() { cp "$root/PKGBUILD" "$root/install.sh" "$tmp/"; }
run() { set +e; "$here/install-parity.sh" "$tmp" >/dev/null 2>&1; echo $?; set -e; }

fresh
check "the checked-in pair matches" 0 "$(run)"

fresh; sed -i 's/ mcal mtm mtune; do/ mcal mtune; do/' "$tmp/install.sh"
check "a binary missing from install.sh fails" 1 "$(run)"

fresh; sed -i '/mshell.service" \\$/,+1d' "$tmp/install.sh"
check "a user unit missing from install.sh fails" 1 "$(run)"

fresh; sed -i 's/ -p mcal -p mtm$/ -p mcal/' "$tmp/install.sh"
check "a crate missing from a build group fails" 1 "$(run)"

fresh; sed -i 's/mvpn,mcal,mtm,mtune}/mvpn,mcal,mtune}/' "$tmp/install.sh"
check "a binary missing from the uninstall fallback fails" 1 "$(run)"

fresh; sed -i 's#"\$pkgdir/usr/share/pixmaps/margo.svg"#"$pkgdir/usr/share/pixmaps/margo2.svg"#' "$tmp/PKGBUILD"
check "a path new in the PKGBUILD fails" 1 "$(run)"

exit "$fail"
