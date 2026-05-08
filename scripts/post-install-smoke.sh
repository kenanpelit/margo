#!/usr/bin/env bash
# vim: ft=bash sw=4 et
#
# Margo post-install smoke test.
#
# Run after `pacman -U margo-git-*.pkg.tar.zst` (or `makepkg -i`) to
# confirm the package landed correctly and a fresh user session would
# come up clean. Designed to be cheap (no Wayland session needed) so
# packagers can wire it into a `.install` hook or the PKGBUILD's
# `check()` phase.
#
# Checks:
#   1. `/usr/bin/margo` exists and prints --version cleanly.
#   2. `/usr/bin/mctl` exists and `mctl check-config --config <example>`
#      passes against the curated example config we ship.
#   3. `mctl actions` returns at least 30 entries (sanity bound for
#      the dispatch catalogue — drop a digit and the check trips).
#   4. The Wayland session entry validates: `desktop-file-validate
#      /usr/share/wayland-sessions/margo.desktop` exits 0.
#   5. xdg-desktop-portal config (when shipped) parses as TOML / INI.
#   6. Shell completions land in the right places.
#   7. License files / docs are installed (downstream attribution).
#
# Diagnostic output goes to stdout. Exit 0 if every check passes,
# 1 if any fail. Pass `--quiet` to skip the per-check ✓ lines and
# only print failures + final verdict.
#
# Usage:
#     scripts/post-install-smoke.sh
#     scripts/post-install-smoke.sh --quiet
#     POSTINSTALL_PREFIX=/usr scripts/post-install-smoke.sh   # default
#     POSTINSTALL_PREFIX=/tmp/staging scripts/post-install-smoke.sh

set -uo pipefail

QUIET=0
[[ "${1:-}" == "--quiet" ]] && QUIET=1

PREFIX="${POSTINSTALL_PREFIX:-/usr}"
PASS=0
FAIL=0
declare -a FAILED=()

ok() {
    [[ "$QUIET" == "1" ]] || printf '  ✓ %s\n' "$*"
    PASS=$((PASS + 1))
}
fail() {
    printf '  ✗ %s\n' "$*" >&2
    FAILED+=("$*")
    FAIL=$((FAIL + 1))
}
note() {
    [[ "$QUIET" == "1" ]] || printf '  · %s\n' "$*"
}
section() {
    [[ "$QUIET" == "1" ]] || { printf '\n'; printf '── %s ──\n' "$*"; }
}

# ── 1. Binaries ──────────────────────────────────────────────────────────────

section "1. Binaries"

MARGO_BIN="$PREFIX/bin/margo"
if [[ -x "$MARGO_BIN" ]]; then
    ok "$MARGO_BIN exists"
else
    fail "$MARGO_BIN missing or not executable"
fi

MCTL_BIN="$PREFIX/bin/mctl"
if [[ -x "$MCTL_BIN" ]]; then
    ok "$MCTL_BIN exists"
else
    fail "$MCTL_BIN missing or not executable"
fi

# `margo --help` exercises clap + every shared lib that link-time
# resolution depends on. A bad ld.so / missing libwayland-client
# trips here.
if [[ -x "$MARGO_BIN" ]]; then
    if "$MARGO_BIN" --help >/dev/null 2>&1; then
        ok "margo --help runs"
    else
        fail "margo --help failed (missing shared lib?)"
    fi
fi

# ── 2. Curated example config parses ─────────────────────────────────────────

section "2. Example config"

EXAMPLE_DIR="$PREFIX/share/doc/margo-git"
EXAMPLE="$EXAMPLE_DIR/config.example.conf"
if [[ -f "$EXAMPLE" ]]; then
    ok "example config: $EXAMPLE"
    if [[ -x "$MCTL_BIN" ]]; then
        if "$MCTL_BIN" check-config --config "$EXAMPLE" >/dev/null 2>&1; then
            ok "example config parses cleanly"
        else
            # Check-config exits 1 on errors. Re-run to print findings.
            note "example config has problems — running check-config:"
            "$MCTL_BIN" check-config --config "$EXAMPLE" 2>&1 | sed 's/^/    /'
            fail "mctl check-config flagged errors in example config"
        fi
    fi
else
    note "no shipped example config (optional) — skipping parse check"
fi

# ── 3. Dispatch action catalogue ─────────────────────────────────────────────

