#!/usr/bin/env bash
# Exercise PKGBUILD check() with a failing cargo stub: failure must stop
# packaging, and tests must use the normal test profile, not release/LTO.
# Usage: ./scripts/test-install-pkgbuild.sh [PKGBUILD]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
pkgbuild="${1:-$root/PKGBUILD}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/src/margo"
printf '#!/bin/sh\nprintf "%%s\\n" "$*" > "%s/calls"\nexit 37\n' "$tmp" > "$tmp/bin/cargo"
chmod +x "$tmp/bin/cargo"
set +e
(
  PATH="$tmp/bin:$PATH"
  export srcdir="$tmp/src"
  # shellcheck disable=SC1090
  . "$pkgbuild"
  check
) >"$tmp/log" 2>&1
status=$?
set -e
fail=0
if [[ $status == 37 ]]; then
  echo 'ok   check() propagates cargo test failure'
else
  echo "FAIL check() hides cargo test failure (exit $status, expected 37)"
  fail=1
fi
if [[ -f "$tmp/calls" ]] && ! grep -qE -- '--release|--profile' "$tmp/calls"; then
  echo 'ok   check() uses the normal test profile'
else
  echo 'FAIL check() rebuilds release/LTO test binaries'
  fail=1
fi
if [[ -f "$tmp/calls" ]] && grep -q -- '--frozen' "$tmp/calls"; then
  echo 'ok   check() is offline and locked'
else
  echo 'FAIL check() is not --frozen'
  fail=1
fi
exit "$fail"
