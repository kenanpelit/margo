# margo Control Center Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new Control Center menu — user header (avatar + uptime + lock/power/settings/edit), volume + brightness sliders, and a 2-column tile grid where connectivity/audio/power tiles expand inline (GNOME `>` style) reusing existing detail components; edit mode chooses visible tiles.

**Architecture:** A new layer-shell menu wired like the Alarm Clock menu (DESIGN.md §6). The root composes a header, a sliders row, and a tile grid. Tiles are either toggle/info tiles or RevealerRow "expand" tiles whose revealed content REUSES existing `*_revealed_content` components (network/bluetooth/audio_out/audio_in/power). Visual language = the audio-dashboard / DESIGN.md tokens.

**Tech Stack:** GTK4 + relm4, wayle services (audio/brightness/network/bluetooth/battery/power), the `RevealerRow` common widget, mshell-config reactive store.

**Spec:** `docs/superpowers/specs/2026-05-27-control-center-design.md`

**Verification:** Mostly GTK UI driven by live services → verified by `cargo clippy -p mshell-frame` + `cargo build -p mshell` per task + the user's manual test after rebuild. No automated UI tests in this codebase. The edit-config schema (Task 6) is pure-ish logic.

**Templates to mirror (read before each relevant task):**
- Menu wiring: the **Alarm Clock** menu — `MenuType`, frame menu-stack, `MenuWidget`, `menus/builder.rs`, `mshell-core/src/ipc.rs` verb, `mshellctl/src/subcommands/menu.rs`, Settings `MenuKind` + `widget_menu_settings.rs`. DESIGN.md §6 is the binding checklist.
- RevealerRow expand: `menus/menu_widgets/audio_out/audio_out_menu_widget.rs` (+ `audio_out_revealed_content.rs`).
- Sliders: `menus/menu_widgets/compact_audio.rs`. Brightness: `mshell-osd/src/brightness_osd.rs` + `mshell_utils::brightness`.
- Toggles: `menus/menu_widgets/quick_action/actions/{do_not_disturb,idle_inhibitor,night_light,color_picker}.rs`; dark mode = `bars/bar_widgets/dark_mode.rs`.
- Config section: `PowerConfig`/`PrivacyConfig` in `mshell-config/src/schema/config.rs`.

---

## Task 1: Control Center menu scaffold + full wiring

**Files:**
- Create: `mshell-crates/mshell-frame/src/menus/menu_widgets/control_center/{mod.rs, control_center_menu_widget.rs}`
- Create: `mshell-crates/mshell-frame/src/bars/bar_widgets/control_center.rs`
- Modify: `menus/menu.rs` (`MenuType::ControlCenter` + const), `frame.rs` (controller field + build_menu + add_to_stack + `FrameInput::ToggleControlCenterMenu` + `BarOutput::ControlCenterClicked` routing), `menus/builder.rs` (build arm), `bars/bar.rs` (pill dispatch + BarOutput), `mshell-config/src/schema/{bar_widgets.rs, menu_widgets.rs, config.rs}` (`ControlCenter` variants + `control_center_menu` Menu default), `mshell-core/src/ipc.rs` + `relm_app.rs` (IPC verb + ShellInput), `mshellctl/src/subcommands/menu.rs` (`control-center`), `mshell-settings` (`MenuKind::ControlCenter` + entry).

- [ ] **Step 1: Read the Alarm Clock menu wiring end-to-end**

Read DESIGN.md §6 (bar→menu wiring checklist) + every Alarm Clock site (`grep -rn "AlarmClock" mshell-crates mshellctl`). The Control Center repeats this exact pattern with the name `ControlCenter` / `control-center` / `control_center`.

- [ ] **Step 2: Create the empty menu widget**

`control_center_menu_widget.rs`: a `#[relm4::component(pub(crate))]` `ControlCenterMenuWidgetModel` with a `gtk::Box.control-center-menu-widget` root (vertical, spacing 16) containing only a `panel-header` (icon `"preferences-system-symbolic"`, title "Control Center") for now. `Init = ControlCenterMenuWidgetInit {}`, empty `Input`/`Output`, plus a `ParentRevealChanged(bool)` input (frame sends it). `mod.rs` declares the modules. Mirror the Alarm Clock menu widget's shape.

- [ ] **Step 3: Wire all sites (mirror Alarm Clock)**

