# Wayland session integration

These files wire margo into a display manager + UWSM session.

As of the current PKGBUILD, the package **installs** the uwsm session
entry, the two wrapper scripts, and the uwsm env file directly (to the
`/usr/bin` + `/etc/xdg` paths below). The systemd drop-in is left as a
copy-in starter (it is environment-specific). For a manual / non-package
install, copy the ones you want.

| File | Install path | Packaged? | Purpose |
|---|---|---|---|
| `margo-uwsm.desktop` | `/usr/share/wayland-sessions/` | yes | Wayland session entry — picked up by gdm / sddm / ly / greetd-tuigreet. References `margo-uwsm-session` (Exec rewritten to `/usr/bin` by the package). |
| `margo-uwsm-session` | `/usr/bin/` (`/usr/local/bin/` for manual installs), `chmod +x` | yes | UWSM-first wrapper. Sets the standard XDG env vars, optionally sources `margo-session-common(.sh)` for richer env cleanup / manager-env sync, resolves the best compositor command (`margo-session` > `start-margo` > `margo`), and hands control to `uwsm start` for a transient-scope session. |
| `margo-session` | `/usr/bin/` (`/usr/local/bin/` for manual installs), `chmod +x` | yes | Minimal compositor launcher. Prefers `start-margo` (the watchdog supervisor) when installed; falls back to bare `margo`. Pass-through for extra arguments. |
| `uwsm-env-margo` | `/etc/xdg/uwsm/env-margo` | yes | uwsm env file sourced for margo sessions (uwsm scans `XDG_CONFIG_DIRS` → `/etc/xdg`). Restores the standard XDG user-bin dirs (`~/.local/bin`, `~/bin`) onto PATH that uwsm's login-shell env rebuild drops, so `uwsm app` keybinds/autostarts can find user-local tools. Per-user overrides go in `~/.config/uwsm/env`. |
| `systemd/user/wayland-wm@margo-session.service.d/10-session-lifecycle.conf` | `~/.config/systemd/user/wayland-wm@margo\x2dsession.service.d/10-session-lifecycle.conf` | no (copy-in) | UWSM `wayland-wm@.service` drop-in: sets `MARGO_LOG`, fires the session-target fan-out, bumps Nice/CPUWeight. |

## Quick install

```bash
# from the cloned repo:
sudo install -m644 contrib/sessions/margo-uwsm.desktop \
    /usr/share/wayland-sessions/margo-uwsm.desktop
sudo install -m755 contrib/sessions/margo-uwsm-session \
    /usr/local/bin/margo-uwsm-session
sudo install -m755 contrib/sessions/margo-session \
    /usr/local/bin/margo-session

# Optional drop-in (matches UWSM's instance-encoded path; the literal
# \x2d is intentional — systemd encodes `-` that way inside template
# instance names):
install -Dm644 \
    "contrib/sessions/systemd/user/wayland-wm@margo-session.service.d/10-session-lifecycle.conf" \
    "$HOME/.config/systemd/user/wayland-wm@margo\\x2dsession.service.d/10-session-lifecycle.conf"
systemctl --user daemon-reload
```

After install, log out and pick **"Margo (UWSM)"** from the DM session
chooser. The chain that runs once you authenticate:

```
DM
 └─ /usr/local/bin/margo-uwsm-session     ← env setup + session_cmd pick
      └─ uwsm start -D margo -e -- margo-session
           └─ /usr/local/bin/margo-session
                └─ exec start-margo -- -c ~/.config/margo/config.conf
                     └─ fork+exec margo  (PR_SET_PDEATHSIG, watchdog loop)
```

## Tailoring

Distros that ship a richer integration can provide `margo-session-common`
or `margo-session-common.sh` in `~/.local/bin`, `/usr/local/bin`, or
`/usr/bin`. When present, `margo-uwsm-session` uses it for runtime-dir
setup, environment.d loading, foreign compositor env scrubbing, PATH /
XDG_DATA_DIRS normalisation, and systemd user-manager env sync. Without
that helper, the wrapper stays minimal and works like the plain template.

If you don't want the watchdog at all (rare — useful only for
profiling a single session), point the `.desktop` `Exec=` line
directly at `margo -c …` and skip the wrappers entirely.
