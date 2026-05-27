# margo Control Center — Design

**Date:** 2026-05-27
**Status:** Approved (design); implementation pending
**Scope:** A new **Control Center** menu — a noctalia/GNOME-style quick-settings panel: a user
header (avatar + uptime + lock/power/settings/edit), volume + brightness sliders, and a 2-column
grid of toggle/info tiles where the connectivity/audio/power tiles **expand inline** (GNOME `>`
style) to reveal their detail. Plus an edit mode to choose which tiles show. One spec; built as a
multi-task feature (edit-config is the last, independently-droppable task).

## Goal

margo has all the backing services + a `quick_action` system + slider/connectivity/system widgets,
but **no unified control-center surface** (quick-settings only exists as the right column of the
`dashboard` combined menu — there is no standalone `quick-settings` IPC verb). This adds a dedicated
Control Center menu that composes the existing services into the reference layout.

## References

`~/Pictures/Screenshots/control_center.png` (noctalia: header avatar+uptime+actions, 2 sliders,
2-col rich tile grid — filled icon-chip when active) and `control_center1.png` (GNOME quick
settings: pill toggles, `>` chevron expands a detail sub-panel, accent fill when active).

## Decisions (locked)

- **New menu** `control-center` (own bar pill + IPC verb + `MenuType` + frame wiring + Settings
  registration). Coexists with `dashboard`/`mshelldash`.
- **Tile interaction: inline expand** (GNOME `>`). Connectivity/audio/power tiles expand a detail
  panel WITHIN the control center — REUSING the existing revealed-content components (network /
  bluetooth / audio_out / audio_in / power), not re-implemented. Pure toggles (DND, Keep Awake,
  Dark Mode, Night Light) flip in place. Color Picker launches mpicker.
- **Header**: avatar (~/.face) + username + uptime; right-side action icons lock / session-power /
  settings / **edit**.
- **Edit mode**: choose which tiles are shown; persisted to mshell-config. Last task — droppable.
- Visual language = the audio-dashboard / DESIGN.md token system already used across the shell.

## Existing infrastructure (build on this — do NOT reinvent)

- **Menu wiring template:** the Alarm Clock menu (`MenuType` + frame menu-stack + `MenuWidget` +
  builder + IPC verb in `mshell-core` + `mshellctl menu` subcommand + Settings registration). Follow
  DESIGN.md §6 (bar→menu wiring checklist) + §8 (Settings).
- **RevealerRow** (`common_widgets/revealer_row`) — the collapsed-row→chevron→reveal pattern; the
  audio_out menu (`audio_out_menu_widget.rs` + `audio_out_revealed_content.rs`) is the exemplar.
  This IS the inline-expand mechanism.
- **Detail components to reuse as revealed content:** `network` (`network_toggle`/`network`
  revealed content), `bluetooth` (`bluetooth_revealed_content` / device rows), `audio_out` /
  `audio_in` revealed content, `power` (profile + battery).
- **Sliders:** `compact_audio.rs` (volume + mic Scale pattern); brightness via
  `brightness_service()` + `mshell_utils::brightness` (`get_brightness_icon`, `spawn_brightness_watcher`).
- **Toggles (quick_action actions):** `quick_action/actions/{do_not_disturb, idle_inhibitor (keep
  awake), night_light, color_picker, airplane_mode, lock, logout, shutdown, settings, reboot,
  screenshot}.rs` — reuse their service calls/state.
- **Dark mode:** the `dark_mode` bar widget's toggle logic.
- **Services:** `network_service`, `bluetooth_service`, `audio_service`, `brightness_service`,
  `battery_service` / `power_profile_service` / `line_power_service`, idle inhibitor, notification
  DND, twilight (night light).
- **Header data:** avatar = `~/.face` then AccountsService icon (the resolver from
  `users_settings.rs`); username = `glib::user_name()` / `whoami`; uptime = parse `/proc/uptime`
  (format "up Hh Mm"); action icons reuse the quick_action lock/session/settings.
- **Disk usage:** the `sysstat` / cpu_dashboard path already reads disk; reuse its source (or
  `statvfs` on `/`).

## Architecture

```
mshell-frame/src/menus/menu_widgets/control_center/
  mod.rs
  control_center_menu_widget.rs   # root: header + sliders + tile grid
  header.rs                       # avatar + user + uptime + action icons
  tile.rs                         # the tile widget contract (toggle + optional RevealerRow expand)
  (reuses existing *_revealed_content components for inline detail)
mshell-config/src/schema/         # control_center tile-visibility config (edit mode)
mshell-style/scss/04-components/_control_center.scss
```

- Root: `gtk::Box.control-center-menu-widget` (vertical) — Header, then a sliders row, then a
  2-column tile grid (`gtk::Grid` or two `gtk::Box` columns, `homogeneous` for equal columns).
- Each tile is either a **toggle tile** (click flips a service state) or an **expand tile** (a
  `RevealerRow` whose revealed content is an existing detail component). The tile chrome is shared:
  rounded icon-chip (filled `--primary-container`/`--on-primary-container` when active, flat +
  `--on-surface-variant` when inactive) + title + subtitle, on a `--surface-container` card.
