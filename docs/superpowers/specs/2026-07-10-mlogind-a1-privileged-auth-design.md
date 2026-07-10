# mlogind A1 — privileged auth core

Date: 2026-07-10
Status: approved, ready for implementation
Scope: `mlogind`, `mgreet`, new `mlogind-proto` crate

## Why

`mlogind` runs the PAM conversation twice and moves the plaintext password
through a file.

Today, `mlogind` (the root daemon) calls `try_validate` → `pam_authenticate`
in its own process, then `fork()`s and lets the child call `pam_open_session`
on the inherited handle while the parent calls `std::mem::forget(creds)` to
dodge a double free (`mlogind/src/main.rs:438`). Authentication happens in one
process, session opening in another, on one PAM handle that straddles a fork.

Meanwhile `mgreet` — running as root under a root `margo` — runs its *own*
full `pam_authenticate` as a pre-flight, then writes
`LOGIN\n<user>\n<session>\n<password>` to a 0600 file in `/run/mlogind`,
which the daemon reads, zero-overwrites and unlinks.

Three concrete consequences:

1. **Multi-step PAM is structurally impossible.** `pam::PasswordConv` replays a
   fixed username/password for every prompt. A fingerprint reader prompts twice,
   a U2F key asks for two taps, an OTP module cannot work at all, and
   "your password has expired, enter a new one" cannot be answered.
2. **The plaintext password exists as a filesystem object.** It is a root-only
   tmpfs and the shred is best-effort, but the file survives any crash between
   `write` and `read_and_shred_greet_result`.
3. **PAM state crosses a `fork()`.** `pam_open_session` determines the calling
   process's cgroup and (with `pam_loginuid`) writes `/proc/self/loginuid`;
   `pam_systemd` takes the *calling* PID as the logind session leader. Getting
   this wrong pollutes the daemon. The current code gets it right only because
   of the `mem::forget` trick.

atrium (`~/.kod/display/atrium`) solves all three by keeping the whole PAM
conversation in a single forked *session runner* process and talking to an
unprivileged greeter over an anonymous pipe pair, streaming prompts instead of
shipping a password. SDDM does the same over a socket, with an explicit prompt
model (`src/auth/AuthPrompt.h`, `AuthRequest.h`, `AuthMessages.h`).

A1 adopts that shape. **A1 does not drop the greeter's privileges** — that is
A2, and it carries the separate risk that `margo` must open DRM as a
non-root user via `LIBSEAT_BACKEND=logind`.

## Non-goals

- Multiseat. `seat0` stays hardcoded. (Decided: out of scope permanently.)
- Dropping greeter privileges, logind `CreateSession`, sysusers/tmpfiles. → A2.
- `signalfd` event loop, `VT_OPENQRY`, VT keyboard suppression, timerfd
  backoff. → B.
- Greeter UX: background images, external themes, keyboard-layout switcher,
  idle blank, autologin, mkeys integration. → D.
- Migrating `mlogind` to edition 2024. → E.
- Adding `pam_putenv` calls. Verified unnecessary: `pam_systemd`'s
  `getenv_harder()` reads the PAM environment first and falls back to the
  process environment, which is what `set_seat_vars`/`set_session_params`
  already populate. atrium's note about `pam_putenv()` does not apply to us.

## Architecture

### Process model

```
mlogind (root daemon)              ── never touches PAM
  ladder: gui → cage → tty
  loop:
    fork() ──→ session runner (root)
       │
       │  Host::Gui   socketpair(SEQPACKET); spawn margo+mgreet, greeter end on MLOGIND_SOCK_FD
       │  Host::Cage  socketpair(SEQPACKET); spawn cage+foot+`mlogind --greet`, same
       │  Host::Tty   socketpair made by the daemon before fork; the daemon *is* the greeter
       │
       │  conversation:  Begin{user,session} → (Prompt ⇄ Response)* → Success | Failure
       │                 Failure → pam_end, wait for a new Begin on the same socket
       │
       │  Success → write last-login cache → wait for the greeter host to exit
       │            pam_open_session()            ← this process becomes the session leader
       │            wait for the logind session to go active (bounded)
       │            fork() ──→ compositor: setgid/initgroups/setuid + env + chdir + exec
       │            waitpid → Drop<Authenticator> → close_session + setcred(DELETE) + end
       │  exit(0)
       │
    waitpid(runner) → interpret exit code → loop or fall down the ladder
```

