# Wayland session integration examples

Reference files that wire margo into a display manager + UWSM session.
None of these are installed by the package; copy the ones you want.

The layout assumes a standard FHS install (margo at `/usr/bin/margo`,
session wrappers at `/usr/local/bin/`), but everything is path-agnostic.

| File | Install path | Purpose |
|---|---|---|
| `margo-uwsm.desktop` | `/usr/share/wayland-sessions/` | Wayland session entry — picked up by gdm / sddm / ly / greetd-tuigreet from the standard `wayland-sessions/` location. References `margo-uwsm-session`. |
| `margo-uwsm-session` | `/usr/local/bin/`, `chmod +x` | UWSM-first wrapper. Sets the standard XDG env vars, resolves the best compositor command (`margo-session` > `start-margo` > `margo`), and hands control to `uwsm start` for a transient-scope session. |
| `margo-session` | `/usr/local/bin/`, `chmod +x` | Minimal compositor launcher. Prefers `start-margo` (the watchdog supervisor) when installed; falls back to bare `margo`. Pass-through for extra arguments. |
| `systemd/user/wayland-wm@margo-session.service.d/10-session-lifecycle.conf` | `~/.config/systemd/user/wayland-wm@margo\x2dsession.service.d/10-session-lifecycle.conf` | UWSM `wayland-wm@.service` drop-in: sets `MARGO_LOG`, fires the session-target fan-out, bumps Nice/CPUWeight. |

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
      └─ uwsm start -D margo:mango -e -- margo-session
           └─ /usr/local/bin/margo-session
                └─ exec start-margo -- -c ~/.config/margo/config.conf
                     └─ fork+exec margo  (PR_SET_PDEATHSIG, watchdog loop)
```

## Tailoring

Distros that ship a richer integration (theme defaults, manager-env
scrubbing, helper-script sourcing) should treat `margo-uwsm-session`
as a starter and layer their logic on top. The other three files are
small enough to use verbatim in nearly every setup.

If you don't want the watchdog at all (rare — useful only for
profiling a single session), point the `.desktop` `Exec=` line
directly at `margo -c …` and skip the wrappers entirely.