- Lazy: detail components load/scan on reveal (RevealerRow `Revealed`), watchers lazy-start on
  menu reveal (`ParentRevealChanged`), honouring the menu-lazy-polling rule.

## Components

### Header (`header.rs`)
- Avatar (`gtk::Picture`/`Image` from `~/.face`/AccountsService, fallback generic user icon),
  username, uptime ("up 7h 54m" — recompute on a slow heartbeat / on reveal).
- Right action icons (flat `panel-action-btn`): **lock** (`mlock`/lock IPC), **session/power**
  (open session menu), **settings** (`open_settings`), **edit** (toggle edit mode).

### Sliders
- Volume: `audio_service().default_output` set_volume (reuse compact_audio's Scale + block-signal).
- Brightness: `brightness_service()` get/set + `spawn_brightness_watcher`. Hidden if no backlight.

### Tiles (the grid)
| Tile | Kind | Detail / action |
|---|---|---|
| Wi-Fi | expand | reveal network detail (SSID list/connect) — reuse network revealed content |
| Bluetooth | expand | reveal bluetooth devices — reuse bluetooth revealed content |
| Audio Output | expand | reveal output device picker — reuse audio_out revealed content |
| Microphone | expand | reveal input device picker — reuse audio_in revealed content |
| Battery | expand | reveal power profiles + battery — reuse power detail |
| Disk | info | `/` usage (statvfs) — read-only |
| Color Picker | action | launch mpicker (color_picker action) |
| Keep Awake | toggle | idle inhibitor |
| Do Not Disturb | toggle (wide) | notification DND |
| Dark Mode | toggle (small) | dark_mode toggle |
| Night Light | toggle (small) | twilight |

Active state = filled icon chip + `.active`. Subtitle shows live state (SSID/%, device, level, …).

### Edit mode (last task)
- The edit (pencil) icon flips an edit mode where each tile shows a visibility checkbox/toggle;
  the set persists to a new `ControlCenterConfig { tiles: Vec<...> }` (or per-tile bools) in
  mshell-config (wired like `PowerConfig`). The grid renders only enabled tiles. Default = all on.

## Wiring (new menu — DESIGN.md §6 checklist)

- `MenuType::ControlCenter` + frame menu-stack entry + `build_menu`/`add_to_stack` + `FrameInput::
  ToggleControlCenterMenu` + `BarOutput::ControlCenterClicked`.
- `MenuWidget::ControlCenter` enum + dispatch + `control_center_menu` Menu config (position, sizes).
- IPC: `IPCCommand::ControlCenter` → zbus `control_center` → `ShellInput::ToggleControlCenterMenu`;
  `mshellctl menu control-center`.
- Bar pill: a `control_center` BarWidget (icon, click → `ControlCenterClicked`) + Settings sidebar.
- Settings: `MenuKind::ControlCenter` widget-menu entry (config the pill/menu).

## Decomposition (one spec; edit-config is last + droppable)

1. **Menu scaffold + full wiring** — empty `control-center` menu reachable via bar pill + `mshellctl
   menu control-center` (MenuType/frame/IPC/bar/Settings). Verify it opens.
2. **Header** — avatar + username + uptime + lock/session/settings action icons (edit icon present
   but inert until task 6).
3. **Sliders** — volume + brightness rows.
4. **Toggle/info tiles** — the tile widget contract + Keep Awake, DND, Dark Mode, Night Light,
   Color Picker, Disk, Battery-info; live state + active styling.
5. **Inline-expand tiles** — Wi-Fi, Bluetooth, Audio Out, Microphone (+ Battery→power) as
   RevealerRow tiles reusing the existing revealed-content components; load/scan on reveal.
6. **Edit mode + config** — tile-visibility `ControlCenterConfig` + the pencil edit UI. (Droppable.)
7. **SCSS** — `_control_center.scss` (tiles, icon-chips, sliders, header) in DESIGN.md tokens.
8. **Final** — workspace clippy/build, Cargo.lock, push.

## Verification

- Per task: `cargo clippy` clean + `cargo build -p mshell`. Menu opens via pill + IPC.
- Manual (user, post-rebuild): control center opens; header shows avatar/user/uptime + actions
  work; sliders move volume/brightness; toggles flip; Wi-Fi/BT/Audio tiles expand inline to their
  detail; edit mode hides/shows tiles + persists.

## Risks / honest limits

- Inline-expand reuse: the existing revealed-content components must embed cleanly inside a tile —
  if a component assumes its own surrounding chrome, light adaptation may be needed (keep it minimal).
- Large feature; the menu-wiring boilerplate (8 sites) + tile composition is the bulk. Edit-config
  is the most optional — sequenced last so the core ships if it's deferred.
- Avatar/uptime are best-effort (no avatar → generic icon; uptime from /proc/uptime).

## Out of scope

Per-tile drag-reorder, custom user-defined tiles/plugins, multi-page tiles, animations beyond the
existing motion tokens.
