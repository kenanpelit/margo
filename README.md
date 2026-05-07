# margo

A feature-rich, dynamically tiled Wayland compositor written in Rust on top of
[Smithay](https://github.com/Smithay/smithay). margo is a daily-driver fork of
[mango](https://github.com/mangowm/mango) (originally C/wlroots) that has been
re-implemented from scratch in Rust — without giving up the dwl-style tag
model, the rich layout family, or the IPC surface that downstream widgets
already depend on.

[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
![Status](https://img.shields.io/badge/status-P0%20complete%20%E2%80%94%20daily%20driver-success)
![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)

> **Status:** P0 (reliable daily session) is complete. P1 is the modern
> desktop-protocol parity sprint — DMA-BUF screencopy, pointer constraints,
> output management, `linux_dmabuf`, presentation-time. See
> [`YOL_HARITASI.md`](YOL_HARITASI.md) for the full roadmap (Turkish).

---

## Highlights

- **15 layouts** out of the box — `tile`, `scroller`, `grid`, `monocle`,
  `deck`, `center_tile`, `right_tile`, `vertical_*` mirrors, `tg_mix`,
  `canvas`, `dwindle`, `overview` — each per-tag, with master/stack control,
  configurable gaps, and animated transitions.
- **Tag model, not workspaces.** Nine tags, multi-select, per-tag layout +
  mfact + nmaster snapshots, per-tag *home monitor* pinning
  (`tagrule = id:N, monitor_name:DP-3`), and dwm-style "press the same tag
  twice to bounce back" behaviour.
- **Window rules** with PCRE2-compatible regexes on `app_id`/`title`,
  size constraints (min/max), forced floating + geometry, screencast
  blackout flag, late `app_id`-driven re-apply, and live reload.
- **Animations.** Bezier-baked move / open / close / tag / focus curves,
  size-snap on tile rearrange so Electron clients (Helium, Spotify,
  Discord) don't shift under their border.
- **XWayland** for legacy clients, alongside native xdg-shell.
- **Live config reload** via `mctl reload` — no logout, no restart,
  picks up window rules and re-applies them to mapped windows.
- **Hot-pluggable monitors.** Plug or unplug a display; outputs come
  and go without a session restart, lock surfaces and layer surfaces
  re-anchor automatically.

## Wayland protocols

| Category | Protocols |
|---|---|
| Core | `wl_compositor`, `wl_subcompositor`, `wl_shm`, `wl_seat`, `wl_output`, `wl_data_device_manager`, `viewporter`, `linux_dmabuf` (input only — egress is on the P1 list) |
| Shell | `xdg_shell`, `xdg_decoration_v1`, `wlr_layer_shell_v1`, `xwayland_shell_v1` |
| Input | `wp_text_input_v3`, `zwp_input_method_v2` (Qt password fields, Fcitx5, IBus) |
| Selection | `wlr_data_control_v1`, `primary_selection_v1` |
| Idle / lock | `ext_idle_notifier_v1`, `zwp_idle_inhibit_manager_v1`, `ext_session_lock_v1` |
| Capture / display | `wlr_screencopy_v1` (SHM), `wlr_gamma_control_v1`, `wlr_output_power_management_v1`, `xdg_output_v1` |
| IPC / surfaces | `dwl_ipc_unstable_v2`, `ext_workspace_v1`, `foreign_toplevel_list_v1`, `wlr_foreign_toplevel_management_v1` |

## Build

### Arch (PKGBUILD)

A reference [PKGBUILD](https://github.com/kenanpelit/margo_build) is maintained
out-of-tree. The standard flow is:

```bash
git clone https://github.com/kenanpelit/margo_build ~/.kod/margo_build
cd ~/.kod/margo_build
makepkg -si
```

### From source (cargo)

```bash
git clone https://github.com/kenanpelit/margo
cd margo
cargo build --release
sudo install -Dm755 target/release/margo /usr/bin/margo
sudo install -Dm755 target/release/mctl  /usr/bin/mctl
sudo install -Dm644 margo.desktop \
    /usr/share/wayland-sessions/margo.desktop
```

Runtime dependencies match the PKGBUILD: `libinput`, `libxkbcommon`,
`wayland`, `mesa`, `seatd`, `pixman`, `libdrm`, `systemd-libs`, `pcre2`,
`xorg-xwayland` (optional but recommended).

### Nix (flake)

```bash
nix run github:kenanpelit/margo
```

The flake exposes `packages.default`, a `devShells.default` with
`rust-analyzer` and `clippy`, plus `nixosModules.margo` and
`hmModules.margo`. See [`nix/`](nix/) and [`flake.nix`](flake.nix).

## Configure

margo reads `~/.config/margo/config.conf` (text, `key = value`).
A curated example shipping every keybind / window rule / tagrule
the maintainer uses is at
[`margo/src/config.example.conf`](margo/src/config.example.conf) and is
also installed to `/usr/share/doc/margo-git/config.example.conf`.

```ini
# Looks
borderpx       = 3
border_radius  = 12
gaps           = 6
focused_opacity   = 1.0
unfocused_opacity = 0.9

# Tags 1–6 live on DP-3, 7–9 on eDP-1.
tagrule = id:1, layout:tile,     monitor_name:DP-3
tagrule = id:7, layout:scroller, monitor_name:eDP-1

# Force Helium onto tag 1 (its tag rule supplies the monitor)
windowrule = tags:1, appid:^Kenp$
windowrule = isfloating:1, width:640, height:260, \
             title:^(Authentication Required|Unlock Keyring)$

# Common keybinds
bind = super,        Return, spawn, kitty
bind = super,        q,      killclient
bind = super,        space,  spawn, qs -c noctalia-shell ipc call launcher toggle
bind = alt,          l,      spawn, qs -c noctalia-shell ipc call lockScreen lock
bind = super+shift,  r,      reload_config
```

## IPC — `mctl`

`mctl` is the in-tree IPC client. It speaks `dwl-ipc-unstable-v2` so existing
dwl/mango ecosystem widgets (status bars, screen-lockers, OSDs) work
unchanged.

```bash
mctl status                   # JSON dump of monitors / tags / clients
mctl watch                    # follow state changes (good for bars)
mctl dispatch togglefullscreen
mctl tags 0x02                # view tag 2
mctl reload                   # hot-reload config
mctl quit                     # graceful shutdown
```

The dispatch surface is the same string keys you bind in `config.conf`
(`spawn`, `view`, `tag`, `setlayout`, `togglefloating`, `zoom`,
`focusmon`, `toggleoverview`, …). Run `mctl --help` for the full list.

## Architecture

```
margo/                 # workspace root
├── margo/             # compositor binary
│   └── src/
│       ├── main.rs            # entry, calloop wiring, panic hook
│       ├── state.rs           # MargoState, all delegate impls
│       ├── input_handler.rs   # libinput → smithay seat plumbing
│       ├── dispatch/          # action name → state.rs method
│       ├── layout/            # 15 tiling algorithms
│       ├── animation/         # bezier curve baking + ticking
│       ├── render/            # rounded borders + clipped surfaces
│       ├── protocols/         # dwl-ipc, ext-workspace, foreign-toplevel,
│       │                      # gamma-control, screencopy
│       ├── input/             # keyboard / pointer / touch / gestures
│       ├── backend/           # winit + udev (DRM + libinput)
│       └── config.example.conf
├── margo-config/      # text config parser (PCRE2 via `regex`)
├── margo-ipc/         # mctl client + shared IPC types
├── protocols/         # Wayland XML for non-upstream protocols
├── nix/               # flake module + NixOS / home-manager configs
├── scripts/           # screenshot, smoke-rules.sh windowrule tester
└── docs/              # extended user docs (mango-derived)
```

The compositor is a single Smithay event loop driven by `calloop`. State
lives in `MargoState`; a `MargoClient` carries per-toplevel geometry,
animation state, tag mask, and rule-derived flags. `MargoMonitor` carries
a `Pertag` block so each tag remembers its own layout / mfact /
nmaster / view selection.

For a deeper tour see [`CLAUDE.md`](CLAUDE.md).

## Roadmap

P0 (reliable daily session) is **done** — `ext_session_lock_v1`,
`ext_idle_notifier_v1`, DRM hotplug (plug + unplug), interactive move /
resize, structured panic + `SIGUSR1` state-dump diagnostics, and a smoke
test harness for window rules.

The P1 sprint focuses on protocol parity with niri / sway: DMA-BUF
screencopy, region-cropped capture, screencast blackout filter,
`pointer_constraints_v1`, `xdg_activation_v1`,
`wlr_output_management_v1`, and `presentation-time`. The exact ordering
and rationale lives in [`YOL_HARITASI.md`](YOL_HARITASI.md).

## Acknowledgements

margo stands on the shoulders of:

- **[Smithay](https://github.com/Smithay/smithay)** — Wayland compositor toolkit in Rust.
- **[niri](https://github.com/YaLTeR/niri)** — patterns for keyboard focus refresh, lock-state machine, transactional resize, and hotplug.
- **[mango](https://github.com/mangowm/mango)** — feature inventory, IPC surface, default keybinds.
- **[dwl](https://codeberg.org/dwl/dwl)** — the original dwm-on-wlroots that the tag model and dispatch shape come from.
- **[anvil](https://github.com/Smithay/smithay/tree/master/anvil)** — Smithay's reference compositor; the input-method handler and X11 wiring trace back to it.
- **[scenefx](https://github.com/wlrfx/scenefx)** — visual effects reference (mango's blur / shadow / radius work).

Original portions of dwl, dwm, sway, tinywl, and wlroots are preserved
under their respective licenses; see `LICENSE.*` files.

## License

GPL-3.0-or-later. See [`LICENSE`](LICENSE).