Add `MenuType::ControlCenter` + `const CONTROL_CENTER_MENU`; the frame controller field + `build_menu`/`add_to_stack` + `FrameInput::ToggleControlCenterMenu` + `BarOutput::ControlCenterClicked`; the `builder.rs` build arm; `MenuWidget::ControlCenter` + dispatch; `control_center_menu` Menu config (position e.g. TopRight, minimum_width ~460, maximum_height ~720); the `control_center` bar pill (icon `"preferences-system-symbolic"`, click → `ControlCenterClicked`); IPC `IPCCommand::ControlCenter` + `async fn control_center` + `ShellInput::ToggleControlCenterMenu`; `mshellctl menu control-center`; Settings `MenuKind::ControlCenter` (the 14-arm match) + a `WidgetEntry::Menu` entry. Use the exact strings `control_center` / `control-center` / "Control Center".

- [ ] **Step 4: Clippy + build + commit**

`cargo clippy -p mshell-config -p mshell-core -p mshell-frame -p mshell-settings` clean; `cargo build -p mshell -p mshellctl`. Then commit:
```
git add -A && git commit -m "feat(control-center): menu scaffold + full bar/IPC/frame/settings wiring"
```
Manual check: `mshellctl menu control-center` opens an (empty) Control Center panel.

---

## Task 2: Header (avatar + username + uptime + action icons)

**Files:**
- Create: `control_center/header.rs`
- Modify: `control_center_menu_widget.rs` (embed the header), `mod.rs`

- [ ] **Step 1: Build the header component**

`header.rs`: a component rendering a `panel-header`-style row:
- Avatar: `gtk::Image`/`Picture`. Resolve path: `~/.face` if it exists, else the AccountsService icon path (`/var/lib/AccountsService/icons/<user>`), else fallback icon `"avatar-default-symbolic"`. (Reuse the avatar resolver logic from `mshell-settings/src/users_settings.rs` — read it; copy the path resolution.) Round via `set_overflow(gtk::Overflow::Hidden)` + a radius class.
- Username: `glib::user_name().to_string_lossy()`.
- Uptime: read `/proc/uptime` (first float = seconds), format "up Hh Mm" (`fn fmt_uptime(secs: u64) -> String`). Recompute on `ParentRevealChanged(true)` (cheap) — don't poll continuously.
- Right action icons (flat `panel-action-btn`, `@include state-layer()`): **lock** (call the lock IPC / `mlock` like `quick_action/actions/lock.rs`), **session/power** (open the session menu — emit an output the menu widget forwards, or call the session menu open), **settings** (`mshell_settings::open_settings()`), **edit** (emit `HeaderOutput::ToggleEdit` — inert handler until Task 6).

`fmt_uptime` is testable:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn fmt() {
        assert_eq!(fmt_uptime(0), "up 0m");
        assert_eq!(fmt_uptime(3*3600 + 5*60), "up 3h 5m");
        assert_eq!(fmt_uptime(54*60), "up 54m");
    }
}
```
Write that test first, run `cargo test -p mshell-frame fmt_uptime` (fail → implement → pass).

- [ ] **Step 2: Embed in the menu widget + commit**

Embed `header.widget()` at the top of the root box. Wire the action outputs (lock/session/settings work; edit forwards to a no-op for now). `cargo clippy -p mshell-frame` clean; `cargo build -p mshell`.
```
git add -A && git commit -m "feat(control-center): header — avatar, username, uptime, action icons"
```

---

## Task 3: Volume + brightness sliders

**Files:**
- Create: `control_center/sliders.rs` (or inline in the menu widget)
- Modify: `control_center_menu_widget.rs`

- [ ] **Step 1: Build the sliders row**

Two `gtk::Scale` rows (mirror `compact_audio.rs`'s Scale + `#[block_signal]` + watcher pattern):
- Volume: icon + Scale 0–100 bound to `audio_service().default_output` volume; `connect_value_changed` → `set_volume`; `spawn_default_output_watcher` + volume watcher to update. Mute icon toggles mute.
- Brightness: icon (`get_brightness_icon`) + Scale bound to `brightness_service()` (read current %, set on change); `spawn_brightness_watcher` to update. **Hidden if `brightness_service()` is `None`** (no backlight).
Use the slim slider styling (`.compact-audio-slider`-like; new `.control-center-slider`).

