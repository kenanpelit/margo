#!/usr/bin/env bash
# The cargo build groups — the one list the CI build/check steps, the MSRV
# and Debian jobs and `just check` read, so they all build the same `-p`
# sets as the package.
#
# Why groups at all: cargo unifies features within one invocation, so
# building the compositor together with the tokio/zbus shell stack would
# switch shell-only features on in margo's shared deps (margo then panics
# at startup with "there is no reactor running"). Each group is its own
# `cargo build`; the PKGBUILD's build() comments say why each crate sits
# where it does. The PKGBUILD and install.sh still spell their groups out
# (makepkg must work from the PKGBUILD alone); `--check` fails when the
# PKGBUILD's groups drift from this file, and scripts/install-parity.sh
# holds install.sh to the PKGBUILD.
#
# Usage:
#   build-groups.sh names                 the group names, one per line
#   build-groups.sh run <group> <cmd…>    run `<cmd…> -p <crate> …` for one group
#   build-groups.sh each <cmd…>           the same for every group, in order;
#                                         stops at the first failure
#   build-groups.sh --check [root]        exit 1 when the PKGBUILD's groups differ
# e.g. `./scripts/build-groups.sh each cargo check --locked`. The script
# builds the argument list itself, so the caller's shell never word-splits it.
set -euo pipefail

compositor="margo start-margo mctl mlock mlayout mscreenshot mvisual mlogind mpower mdots mcal mtm"
shell="mshell mshellctl mshellshare mpicker mwizard mkeys mvpn mgreet margo-portal mshell-privileged"
# Built on its own (its async-io zbus stack must not unify with the shell's tokio).
mtune="mtune"

names() { printf '%s\n' compositor shell mtune; }

crates() {
    case "$1" in
    compositor) echo "$compositor" ;;
    shell) echo "$shell" ;;
    mtune) echo "$mtune" ;;
    *)
        echo "build-groups: unknown group '$1' (one of: $(names | paste -sd' '))" >&2
        return 1
        ;;
    esac
}

# sorted_set <words…> — the words sorted onto one line.
sorted_set() { tr -s ' ' '\n' <<<"$*" | sed '/^$/d' | sort | tr '\n' ' '; }

check() {
    local root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
    local ours theirs
    ours="$(for g in $(names); do sorted_set "$(crates "$g")"; echo; done | sort)"
    # One line per `cargo build` in the PKGBUILD's build(), comments dropped
    # and backslash-continued lines joined: its sorted -p list.
    theirs="$(awk '/^build\(\) *\{/ {on=1} on {print} on && /^}/ {exit}' "$root/PKGBUILD" |
        grep -v '^[[:space:]]*#' | sed -e ':a' -e '/\\$/N' -e 's/\\\n/ /' -e 'ta' |
        grep -E '^[[:space:]]*cargo build' | while read -r cmd; do
            sorted_set "$(grep -oE -- '-p [A-Za-z0-9_-]+' <<<"$cmd" | awk '{print $2}')"
            echo
        done | sort)"
    if [ "$ours" != "$theirs" ]; then
        echo "build-groups: scripts/build-groups.sh and the PKGBUILD's cargo build groups differ"
        diff <(echo "$ours") <(echo "$theirs") | sed -n 's/^< /  build-groups.sh only: /p; s/^> /  PKGBUILD only:       /p'
        return 1
    fi
    echo "build-groups: the PKGBUILD builds the same groups"
}

# run_group <group> <cmd…> — echo, then run `<cmd…> -p <crate> …`.
run_group() {
    local group="$1" list c
    shift
    list="$(crates "$group")" || return 1
    local args=()
    for c in $list; do args+=(-p "$c"); done
    echo "build-groups: [$group] $* ${args[*]}" >&2
    "$@" "${args[@]}"
}

usage() {
    echo "usage: build-groups.sh names | run <group> <cmd…> | each <cmd…> | --check [repo-root]" >&2
    exit 2
}

case "${1:-}" in
names) names ;;
run)
    [ $# -ge 3 ] || usage
    shift
    run_group "$@"
    ;;
each)
    [ $# -ge 2 ] || usage
    shift
    for g in $(names); do run_group "$g" "$@"; done
    ;;
--check) check "${2:-}" ;;
*) usage ;;
esac
