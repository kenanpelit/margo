# mshell

The **GTK4 desktop shell** for margo — top/bottom bars, menus, notifications,
OSD, an in-app Settings window, and a dashboard. Built with GTK4 + relm4, it
mirrors the compositor's state (via `dwl-ipc-v2` + `state.json`) into a
reactive store and hosts the `com.mshell.Shell` D-Bus service.

## What it gives you

- **Bar with a configurable pill set** — workspace/tag pills, active-window,
  clock, media player, network speed, battery, audio, system tray,
  notifications. Plus opt-in indicators (privacy mic/cam, CPU/RAM/temp, lock
  keys, dark-mode toggle, keep-awake, rounded corners) and system widgets
  (update badge, display→layout panel driving `mlayout`).
- **Menus & dashboard** — quick-settings/control-center, calendar, weather,
  media, wallpaper, and a tabbed dashboard.
- **Notifications, OSD, launcher** — corner toasts, volume/brightness pulses,
  and a provider-based launcher (apps, calc, ssh, sysinfo, …).
- **In-app Settings** — GNOME-parity pages for appearance, bars/widgets,
  network, Bluetooth, power, idle, privacy, keybinds, and more.
- **Material You theming** — palette extracted from the wallpaper via matugen;
  SCSS compiled to CSS at build time.

Control it from the CLI with [`mshellctl`](../mshellctl/).

## Build

```bash
cargo build --release -p mshell
sudo install -m755 target/release/mshell /usr/bin/mshell
systemctl --user restart mshell
```

> SCSS lives in `mshell-crates/mshell-style/scss/` and is baked into the binary
> at build time — style edits require a recompile + restart, not just a reload.

The packaged `mshell.service` auto-starts the shell when a margo graphical
session comes up (gated by `ConditionEnvironment` to margo).

## License

GPL-3.0-or-later. Forked from OkShell.