- [ ] **Step 2: Clippy + build + commit**

`cargo clippy -p mshell-frame` clean; `cargo build -p mshell`.
```
git add -A && git commit -m "feat(control-center): volume + brightness sliders"
```

---

## Task 4: Tile widget contract + toggle/info tiles

**Files:**
- Create: `control_center/tile.rs` (the shared tile widget)
- Modify: `control_center_menu_widget.rs` (the grid), `mod.rs`

- [ ] **Step 1: Tile widget**

`tile.rs`: a reusable tile = a card (`gtk::Button` or Box+GestureClick) with a rounded icon-chip (Image in a `.control-center-tile-icon` box) + a vertical label stack (title + subtitle). Props: `icon_name`, `title`, `subtitle`, `active: bool`. `.active` class toggles the filled chip (`--primary-container`/`--on-primary-container`) vs flat (`--surface-container`/`--on-surface-variant`). Click → an output `TileClicked`. Support a `wide` flag (spans both columns) and a `small` flag (icon-only, for Dark Mode/Night Light).

- [ ] **Step 2: The grid + toggle/info tiles**

In the menu widget, add a 2-column grid (`gtk::Grid` with `set_column_homogeneous(true)` or two equal `hexpand` columns). Add these tiles with live state + click action (reuse the service calls from the named quick_action actions / bar widgets):
- **Keep Awake** (idle inhibitor) — `quick_action/actions/idle_inhibitor.rs` logic; active = inhibited.
- **Do Not Disturb** (wide) — `do_not_disturb.rs` logic; active = DND on.
- **Dark Mode** (small) — `bars/bar_widgets/dark_mode.rs` toggle.
- **Night Light** (small) — `night_light.rs` / twilight toggle.
- **Color Picker** — `color_picker.rs` (launch mpicker on click; not a toggle).
- **Disk** (info) — `/` usage via `nix::sys::statvfs` or reading the sysstat source; subtitle "79.0G / 186.7G (43%)"; not clickable (or click = open a file manager — skip).
- **Battery** (info for now; becomes expand in Task 5) — `battery_service()` % + charging state.
Each tile's subtitle/active updates via the relevant watcher (lazy-start on `ParentRevealChanged`).

- [ ] **Step 3: Clippy + build + commit**

`cargo clippy -p mshell-frame` clean; `cargo build -p mshell`.
```
git add -A && git commit -m "feat(control-center): tile widget + toggle/info tiles (keep-awake, DND, dark, night light, color picker, disk, battery)"
```

---

## Task 5: Inline-expand tiles (Wi-Fi, Bluetooth, Audio Out, Microphone, Battery→power)

**Files:**
- Modify: `control_center_menu_widget.rs` (+ a small `control_center/expand_tile.rs` if needed)

- [ ] **Step 1: Build the expand-tile wrapper**

Use the shared `RevealerRow` (`common_widgets/revealer_row`) — exactly as `audio_out_menu_widget.rs` does — OR wrap a `tile.rs` tile + a `gtk::Revealer` whose child is the reused revealed-content component. For each expandable tile, the collapsed row is a Control Center tile (icon-chip + title + live subtitle) with a chevron; clicking the chevron reveals the detail; the detail loads/scans on reveal (emit `Revealed`/`Hidden` to the revealed-content component).

- [ ] **Step 2: Wire the five expand tiles reusing existing detail components**

