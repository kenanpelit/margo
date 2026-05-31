# margo wiki

**margo** is a Rust + [Smithay](https://github.com/Smithay/smithay) Wayland
tiling compositor in the dwl/mango tradition — tags instead of workspaces, a
15-layout catalogue, spring/bezier animations, and a complete first-party
desktop stack (shell, locker, login manager, screenshot, power manager).

## Start here

- **[[Configuration]]** — the complete config guide: layouts, the full
  dispatch-action catalogue (`setlayout`, `switch_layout`,
  `switch_proportion_preset`, …), keybindings, and window/tag/layer rules.
- **Full annotated reference** — every knob with inline commentary:
  <https://kenanpelit.github.io/margo/config-reference/> (rendered from
  [`margo/src/config.example.conf`](https://github.com/kenanpelit/margo/blob/main/margo/src/config.example.conf)).
- **Companion tools** — `mctl`, `mlayout`, `mscreenshot`, `mlogind`, `mpower`:
  <https://kenanpelit.github.io/margo/companion-tools/>.
- **Install** — <https://kenanpelit.github.io/margo/install/>.

## At a glance

- Config lives at `~/.config/margo/config.conf` (plain `key = value`,
  hot-reloadable with `mctl reload`).
- Validate offline before reloading: `mctl check-config`.
- Discover everything the running compositor accepts: `mctl actions --verbose`.

> The website (<https://kenanpelit.github.io/margo/>) is the canonical,
> always-current documentation. This wiki mirrors the configuration guide for
> quick browsing on GitHub.
