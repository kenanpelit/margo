# margo

The **Wayland compositor** — the core binary of the margo stack. A Rust +
[Smithay](https://github.com/Smithay/smithay) port of [mango](https://github.com/mangowm/mango)
(dwl/dwm lineage) with **tags instead of workspaces**, a 14-layout tiling
catalogue, spring/bezier animations, and a complete first-party desktop built
around it.

## What it is

`margo` drives DRM/KMS directly: outputs, input, the scene graph, tiling
layout, animations, window rules, the D-Bus surface, the screencast portal
pipeline, and the blue-light filter all live here. It speaks `dwl-ipc-v2`, so
the first-party shell (`mshell`) — or any third-party dwl/mango widget like
[noctalia](https://github.com/noctalia-dev/noctalia-shell) — surfaces on
screen against it.

## Highlights

- **Tags, not workspaces** — nine multi-select tags, per-tag layout memory,
  regex window→tag matching at map time.
- **Layout catalogue** — tile, scroller, grid, monocle, deck, dwindle,
  center/right/vertical mirrors, overview.
- **Animations** — niri-style spring physics with mid-flight retarget, bezier
  open/close/tag/focus transitions, SDF shadows, rounded corners.
- **Modern protocols** — `ext-session-lock-v1`, `ext-idle-notify-v1`, DMA-BUF
  screencopy, pointer constraints, `xdg_activation`, runtime
  `wlr_output_management`, `presentation-time`, `wp_color_management_v1`.
- **Window rules with PCRE2** — float/pin/blackout/swallow by `app_id`/`title`.
- **Embedded Rhai scripting** — `~/.config/margo/init.rhai`, with
  `on_focus_change` / `on_tag_switch` / `on_window_open` hooks.
- **Hot reload** — `mctl reload` re-applies rules, binds, monitors, curves
  without a logout.
- **In-compositor screencast portal** — Mutter D-Bus shims + PipeWire, no
  gnome-shell needed (see [`margo-portal`](../margo-portal/)).

## Build

```bash
cargo build --release -p margo
sudo install -m755 target/release/margo /usr/bin/margo
```

Launch it through the [`start-margo`](../start-margo/) supervisor rather than
directly — it adds a crash budget, `sd_notify`, and clean signal forwarding.

## Configure

Config is plain-text `~/.config/margo/config.conf` (key = value, PCRE2 window
rules). Control and introspect the running compositor with
[`mctl`](../mctl/).

## Documentation

Full docs — configuration, the action reference, design notes — live at
**<https://kenanpelit.github.io/margo/>**. The repo [README](../README.md) is
the top-level tour.

## License

GPL-3.0-or-later.