Fork depth is unchanged (daemon → runner → compositor). Today's
`session_child` is promoted to the runner and the greeter conversation moves in
front of it. Nothing — no PAM handle, no environment, no fd — survives from one
runner to the next; that is atrium's central invariant and we inherit it for
free because the runner is a fresh `fork()` of a daemon that never ran PAM.

**The runner spawns the greeter host, not the daemon.** This preserves today's
ordering: the session compositor is only launched after `margo` has exited and
released its DRM outputs. If the daemon owned the host, the runner could open
the session while `margo` was still tearing down.

### Runner exit codes

| Code | Meaning | Daemon reaction |
|---|---|---|
| `0` | Session ran and ended | Loop: fork a fresh runner, show the greeter again |
| `10` | Greeter quit without a login | Return `Ok(())`; `mlogind` shuts down |
| `11` | Greeter host could not start at all | Return `Err`; fall to the next host in the ladder |
| other | Runner or session failed | Log, loop |

The `other` branch needs a floor or a runner that dies instantly gives us an
infinite fork loop at boot. Minimal guard, introduced by this change and not a
substitute for phase B's timerfd backoff: five consecutive non-zero exits, each
within 2 s of its fork, aborts the host with `Err` and falls down the ladder.

### `pam` crate facts this design leans on

Verified against `pam-0.7.0` in the local registry:

- `pub trait Converse { prompt_echo, prompt_blind, info, error, username }` and
  `Authenticator::with_handler(service, converse)` exist. `PasswordConv` is
  merely the trivial impl; we supply our own.
- `Authenticator::authenticate()` already calls `pam_acct_mgmt` after
  `pam_authenticate`, and resets on failure.
- `Authenticator::open_session()` already does
  `setcred(ESTABLISH_CRED) → pam_open_session → setcred(REINITIALIZE_CRED)`
  and then copies the PAM environment into the process environment.
- `Drop for Authenticator` already calls `close_session` +
  `setcred(DELETE_CRED)` + `pam_end`.
- `open_session()` internally calls `Converse::username()` to look the user up,
  so our conversation type must retain the username from `Begin`.

So atrium's five-stage flow is already wrapped by this crate. We do not
reimplement it; we replace the *conversation*.

`Authenticator::with_handler` takes the `Converse` by value. Our
`GreeterConv<'a, T>` therefore borrows the transport rather than owning it
(`&'a mut T`), so the runner can drop a failed `Authenticator` and start a new
`pam_start` on the same socket for the next `Begin`.

**A new `Authenticator` per `Begin`.** Retrying `authenticate()` on a handle
whose `acct_mgmt` failed is not well-defined; atrium likewise calls `pam_start`
per attempt.

**PAM will prompt for the username** because the crate calls
`pam_start(service, None, …)`. `GreeterConv::prompt_echo` answers the *first*
echo prompt from the username carried in `Begin`, without a round trip; any
subsequent echo prompt is forwarded to the greeter. This preserves today's
behaviour and keeps the common path at one round trip.

## `mlogind-proto`

New workspace crate. `license = "MIT OR Apache-2.0"` so both `mlogind`
(MIT/Apache) and `mgreet` (GPL-3.0-or-later, which may consume permissive code)
can depend on it. Dependencies: `zeroize` only.

This is the shared core atrium keeps in `lib/ipc.c` and consumes from both
`greeter/main-gtk.c` and `greeter/main-txt.c`. We have the same two greeters.

Transport: `socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0)`. Message
boundaries are preserved by the kernel; a partial frame is impossible.

Frame: `u8 tag | u32 len (big-endian) | payload`, `MAX_FRAME = 64 KiB`.

Greeter → runner (`Request`):

| Variant | Payload |
|---|---|
| `Begin { user, session }` | two length-prefixed UTF-8 strings |
| `Response { secret }` | one length-prefixed byte string, `Zeroizing<Vec<u8>>` |
| `Cancel` | — |

Runner → greeter (`Event`):

| Variant | Payload |
|---|---|
| `Prompt { echo: bool, text }` | `u8` + string |
| `Info { text }` | string |
| `Error { text }` | string |
| `Success` | — |
| `Failure { reason }` | string |