- **Wi-Fi** → reveal the network detail (the access-point list/connect content used by the network menu — reuse `network_toggle`/`network` revealed content). Subtitle = SSID + signal%.
- **Bluetooth** → reveal `bluetooth` device list (reuse the restored `bluetooth_revealed_content`). Subtitle = connected device / state.
- **Audio Output** → reveal `audio_out_revealed_content` (device picker). Subtitle = device name.
- **Microphone** → reveal `audio_in_revealed_content`. Subtitle = device / level.
- **Battery** → reveal the power detail (profile selector + battery — reuse the power menu's content). Subtitle = % + charging.
If a revealed-content component assumes surrounding chrome, adapt minimally (don't rewrite it). Lazy: only emit `Revealed` (load/scan) when expanded.

- [ ] **Step 3: Clippy + build + commit**

`cargo clippy -p mshell-frame` clean; `cargo build -p mshell`.
```
git add -A && git commit -m "feat(control-center): inline-expand Wi-Fi/Bluetooth/Audio/Mic/Battery tiles (reuse detail components)"
```

---

## Task 6: Edit mode + tile-visibility config (LAST — droppable)

**Files:**
- Modify: `mshell-config/src/schema/config.rs` (`ControlCenterConfig`), `control_center_menu_widget.rs`, `header.rs` (edit button)

- [ ] **Step 1: Config**

Add `ControlCenterConfig { tiles: ... }` to `config.rs` mirroring `PowerConfig`. Use a per-tile bool set (e.g. `wifi: bool, bluetooth: bool, audio_out: bool, mic: bool, battery: bool, disk: bool, color_picker: bool, keep_awake: bool, dnd: bool, dark_mode: bool, night_light: bool` — all default true) so `config_manager().config().control_center().wifi()` etc. exist. (Simple bools avoid serializing an enum list.)

- [ ] **Step 2: Edit mode UI**

The header's edit (pencil) button toggles an `edit_mode: bool` in the menu model. When on, each tile overlays a small checkbox/eye toggle; flipping it writes the corresponding `config.control_center.<tile> = on`. When off, the grid renders only tiles whose config bool is true (read in the grid build + an EffectScope to rebuild on change). Default all-on means no visible change until the user edits.

- [ ] **Step 3: Clippy + build + commit**

`cargo clippy -p mshell-config -p mshell-frame` clean; `cargo build -p mshell`.
```
git add -A && git commit -m "feat(control-center): edit mode — per-tile visibility config"
```

---

## Task 7: SCSS

**Files:**
- Create: `mshell-crates/mshell-style/scss/04-components/_control_center.scss`
- Modify: `_index.scss`

- [ ] **Step 1: Style + register + build**

Style `.control-center-menu-widget`, the header (avatar radius, `panel-action-btn`), `.control-center-slider`, `.control-center-tile` (card + `.active` filled `--primary-container` icon-chip vs flat), tile icon-chip, wide/small variants, expand chevron, edit-mode overlay — all in DESIGN.md matugen tokens + `@include state-layer()`; mirror `_audio_dashboard.scss`. Add `@use "control_center";` to `_index.scss`. `cargo build -p mshell-style` succeeds.
```
git add -A && git commit -m "style(control-center): panel + tiles + sliders + header (DESIGN.md tokens)"
```

---

## Task 8: Final clippy/build/Cargo.lock + push

- [ ] **Step 1:** `cargo clippy -p mshell-config -p mshell-core -p mshell-frame -p mshell-settings -p mshell-style` → clean.
- [ ] **Step 2:** `cargo build -p mshell -p mshellctl` → links.
- [ ] **Step 3:** `git status` — commit `Cargo.lock` if any dep changed.
- [ ] **Step 4:** `git push origin main`.

**Manual verification (user, post-rebuild):** `mshellctl menu control-center` (+ bar pill) opens; header avatar/user/uptime + lock/session/settings work; volume + brightness sliders move; Keep Awake/DND/Dark Mode/Night Light toggle; Color Picker launches mpicker; Wi-Fi/Bluetooth/Audio/Mic/Battery tiles expand inline to their detail; edit mode hides/shows tiles + persists.

---

## Self-review

- **Spec coverage:** new menu + full wiring (T1) ✓; header avatar/user/uptime/actions incl. edit (T2/T6) ✓; volume+brightness sliders (T3) ✓; toggle/info tiles — keep-awake/DND/dark/night/color-picker/disk/battery (T4) ✓; inline-expand Wi-Fi/BT/audio/mic/battery reusing detail components (T5) ✓; edit mode + tile-visibility config (T6, last/droppable) ✓; SCSS (T7) ✓; final (T8) ✓.
- **Placeholders:** none — wiring mirrors the named Alarm Clock template + DESIGN.md §6 (boilerplate, not re-listed line-by-line); the tile contract, header (with a TDD'd `fmt_uptime`), slider reuse, inline-expand reuse, and config are concrete with named source components.
- **Type consistency:** names stable across tasks — `MenuType::ControlCenter`, `MenuWidget::ControlCenter`, `control-center` IPC verb, `.control-center-tile`/`.active`, `ControlCenterConfig.<tile>` bools, `fmt_uptime`. Tile `active`/`wide`/`small` props consistent between T4 (definition) and T5 (expand reuse).
