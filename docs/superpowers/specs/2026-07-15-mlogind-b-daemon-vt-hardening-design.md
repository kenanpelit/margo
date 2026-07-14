# mlogind phase B — signal-driven daemon loop, VT keyboard suppression, dynamic VT

**Date:** 2026-07-15
**Status:** implemented
**Scope:** the roadmap's phase B for `mlogind` (queued since A1, 2026-07-10):
`signalfd` event loop, `KDSKBMODE` keyboard suppression, `VT_OPENQRY` dynamic
VT, VT acquire/release, timerfd crash backoff. Phase D (greeter UX: themes,
autologin, OSK, echo-on prompts) is untouched.

## Why B exists

A1/A2 fixed *who runs PAM and as whom*. What they left behind is a daemon that
waits blindly:

1. **`waitpid` + `thread::sleep` are signal-opaque.** `systemctl stop mlogind`
   (or a reboot) SIGTERMs a daemon that is blocked in `waitpid` on the runner
   or asleep in a crash backoff. It dies on the spot; no destructor runs. The
   VT is left in `KD_GRAPHICS` — and once B adds keyboard suppression, that
   careless death would also leave the keyboard in `K_OFF`: a console that
   neither draws nor types.
2. **Password keys leak into the TTY buffer.** The greeter compositor reads
   input via evdev, but the kernel keyboard driver *also* cooks every key
   pressed on the active VT into the tty's input buffer. Keys typed before the
   compositor's first frame (the user starts typing early — the exact case the
   seamless-boot work optimises for) accumulate invisibly and replay into
   whatever reads that VT next: the TTY-greeter fallback, a getty, a root
   shell. Half a password in a VT buffer is a credential leak.
3. **The crash backoff is linear and uninterruptible.** A1 shipped a crude
   5-fast-crashes counter with `sleep(250ms × n)`; atrium does a real timerfd
   backoff.
4. **The VT is chosen blindly and never re-taken.** `tty = 2` is taken as-is
   even if something lives there, and after a session ends the daemon
   re-greets on its VT without making that VT the *active* one — a user who
   switched to tty3 mid-session comes back to a console, not a login.

## What ships

### B1 — `daemon::Events`: the signal-driven wait loop

New module `mlogind/src/daemon.rs`. On construction it blocks
`{SIGTERM, SIGINT, SIGHUP, SIGCHLD}` with `sigprocmask` (saving the old mask)
and opens a `SignalFd` (`SFD_CLOEXEC|SFD_NONBLOCK`) plus a monotonic `TimerFd`
(`TFD_CLOEXEC`) — all via the `nix` 0.23 wrappers already in the tree; no new
dependency. Every daemon wait becomes a `poll` over those fds:

- `wait_child(pid) -> Wait::{Exited(code), Terminated}` — replaces the
  blocking `waitpid` in `run_hosted`. Reaps with `WNOHANG` (checked *before*
  each poll, so a signal consumed by an earlier drain can never lose an exit).
- `sleep(dur) -> Sleep::{Elapsed, Terminated}` — replaces `thread::sleep` for
  the crash backoff; a termination signal cuts the backoff short.
- `terminate_child(pid) -> code` — the shutdown path: SIGTERM the runner,
  give it 5 s on the timerfd, then SIGKILL. Called when any wait observes
  `Terminated`, *before* the daemon returns and the VT guard drops — so the
  console is always restored on the way out.

`run_hosted` now returns `HostExit::{Quit, Terminated}`; `Terminated` exits
mlogind cleanly **without** falling down the `gui → cage → tty` ladder (a
stopping service must not respawn a greeter).

**Mask hygiene (load-bearing):** the blocked mask is inherited across fork
*and* exec. `runner::run` resets it to empty first thing
(`daemon::reset_signal_mask()`), otherwise the user's session itself would run
with SIGTERM blocked. The TTY host is untouched: `Events` is dropped (restoring
the old mask) before `run_tty_host`, so the classic path keeps its classic
signal behaviour, and its blocking `wait_for` stays.

If `Events::new()` fails (fd exhaustion — effectively never), mlogind logs it
and degrades straight to the TTY greeter rather than running graphical hosts
without a teardown path.

### B2 — keyboard suppression in the VT guard

`vt_blank.rs` becomes `vt_guard.rs`; the `VtBlank` blank-only guard becomes
`VtGuard`, same lifetime, same call site, same "drop before the TTY greeter"
rule. On top of `KD_GRAPHICS` it now snapshots the keyboard mode with
`KDGKBMODE` and sets `KDSKBMODE K_OFF`, restoring the snapshot (keyboard
first, then `KD_TEXT`) on drop.

Lockout rules, in order of importance:
- **A mode we cannot snapshot is a mode we must not set.** If `KDGKBMODE`
  fails, suppression is skipped and only the blank is held.
