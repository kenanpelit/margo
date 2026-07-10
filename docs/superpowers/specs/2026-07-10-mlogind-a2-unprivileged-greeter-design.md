# mlogind A2 — the greeter stops being root

Date: 2026-07-10
Status: approved, ready for implementation
Scope: `mlogind`, `mlogind-proto`, `mgreet`, packaging
Depends on: [A1](2026-07-10-mlogind-a1-privileged-auth-design.md) (shipped, `06554603`)

## Why

The login screen is the most exposed surface a desktop has, and ours runs every
line of it as root: a root `margo`, a root GTK4, root `gdk-pixbuf` decoding
whatever image the theme points at, root `grass`-compiled CSS, root Pango
shaping a hostname read off the network.

atrium runs its greeter as a dedicated system user and lets logind hand it the
seat's DRM device (`daemon/session/greeter.c`). SDDM does the same with the
`sddm` user. We are the outlier.

A1 removed the reason the greeter *needed* privilege: it no longer runs PAM and
no longer writes a credential file. What remains is inertia and one unverified
assumption — that `margo` can open DRM through libseat's logind backend rather
than the `seatd` shim we start by hand.

## The shape

atrium creates the greeter's logind session by calling `CreateSession` over
D-Bus directly, and says why: *"`pam_systemd` would tie the session to the
calling process, and we would have to fork another dedicated subprocess just for
this purpose."*

That reason does not apply to us. A1 already forks a session runner per login.
One more fork is free, and taking the PAM route means we do not pull `zbus` +
`zvariant` + `async-io` into a login manager whose whole virtue is being small
and obvious. This is SDDM's structure (`src/helper/HelperApp.cpp`,
`src/helper/UserSession.cpp`).

```
runner (root)                       ── owns the user's PAM conversation (A1)
  │
  └─ fork() → greeter session (root)
        pam_start("mlogind-greeter", greeter_user)
        XDG_SEAT=seat0  XDG_VTNR=<tty>
        XDG_SESSION_TYPE=wayland  XDG_SESSION_CLASS=greeter
        authenticate()            ← a pam_permit stack; nothing is asked
        open_session()            ← THIS pid is the logind session leader
        session_active::wait()    ← logind grants DRM asynchronously (A1's wait)
        │
        └─ Command::new(margo)    ← setgroups + setgid + setuid, then exec
              wait()
        Drop<Authenticator>       ← pam_close_session + setcred(DELETE) + pam_end
        exit(status)
  │
  runner: waitpid(greeter session)
          session_active::wait_vt_free(tty)   ← the user session wants this VT
          … pam_open_session for the user, as in A1
```

`margo` inherits the greeter session's cgroup, so `sd_pid_get_session()` resolves
for it and libseat's logind backend works. **`ensure_seatd()` and
`LIBSEAT_BACKEND=seatd` are deleted.** Their only reason for existing was the
comment above them: *"the root orchestrator has no logind session."* Now it does.

### VT reuse

atrium holds one VT per seat and runs the greeter session and the user session on
it in sequence. So do we — `config.tty`, `chvt`'d at startup. logind must have
torn the greeter's session down before `pam_systemd` creates the user's on the
same `VTNr`.

`pam_close_session` in the greeter-session process, followed by its exit, is what
triggers that; but logind acts asynchronously. So the runner polls
`/run/systemd/sessions/*` until no session file claims our `VTNr`, bounded at 2 s,
then proceeds anyway with a warning. Same discipline as A1's `session_active`: a
greeter never refuses to proceed because a wait did not converge. The scan is a
pure function over `(filename, contents)` pairs, so it is unit-tested.

### Never lock the user out

`greeter_user` is a new `config.toml` knob, defaulting to `"mlogind-greeter"`.

- Empty string → no privilege drop. The greeter runs as root, as it does today.
- Set but absent from `/etc/passwd` (a package upgrade where `systemd-sysusers`
  never ran) → **log an error and run as root anyway**. A machine that cannot be
  logged into is a worse outcome than a greeter with too much privilege, and the
  error is loud.

The logind session is created either way; only the `setuid` is conditional.

## What deprivileging breaks, and how

Each of these is a consequence, not a nice-to-have. The greeter does not start
without them.

### 1. Power actions

`mgreet/src/power.rs` and `mlogind/src/ui/key_menu.rs` both run the configured
command with `sh -c`. An unprivileged greeter cannot `systemctl poweroff`.

