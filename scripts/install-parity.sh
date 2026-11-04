#!/usr/bin/env bash
# install.sh's Debian path must ship what the PKGBUILD ships. The two are
# kept by hand, so this gate diffs them:
#   1. the cargo build groups (`-p` sets per invocation),
#   2. the binaries installed to /usr/bin (and the no-manifest uninstall list),
#   3. every installed destination path (units, wants links, assets, docs).
# Usage: ./scripts/install-parity.sh [repo-root]   (exit 1 on drift)
set -euo pipefail
root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
pkgbuild="$root/PKGBUILD"
installsh="$root/install.sh"
fail=0

# Paths only the PKGBUILD has, on purpose. The mlogind retire helper runs from
# pacman's post_upgrade scriptlet; dpkg has no such hook, so install.sh runs it
# straight from the repo at install time instead of shipping a copy.
pkgbuild_only=(
    usr/lib/mlogind/retire-old-sync-variables.sh
    usr/lib/mlogind/variables.toml.default
)

# body <file> <function> — one shell function, comments dropped and
# backslash-continued lines joined.
body() {
    awk -v f="$2" '$0 ~ "^"f"\\(\\) *\\{" {on=1} on {print} on && /^}/ {exit}' "$1" |
        grep -v '^[[:space:]]*#' | sed -e ':a' -e '/\\$/N' -e 's/\\\n/ /' -e 'ta'
}

# groups — one line per `cargo build`, its sorted -p list.
groups() {
    grep -E '^[[:space:]]*cargo build' | while read -r cmd; do
        grep -oE -- '-p [A-Za-z0-9_-]+' <<<"$cmd" | awk '{print $2}' | sort | tr '\n' ' '
        echo
    done | sort
}

# bins — the words of the first `for bin in … ; do` loop.
bins() {
    grep -oE 'for bin in [^;]*;' | head -1 |
        sed 's/^for bin in //; s/;$//' | tr -s ' ' '\n' | sed '/^$/d' | sort -u
}

# norm — map both scripts' spellings onto one form, and drop directories
# (an entry that is a prefix of another entry): only files are compared.
norm() {
    sed -E 's#^/##; s#/$##; s#\$\{([A-Za-z_]+)\}#$\1#g; s#\$(pkgname|DEB_DOCNAME)#<doc>#g; s#\$\(basename.*#*#' |
        sort -u | awk '{a[NR]=$0} END {for (i=1;i<=NR;i++) {d=0; for (j=1;j<=NR;j++) if (j!=i && index(a[j], a[i]"/")==1) d=1; if (!d) print a[i]}}'
}

report() { # label left-only right-only
    if [ -n "$2" ] || [ -n "$3" ]; then
        echo "install-parity: $1 differ"
        [ -n "$2" ] && sed 's/^/  PKGBUILD only:   /' <<<"$2"
        [ -n "$3" ] && sed 's/^/  install.sh only: /' <<<"$3"
        fail=1
    fi
}

# 1. build groups
pk_groups="$(body "$pkgbuild" build | groups)"
is_groups="$(body "$installsh" debian_build | groups)"
report "cargo build groups" "$(comm -23 <(echo "$pk_groups") <(echo "$is_groups"))" \
    "$(comm -13 <(echo "$pk_groups") <(echo "$is_groups"))"

# 2. binaries
pk_bins="$(body "$pkgbuild" package | bins)"
is_bins="$(body "$installsh" debian_install_files | bins)"
report "/usr/bin binaries" "$(comm -23 <(echo "$pk_bins") <(echo "$is_bins"))" \
    "$(comm -13 <(echo "$pk_bins") <(echo "$is_bins"))"
un_bins="$(body "$installsh" debian_uninstall | grep -oE '/usr/bin/\{[^}]*\}' | head -1 |
    sed 's#/usr/bin/{##; s#}##' | tr ',' '\n' | sort -u)"
report "uninstall fallback binaries" "$(comm -23 <(echo "$pk_bins") <(echo "$un_bins"))" \
    "$(comm -13 <(echo "$pk_bins") <(echo "$un_bins"))"

# 3. destinations (the binary loops are covered above)
pk_paths="$(body "$pkgbuild" package | grep -oE '"\$pkgdir/[^"]*' | sed 's#^"\$pkgdir##' | norm |
    grep -v '^usr/bin/\$bin$' | grep -vxF -f <(printf '%s\n' "${pkgbuild_only[@]}"))"
is_paths="$(body "$installsh" debian_install_files |
    grep -E '^[[:space:]]*(install_file|install_tree|install_link|\$SUDO install -d)|^[0-9]+\|' |
    grep -oE '(^|[" |])/(usr|etc)/[^" ]*' | sed -E 's#^[" |]##' | norm |
    grep -v '^usr/bin/\$bin$' | grep -v '^usr/local/')"
report "installed paths" "$(comm -23 <(echo "$pk_paths") <(echo "$is_paths"))" \
    "$(comm -13 <(echo "$pk_paths") <(echo "$is_paths"))"

if [ "$fail" = 0 ]; then
    echo "install-parity: install.sh matches the PKGBUILD"
fi
exit "$fail"