- **Never restore `K_OFF`.** If the snapshot itself reads `K_OFF` (a previous
  holder died un-restored), the restore target is snapped to `K_UNICODE`.
- Best-effort throughout: any failure is a cosmetic flash or a harmless
  duplicate input stream, never a blocked login.

The suppression shares the blank's exposure: a SIGKILL'd daemon restores
nothing. That exposure already exists for `KD_GRAPHICS` and has been fine on
hardware; B1's signal loop is precisely what makes the *ordinary* kill paths
(TERM/INT/HUP) restore reliably.

### B3 — dynamic VT + re-activation (and the VT_PROCESS non-decision)

- `chvt::first_free_vt()` wraps `VT_OPENQRY`. New config knob
  `[display] dynamic_vt` (default **false** — a fixed, predictable VT is right
  for a machine whose gettys are laid out around it). When on, and `--tty` was
  not given, the kernel's offer (clamped to 1–12) replaces `config.tty` before
  anything uses it — chvt, the VT guard, `XDG_VTNR`, utmpx and
  `wait_vt_free` all read `config.tty`, so one assignment covers the tree.
- `run_hosted` re-activates the greeter VT (`chvt`) at the top of every runner
  cycle: after a session ends, the active VT is wherever the user last
  switched; the fresh greeter renders on ours, so make ours active. A no-op on
  the first pass and on machines that never VT-switch.
- While here: `chvt.rs`'s inherited error checks (`ioctl(...) > 0`) were dead
  code — `ioctl` reports errors as `-1`. Fixed to `< 0`, so activate/wait
  failures are finally *visible* (they were always best-effort logged).

**Deliberately NOT doing atrium's `VT_SETMODE(VT_PROCESS)` handshake.** In
atrium the daemon is the VT's only manager. Under A2 our greeter (and every
session) runs in a real logind session, and logind takes VT_PROCESS ownership
of the session VT itself; a second VT_PROCESS owner on the same VT fights
logind and an unanswered `VT_RELDISP` wedges VT switching machine-wide — a
lockout risk for zero gain. The practical benefit atrium gets from
acquire/release (the greeter's VT becomes active again on re-greet) is exactly
the `chvt` re-activation above.

### Backoff shape

`crash_backoff(n) = 250ms × 2^(n-1)`, capped at 4 s (250, 500, 1000, 2000 ms
for n = 1..4; the 5th fast crash still falls down the host ladder, unchanged).
"Fast" still means "the runner lived < 2 s". A normal session end still resets
the counter and re-greets with no delay.

## Out of scope / follow-ups

- A graceful TERM handler inside the *runner* (close the PAM session before
  dying). Today logind tears the session down when the leader dies, which is
  also what the previous cgroup-wide kill did; unchanged.
- Phase D (greeter UX) and phase E (edition 2024, sysusers).
- Multiseat: permanently out (single machine, one GPU).

## Testing

- `crash_backoff` sequence + cap: pure unit tests.
- `Config::default().display.dynamic_vt == false` pins the packaged default
  (the baked `extra/config.toml` must carry the key — `Config` derives a
  non-optional `Deserialize`, so forgetting it would abort at startup; the
  test would catch that too).
- A timerfd-through-poll smoke test pins the nix 0.23 API usage without
  touching process signal state (signal-mask tests are unsafe under cargo's
  threaded test runner: a process-directed SIGTERM landing on an unmasked
  sibling thread kills the harness).
- On hardware (user): reboot → login → logout → login (re-greet + VT
  re-activation); `systemctl stop mlogind` from SSH while the greeter is up
  must leave the console drawing and typing (KD_TEXT + keyboard restored);
  keys typed at the greeter before the card appears must not surface on any
  VT afterwards.

## Files touched

- `mlogind/src/daemon.rs` — new: Events (sigmask + signalfd + timerfd + poll),
  Wait/Sleep, terminate_child, reset_signal_mask, decode_wait_status.
- `mlogind/src/main.rs` — run_hosted rewrite (HostExit, wait_child, chvt
  re-activation, interruptible backoff), Events wiring + drop-before-TTY,
  dynamic-VT resolve, wait_for → decode_wait_status.
- `mlogind/src/vt_blank.rs` → `mlogind/src/vt_guard.rs` — KDGKBMODE/KDSKBMODE
  hold added to the KD_GRAPHICS guard.
- `mlogind/src/chvt.rs` — VT_OPENQRY + `first_free_vt`, `< 0` error checks.
- `mlogind/src/runner/mod.rs` — signal-mask reset at the top of `run`.
- `mlogind/src/config.rs` + `mlogind/extra/config.toml` — `[display]
  dynamic_vt` (default false).
- No new dependencies; `Cargo.lock` untouched.