The greeter must also never hand the root runner an arbitrary command string —
that would give back, in one line, exactly the privilege we just took away. So:

- **`Request::Power { index: u32 }`.** An index into the runner's own resolved
  `power_controls` (`base_entries` then `entries`, in that order). The runner
  looks the command up in its own config and runs it. The greeter cannot name a
  command that is not already in `/etc/mlogind/config.toml`.
- The runner always replies — `Event::Info` on success, `Event::Error` on
  failure — so a greeter that blocks for the answer (the TUI) cannot hang. Most
  power actions never get to see the reply.
- `MLOGIND_POWER` loses its `cmd` field: `key<TAB>hint` per line, index implied
  by line order. There is no reason to ship root commands into an unprivileged
  process's environment.
- Power keys are refused while a conversation is in flight. Otherwise a `Power`
  frame lands in the runner's PAM conversation callback, where it does not
  belong. The runner ignores one defensively and keeps waiting for its prompt
  answer; `mgreet` gates on `conversing`; the TUI cannot even reach the keyboard
  mid-`pump`.
- `--preview` no longer executes power commands. It never should have —
  `mlogind --preview` plus F1 shut the machine down. `mgreet` already gated this
  (`power_live`); the TUI did not.

Both greeters now route power through the runner, including the root TTY greeter.
One path.

We also ship `usr/share/polkit-1/rules.d/60-mlogind-greeter.rules` granting the
greeter user `login1.power-off`/`reboot`/`suspend`/`hibernate`, the way GDM and
SDDM do. It is not on the greeter's own path — the protocol is — but it makes any
other tool run under the greeter session (a future lock screen, an accessibility
helper) behave, and it costs one file.

### 2. Filesystem

| Path | Was | Now | Why |
|---|---|---|---|
| `/run/mlogind` | 0700 root | 0755 root | `margo` must read `greeter.conf`. A1 removed the only secret that ever lived here. |
| `/run/mlogind/greeter.conf` | 0600 | 0644 | keyboard layout, nothing else |
| `/run/mlogind/mgreet.log` | shell `2>` redirect | gone | the redirect needed write access to the dir. `mgreet`'s stderr now flows through `margo`'s, into `margo-greeter.log` — which the *runner* opens as root and passes as an inherited fd, so the greeter needs no directory write at all. |
| `/var/cache/mlogind` | root, 0644 by umask | root, explicit 0644 | the greeter pre-fills the last username from it. Relying on the daemon's inherited umask is not a permission model. |
| `$XDG_RUNTIME_DIR` | `/run/mlogind` | `/run/user/<uid>`, from `pam_systemd` | stop overriding it |
| `config.client_log_path` | `/var/log/mlogind.client.log` | `$XDG_RUNTIME_DIR/mlogind-greeter.log` when hosted | root-only path; a hosted `mlogind --greet` cannot write it |

And `setup_logger` stops calling `std::process::exit(1)` when it cannot open its
file. It warns on stderr and runs without a log. A login manager that refuses to
start because a log is unwritable is a lockout with extra steps.

### 3. Packaging

- `mlogind/extra/mlogind-greeter.sysusers` → `/usr/lib/sysusers.d/mlogind.conf`,
  creating `mlogind-greeter` with no shell, no home, no supplementary groups.
  logind's ACLs grant `/dev/dri/*` and `/dev/input/*` to the *active session*;
  membership in `video`/`input` is not needed and would be a downgrade.
- `mlogind/extra/mlogind-greeter.pam` → `/etc/pam.d/mlogind-greeter`.
- `mlogind/extra/60-mlogind-greeter.rules` → the polkit rule.
- `PKGBUILD` (`backup=` for the pam file, `install -Dm644` for all three) and
  `install.sh`.

The PAM stack is a `pam_permit` auth with a real session:

```
auth      required  pam_env.so
auth      required  pam_permit.so
account   required  pam_permit.so
password  required  pam_deny.so
session   optional  pam_keyinit.so force revoke
session   required  pam_limits.so
session   required  pam_loginuid.so
-session  optional  pam_systemd.so
```

`pam_systemd` is what creates the session. `pam_loginuid` is what makes audit
attribute the greeter's actions to it. Nothing authenticates: the greeter user is
not a login.

## Changes per crate

**`mlogind-proto`** — `Request::Power { index: u32 }` (tag 4). Encode/decode +
tests: round-trip, an index at `u32::MAX`, and that it is rejected by
`decode_event` (wrong direction).

