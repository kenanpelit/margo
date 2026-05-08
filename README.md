# margo

> A fast, dynamically-tiled Wayland compositor written in Rust.

[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-daily%20driver-success)](road_map.md)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](Cargo.toml)
[![Smithay](https://img.shields.io/badge/built%20on-Smithay-blueviolet)](https://github.com/Smithay/smithay)

margo is a feature-complete Wayland compositor — a Rust + [Smithay] port of [mango], in the dwl/dwm tradition. It keeps the **tag** model (no workspaces), ships **15 tiling layouts**, and runs on Smithay's modern protocol stack. Every commit goes through a daily driver before tagging.

[Smithay]: https://github.com/Smithay/smithay
[mango]: https://github.com/mangowm/mango

---

## Highlights

- **15 layouts** — `tile`, `scroller`, `grid`, `monocle`, `deck`, `center_tile`, `right_tile`, vertical mirrors, `tg_mix`, `canvas`, `dwindle`, plus a full overview. Each tag remembers its own choice.
- **9 multi-select tags**, dwm-style — press the same tag twice to bounce back, pin tags to a home monitor (`tagrule = id:N, monitor_name:DP-3`), tag windows by regex.
- **Spring physics + bezier curves** — niri-style move animation with mid-flight retarget velocity preservation, bezier curves for open/close/tag/focus/layer.
- **Real protocol stack** — DMA-BUF screencopy, `pointer_constraints`, `relative_pointer`, `xdg_activation`, `wlr_output_management` (incl. runtime mode change), `presentation-time` with VBlank-accurate timestamps, drop shadows.
- **Window rules** with PCRE2 regex on `app_id`/`title` — floating geometry, tag pinning, screencast block-out for password managers, terminal swallowing.
- **Embedded Rhai scripting** — drop a `~/.config/margo/init.rhai`; call any compositor action from script, hook `on_focus_change` / `on_tag_switch` / `on_window_open`.
- **Hot-reload everything** — `mctl reload` (or Super+Ctrl+R) re-applies window rules, key binds, monitor topology, animation curves, and gestures without restarting your session.
- **DRM hotplug** that actually works — dock or undock a laptop, plug a second monitor; outputs come and go without a logout.
- **`mctl` IPC** speaks `dwl-ipc-unstable-v2` — drop-in for noctalia, waybar-dwl, fnott, and any dwl/mango widget.

## Install

### Arch (PKGBUILD)

```bash
git clone https://github.com/kenanpelit/margo_build ~/.kod/margo_build
cd ~/.kod/margo_build && makepkg -si
```

### From source

```bash
git clone https://github.com/kenanpelit/margo
cd margo && cargo build --release
sudo install -Dm755 target/release/margo /usr/bin/margo
sudo install -Dm755 target/release/mctl  /usr/bin/mctl
sudo install -Dm644 margo.desktop /usr/share/wayland-sessions/margo.desktop
```

System dependencies: `wayland`, `libinput`, `libxkbcommon`, `seatd`, `mesa`, `libdrm`, `pixman`, `pcre2`, `xorg-xwayland` (optional).

### Nix flake

```bash
nix run github:kenanpelit/margo
```

The flake exposes `packages.default`, a `devShells.default` with `rust-analyzer` + `clippy`, plus `nixosModules.margo` and `hmModules.margo`.

## Configure

`~/.config/margo/config.conf` — text, `key = value`, hot-reloadable. A complete example with every binding lives in [`margo/src/config.example.conf`](margo/src/config.example.conf).

```ini
# Look
borderpx        = 3
border_radius   = 12
gappih          = 12
gappiv          = 12
focused_opacity   = 1.0
unfocused_opacity = 0.9

# Tags 1–6 home on DP-3, 7–9 home on eDP-1
tagrule = id:1, layout_name:scroller, monitor_name:DP-3
tagrule = id:7, layout_name:scroller, monitor_name:eDP-1

# Pin Helium to tag 1
windowrule = tags:1, appid:^Kenp$
# Float password prompts
windowrule = isfloating:1, width:640, height:260, \
             title:^(Authentication Required|Unlock Keyring)$

# Animations — niri-style
animation_clock_move = spring     # mid-flight velocity preserved
animation_clock_tag  = bezier     # snap-stop, no overshoot
animation_curve_open = 0.16,1.0,0.30,1.0   # ease-out-expo

# Keys
bind = super,       Return, spawn, kitty
bind = super,       q,      killclient
bind = super,       space,  spawn, qs -c noctalia-shell ipc call launcher toggle
bind = super+ctrl,  s,      sticky_window
bind = super+ctrl,  r,      reload_config
```

Validate before reloading:

```bash
mctl check-config
```

## IPC — `mctl`

```bash
mctl status                          # JSON: monitors, tags, focused window
mctl status --json | jq '.outputs[]'
mctl watch                           # follow state changes (great for bars)
mctl actions --verbose               # list every dispatchable action
mctl dispatch togglefullscreen
mctl dispatch view 4                 # tag bitmask 4 = tag 3
mctl rules --appid Kenp              # show which window rules match
mctl reload                          # hot-reload config
```

Actions are the same string keys you bind in `config.conf` (`spawn`, `view`, `tag`, `setlayout`, `togglefloating`, `zoom`, `focusmon`, `toggleoverview`, `sticky_window`, …). Shell completions for bash / zsh / fish ship with the package.

## Scripting

Drop `~/.config/margo/init.rhai` and margo evaluates it at startup:

```rhai
// Auto-tag Spotify into tag 8
on_window_open(|| {
    if focused_appid() == "spotify" {
        dispatch("tagview", [tag(8)]);
    }
});

// Quiet bar when entering tag 9
on_tag_switch(|| {
    if current_tag() == 9 {
        spawn("pkill -SIGUSR1 waybar");
    }
});
```

Engine: [Rhai] (Rust, sandboxed). Full reference: [`docs/scripting-design.md`](docs/scripting-design.md).

[Rhai]: https://rhai.rs

## Documentation

- **[`road_map.md`](road_map.md)** — what's shipped, what's queued, design tradeoffs.
- **[`docs/`](docs/)** — design plans for in-flight features (HDR / portal / scripting) and the post-install validation checklist.
- `mctl --help` and `mctl actions --verbose` — full action + binding reference, generated from source.

## Acknowledgements

[Smithay] (compositor toolkit) · [niri](https://github.com/YaLTeR/niri) (focus oracle, hotplug, transactional resize patterns) · [mango](https://github.com/mangowm/mango) (feature inventory, IPC surface, default keybinds) · [dwl](https://codeberg.org/dwl/dwl) (the original dwm-on-wlroots) · [anvil](https://github.com/Smithay/smithay/tree/master/anvil) (Smithay's reference compositor) · [Hyprland](https://hypr.land) (color-management protocol shape).

Original portions of dwl, dwm, sway, tinywl, and wlroots are preserved under their respective licenses — see `LICENSE.*`.

## License

GPL-3.0-or-later. See [`LICENSE`](LICENSE).
