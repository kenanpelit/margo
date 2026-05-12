<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/margo-banner.svg">
    <img src="docs/assets/margo-banner-dark.svg" alt="margo" width="600">
  </picture>
</p>

<p align="center">
  <em>A Wayland tiling compositor and desktop suite — Rust + Smithay, tag-based, with first-party bar, lock, idle and IPC binaries.</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg" alt="License"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/version-0.4.0-success" alt="Version"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust"></a>
  <a href="https://github.com/Smithay/smithay"><img src="https://img.shields.io/badge/built%20on-Smithay-blueviolet" alt="Smithay"></a>
  <a href="https://kenanpelit.github.io/margo/"><img src="https://img.shields.io/badge/docs-online-blue" alt="Docs"></a>
</p>

---

**margo** is a Wayland compositor in the dwl/mango tradition — a Rust + [Smithay] port of [mango] with tags instead of workspaces, a deep tiling layout catalogue, and a coherent set of companion binaries for everyday use: a bar (`mshell`), a lock screen (`mlock`), an idle manager (`midle`), and a control CLI (`mctl`). The whole stack ships from one workspace and one release.

[Smithay]: https://github.com/Smithay/smithay
[mango]: https://github.com/mangowm/mango

## Binaries

| Binary | Role |
|---|---|
| **`margo`** | Wayland compositor — DRM/KMS backend, tag workflow, layout engine |
| **`mshell`** | Bar + notifications + OSD + settings panel + system tray (iced + layer-shell) |
| **`mctl`** | Compositor IPC + control — `status / clients / outputs / dispatch / tags / layout / reload / theme / twilight / migrate / actions / check-config` |
| **`mlock`** | Screen locker — `ext-session-lock-v1`, cairo + pango, PAM auth, wallpaper backdrop, avatar, F1/F2/F3 power keys |
| **`midle`** | Idle manager — `ext-idle-notify-v1`, sequential timeouts, app/media/logind inhibitors, TOML config + Unix-socket CLI |
| **`mlayout`** | Named monitor profiles — `mlayout suggest` writes presets for the detected setup, `mlayout set <name>` flips between them |
| **`mscreenshot`** | Screen / region / window capture — wraps `grim` + `slurp` + `wl-copy` + optional editor (`swappy` / `satty`) |
| **`mvisual`** | Visual debugger for the renderer |

Every directory in the workspace maps 1:1 to a binary it produces. Library-only crates (`margo-config`, `margo-layouts`) keep the `margo-` prefix.

## Compositor highlights

- **Tags, not workspaces.** Nine multi-select tags; press the same tag twice to bounce back, several together to view a union, pin tags to a home monitor, regex-match windows into tags at map time.
- **Layouts that remember.** Tile, scroller, grid, monocle, deck, dwindle, center / right / vertical mirrors and an overview. Each tag holds its own layout choice.
- **Spring + bezier animations.** Niri-style spring physics with mid-flight retarget for window movement; bezier curves for open / close / tag / focus / layer transitions. SDF drop shadows, rounded corners, focus-fade opacity.
- **Modern protocol stack.** `ext-session-lock-v1`, `ext-idle-notify-v1`, DMA-BUF screencopy, `pointer_constraints` + `relative_pointer`, `xdg_activation` with anti-focus-steal, runtime `wlr_output_management`, VBlank-accurate `presentation-time`, `wp_color_management_v1`.
- **Window rules with PCRE2.** Float password prompts, pin apps to tags, screencast-blackout password managers, swallow terminal children, force CSD per-app — all by `app_id` / `title` regex.
- **In-compositor screencast portal.** Five Mutter D-Bus shims + a PipeWire pipeline so `xdg-desktop-portal-gnome` serves Window / Entire-Screen tabs without gnome-shell.
- **Embedded scripting.** Drop `~/.config/margo/init.rhai`; call any compositor action from a sandboxed Rhai interpreter, hook `on_focus_change` / `on_tag_switch` / `on_window_open`.
- **Hot reload.** `mctl reload` (or `Super+Ctrl+R`) re-applies window rules, key binds, monitor topology, animation curves, gestures — no logout.
- **DRM hotplug.** Dock / undock, plug a second monitor mid-session; outputs come and go cleanly.
- **`dwl-ipc-v2` compatibility.** Drop-in for noctalia, waybar-dwl, fnott, and any other dwl/mango widget.

