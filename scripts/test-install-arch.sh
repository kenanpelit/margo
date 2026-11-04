#!/usr/bin/env bash
# Tests for install.sh's Arch path, with makepkg/git stubbed on PATH. The
# incremental run must still run the PKGBUILD's prepare() (cargo fetch
# --locked + the rust-toolchain.toml install): build() is `--frozen`, so a
# Cargo.lock that gained a crate fails offline unless prepare() fetched it.
#   ./scripts/test-install-arch.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
check() { # name condition-exit
    if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi
}

mkdir -p "$tmp/bin"
for tool in makepkg git; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\n' "$tool" "$tmp" > "$tmp/bin/$tool"
    chmod +x "$tmp/bin/$tool"
done
# install.sh minus its trailing `main "$@"`, next to the files it reads.
sed '$d' "$root/install.sh" > "$tmp/install.sh"
cp "$root/Cargo.toml" "$root/PKGBUILD" "$tmp/"

run_arch_install() { # build-dir
    : > "$tmp/calls"
    ( PATH="$tmp/bin:$PATH" MARGO_BUILD_DIR="$1"
      # shellcheck disable=SC1091
      . "$tmp/install.sh"; arch_install ) >/dev/null 2>&1 || true
}
makepkg_runs_prepare() { # no -e / --noextract / --noprepare on the makepkg call
    grep -q '^makepkg ' "$tmp/calls" &&
        ! grep -E '^makepkg ' "$tmp/calls" | grep -qE -- ' -[a-zA-Z]*e|--noextract|--noprepare'
}

run_arch_install "$tmp/fresh"
if makepkg_runs_prepare; then r=0; else r=1; fi; check "first build runs prepare()" "$r"

mkdir -p "$tmp/incr/margo" "$tmp/incr/src/margo"
run_arch_install "$tmp/incr"
if makepkg_runs_prepare; then r=0; else r=1; fi; check "incremental build runs prepare()" "$r"

exit "$fail"
