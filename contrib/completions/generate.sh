#!/usr/bin/env bash
#
# Regenerate the checked-in shell completions for mctl, mshellctl and mdots.
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

# write_all <bindir> <outdir> — the nine files, named as checked in.
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
        if ! diff -u "$here/$name" "$f" >"$tmp/$name.diff"; then
            echo "drift: contrib/completions/$name ($(grep -c '^[-+][^-+]' "$tmp/$name.diff") changed lines)"
            drift=1
        fi
    done
    if (( drift )); then
        echo "the checked-in completions are stale: run ./contrib/completions/generate.sh and commit"
        exit 1
    fi
    echo "completions: up to date"
    exit 0
fi

echo "building ${bins[*]} (release)…"
cargo build --release -p mctl -p mshellctl -p mdots
write_all "${CARGO_TARGET_DIR:-$root/target}/release" "$here"
echo "  ✓ ${bins[*]} — bash/zsh/fish"
echo "done — review & commit contrib/completions/"
