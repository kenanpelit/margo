# Features

A tour of what margo ships. margo is a Rust-native Wayland compositor **plus** a
complete first-party desktop stack — there is no GNOME/KDE underneath. This page
is the map; each section links to the detailed guide.

## Compositor

The [Smithay](https://github.com/Smithay/smithay)-based tiling compositor, in
the dwl/mango tradition — **tags instead of workspaces**, driven by `mctl`.

- **15 tiling layouts** — tile, scroller (PaperWM-style, the default), grid,
  monocle, deck, dwindle, and more; each tag remembers its own. Set with
  `setlayout <name>`, cycle with `switch_layout`. See
  [Configuration → Layouts](configuration.md#layouts).
- **Dispatch actions** — the full keybind/IPC verb catalogue (focus, move,
  resize, scratchpad, overview, summon/focusapp, …). Always-current list:
  `mctl actions --verbose`.
- **Animations** — spring / bezier window + layer motion, pre-baked into LUTs.
- **Twilight** — built-in blue-light filter with geo/schedule phases
  (`mctl twilight`).
- **Built-in xdg-desktop-portal** — native ScreenCast backend (window + monitor
  share). See [Built-in portal](portal-design.md).
- **HDR + colour management** — see [HDR + colour management](hdr-design.md).
- **XWayland**, output hotplug, and a [Wayland protocol matrix](protocol-matrix.md)
  / [comparison](protocol-comparison.md) against the other compositors.
- **Window / tag / layer rules** — PCRE2-style regex matching to place, float,
  size, or tag clients. See [Configuration → rules](configuration.md#window-rules).
- **Scripting engine** — Rhai scripting hooks. See
  [Scripting](scripting.md) and the [design notes](scripting-design.md).

## Desktop shell (mshell)

The GTK4 + relm4 shell hosting the bars, menus, settings, and dashboard.

- **Bars + widgets** — top/bottom bars with 50+ pills. Full reference:
  **[Bar widgets](widgets.md)**.
- **mdash dashboard** — greeting, calendar, weather, media, quick-settings
  tiles, menu-shortcut grid (`mshellctl menu mdash`).
- **Control Center** — quick-settings menu (sliders + toggle grid).
- **Settings** — in-shell GTK settings for the whole stack (Appearance, Bar,
  Widgets, Network, Bluetooth, Power, Sound, Animations, Keybinds, AI, Toasts,
  and more).
- **Notifications** — first-party center with history, inline reply, sounds,
  progress, and Do Not Disturb.
- **Toasts** — corner cards announcing *state changes* (power, audio device,
  network, Bluetooth, VPN, power profile, keyboard layout, DND, Game Mode, …),
  separate from app notifications. Per-event switches in Settings → Toasts;
  `mshellctl toast` is the `notify-send` equivalent.
- **OSD** — volume / brightness value pulses.
- **Launcher** — app / calc / ssh / sysinfo / emoji providers.
- **Material You theming** — matugen palette extraction from the wallpaper,
  applied across the shell, the compositor border, and the login screen.
- **Game Mode** — one toggle drops compositor effects, enables Do Not Disturb,
  and holds the idle inhibitor (`mshellctl gamemode toggle`).
- **WASM plugins** — capability-sandboxed plugin runtime (wasmtime). See
  [WASM plugins](mplugins-wasm-design.md).

## Companion binaries

Standalone tools that ship with margo. Full guide:
[Companion tools](companion-tools.md).

| Tool | Role |
| --- | --- |
| `mctl` | Compositor IPC client — control + introspect margo. |
| `mshellctl` | Shell IPC client — toggle menus, query state. |
| `mvpn` | Mullvad VPN control (CLI + GTK panel). |
| `mplay` | mpv companion: window control + video-wallpaper engine. |
| `mkeys` | On-screen keyboard. |
| `mpicker` | Screen colour picker. |
| `mscreenshot` | Screenshot CLI (region / window / output). |
| `mpower` | Automatic power-profile manager (CPU + AC/battery aware). |
| `mlock` | Lock screen (PAM + ext-session-lock-v1). |
| `mlogind` | TUI login / display manager. |
| `mlayout` | Saved tiling-layout snapshots. |
| `start-margo` | Hang-aware session supervisor (systemd watchdog, restart backoff). |

## Configuration & IPC

- **[Configuration](configuration.md)** — the `config.conf` guide (layouts,
  dispatch actions, keybindings, gestures, rules) and the
  [full annotated reference](config-reference.md).
- **[IPC](ipc.md)** — the `mctl ↔ margo` Unix socket (`get` / `watch` /
  `dispatch`) and the `mshellctl ↔ mshell` D-Bus surface.
- **[Scripting](scripting.md)** — automating the desktop.
- **[Install](install.md)** — packaged installer + manual build.

> The website (<https://kenanpelit.github.io/margo/>) is the canonical,
> always-current documentation.
