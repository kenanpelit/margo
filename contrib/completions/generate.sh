#!/usr/bin/env bash
#
# Regenerate the checked-in shell completions for mctl + mshellctl.
#
# The completion scripts in this directory are produced by clap
# (`<bin> completions <shell>`), so they cover EVERY subcommand, flag,
# and value-enum automatically and can never drift from the CLI. Run this
# after adding/renaming any subcommand or argument, then commit the result.
#
#   ./contrib/completions/generate.sh      # or: just completions
#
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$root"

echo "building mctl + mshellctl (release)…"
cargo build --release -p mctl -p mshellctl

for bin in mctl mshellctl; do
    exe="target/release/$bin"
    "$exe" completions bash >"$here/$bin.bash"
    "$exe" completions zsh  >"$here/_$bin"
    "$exe" completions fish >"$here/$bin.fish"
    echo "  ✓ $bin — bash/zsh/fish"
done

echo "done — review & commit contrib/completions/"