`Response` carries bytes, not `String`: a PAM response is opaque and need not be
UTF-8, and `Zeroizing<Vec<u8>>` scrubs on drop where `String` would not help us
across a `from_utf8` copy.

No `Power` variant in A1. The greeter is still root and runs `systemctl
poweroff` itself, exactly as today. A2 makes the greeter unprivileged, and that
is when `Power { action }` earns its place.

The transport is a trait so the conversation engine is testable without a
socket:

```rust
pub trait Transport {
    fn send(&mut self, frame: &[u8]) -> io::Result<()>;
    fn recv(&mut self, buf: &mut Vec<u8>) -> io::Result<usize>; // 0 == EOF
}
```

`FdTransport` wraps a `RawFd` (borrowed — the runner owns the `OwnedFd`).
Tests use an in-memory duplex. This mirrors the `trait Authenticate` seam
already in `mgreet/src/auth.rs`.

### fd passing

`socketpair` is created with `SOCK_CLOEXEC`. Immediately before spawning the
greeter host the runner clears `FD_CLOEXEC` on the greeter end
(`fcntl(fd, F_SETFD, 0)`) and passes `MLOGIND_SOCK_FD=<n>` in the environment —
atrium's `CREDENTIALS_FD` / `RESULT_FD` idiom. The runner closes its copy of the
greeter end after the spawn, so the greeter exiting produces a clean EOF.

`mlogind` is single-threaded, so there is no `fork`-vs-`exec` fd race, and no
`pre_exec` closure is needed.

For `Host::Gui` the fd is inherited through `margo` → `sh -c` → `mgreet`. The
`mctl dispatch quit` that follows `mgreet` in the startup command also inherits
it; harmless, and it closes on exec-exit.

## Changes per crate

### `mlogind-proto` (new, ~250 lines + tests)

`lib.rs`: `Request`, `Event`, `encode_request`, `decode_request`,
`encode_event`, `decode_event`, `Transport`, `FdTransport`, `Conn` (a
`Transport` + framing buffer offering `send_event`/`recv_request` and the
mirror pair), `ProtoError`.

### `mlogind`

New `mlogind/src/runner.rs`:
- `Host { Gui, Cage, Tty }`
- `run(config, host) -> !` — the forked child body.
- `converse.rs`: `GreeterConv<'a, T: Transport>` implementing `pam::Converse`.
- `session_active.rs`: bounded wait for the logind session to become active.

`mlogind/src/main.rs`:
- Delete `Hooks`, `start_session`, `session_child`, `StartSessionError`,
  `GreetResult`, `read_and_shred_greet_result`, `launch_greet_result`,
  `MLOGIND_RESULT_PATH`.
- `run_gui_host` / `run_cage_host` collapse into `run_hosted(config, host)`:
  `ensure_seatd()`, then the fork/waitpid/exit-code loop above. The
  `write_greeter_conf` + `margo`/`cage` command construction moves into the
  runner.
- The TTY fallback becomes `run_tty_host`: create the socketpair, fork the
  runner (`Host::Tty`), run `LoginForm` in the daemon over the greeter end.

`mlogind/src/auth/`:
- Delete `ValidatedCredentials`, `try_validate`, `validate_credentials`,
  `open_session` as separate stages. `auth::pam` shrinks to user lookup
  (`uid`, `gids`, `home_dir`, `shell`) plus the error enum.

`mlogind/src/ui/mod.rs`:
- Delete `write_greet_result`, `greet_result_path`, `into_greeter`.
- `LoginForm` gains `conn: Option<Conn<FdTransport>>`. On submit it sends
  `Begin` and pumps events. `Prompt` retargets the input field: `echo == true`
  → the username field, `echo == false` → the password field, with the prompt
  text as the field label. Additional prompts beyond the first pair reuse the
  password widget with the prompt's own label. `Info`/`Error`/`Failure` land in
  the existing status-message line. `Success` exits the TUI so the runner can
  take the VT.
- `--preview` and a bare `mlogind --greet` (no `MLOGIND_SOCK_FD`) keep today's
  dry-run: `conn == None`, submit only animates.

### `mgreet`

- Delete `handoff.rs` entirely.
- `auth.rs`: `trait Authenticate` loses its PAM impl and gains a protocol impl.
  The `pam` dependency is dropped from `mgreet/Cargo.toml`.