**`mlogind`**
- `runner/greeter_session.rs` (new): the forked greeter-session body. PAM, the
  `XDG_*` environment, `session_active::wait()`, the privilege-dropping
  `Command`, `waitpid`, `pam_close_session`.
- `runner/mod.rs`: `spawn_greeter` becomes a `fork()` rather than a `Command`;
  `Greeter { pid, log }` and a hand-rolled reap. The greeter-session child closes
  its inherited copy of the runner's socket end immediately — it is `CLOEXEC`, but
  a `fork` is not an `exec`. `serve()` grows a `Power` arm. `ensure_seatd` and the
  `LIBSEAT_BACKEND` env go.
- `runner/session_active.rs`: `wait_vt_free(vtnr)` + `vt_of(contents)`.
- `runner/converse.rs`: `Request::Power` mid-prompt is logged and ignored, and the
  conversation keeps waiting for its answer.
- `auth.rs`: `lookup_greeter(config)` → `Option<UserInfo>`, loud on a missing user.
- `config.rs`: `greeter_user`, `greeter_pam_service`.
- `ui/key_menu.rs`: `key_press` → `power_index(key) -> Option<usize>`. It stops
  owning a `Command` and a `system_shell`.
- `ui/mod.rs`: an F-key with a `conn` sends `Power`, then blocks for one
  `Info`/`Error`. Without a `conn` (preview) it does nothing.
- `main.rs`: hosted `--greet` logs to `$XDG_RUNTIME_DIR`; `setup_logger` is
  non-fatal.
- `info_caching.rs`: `set_cache` chmods 0644.

**`mgreet`**
- `power.rs`: `PowerAction { key, hint }`, no `cmd`. `parse_power` yields indices.
- `ui.rs`: an F-key sends `Request::Power { index }` when `real() && !conversing`.

## Error handling

- Greeter user missing → error log, no `setuid`, greeter runs as root. Not fatal.
- `pam_open_session` for the greeter fails → the runner exits
  `EXIT_HOST_UNAVAILABLE`, and the daemon falls down the `gui → cage → tty`
  ladder exactly as it does for a missing binary.
- `wait_vt_free` times out → warn, proceed. logind will refuse or reuse; either
  way the user sees a greeter again rather than a black screen.
- `Power` with an out-of-range index → `Event::Error`, nothing runs. The greeter
  and the runner derive the list from the same config in the same order, so this
  only fires if they disagree — which is worth saying out loud.
- A `Power` frame arriving while PAM holds a prompt open → warn, ignore, keep
  waiting. It cannot be answered as a prompt response and must not abort the
  attempt.

## Testing

Unprivileged, no PAM, no socket, no root:

- `mlogind-proto`: `Power` round-trips; a `u32::MAX` index survives; a `Power`
  frame is `UnknownTag` to `decode_event`.
- `session_active::vt_of`: `VTNr=1`, absent key, non-numeric, whitespace.
  `sessions_on_vt` over a fixture list of `(name, contents)`.
- `power_index`: the base list then the extra list, in order; an unbound key is
  `None`; a duplicate key resolves to the first.
- `mgreet::power::parse_power`: two fields now, index = line order; a line with
  a trailing tab; blank lines skipped.
- `auth::lookup_greeter`: an empty `greeter_user` is `None` without a lookup; a
  missing user is `None`; `root` resolves.

Hardware, and only hardware:

1. `margo` opens DRM with no `seatd` running at all (`systemctl stop seatd`).
2. `loginctl` shows the greeter session with `Class=greeter` and the right VT.
3. `ps -o user= -C mgreet` prints `mlogind-greeter`.
4. F1 shuts down from `mgreet`, and from `mlogind --greet` under cage.
5. Log in, log out, log in again — the second greeter session gets the VT back.
6. `mlogind --preview`, press F1, nothing happens.
7. Delete the `mlogind-greeter` user; the greeter still comes up, as root, and
   says so in the log.

## Rollout risk

This is the risky half of A2, and it is two risks at once — which is what the
staging in the A1 spec was meant to avoid, and which was overruled deliberately.
So: if the first boot after this fails, the two candidates are

- `margo` cannot open DRM under libseat's logind backend (look for `libseat` in
  `/run/mlogind/margo-greeter.log`), or
- the greeter user cannot read something (look for `EACCES`).

The `gui → cage → tty` ladder still holds, and the TTY greeter runs as the root
daemon and needs none of this. **Keep a root shell on a second TTY.**
