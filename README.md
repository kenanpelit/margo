<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/margo-banner.svg">
    <img src="docs/assets/margo-banner-dark.svg" alt="margo" width="600">
  </picture>
</p>

<p align="center">
  <em>A modern Wayland tiling compositor — Rust + Smithay, with a tag-based workflow.</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg" alt="License"></a>
  <a href="road_map.md"><img src="https://img.shields.io/badge/status-daily%20driver-success" alt="Status"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust"></a>
  <a href="https://github.com/Smithay/smithay"><img src="https://img.shields.io/badge/built%20on-Smithay-blueviolet" alt="Smithay"></a>
</p>

**margo** is a Wayland compositor in the dwl/dwm tradition — a Rust + [Smithay] port of [mango]. Tags instead of workspaces, a deep tiling-layout catalogue, and a small set of companion CLIs (`mctl`, `mlayout`, `mscreenshot`) that turn the compositor into a scriptable workstation. Built and used as a daily driver: every commit ships through a real session before tagging.

[Smithay]: https://github.com/Smithay/smithay
[mango]: https://github.com/mangowm/mango

---

## Highlights

- **Tags, not workspaces.** Nine multi-select tags, dwm-style: press the same tag twice to bounce back, OR several together to view a union, pin tags to a home monitor, regex-match windows into tags at map time.
- **Layouts that remember.** Tile, scroller, grid, monocle, deck, dwindle, plus center / right / vertical mirrors and a global overview. Each tag holds its own layout choice; switch tags and the layout follows.
- **Animations done right.** Niri-style spring physics with mid-flight retarget for window movement; carefully-tuned bezier curves for open / close / tag / focus / layer transitions. Drop shadows, rounded corners, focus-fade opacity.
- **Modern protocol stack.** DMA-BUF screencopy, `pointer_constraints` + `relative_pointer` for FPS games, `xdg_activation` with anti-focus-steal, runtime `wlr_output_management` (mode + position changes apply live), VBlank-accurate `presentation-time`, `wp_color_management_v1` for HDR-aware clients.
- **Window rules with PCRE2.** Float password prompts, pin apps to tags, screencast-blackout password managers, swallow terminal children, force CSD per-app — all by `app_id` / `title` regex.
- **In-compositor screencast portal.** Five Mutter D-Bus shims + a PipeWire pipeline so xdp-gnome serves Window / Entire-Screen tabs in browser meeting clients without gnome-shell.
- **Embedded scripting.** Drop `~/.config/margo/init.rhai`; call any compositor action from a sandboxed Rhai interpreter, hook `on_focus_change` / `on_tag_switch` / `on_window_open`.
- **Hot-reload everything.** `mctl reload` (or Super+Ctrl+R) re-applies window rules, key binds, monitor topology, animation curves, gestures — no logout.
- **DRM hotplug that works.** Dock / undock, plug a second monitor mid-session; outputs come and go cleanly.
- **`dwl-ipc-v2` compatibility.** Drop-in for noctalia, waybar-dwl, fnott, and any other dwl/mango widget.

## Companion tools

Margo ships four binaries that share its workspace:

| Binary | What it does |
|---|---|
| **`margo`** | the compositor itself |
| **`mctl`** | IPC and dispatch — `mctl status / clients / outputs / focused / watch / dispatch / tags / layout / reload / actions / rules / check-config` |
| **`mlayout`** | named monitor profiles — `mlayout suggest` writes presets for the detected setup, `mlayout set <name>` flips between them and re-positions outputs via `wlr-randr` |
| **`mscreenshot`** | screen / region / window capture — wraps `grim` + `slurp` + `wl-copy` + an optional editor (`swappy` / `satty`); modes: `rec`, `area`, `screen`, `window`, `open`, `dir` |

Run any of them with `--help` for the full command surface; `mctl actions --verbose` enumerates every dispatchable action with examples.

## Install

### Arch (PKGBUILD)

```bash
git clone https://github.com/kenanpelit/margo_build ~/.kod/margo_build
cd ~/.kod/margo_build && makepkg -si
```

This installs `margo`, `mctl`, `mlayout`, `mscreenshot`, the Wayland-session entry, and the example layouts. Required runtime tools (`grim`, `slurp`, `wl-clipboard`) come in as dependencies; `swappy` / `satty` are optional editors picked up at runtime.

### From source

