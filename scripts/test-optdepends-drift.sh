#!/usr/bin/env bash
# Tests for scripts/optdepends-drift.sh. Works on copies of the PKGBUILD, the
# tool table and the Rust sources in a temp dir; touches nothing in the
# real checkout.
#   ./scripts/test-optdepends-drift.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
check() { # name expected-exit actual-exit
    if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (expected exit $2, got $3)"; fail=1; fi
}
fresh() {
    rm -rf "$tmp/r"
    mkdir -p "$tmp/r/scripts"
    cp "$root/PKGBUILD" "$tmp/r/"
    cp "$root/scripts/runtime-tools.txt" "$tmp/r/scripts/"
    # The same tree the gate scans (find, not git: CI's container git may
    # refuse the checkout, which left cp without a single file).
    (cd "$root" && find . \( -name target -o -name .git \) -prune -o -name '*.rs' -print0 |
        xargs -0 cp --parents -t "$tmp/r")
}
run() { set +e; "$here/optdepends-drift.sh" "$tmp/r" >/dev/null 2>&1; echo $?; set -e; }

fresh
check "the checked-in tree passes" 0 "$(run)"

fresh; printf 'fn f() { let _ = std::process::Command::new("brand-new-tool"); }\n' > "$tmp/r/new.rs"
check "a program the table doesn't know fails" 1 "$(run)"

fresh; printf 'async fn f() { let _ = which("another-tool").await; }\n' > "$tmp/r/new.rs"
check "a which() probe the table doesn't know fails" 1 "$(run)"

fresh; sed -i '/^  "tesseract: /d' "$tmp/r/PKGBUILD"
check "a table package missing from optdepends fails" 1 "$(run)"

fresh; printf 'retired-tool  -  gone\n' >> "$tmp/r/scripts/runtime-tools.txt"
check "a row for a program the code no longer names fails" 1 "$(run)"

fresh; printf 'fn f() { let _ = std::process::Command::new("/usr/bin/env"); }\n' > "$tmp/r/new.rs"
check "an absolute path is not a program name" 0 "$(run)"

# Enough files that the scan spans several grep batches, some of them
# with no program name at all: that is no drift.
fresh; mkdir -p "$tmp/r/pad"
for i in $(seq 1 5000); do printf '// nothing here\n' >"$tmp/r/pad/f$i.rs"; done
check "batches of files that name no program still pass" 0 "$(run)"

exit "$fail"