## Shell highlights (`mshell`)

- iced 0.13 + `iced_layershell` bar with multi-monitor surfaces, animated open/close (fade + slide), drop-shadowed menus, frosted sub-panels.
- Modules: workspaces (with per-tag colors + window-count badges), window-title, system-info (CPU/RAM/Temp/Disk), network-speed (Download/Upload + LAN IP + VPN IP), DNS/VPN switcher, UFW, power profile, Podman, media-player (MPRIS), tray (StatusNotifierWatcher), notifications (toast + history grouped by date), updates, weather/clock (tempo), settings panel.
- `mshell matugen [WALLPAPER]` — generates a Material You palette from the active wallpaper and live-applies it to margo via `mctl reload`; `[matugen].auto_on_wallpaper_change = true` re-runs on every wallpaper swap.
- Notification IPC: `mshell msg notifications` (open history), `notifications-read` (silently dismiss toasts, keep history), `notifications-clear` (drop history).
- Mako-style toast configuration: `toast_width`, `toast_radius`, `toast_opacity`, summary/body font sizes, per-app overrides, `on_notify_command` shell hook.
- Localisation: English + Turkish Fluent catalogues; renderer log noise suppressed by default.

## Lock + idle highlights

- **`mlock`** uses `ext-session-lock-v1`, so the compositor cooperates: locked sessions stay locked across mlock crashes, and only `margo`'s `force_unlock` keybind can break out. Renders a blurred wallpaper, large clock, time-of-day greeting, avatar (`~/.face` or AccountsService), frosted password card with shake-on-fail and an attempt counter. Authenticates the session owner via PAM. Battery indicator and `F1/F2/F3` power keys with a two-press confirmation banner.
- **`midle`** speaks `ext-idle-notify-v1`. Each `[[step]]` in `~/.config/margo/midle.toml` runs a shell command (and a matching `resume_command` on activity) at its own threshold. Three built-in inhibitor sources stop the chain from firing: regex on `/proc/<pid>/cmdline`, PulseAudio/PipeWire `RUNNING` sink-inputs, and logind's `PrepareForSleep` signal (lets you `mlock` pre-suspend). Controlled at runtime via `midle info / pause [dur] / resume / toggle-inhibit / reload / stop` over a Unix socket.

## Install

### Arch (PKGBUILD)

```bash
git clone https://github.com/kenanpelit/margo_build ~/.kod/margo_build
cd ~/.kod/margo_build && makepkg -si
```

Installs all eight binaries, the Wayland session entry, example layouts and example configs. PAM service file for `mlock` lands in `/etc/pam.d/`. Runtime tools (`grim`, `slurp`, `wl-clipboard`, `pactl`) come in as dependencies; `swappy` / `satty` are optional screenshot editors picked up at runtime.

### From source

```bash
git clone https://github.com/kenanpelit/margo
cd margo && cargo build --release --workspace
for bin in margo mshell mctl midle mlock mlayout mscreenshot mvisual; do
  sudo install -Dm755 target/release/$bin /usr/bin/$bin
done
sudo install -Dm644 margo.desktop /usr/share/wayland-sessions/margo.desktop
```

System dependencies: `wayland`, `libinput`, `libxkbcommon`, `seatd`, `mesa`, `libdrm`, `pixman`, `pcre2`, `cairo`, `pango`, `pam`, `xorg-xwayland` (optional). Runtime: `grim`, `slurp`, `wl-clipboard` for screenshots; `wlr-randr` for live monitor re-layout; `pactl` for `midle`'s media inhibitor.

### Nix flake

```bash
nix run github:kenanpelit/margo
```

The flake exposes `packages.default`, a `devShells.default` with `rust-analyzer` + `clippy`, plus `nixosModules.margo` and `hmModules.margo`.

## Configure

All user config lives in `~/.config/margo/`:

```
~/.config/margo/
├── config.conf      # margo — compositor
├── mshell.toml      # mshell — bar / notifications / OSD
├── midle.toml       # midle — idle manager
└── layout_*.conf    # mlayout — monitor profiles
```

