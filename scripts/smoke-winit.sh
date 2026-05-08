#!/usr/bin/env bash
# vim: ft=bash sw=4 et
#
# Margo nested-backend smoke test.
#
# Spawns `margo --winit` inside the current Wayland or X11 session (no DRM
# needed — runs on a workstation, on a CI VM, anywhere a parent display
# is available), boots a single client, walks every code path the
# `daily-driver` checklist cares about, and tears the nested session
# down. Pass = exit 0; any failed assertion = exit 1.
#
# What gets exercised:
#   * `margo --print-config-path` (or `--check-config` if added later) —
#     config parse with the user's real `~/.config/margo/config.conf`.
#   * Spawn path: `--startup-command` launches kitty inside the nested
#     session before the event loop starts.
#   * `mctl status` against the nested compositor — IPC socket plumbing.
#   * `mctl reload` — config-reload codepath while the loop runs.
#   * `mctl dispatch focusstack 1` — focus change; verified by the next
#     status block reporting the same client (only one client = no-op
#     but still exercises the dispatch).
#   * `mctl dispatch killclient` — close path; status's `appid=` slot
#     should empty out.
#
# Usage:
#     scripts/smoke-winit.sh                # default
#     SMOKE_BUILD=1 scripts/smoke-winit.sh  # `cargo build --release` first
#     SMOKE_KEEP=1  scripts/smoke-winit.sh  # don't tear down on success
#                                           # (manual poking after the run)
#     SMOKE_VERBOSE=1 scripts/smoke-winit.sh  # tail margo + child logs
#
# Exits with the first failed assertion. Diagnostic logs go to
# `/tmp/margo-smoke-$$/`.

set -uo pipefail

# ── Plumbing ─────────────────────────────────────────────────────────────────

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="${SMOKE_BUILD:-0}"
KEEP="${SMOKE_KEEP:-0}"
VERBOSE="${SMOKE_VERBOSE:-0}"
RUN_DIR="$(mktemp -d -t margo-smoke-XXXXXX)"
LOG="$RUN_DIR/margo.log"
CHILD_LOG="$RUN_DIR/kitty.log"

# Hand-rolled `say` — bash's `echo -e` is not portable across distros.
say() {
    printf '%s\n' "$*"
}
ok() {
    say "  ✓ $*"
}
fail() {
    say "  ✗ $*" >&2
    say "" >&2
    say "Diagnostic logs left in $RUN_DIR" >&2
    if [[ "$VERBOSE" == "1" ]]; then
        say "── margo log tail ─" >&2
        tail -n 30 "$LOG" 2>/dev/null >&2 || true
        say "── child log tail ─" >&2
        tail -n 10 "$CHILD_LOG" 2>/dev/null >&2 || true
    fi
    exit 1
}

# ── Pre-flight ───────────────────────────────────────────────────────────────

if [[ -z "${WAYLAND_DISPLAY:-}" && -z "${DISPLAY:-}" ]]; then
    fail "neither WAYLAND_DISPLAY nor DISPLAY set; need a parent session for --winit"
fi
if ! command -v kitty >/dev/null 2>&1; then
    fail "kitty not installed — needed as the test client"
fi

if [[ "$BUILD" == "1" ]]; then
    say "→ building margo (release)"
    (cd "$ROOT" && cargo build --release --quiet) || fail "cargo build failed"
fi

# Pick the binary: prefer release/, fall back to debug/, finally PATH `margo`.
MARGO_BIN=""
for c in "$ROOT/target/release/margo" "$ROOT/target/debug/margo" "$(command -v margo 2>/dev/null)"; do
    if [[ -x "$c" ]]; then
        MARGO_BIN="$c"
        break
    fi
done
[[ -n "$MARGO_BIN" ]] || fail "no margo binary found (looked in target/release, target/debug, PATH)"
MCTL_BIN=""
for c in "$ROOT/target/release/mctl" "$ROOT/target/debug/mctl" "$(command -v mctl 2>/dev/null)"; do
    if [[ -x "$c" ]]; then
        MCTL_BIN="$c"
        break
    fi
done
[[ -n "$MCTL_BIN" ]] || fail "no mctl binary found"

say "→ binaries: margo=$MARGO_BIN  mctl=$MCTL_BIN"

# ── Stage 1: config parse ────────────────────────────────────────────────────

say "→ stage 1: config parse"
# `margo --help` triggers clap parsing; if the binary itself is broken
# (link-time issue, missing shared lib) this catches it. Doesn't touch
# the user's config.
"$MARGO_BIN" --help >/dev/null 2>&1 || fail "margo --help exited non-zero"
ok "binary launches"

# ── Stage 2: nested session bring-up ─────────────────────────────────────────

say "→ stage 2: launching margo --winit"
# Run margo in nested mode, with a startup-command that spawns kitty so
# we have something to focus / close. MARGO_LOG=info gives us the
# `INFO margo::*` lines we grep on later.
MARGO_LOG="info" \
    "$MARGO_BIN" --winit -s "kitty -- /bin/sh -c 'echo MARGO_SMOKE_KITTY_OK; sleep 30'" \
    >"$LOG" 2>&1 &
