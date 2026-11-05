#!/usr/bin/env bash
# The external programs the code runs must reach the package. Gate:
#   1. every program the Rust code names in Command::new("x"),
#      command_exists("x"), which("x"), on_path("x"), find_in_path("x"), …
#      has a row in scripts/runtime-tools.txt (tool -> Arch package);
#   2. every package in that table is in the PKGBUILD's depends or optdepends;
#   3. no row names a program the code no longer mentions (stale row).
# Programs reached another way (a list of argv strings, a D-Bus service, a
# plugin file) can't be found by (1); add their row by hand.
# Usage: ./scripts/optdepends-drift.sh [repo-root]   (exit 1 on drift)
set -euo pipefail
root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
table="$root/scripts/runtime-tools.txt"
fail=0

sources() { find "$root" \( -name target -o -name .git \) -prune -o -name '*.rs' -print0; }

# (1) program names in the code: the call forms that take a program name
# (absolute paths are left out: they name a file, not a package).
# One grep over every file's text: `xargs grep` runs grep per batch of
# files, and a batch with no match exits 1, which xargs turns into 123 and
# pipefail into a failed gate (it hit as soon as the file count moved a
# batch boundary). A tree with no match at all is fine too.
text() { sources | xargs -0 cat --; }
code_tools="$(text | { grep -ohE \
    '\b(Command::new|command_exists|which_exists|which|on_path|is_command_available|find_on_path|find_in_path)\(\s*"[A-Za-z0-9][A-Za-z0-9_.+-]*"' || true; } |
    sed -E 's/.*"([^"]+)"$/\1/' | sort -u)"
# Every string literal, for the stale-row check.
literals="$(text | { grep -ohE '"[A-Za-z0-9][A-Za-z0-9_.+-]*"' || true; } | tr -d '"' | sort -u)"

rows="$(grep -vE '^[[:space:]]*(#|$)' "$table" | awk '{print $1, $2}')"
table_tools="$(awk '$1 != "-" {print $1}' <<<"$rows" | sort -u)"
table_pkgs="$(awk '$2 != "-" {print $2}' <<<"$rows" | sort -u)"

missing="$(comm -23 <(echo "$code_tools") <(echo "$table_tools"))"
if [ -n "$missing" ]; then
    echo "optdepends-drift: programs the code runs with no row in scripts/runtime-tools.txt:"
    sed 's/^/  /' <<<"$missing"
    echo "  add '<tool> <arch-package> <feature>' (and the package to PKGBUILD optdepends),"
    echo "  or '<tool> - <why it is not a packaging matter>'"
    fail=1
fi

stale="$(comm -23 <(echo "$table_tools") <(echo "$literals"))"
if [ -n "$stale" ]; then
    echo "optdepends-drift: rows for programs the code no longer mentions:"
    sed 's/^/  /' <<<"$stale"
    fail=1
fi

# (2) the PKGBUILD's depends + optdepends package names.
# shellcheck disable=SC2154 # depends / optdepends are set by the sourced PKGBUILD
pkg_names="$(
    # shellcheck disable=SC1091
    cd "$root" && . ./PKGBUILD >/dev/null 2>&1
    for d in "${depends[@]}"; do printf '%s\n' "${d%%[<>=]*}"; done
    for o in "${optdepends[@]}"; do printf '%s\n' "${o%%:*}"; done
)"
undeclared="$(comm -23 <(echo "$table_pkgs") <(sort -u <<<"$pkg_names"))"
if [ -n "$undeclared" ]; then
    echo "optdepends-drift: packages scripts/runtime-tools.txt needs but the PKGBUILD doesn't declare:"
    sed 's/^/  /' <<<"$undeclared"
    fail=1
fi

if [ "$fail" = 0 ]; then
    echo "optdepends-drift: $(wc -l <<<"$code_tools") programs, all packaged or accounted for"
fi
exit "$fail"