A complete annotated `config.conf` ships at [`margo/src/config.example.conf`](margo/src/config.example.conf); `mshell.example.toml` exercises every module. Both are hot-reloadable — `mctl reload` (or `Super+Ctrl+R`) re-applies window rules, key binds, monitor topology, animation curves.

```ini
# config.conf — small excerpt
borderpx          = 3
border_radius     = 12
focused_opacity   = 1.0
unfocused_opacity = 0.9

tagrule = id:1, layout_name:scroller, monitor_name:DP-3
tagrule = id:7, layout_name:scroller, monitor_name:eDP-1

windowrule = tags:1, appid:^Kenp$
windowrule = isfloating:1, width:640, height:260, \
             title:^(Authentication Required|Unlock Keyring)$

animation_clock_move = spring
animation_clock_tag  = bezier
animation_curve_open = 0.16,1.0,0.30,1.0

bind = super,       Return, spawn, kitty
bind = super,       q,      killclient
bind = super+ctrl,  s,      sticky_window
bind = super+ctrl,  r,      reload_config
bind = alt,         l,      spawn, mlock
bind = NONE,        Print,  screenshot-region-ui
```

Validate before reloading:

```bash
mctl check-config
```

## At a glance

```bash
# Compositor inspection
mctl status                          # per-output: focused / tags / layout
mctl clients --tag 2                 # every window on tag 2 (table)
mctl outputs --json | jq '.[].name'
mctl focused                         # `app_id · title`, scriptable

# Compositor control
mctl dispatch togglefullscreen
mctl dispatch view 4                 # tag bitmask 4 = tag 3
mctl twilight toggle                 # blue-light filter
mctl reload

# Theme from wallpaper
mshell matugen                       # active output's wallpaper → Material You

# Idle manager
midle info                           # JSON snapshot
midle pause 30m                      # suspend the timer
midle toggle-inhibit                 # for status bars

# Layout profiles
mlayout suggest                      # propose & activate a preset
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

// Notify the bar when entering tag 9
on_tag_switch(|| {
    if current_tag() == 9 {
        spawn("pkill -SIGUSR1 waybar");
    }
});
```

Engine: [Rhai] (pure Rust, sandboxed by default). Reference: [`docs/scripting-design.md`](docs/scripting-design.md).

[Rhai]: https://rhai.rs

## Documentation

- **[Documentation site](https://kenanpelit.github.io/margo/)** — install, configuration, scripting, design notes (mkdocs-material).
- [`CHANGELOG.md`](CHANGELOG.md) — release-by-release history (Keep-a-Changelog).
- [`road_map.md`](road_map.md) — what's shipped, what's queued, design trade-offs.
- [`docs/`](docs/) — design notes for in-flight features and the post-install validation checklist.
- `mctl --help`, `mctl actions --verbose`, `mshell --help`, `midle --help`, `mlock --help`, `mlayout --help`, `mscreenshot --help` — generated from source, always current.

## Acknowledgements

Built on [Smithay]. Patterns and inventory borrowed from [niri](https://github.com/YaLTeR/niri) (focus oracle, hotplug, screencast portal, transactional resize), [mango](https://github.com/mangowm/mango) (feature inventory, IPC surface, default keybinds), [dwl](https://codeberg.org/dwl/dwl) (the original dwm-on-wlroots), [anvil](https://github.com/Smithay/smithay/tree/master/anvil) (Smithay's reference compositor), and [Hyprland](https://hypr.land) (color-management protocol shape). `mlock` follows the architecture of [nlock](https://github.com/OldUser101/nlock) and [waylock](https://codeberg.org/ifreund/waylock); `midle` follows [stasis](https://github.com/saltnpepper97/stasis); `mshell` originated as a fork of [ashell](https://github.com/MalpenZibo/ashell).

Original portions of dwl, dwm, sway, tinywl, and wlroots are preserved under their respective licenses — see `LICENSE.*`.

## License

GPL-3.0-or-later. See [`LICENSE`](LICENSE).

<p align="center">
  <img src="docs/assets/margo-icon.svg" alt="margo" width="48"><br>
  <sub>GPL-3.0-or-later · 0.4.0</sub>
</p>