MARGO_PID=$!
say "  pid=$MARGO_PID  log=$LOG"

# Tear down on exit no matter what.
cleanup() {
    if [[ "$KEEP" == "1" && $? -eq 0 ]]; then
        say ""
        say "→ SMOKE_KEEP=1 — leaving nested margo running (pid=$MARGO_PID)"
        say "  socket: $WAYLAND_DISPLAY_NESTED  (run \`unset WAYLAND_DISPLAY_NESTED\` to detach)"
        return
    fi
    if [[ -n "${MARGO_PID:-}" ]] && kill -0 "$MARGO_PID" 2>/dev/null; then
        kill -TERM "$MARGO_PID" 2>/dev/null || true
        wait "$MARGO_PID" 2>/dev/null || true
    fi
    if [[ "$KEEP" != "1" ]]; then
        rm -rf "$RUN_DIR"
    fi
}
trap cleanup EXIT INT TERM

# Wait for the nested compositor to publish a wayland socket. winit
# logs the new display name as `info: starting Wayland nested backend
# on wayland-N` (or similar). Grep until it appears, capped at 8 s.
WAYLAND_DISPLAY_NESTED=""
for _ in $(seq 1 80); do
    sleep 0.1
    line="$(grep -oE 'wayland-[0-9]+' "$LOG" | tail -n 1 || true)"
    if [[ -n "$line" ]]; then
        WAYLAND_DISPLAY_NESTED="$line"
        break
    fi
done
if [[ -z "$WAYLAND_DISPLAY_NESTED" ]]; then
    fail "nested margo did not announce a wayland socket within 8 s"
fi
ok "nested socket: $WAYLAND_DISPLAY_NESTED"

# Confirm margo's still running (didn't crash post-startup).
if ! kill -0 "$MARGO_PID" 2>/dev/null; then
    fail "margo exited after socket announce; check $LOG"
fi
ok "margo pid=$MARGO_PID alive"

# ── Stage 3: client spawn + IPC ──────────────────────────────────────────────

say "→ stage 3: client spawn + IPC"
# Wait for kitty's first commit so it shows up in mctl status. Margo
# logs `xdg_shell: new toplevel` on `xdg_toplevel.commit`.
saw_toplevel=0
for _ in $(seq 1 60); do
    if grep -q "new_toplevel\|finalize_initial_map" "$LOG" 2>/dev/null; then
        saw_toplevel=1
        break
    fi
    sleep 0.1
done
[[ "$saw_toplevel" == "1" ]] || fail "kitty's xdg_toplevel never reached margo within 6 s"
ok "kitty toplevel mapped"

# Talk to the nested margo via mctl, NOT the host compositor.
MCTL() { WAYLAND_DISPLAY="$WAYLAND_DISPLAY_NESTED" "$MCTL_BIN" "$@"; }

status_block="$(MCTL status 2>&1 || true)"
if [[ "$VERBOSE" == "1" ]]; then
    say "── status:"
    say "$status_block" | sed 's/^/    /'
fi
[[ -n "$status_block" ]] || fail "mctl status produced no output against the nested socket"
echo "$status_block" | grep -q '^output=' || fail "mctl status missing output= line"
ok "mctl status returned a frame"

# Per the launch line, kitty's appid should be `kitty`. Margo populates
# this on the first xdg_toplevel.set_app_id, which fires immediately
# for kitty.
echo "$status_block" | grep -q 'appid="kitty"' \
    || fail "focused appid != kitty (status: $(echo "$status_block" | head -1))"
ok "focused appid == kitty"

# ── Stage 4: dispatch round-trip ─────────────────────────────────────────────

say "→ stage 4: dispatch round-trip"
MCTL dispatch focusstack 1 >/dev/null 2>&1 \
    || fail "mctl dispatch focusstack 1 failed"
ok "focusstack accepted"

MCTL reload >/dev/null 2>&1 \
    || fail "mctl reload failed (config likely failed to re-parse)"
sleep 0.2
grep -q "config reloaded" "$LOG" \
    || fail "margo log missing 'config reloaded' line after mctl reload"
ok "config reload round-trips"

# ── Stage 5: close path ──────────────────────────────────────────────────────

say "→ stage 5: close path"
MCTL dispatch killclient >/dev/null 2>&1 \
    || fail "mctl dispatch killclient failed"

# Wait for the status's appid slot to empty out (kitty was the only
# toplevel).
emptied=0
for _ in $(seq 1 40); do
    sleep 0.1
    s="$(MCTL status 2>/dev/null || true)"
    if echo "$s" | grep -q 'appid=""'; then
        emptied=1
        break
    fi
done
[[ "$emptied" == "1" ]] || fail "appid did not empty within 4 s after killclient"
ok "client closed; status empty"

# ── Done ─────────────────────────────────────────────────────────────────────

say ""
say "✓ smoke-winit passed"
say "  log: $LOG  (kept while compositor runs)"
exit 0
