#!/usr/bin/env bash
#
# Margo windowrule smoke test.
#
# Goal: validate that the user's `~/.config/margo/config.conf` produces the
# expected outcomes for a handful of representative apps. Designed to run
# inside a live margo session — spawns each test client, waits for it to
# map, queries dwl-ipc state via `mctl status`, and asserts what we see
# matches the rule.
#
# Usage:
#     scripts/smoke-rules.sh           # run all installed test cases
#     scripts/smoke-rules.sh --reload  # `mctl reload` first (pick up local edits)
#     SMOKE_TIMEOUT=2 scripts/smoke-rules.sh   # per-case wait, default 1s
#     SMOKE_VERBOSE=1 scripts/smoke-rules.sh   # dump full mctl status on FAIL
#
# Exits 0 if every installed test case passes, 1 otherwise. Tests for apps
# that aren't installed are reported as SKIP (don't count against pass/fail).

set -uo pipefail

# ── Plumbing ──────────────────────────────────────────────────────────────────

if ! command -v mctl >/dev/null 2>&1; then
    echo "mctl not found in PATH; install margo first." >&2
    exit 2
fi
if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
    echo "WAYLAND_DISPLAY unset — must run inside a margo session." >&2
    exit 2
fi

TIMEOUT="${SMOKE_TIMEOUT:-1}"
VERBOSE="${SMOKE_VERBOSE:-0}"

if [[ "${1:-}" == "--reload" ]]; then
    echo "→ reloading margo config"
    mctl reload >/dev/null 2>&1 || true
    sleep 0.5
fi

PASS=0
FAIL=0
SKIP=0
declare -a FAILED_NAMES=()

# Normalize mctl status output: strip ANSI, drop empty lines.
mctl_status() {
    mctl status 2>/dev/null | sed -e 's/\x1b\[[0-9;]*m//g'
}

# Wait for an app_id substring to appear in mctl status, up to TIMEOUT seconds.
wait_for_app() {
    local pattern="$1"
    local deadline=$(( $(date +%s) + TIMEOUT + 1 ))
    while (( $(date +%s) < deadline )); do
        if mctl_status | grep -Eq "(app[_]?id|appid|class)[^a-zA-Z0-9]*$pattern"; then
            return 0
        fi
        sleep 0.1
    done
    return 1
}

# Run one named test case.
#   $1 = test name (printed)
#   $2 = bin to look up via `command -v`
#   $3 = launch command (background)
#   $4 = pattern that should appear in mctl status (regex on app_id/title)
#   $5 = optional grep on `mctl status` output that must succeed for PASS
test_case() {
    local name="$1" probe="$2" launch="$3" map_pattern="$4" assert_pattern="$5"

    if ! command -v "$probe" >/dev/null 2>&1; then
        printf '  \e[33mSKIP\e[0m  %-30s  (%s not installed)\n' "$name" "$probe"
        SKIP=$((SKIP + 1))
        return
    fi

    # Spawn in background, suppress its output, store PID.
    bash -c "$launch" </dev/null >/dev/null 2>&1 &
    local pid=$!

    if ! wait_for_app "$map_pattern"; then
        printf '  \e[31mFAIL\e[0m  %-30s  (window never showed in mctl status)\n' "$name"
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("$name")
        kill "$pid" 2>/dev/null
        return
    fi

    # Give the rule one extra tick to settle (windowrule reapply on
    # late-arriving app_id can delay floating placement by ~50ms).
    sleep 0.2

    local status_out
    status_out="$(mctl_status)"

    if echo "$status_out" | grep -Eq "$assert_pattern"; then
        printf '  \e[32mPASS\e[0m  %-30s\n' "$name"
        PASS=$((PASS + 1))
    else
        printf '  \e[31mFAIL\e[0m  %-30s\n' "$name"
        if [[ "$VERBOSE" == "1" ]]; then
            echo "    expected to match: $assert_pattern"
            echo "    got mctl status:"
            sed 's/^/      /' <<<"$status_out"
        fi
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("$name")
    fi

    # Best-effort cleanup. Some apps fork into a session-bus daemon; if so
    # leave them — the user's config probably wants them long-lived.
    kill "$pid" 2>/dev/null
    sleep 0.1
}

# ── Test cases ────────────────────────────────────────────────────────────────
#
# Each line tests one rule from the canonical rule set. Add new entries as
# new rules land. `assert_pattern` is intentionally loose — we only check
# the *consequence* of the rule (floating? specific size? specific tag?),
# not the entire mctl output, so cosmetic IPC changes don't break tests.

echo "→ margo windowrule smoke (timeout ${TIMEOUT}s per case)"
echo

# Plain terminal — no rule, baseline (should NOT be floating).
test_case \
    "kitty (no rule, tiled)" \
    "kitty" \
    "kitty --class margo-smoke-tiled sleep 5" \
    "margo-smoke-tiled" \
    "margo-smoke-tiled.*floating *(false|0)|tiled.*margo-smoke-tiled"

# Test the auth/dialog title rule (line 191 of the user config).
# The most reliable thing to spawn a known title is `zenity` or `dialog`.
# zenity is part of GNOME and usually installed alongside KeePass etc.
test_case \
    "zenity dialog (title rule → floating)" \
    "zenity" \
    "zenity --info --title 'Authentication Required' --text 'smoke test' --timeout 3" \
    "Authentication Required" \
    "Authentication Required.*floating *(true|1)|floating *(true|1).*Authentication Required"

# pavucontrol → floating per windowrule.
test_case \
    "pavucontrol → floating" \
    "pavucontrol" \
    "pavucontrol" \
    "pavucontrol|org\\.pulseaudio\\.pavucontrol" \
    "(pavucontrol|pulseaudio).*floating *(true|1)"

# CopyQ → floating + named scratchpad (depending on user config).
test_case \
    "copyq → floating" \
    "copyq" \
    "copyq show" \
    "copyq" \
    "copyq.*floating *(true|1)"

# Calculator → floating per the size-pinned rule (most common font).
test_case \
    "kcalc → floating 540×640" \
    "kcalc" \
    "kcalc" \
    "kcalc|org\\.kde\\.kcalc" \
    "kcalc.*floating *(true|1)"

# ── Summary ───────────────────────────────────────────────────────────────────

echo
total=$((PASS + FAIL + SKIP))
if (( FAIL == 0 && PASS > 0 )); then
    printf '\e[32m✓ all %d ran (%d pass, %d skip, 0 fail)\e[0m\n' "$total" "$PASS" "$SKIP"
    exit 0
elif (( PASS == 0 && SKIP > 0 && FAIL == 0 )); then
    echo "no tests ran — install at least one of the expected probes." >&2
    exit 2
else
    printf '\e[31m✗ %d/%d failed\e[0m  (%d pass, %d skip)\n' "$FAIL" "$total" "$PASS" "$SKIP"
    if (( ${#FAILED_NAMES[@]} > 0 )); then
        printf '\nfailing: %s\n' "${FAILED_NAMES[*]}"
    fi
    if [[ "$VERBOSE" != "1" ]]; then
        echo "rerun with SMOKE_VERBOSE=1 for full mctl output on each failure."
    fi
    exit 1
fi
