# mdock — design

**Date:** 2026-06-07
**Status:** Approved (design); implementation plan to follow.
**Source port:** `~/.kod/dock/hydock` (Sergey Desyatkov, GPL-3.0) — a 545-line
Rust + GTK4 + gtk4-layer-shell dock driven by Hyprland IPC. We port its
UX/feature set but back it with **margo's** IPC and ship it as a far more
capable, dual-mode, Settings-driven dock.

## Goal

One dock component, **two display modes**, fully config-driven, branded
**mdock**, superseding the current bar-only dock:

1. **Bar widget** — a pill embedded in an mshell bar (today's `MargoDock`).
2. **Standalone surface** — its own per-output layer-shell window that can be an
   always-visible edge dock, an auto-hide edge dock (hydock-style), or a
   toggle-on-demand surface (opened like a menu via command/keybind).

Both modes share one rendering core, one config, one Settings page.

## Current state

- `mshell-config`: a `dock: Dock` config section.
- `mshell-frame/bars/bar_widgets/margo_dock.rs` (container) + `margo_dock_item.rs`
  (per-app item, ~889 lines): the **bar-only** dock. Item already does
  click→focus/launch, middle-click→new instance, scroll→cycle windows,
  per-class icon override, context menu, running state.
- `mshell-settings/dock_settings.rs`: the (limited) Settings page.
- Running windows come from margo IPC via `mshell-margo-client`
  (`clients`, `Address`, `focuswindow` dispatch).
- mshell builds layer-shell surfaces with `init_layer_shell` / `Layer` /
  `set_anchor` / `KeyboardMode` (see `frame.rs`); menus are on-demand layer
  surfaces — the model for mdock's standalone window.
- Known parked bug (task #13): "click → focus exact window" is broken; fixed as
  part of this work (the focus-by-`Address` path).

## Decisions (locked)

| Question | Decision |
|---|---|
| Where standalone runs | **mshell-hosted** layer-shell surface (reuse margo IPC, icons, theme, config, Settings) — not a separate binary. |
| Relation to bar widget | **Upgrade** the existing dock into mdock: one core, two modes. No second dock. |
| Standalone behavior | **All three, config-selectable**: `always` / `autohide` / `toggle`. |
| Feature set | Full: click=focus/launch, middle=new instance, scroll=cycle, pin/ignore lists, per-class icon override, running dots, launcher button, separator, 4-edge position, auto-hide; **hover preview** (see below). |
| Hover preview | v1 = a **rich card** (large icon + window title(s) + workspace). Live pixel thumbnails are a *future enhancement* — they need per-toplevel capture from margo, which isn't available yet (output capture exists; per-window is "Step 2.5"). |
| Naming | **User-facing = "mdock"** (Settings title, `mshellctl dock`, docs). Internal Rust identifiers (`Dock`, `MargoDock`, `dock_*`) and the config key `dock` are kept to avoid churn + a config break. |
| Standalone outputs | v1 shows on the **active output** only (avoid N simultaneous docks). |

## Architecture

### Component 1 — config (`mshell-config`)

Grow the existing `Dock` struct (serde key `dock`, manual `Default`). New shape
(all serde-defaulted so old configs load):

```
Dock {
  // Bar-widget mode
  in_bar: bool,                       // show the dock as a bar pill (default true)

  // Standalone surface mode
  standalone: bool,                   // enable the standalone surface (default false)
  behavior: DockBehavior,             // Always | AutoHide | Toggle (default AutoHide)
  position: DockPosition,             // Top | Bottom | Left | Right (default Bottom)

  // Contents
  pinned: Vec<String>,                // app class/desktop-id, always shown
  ignore: Vec<String>,                // app classes never shown
  icon_overrides: Vec<(String,String)>, // class -> icon name/path
  show_running_dots: bool,            // running-instance indicator (default true)
  show_labels: bool,                  // hover tooltip with window title (default true)
  hover_preview: bool,                // rich preview card on hover (default true)
  separator: bool,                    // separator between apps and launcher (default true)
  only_current_output: bool,          // filter running apps to focused output (default false)
  only_current_workspace: bool,       // filter to current workspace (default false)

  // Launcher button
  launcher_enabled: bool,             // app-launcher button on the dock (default true)
  launcher_icon: String,             // icon name (default "view-app-grid-symbolic")
  launcher_command: String,           // shell command (default opens the mshell launcher)

  // Geometry
  icon_size: u32,                     // px (default 40)
  spacing: u32,                       // px between items (default 6)
}
```

`DockBehavior` / `DockPosition` are small string-serde enums with `JsonSchema`.
`ConfigStoreFields` already exposes `.dock()`; the new sub-fields get accessors
via the `Store` derive.

### Component 2 — dock core (`mshell-frame`)

A reusable **dock strip**: given the live `clients` (margo store) + `Dock`
config, it builds the ordered item list (pinned first, then running not in
pinned, minus `ignore`, optional output/workspace filters), plus the optional
separator + launcher button. Refactor `margo_dock.rs`'s container into this
strip so **both modes embed the same widget**. The per-app item
(`margo_dock_item.rs`) is reused/upgraded:

- click → focus the app's window by `Address` (**fix task #13**), or launch if
  none running;
- middle-click → launch a new instance;
- scroll → cycle the app's windows;
- right-click → context menu (Focus / New window / Pin or Unpin / Close);
- running-instance dots; per-class icon override; hover tooltip;
- hover → preview card (icon + title(s) + workspace) when `hover_preview`.

Ordering, pin/ignore filtering, and class→icon resolution are **pure functions**
(unit-tested), separate from the widget.

### Component 3 — bar widget mode

`BarWidget::MargoDock` (unchanged enum) renders the dock strip horizontally
inside the bar, gated by `in_bar`.

### Component 4 — standalone surface (`mshell-frame`)

A new per-output layer-shell window (sibling to the bar Frame, created when
`standalone` is true on the active output), hosting the dock strip oriented per
`position` (horizontal for Top/Bottom, vertical for Left/Right):

- **Always**: visible; `auto_exclusive_zone_enable()` so tiled windows don't
  overlap it.
- **AutoHide**: dock hidden; a **1px trigger** layer-shell strip anchored to the
  same edge reveals the dock on pointer-enter; an `EventControllerMotion`
  `leave` on the dock hides it again (hydock pattern). No exclusive zone.
- **Toggle**: hidden; shown/hidden by `mshellctl dock toggle` (and a keybind).
  Exclusive zone optional (off in v1 — behaves like a menu).

`KeyboardMode::None` (the dock never grabs the keyboard). Reveal/hide uses a
GTK `Revealer` slide matching the menu motion language (DESIGN.md §Motion).

### Component 5 — IPC + keybind (`mshell-core`, `mshellctl`)

`mshellctl dock toggle | show | hide` → D-Bus methods on `com.mshell.Shell`
routed to the standalone surface controller (toggle/show/hide its revealer).
Bind `dock toggle` in `binds.conf` for a keyboard summon. The bar pill is
independent of these.

### Component 6 — Settings → Widgets → mdock (`mshell-settings`)

Expand `dock_settings.rs` (title "mdock") with sections:

- **Modes**: `in_bar` switch; `standalone` switch; `behavior` dropdown
  (Always/Auto-hide/Toggle); `position` dropdown (Top/Bottom/Left/Right).
- **Contents**: pinned-apps editor (add/remove by class), ignore-apps editor,
  per-class icon overrides editor, toggles for running dots / labels / hover
  preview / separator / current-output / current-workspace.
- **Launcher**: enable switch + icon entry + command entry.
- **Geometry**: icon size + spacing spin buttons.

Writes go through `config_manager().update_config(...)` (live via the reactive
store) — the strip and surface re-read reactively. Buttons follow the compact
`.settings-page .ok-button-surface` rule. Register in `settings.rs` as usual
(it's an existing page; mostly content growth).

## Data flow

```
margo IPC (clients store)  ─┐
Dock config (reactive)     ─┴─► dock_strip.rebuild()
                                  ├─ bar widget (in_bar)
                                  └─ standalone surface (standalone + behavior)
item click ─► focuswindow(Address) / launch ; middle ─► new instance ; scroll ─► cycle
pin/unpin (context menu) ─► update_config(dock.pinned) ─► reactive rebuild
mshellctl dock toggle ─IPC─► standalone surface revealer
Settings ─► update_config(dock.*) ─► reactive rebuild + surface re-anchor
```

## Error handling / edges

- Empty (no pins, no running) → strip shows only the launcher button (if
  enabled), else nothing.
- Missing icon → existing fallback chain (`app_icon`), then `icon_overrides`,
  then a generic icon.
- AutoHide trigger must NOT reserve an exclusive zone (only the dock does, and
  only in Always).
- `standalone` off → no surface created; `in_bar` off → no pill. Both off is
  valid (dock disabled).
- Active-output resolution reuses the bar's focused-output heuristic; if it
  can't resolve, fall back to the primary output.

## Testing

- Pure fns (unit tests): item ordering (pinned ∪ running − ignore, filters),
  class→icon resolution (override > theme > fallback), `DockBehavior` →
  layer-shell flag mapping (anchor edge, orientation, exclusive-zone on/off),
  `DockPosition` → (Edge, Orientation).
- IPC verb parse for `dock toggle|show|hide`.
- Config serde: old `dock:` loads with new fields defaulted.

## Out of scope (deferred)

- **Live pixel window thumbnails** in the hover preview — needs margo
  per-toplevel capture (compositor work, "Step 2.5"); v1 preview is the rich
  card. Revisit when per-window capture lands.
- Per-output standalone docks on *all* outputs simultaneously (v1 = active
  output).
- Drag-to-reorder pinned apps (pin order follows config list order for now).

## Attribution

Port of hydock (https://github.com/desyatkoff/hydock), GPL-3.0-or-later — same
licence as margo. Credit the upstream author in the module header.