```bash
git clone https://github.com/kenanpelit/margo
cd margo && cargo build --release --workspace
sudo install -Dm755 target/release/margo        /usr/bin/margo
sudo install -Dm755 target/release/mctl         /usr/bin/mctl
sudo install -Dm755 target/release/mlayout      /usr/bin/mlayout
sudo install -Dm755 target/release/mscreenshot  /usr/bin/mscreenshot
sudo install -Dm644 margo.desktop /usr/share/wayland-sessions/margo.desktop
```

System dependencies: `wayland`, `libinput`, `libxkbcommon`, `seatd`, `mesa`, `libdrm`, `pixman`, `pcre2`, `xorg-xwayland` (optional). Runtime: `grim`, `slurp`, `wl-clipboard` for screenshots; `wlr-randr` for live monitor re-layout.

### Nix flake

```bash
nix run github:kenanpelit/margo
```

The flake exposes `packages.default`, a `devShells.default` with `rust-analyzer` + `clippy`, plus `nixosModules.margo` and `hmModules.margo`.

## Configure

`~/.config/margo/config.conf` — plain `key = value`, hot-reloadable. A complete annotated example lives in [`margo/src/config.example.conf`](margo/src/config.example.conf).

```ini
# Look
borderpx          = 3
border_radius     = 12
gappih            = 12
gappiv            = 12
focused_opacity   = 1.0
unfocused_opacity = 0.9

# Tags 1–6 home on DP-3, 7–9 home on eDP-1
tagrule = id:1, layout_name:scroller, monitor_name:DP-3
tagrule = id:7, layout_name:scroller, monitor_name:eDP-1

# Pin Helium to tag 1; float password prompts
windowrule = tags:1, appid:^Kenp$
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

# Screenshot dispatch — uses mscreenshot under the hood.
bind = NONE,Print,screenshot-region-ui    # region → editor → file + clip
bind = alt, Print,screenshot-window
```

Validate before reloading:

```bash
mctl check-config
```

## At a glance

```bash
# Inspect
mctl status                          # per-output block: focused / tags / layout
mctl clients --tag 2                 # every window on tag 2 (table)
mctl outputs --json | jq '.[].name'
mctl focused                         # `app_id · title`, scriptable

# Drive
mctl dispatch togglefullscreen
mctl dispatch view 4                 # tag bitmask 4 = tag 3
mctl reload

# Layout profiles
mlayout suggest                      # propose & activate a preset for the live setup
mlayout set vertical-ext-top         # apply a saved profile

# Screenshots
mscreenshot rec                      # region → editor → file + clipboard
mscreenshot screen                   # focused output
mscreenshot window                   # focused window
```

## Scripting

Drop `~/.config/margo/init.rhai`; margo evaluates it at startup.

```rhai
// Auto-tag Spotify into tag 8
on_window_open(|| {
    if focused_appid() == "spotify" {
        dispatch("tagview", [tag(8)]);
    }
});

// Tell the bar when entering tag 9
on_tag_switch(|| {
    if current_tag() == 9 {
        spawn("pkill -SIGUSR1 waybar");
    }
});
```

Engine: [Rhai] (pure Rust, sandboxed by default). Full reference: [`docs/scripting-design.md`](docs/scripting-design.md).

[Rhai]: https://rhai.rs

## Documentation

- **[`road_map.md`](road_map.md)** — what's shipped, what's queued, design trade-offs.
- **[`docs/`](docs/)** — design notes for in-flight features (HDR, portal, scripting) and the post-install validation checklist.
- `mctl --help`, `mctl actions --verbose`, `mlayout --help`, `mscreenshot --help` — generated from source, always current.

## Acknowledgements

Built on [Smithay] (compositor toolkit). Patterns and inventory borrowed from [niri](https://github.com/YaLTeR/niri) (focus oracle, hotplug, screencast portal, transactional resize), [mango](https://github.com/mangowm/mango) (feature inventory, IPC surface, default keybinds), [dwl](https://codeberg.org/dwl/dwl) (the original dwm-on-wlroots), [anvil](https://github.com/Smithay/smithay/tree/master/anvil) (Smithay's reference compositor), and [Hyprland](https://hypr.land) (color-management protocol shape).

Original portions of dwl, dwm, sway, tinywl, and wlroots are preserved under their respective licenses — see `LICENSE.*`.

## License

GPL-3.0-or-later. See [`LICENSE`](LICENSE).

<p align="center">
  <img src="docs/assets/margo-icon.svg" alt="margo" width="48"><br>
  <sub>GPL-3.0-or-later</sub>
</p>
