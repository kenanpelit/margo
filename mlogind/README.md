# mlogind

margo's standalone **TUI login / display manager** — a bare-TTY greeter
that authenticates with PAM and launches your X11 / Wayland session
(including margo). It runs on the console itself; no compositor needs to
be running to log in.

> **Fork.** mlogind is a fork of
> [lemurs](https://github.com/coastalwhite/lemurs) by Gijs Burghoorn
> (MIT OR Apache-2.0 — see `LICENSE-MIT` / `LICENSE-APACHE`), brought
> under the margo workspace and being adapted + improved for it. Upstream
> credit and the dual license are preserved.

## Status

Work in progress. The import builds as the `mlogind` crate and the
internal `lemurs` → `mlogind` rename is done (config dir, PAM service,
cache/log paths, CLI). Margo-integration improvements — matugen theming,
shared auth with `mlock`, better session detection, fingerprint / u2f —
are tracked as follow-ups.

## How it works

1. A systemd service (`extra/mlogind.service`) runs `mlogind` as root on a
   dedicated VT.
2. mlogind draws a `ratatui` TUI: user + session switcher + password.
3. It authenticates the chosen user through PAM (service `mlogind`,
   configured at `/etc/pam.d/mlogind`).
4. On success it sets up the environment + utmpx record and execs the
   selected session (`/usr/share/{wayland-sessions,xsessions}` entries,
   or the script dirs below), returning to the greeter when it exits.

## Paths

| Purpose | Default |
|---|---|
| Main config | `/etc/mlogind/config.toml` |
| Variables | `/etc/mlogind/variables.toml` |
| Wayland session scripts | `/etc/mlogind/wayland` |
| WM / X session scripts | `/etc/mlogind/wms` |
| X setup script | `/etc/mlogind/xsetup.sh` |
| Last user / session cache | `/var/cache/mlogind` |
| Logs | `/var/log/mlogind*.log` |
| PAM service | `/etc/pam.d/mlogind` (template: `extra/mlogind.pam`) |

## Usage

```bash
# Preview in an existing session (no root, no real login):
mlogind --preview

# Real use: install extra/mlogind.service + extra/mlogind.pam +
# extra/config.toml, disable your current display manager, enable mlogind.
```

See `extra/config.toml` for the full set of customization options.