- `cache.rs`: the read stays (pre-fill username/session); the *write* moves to
  the runner. A root greeter writing `/var/cache/mlogind` was never right, and
  under A2 it becomes impossible.
- `ui.rs`: the password field becomes a generic prompt field driven by
  `Prompt { echo, text }`. Caps-lock warning, battery, power footer, per-monitor
  windows, hotplug: unchanged.
- The socket is read from the GTK main loop via `glib::unix_fd_add_local`, so
  the UI never blocks on PAM.
- `MLOGIND_SOCK_FD` absent → `--preview` behaviour, as today.

## Error handling

- Greeter EOF mid-conversation → runner aborts the conversation, reaps the host,
  exits `10` (no login). The `gui → cage → tty` ladder is untouched.
- Unknown tag, truncated frame, `len > MAX_FRAME`, non-UTF-8 in a string field →
  `ProtoError`. The runner logs, closes the socket, and exits `10`. Never
  `panic!` — this is the login gate and the panic ratchet is a CI gate.
- `pam_open_session` failure → log, exit non-zero; the daemon loops and the user
  sees the greeter again rather than a black screen.
- The logind-active wait is bounded (2 s, 20 ms poll). On timeout it logs a
  warning and launches the compositor anyway. A greeter must never lock the user
  out because a wait did not converge.
- `Failure` does not tear down the socket. The greeter clears its fields and the
  user retries on the same connection; the runner starts a fresh `pam_start`.

### Waiting for the logind session

Implemented without linking `libsystemd`: `sd_session_is_active(id)` reads
`/run/systemd/sessions/<id>` and checks the session's state. We parse that file
for `STATE=active` (falling back to `ACTIVE=1`), keyed on the `XDG_SESSION_ID`
that `open_session()` has just copied into our environment. The parse is a pure
function over a `&str`, so it is unit-tested against fixtures; the poll wraps it.

If `XDG_SESSION_ID` is unset (a PAM stack without `pam_systemd`), the wait is
skipped entirely.

## Testing

Everything below runs unprivileged, without PAM, without a socket.

- `mlogind-proto`: encode/decode round-trip for every variant; truncated frame;
  `len > MAX_FRAME`; unknown tag; embedded `\n` and NUL inside `Response`;
  empty user/session/secret; non-UTF-8 in a string field is rejected; a
  `Zeroizing` response buffer is zero after drop.
- Conversation engine over the in-memory `Transport`: the `Begin → Prompt →
  Response → Success` happy path; `Failure → Begin` retry on the same transport;
  greeter EOF mid-prompt; a second echo prompt is forwarded rather than
  auto-answered; the first echo prompt is auto-answered from `Begin`.
- `session_active`: pure parse over fixture strings (`active`, `online`,
  `closing`, missing key, missing file).
- `mgreet`: `Event → UI state` as a pure `decide()` function, extending the
  existing `decide_submit`. Prompt retargeting (echo on/off) is covered there.
- `mlogind` TUI: prompt-to-field mapping is a pure function, tested directly.

What cannot be tested here and must be verified on hardware: a real login
through each of the three hosts, a wrong password followed by a correct one on
the same connection, and `Esc` quitting the greeter.

## Rollout risk

A1 rewrites the login path. If it is wrong, the machine does not log in.

- The `gui → cage → tty` ladder is preserved, and all three now share one PAM
  implementation — so a bug is unlikely to hit exactly one host.
- Keep a root shell on a second TTY for the first reboot after this lands.
- `mlogind --preview` still exercises the TUI without forking, so the widget
  changes can be eyeballed from a normal terminal.

`mlogind` gains no new runtime dependency (`zeroize` is already in the
workspace). `Cargo.lock` must be regenerated and committed in the same commit:
AUR `makepkg` builds `--locked`.

## Deleted by this change

`mgreet/src/handoff.rs`; `mlogind`'s `Hooks`, `start_session`, `session_child`,
`StartSessionError`, `ValidatedCredentials`, `try_validate`,
`read_and_shred_greet_result`, `GreetResult`, `launch_greet_result`,
`write_greet_result`, `into_greeter`, `MLOGIND_RESULT_PATH`, and the
`std::mem::forget` across `fork()`.