section "3. Dispatch catalogue"

if [[ -x "$MCTL_BIN" ]]; then
    ACTION_COUNT="$("$MCTL_BIN" actions --names 2>/dev/null | wc -l)"
    if [[ "$ACTION_COUNT" -ge 30 ]]; then
        ok "mctl actions: $ACTION_COUNT entries"
    else
        fail "mctl actions returned $ACTION_COUNT entries (expected ≥ 30)"
    fi
fi

# ── 4. Wayland session entry ─────────────────────────────────────────────────

section "4. Wayland session entry"

DESKTOP="$PREFIX/share/wayland-sessions/margo.desktop"
if [[ -f "$DESKTOP" ]]; then
    ok "session entry: $DESKTOP"
    if command -v desktop-file-validate >/dev/null 2>&1; then
        if desktop-file-validate "$DESKTOP" 2>&1 | tee /tmp/.margo-postinstall-desktop.log | grep -q .; then
            note "desktop-file-validate output:"
            sed 's/^/    /' /tmp/.margo-postinstall-desktop.log
            fail "desktop-file-validate flagged issues"
        else
            ok "desktop-file-validate clean"
        fi
        rm -f /tmp/.margo-postinstall-desktop.log
    else
        note "desktop-file-validate not installed — skipping syntax check"
    fi
    # Sanity bits we care about regardless of whether the validator is
    # available.
    if grep -q '^Type=Application' "$DESKTOP" \
        && grep -q '^Exec=' "$DESKTOP" \
        && grep -q '^Name=' "$DESKTOP"; then
        ok "session entry has Type/Exec/Name fields"
    else
        fail "session entry missing Type, Exec, or Name"
    fi
else
    fail "session entry missing — display managers won't list margo"
fi

# ── 5. xdg-desktop-portal config (optional) ──────────────────────────────────

section "5. xdg-desktop-portal config"

PORTAL="$PREFIX/share/xdg-desktop-portal/margo-portals.conf"
if [[ -f "$PORTAL" ]]; then
    ok "portal config: $PORTAL"
    # Lazy syntax check: must contain `[preferred]` section header.
    if grep -q '^\[preferred\]' "$PORTAL"; then
        ok "portal config has [preferred] section"
    else
        fail "portal config missing [preferred] header (file scheme broken)"
    fi
else
    note "no portal config shipped (optional) — skipping"
fi

# ── 6. Shell completions ─────────────────────────────────────────────────────

section "6. Shell completions"

declare -A COMPLETIONS=(
    [bash]="$PREFIX/share/bash-completion/completions/mctl"
    [zsh]="$PREFIX/share/zsh/site-functions/_mctl"
    [fish]="$PREFIX/share/fish/vendor_completions.d/mctl.fish"
)
for shell in bash zsh fish; do
    path="${COMPLETIONS[$shell]}"
    if [[ -f "$path" ]]; then
        ok "$shell completion: $path"
    else
        note "$shell completion missing: $path"
    fi
done

# ── 7. Licenses + docs ───────────────────────────────────────────────────────

section "7. Licenses / docs"

LICENSE_DIR="$PREFIX/share/licenses/margo-git"
if [[ -f "$LICENSE_DIR/LICENSE" ]]; then
    ok "primary LICENSE installed"
else
    fail "primary LICENSE missing in $LICENSE_DIR"
fi
# Upstream attribution headers (dwl/dwm/sway/tinywl/wlroots). These
# may legitimately be absent if margo loses the corresponding code
# during refactors — note rather than fail so we don't trip every
# release.
for lic in LICENSE.dwl LICENSE.dwm LICENSE.sway LICENSE.tinywl LICENSE.wlroots; do
    if [[ -f "$LICENSE_DIR/$lic" ]]; then
        note "  $lic present"
    fi
done

# ── Verdict ──────────────────────────────────────────────────────────────────

printf '\n'
if [[ "$FAIL" -eq 0 ]]; then
    printf '✓ post-install smoke passed (%d checks)\n' "$PASS"
    exit 0
else
    printf '✗ post-install smoke FAILED — %d failure%s\n' \
        "$FAIL" "$([[ $FAIL -eq 1 ]] && echo '' || echo 's')"
    for f in "${FAILED[@]}"; do
        printf '    %s\n' "$f"
    done
    exit 1
fi
