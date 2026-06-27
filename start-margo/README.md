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

start-margo is **event-driven**: a single `poll(2)` over a `signalfd`, the
child's `pidfd`, and margo's readiness pipe. It sleeps until something actually
happens — no busy-poll, zero idle CPU for the whole session — and reacts to
readiness, shutdown and crashes instantly. On top of that:

1. **Crash budget + backoff.** `--max-crashes 3 --restart-window-secs 60` by
   default — after that many abnormal exits in the window it exits non-zero and
   returns you to the display manager, instead of pinning a CPU respawning a
   broken config. Restarts back off exponentially (250 ms → 5 s).
   `--max-restarts` is kept as a compatibility alias. With `--safe-config
   <PATH>` the supervisor makes one last attempt with a known-good config
   before giving up.
2. **systemd `sd_notify` + watchdog.** Emits `READY=1` only after margo signals
   that its Wayland socket, backend, compositor environment and XWayland/portal
   setup are ready; emits `STOPPING=1` on shutdown. When the unit sets
   `WatchdogSec=`, start-margo forwards `WATCHDOG=1` keep-alives driven by a
   heartbeat margo writes from its own event loop — so systemd can recover a
   *hung* compositor, not just a crashed one. A `Type=notify` unit (uwsm's
   `wayland-wm@.service`) sees the session as active without polling.
3. **Race-free, signal-preserving forwarding.** SIGTERM / SIGINT / SIGHUP are
   blocked process-wide and drained from the `signalfd`, so a signal that
   arrives in the window *before* the child is spawned is still delivered to
   it, with the original signal preserved — margo's teardown (surface
   destruction, ext-session-lock cleanup, `session.json` snapshot) always runs
   end-to-end. If the compositor does not exit within `--shutdown-timeout-secs`
   (default 5), the supervisor escalates to SIGKILL so logout cannot hang
   forever.
4. **A small readiness protocol.** margo speaks sd_notify-style lines over the
   readiness pipe (`READY=1`, `WATCHDOG=1`, `STATUS=…`, `FATAL=1`); start-margo
   forwards them to systemd and treats `FATAL=1` as "don't restart" so an
   unrecoverable init failure returns to the DM immediately.

Before each session, `start-margo` also runs `mctl check-config` unless
`--no-preflight` is passed. That catches syntax/regex/bind errors before the
watchdog ever enters a restart loop. Diagnostics are written through the shared
`margo-logging` file sink (`~/.local/state/margo/logs/start_margo-*.log`), so
the reason a session bounced back to the DM survives even without a journal.

start-margo's own terminal conditions use distinct exit codes: `78` (config
preflight failed), `69` (crash budget exhausted), `127` (margo binary could not
be spawned); any other non-zero code is margo's own.

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
