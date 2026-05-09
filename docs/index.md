---
hide:
  - navigation
---

# margo

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="assets/margo-banner.svg">
    <img src="assets/margo-banner-dark.svg" alt="margo" width="600">
  </picture>
</p>

<p align="center">
  <em>A modern Wayland tiling compositor — Rust + Smithay, with a tag-based workflow.</em>
</p>

<p align="center">
  <a href="https://github.com/kenanpelit/margo/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg" alt="License"></a>
  <a href="roadmap/"><img src="https://img.shields.io/badge/status-daily%20driver-success" alt="Status"></a>
  <a href="https://github.com/kenanpelit/margo"><img src="https://img.shields.io/badge/built%20on-Smithay-blueviolet" alt="Smithay"></a>
</p>

**margo** is a Wayland compositor in the dwl/dwm tradition — a Rust + [Smithay] port of [mango]. Tags instead of workspaces, a deep tiling-layout catalogue, and a small set of companion CLIs (`mctl`, `mlayout`, `mscreenshot`) for inspection and control from the shell. Built and used as a daily driver: every commit ships through a real session before tagging.

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
- **Embedded scripting.** Drop `~/.config/margo/init.rhai`; call any compositor action from a sandboxed Rhai interpreter, hook `on_focus_change` / `on_tag_switch` / `on_window_open` / `on_window_close`. Plugin packaging via `~/.config/margo/plugins/<name>/`.
- **Hot-reload everything.** `mctl reload` (or Super+Ctrl+R) re-applies window rules, key binds, monitor topology, animation curves, gestures — no logout.
- **DRM hotplug that works.** Dock / undock, plug a second monitor mid-session; outputs come and go cleanly.
- **`dwl-ipc-v2` compatibility.** Drop-in for noctalia, waybar-dwl, fnott, and any other dwl/mango widget.

## Where to next

<div class="grid cards" markdown>

-   :material-download:{ .lg .middle } **Install**

    ---

    Arch package, source build, or Nix flake.

    [:octicons-arrow-right-24: Install guide](install.md)

-   :material-cog:{ .lg .middle } **Configure**

    ---

    `~/.config/margo/config.conf` — tags, rules, keys, animations.

    [:octicons-arrow-right-24: Configuration](configuration.md) · [Full reference](config-reference.md)

-   :material-tools:{ .lg .middle } **Companion tools**

    ---

    `mctl`, `mlayout`, `mscreenshot` — drive margo from the shell.

    [:octicons-arrow-right-24: Companion tools](companion-tools.md)

-   :material-language-rust:{ .lg .middle } **Scripting**

    ---

    Embedded Rhai engine + plugin packaging.

    [:octicons-arrow-right-24: Scripting](scripting.md)

-   :material-clipboard-check:{ .lg .middle } **Manual checklist**

    ---

    Post-install validation pass.

    [:octicons-arrow-right-24: Checklist](manual-checklist.md)

-   :material-map-marker-path:{ .lg .middle } **Roadmap**

    ---

    What's shipped, what's queued.

    [:octicons-arrow-right-24: Roadmap](roadmap.md)

</div>

## Acknowledgements

Built on [Smithay] (compositor toolkit). Patterns and inventory borrowed from [niri](https://github.com/YaLTeR/niri) (focus oracle, hotplug, screencast portal, transactional resize), [mango](https://github.com/mangowm/mango) (feature inventory, IPC surface, default keybinds), [dwl](https://codeberg.org/dwl/dwl) (the original dwm-on-wlroots), [anvil](https://github.com/Smithay/smithay/tree/master/anvil) (Smithay's reference compositor), and [Hyprland](https://hypr.land) (color-management protocol shape).

Original portions of dwl, dwm, sway, tinywl, and wlroots are preserved under their respective licenses.
