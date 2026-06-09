# mvpn

Native Mullvad VPN control for margo — one binary that is both a full CLI and a
GTK4 layer-shell control panel. Drives the `mullvad` CLI directly; no daemon of
its own (the Mullvad daemon + `~/.mullvad/*` files are the source of truth).

## CLI

```sh
mvpn                       # status (no args)
mvpn status [--pill|-v|--json]
mvpn connect | disconnect | toggle | reconnect
mvpn de                    # random relay in a country
mvpn us nyc                # random relay in a city
mvpn random | owned [cc] | rented [cc]
mvpn protocol              # toggle WireGuard ↔ OpenVPN
mvpn fastest [cc]          # ping a sample, connect to the lowest, save to favs
mvpn fastest-fav-sweep europe [n]   # seed favorites across a country group
mvpn fav add|remove <relay>|list|connect|refresh [cc]
mvpn obf [auto|off|udp2tcp|shadowsocks|quic] | obf cycle | obf hunt443
mvpn lockdown on|off       # block traffic when the tunnel drops
mvpn auto-connect on|off
mvpn slot whoami|list|revoke <dev>|recycle [--dry-run]|status|disconnect
mvpn timer start <min> | stop | status   # auto-switch relays
mvpn test                  # leak check (am.i.mullvad.net)
mvpn split                 # split-tunnel excluded processes
mvpn ensure                # blocky DNS-guard fail-safe
mvpn menu                  # open the GTK panel
```

`obfuscation` is the daemon's `anti-censorship` on current Mullvad. Favorites
live in `~/.mullvad/favorites.txt` (`relay|ping`, fastest-first) and the
device-slot state in `~/.mullvad/slot.state` — both osc-mullvad-compatible.

## Bar pill

```sh
mvpn install-pill          # prints a custom_widget snippet for your mshell profile
```

The pill polls `mvpn status --pill` (emits `#active` when connected → accent
tint), left-click opens `mvpn menu`, right-click `mvpn toggle`.

## Panel (`mvpn menu`)

A layer-shell window themed from margo's matugen palette
(`~/.cache/margo/mshell-colors.toml`, dark fallback): status hero, primary
actions, quick chips (Random/Fastest/Protocol/Obf), Lockdown/Auto-connect
switches, ping-sorted favorites, searchable country list, device + leak test.
Esc closes. All `mullvad` calls run off the UI thread.

## Notes

- Privileged steps (blocky `systemctl`, device-slot) use **non-interactive
  sudo** (`sudo -n`) and never a polkit prompt, so a focused panel can't hang.
- The device account number comes from `$MULLVAD_ACCOUNT_NUMBER` or
  `pass show mullvad/account`.

## Build

```sh
cargo build --release -p mvpn
sudo install -m755 target/release/mvpn /usr/bin/mvpn
```
