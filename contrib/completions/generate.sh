#!/usr/bin/env bash
#
# Regenerate the checked-in shell completions for mctl, mshellctl and mdots,
# and man/mdots.1 (`mdots man`, clap_mangen output).
#
# The completion scripts in this directory are produced by clap
# (`mctl|mshellctl completions <shell>`, `mdots completion <shell>`), so they
# cover EVERY subcommand, flag, and value-enum. Run this after adding/renaming
# any subcommand or argument, then commit the result; CI runs `--check` and
# fails when they drift.
#
#   ./contrib/completions/generate.sh                 # build (release) + rewrite; or: just completions
#   ./contrib/completions/generate.sh --check [DIR]   # fail if the checked-in files drift
#
# --check uses already-built binaries from DIR (default target/debug; CI
# passes its build dir) and only diffs, so it needs no extra build.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$root"

bins=(mctl mshellctl mdots)

# write_all <bindir> <outdir> — the nine completion files, named as checked
# in, plus mdots.1. (man/mctl.1 and man/mshellctl.1 are hand-written; their
# drift tests live in mctl/src/man_page_tests.rs and mshellctl/tests/.)
write_all() {
    local bindir="$1" out="$2" bin sub exe
    for bin in "${bins[@]}"; do
        exe="$bindir/$bin"
        [[ -x "$exe" ]] || { echo "missing $exe (build it first)" >&2; return 1; }
        sub=completions
        [[ "$bin" == mdots ]] && sub=completion
        "$exe" "$sub" bash >"$out/$bin.bash"
        "$exe" "$sub" zsh  >"$out/_$bin"
        "$exe" "$sub" fish >"$out/$bin.fish"
    done
    "$bindir/mdots" man >"$out/mdots.1"
}

# checked_in <name> — where a generated file lives in the tree.
checked_in() {
    if [[ "$1" == *.1 ]]; then echo "$root/man/$1"; else echo "$here/$1"; fi
}

if [[ "${1:-}" == "--check" ]]; then
    bindir="${2:-${CARGO_TARGET_DIR:-$root/target}/debug}"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    mkdir "$tmp/out"
    write_all "$bindir" "$tmp/out"
    drift=0
    for f in "$tmp"/out/*; do
        name="$(basename "$f")"
        dest="$(checked_in "$name")"
        if ! diff -u "$dest" "$f" >"$tmp/$name.diff"; then
            echo "drift: ${dest#"$root"/} ($(grep -c '^[-+][^-+]' "$tmp/$name.diff") changed lines)"
            drift=1
        fi
    done
    if (( drift )); then
        echo "the checked-in completions / man/mdots.1 are stale: run ./contrib/completions/generate.sh and commit"
        exit 1
    fi
    echo "completions + man/mdots.1: up to date"
    exit 0
fi

echo "building ${bins[*]} (release)…"
cargo build --release -p mctl -p mshellctl -p mdots
write_all "${CARGO_TARGET_DIR:-$root/target}/release" "$here"
mv "$here/mdots.1" "$root/man/mdots.1"
echo "  ✓ ${bins[*]} — bash/zsh/fish, man/mdots.1"
echo "done — review & commit contrib/completions/ and man/"
