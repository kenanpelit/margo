# start-margo

A small Rust **watchdog supervisor** for the margo compositor. Start your
session through it instead of calling `margo` directly — it validates the config
before launch, restarts the compositor on crash (within a budget), speaks
systemd `sd_notify` after margo is genuinely ready, and forwards signals
cleanly so margo's own teardown always runs.

## Usage

```bash
# Wayland-session .desktop (e.g. /usr/share/wayland-sessions/margo.desktop)
Exec=start-margo

# Or under uwsm, replacing the compositor leaf:
uwsm app -a start-margo -- start-margo
```

Everything after `--` is forwarded to `margo`; `--path` points at a
dev/staging build.

## Why a supervisor

Three improvements over Hyprland's `start-hyprland`:

1. **Crash budget.** `--max-crashes 3 --restart-window-secs 60` by default —
   after that many abnormal exits in the window it exits non-zero and returns
   you to the display manager, instead of pinning a CPU respawning a broken
   config. `--max-restarts` is kept as a compatibility alias.
2. **systemd `sd_notify`.** Emits `READY=1` only after margo signals that its
   Wayland socket, backend, compositor environment and XWayland/portal setup
   are ready; emits `STOPPING=1` on shutdown. A `Type=notify` unit (uwsm's
   `wayland-wm@.service`) sees the session as active without polling.
3. **Signal forwarding preserves the signal.** SIGTERM / SIGINT / SIGHUP are
   forwarded as-is, so margo's teardown (surface destruction,
   ext-session-lock cleanup, `session.json` snapshot) runs end-to-end. If the
   compositor does not exit within `--shutdown-timeout-secs` (default 5), the
   supervisor escalates to SIGKILL so logout cannot hang forever.

Before each session, `start-margo` also runs `mctl check-config` unless
`--no-preflight` is passed. That catches syntax/regex/bind errors before the
watchdog ever enters a restart loop.

Shared with `start-hyprland`: `PR_SET_PDEATHSIG(SIGKILL)` so a `kill -9
start-margo` can't orphan the compositor.

## Build

```bash
cargo build --release -p start-margo
sudo install -m755 target/release/start-margo /usr/bin/start-margo
```

Ready-to-copy session glue (`.desktop`, uwsm wrapper, `margo-session`
launcher, systemd drop-in) lives in [`contrib/sessions/`](../contrib/sessions/).

## License

GPL-3.0-or-later.
