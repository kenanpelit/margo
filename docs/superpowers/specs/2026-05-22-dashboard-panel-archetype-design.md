# Dashboard → §12 panel archetype overhaul

**Date:** 2026-05-22
**Surface:** the classic dashboard (`mshellctl menu dashboard`,
`MenuType::Dashboard`, css `quick-settings-menu dashboard-menu`).
**Binding spec:** `mshell-crates/mshell-frame/DESIGN.md` — §12 (panel
archetype), §7 (dashboard/container layout), §1 (tokens), §0/§14
(accent discipline). This doc only records the dashboard-specific
application; the rules live in DESIGN.md.

## 1. Intent

Bring the dashboard in line with the Phase 2 panel archetype the
clipboard panel established (§12): a composed, calm, intentional
surface with a real header, no decorative accent, and tiles that obey
the token/tier/metadata discipline. The dashboard stays a §7
two-column tile dashboard — this is a *chrome + per-tile* pass, **not**
a re-layout or a rethink of *what* it shows.

Out of scope: changing which tiles exist, the two-column structure, the
tile-merge seams, or the standalone clock menu (`mshellctl menu clock`).

## 2. Decisions (locked via brainstorming)

- **Scope:** chrome + every tile (not chrome-only, not a full
  re-layout).
- **Header:** `[view-grid] Dashboard` (SemiBold 24px title, hexpand) +
  `Fri · May 22` live date as dim `--outline` metadata + a circular ⚙
  settings button. The big clock hero is dropped (redundant with the
  bar clock; the calendar shows the date). The decorative primary
  underline goes away with it.
- **Header mechanism:** a new reusable `MenuWidget::PanelHeader`
  config widget (chosen over render-time injection or mutating the
  Clock widget). Any future panel can drop it in.
- **Footer:** unchanged — `QuickActionWidget::Settings` stays in the
  footer strip (user decision). The header ⚙ and the footer Settings
  intentionally coexist.

## 3. Components / architecture

### 3.1 New `MenuWidget::PanelHeader { title: String }`

A reusable §12 panel header. Renders, left→right:
- leading symbolic icon (`view-grid-symbolic` for the dashboard),
- `title` label — `.dashboard-title`-style class (SemiBold 24px,
  `--on-surface`), `set_hexpand: true`,
- live date metadata label at the dim `--outline` tier
  (`Fri · May 22`), sourced from the same clock mechanism
  `MenuWidget::Clock` already uses,
- circular ⚙ action button (reuse the clipboard `.clipboard-action-btn`
  pattern — generalise it to a shared `.panel-action-btn` class):
  perfect circle `--radius-pill`, ≥40×40, transparent at rest, 14%
  primary hover wash. Click → `mshell_settings::open_settings()` (or
  `open_settings_at_section(...)` — section decided in planning;
  default plain `open_settings`). Opening Settings runs the frame's
  `toggle_menu`, which already hides this panel, so **no CloseMenu
  emit** (same trap the clipboard gear hit).

Title is a config field so the widget is reusable; the dashboard
default sets `title: "Dashboard"`.

**Plumbing touch points** (mirror an existing simple MenuWidget):
1. `mshell-config/.../schema/menu_widgets.rs` — `PanelHeader(PanelHeaderConfig)`
   enum variant + `display_name()` ("Panel Header") + `all_defaults()`
   entry + the `PanelHeaderConfig` struct (`title: String`,
   `#[serde(default)]`).
2. `mshell-frame/.../menus/builder.rs` — `MenuWidget::PanelHeader` arm
   → build the header component.
3. `mshell-frame/.../menus/menu_widgets/panel_header/` — new component
   (+ `mod.rs`, + `menu_widgets/mod.rs` entry).
4. `mshell-config/.../schema/config.rs` — dashboard default tree:
   replace the leading `MenuWidget::Clock` with
   `MenuWidget::PanelHeader(PanelHeaderConfig { title: "Dashboard" })`.
   (Footer QuickActions list unchanged — Settings stays.)
5. SCSS — `_clipboard.scss` `.clipboard-action-btn` generalised to a
   shared `.panel-action-btn` (or a new shared rule) + a
   `.panel-header` / `.panel-title` rule (24px SemiBold), date metadata
   at `--outline`.

### 3.2 Accent cleanup

- The `.qs-clock-hero` primary underline (`inset 0 -2px 0 0 var(--primary)`)
  no longer appears on the dashboard once the Clock hero is replaced by
  PanelHeader. `.qs-clock-hero` itself is **left untouched** (the
  standalone clock menu still uses it) unless the standalone clock menu
  is separately confirmed to want the same cleanup.
- Tile-merge seams (`inset 0 1px 0 0 var(--outline-variant)` between
  calendar+weather, audio pair, network+bluetooth) are §1-compliant
  hairlines — **kept**.

### 3.3 Per-tile audit (§12/§1)

Each dashboard tile is checked against the panel-archetype rules.
Finding after the audit: the actual dashboard tiles
(`_overview_intel`, `_connectivity`, `_compact_audio`,
`_system_status`, `_media_player`, `_weather`, `_calendar`) are already
§-compliant — no hardcoded hex (only §2-sanctioned `var(--error,
#ef4444)` fallbacks), and every `--primary` use is live/active (§3) or
severity (§2), not decoration.

- **`_power.scss` is OUT OF SCOPE.** Its hardcoded `#8ec07c`/`#1e2326`
  belong to `MenuWidget::Power` (the standalone power-profile menu /
  bar pill), which the dashboard does **not** render — the dashboard
  uses `SystemStatus` for power. Left untouched.
- **Metadata tiers:** verify each tile's secondary/metadata text reads
  at the right tier — captions/labels at `--on-surface-variant`, the
  dimmest metadata (timestamps, hints, units where they're incidental)
  at `--outline`, matching §1 / the clipboard panel.
- **Accent discipline:** confirm every `--primary` use is live/active
  (§3) or severity warn (§2), not decoration. Scan showed
  `_compact_audio` (slider fill — OK, §1), `_connectivity` (connected
  tint — OK, §3), `_system_status` / `_overview_intel` (severity — OK,
  §2). `var(--error, #ef4444)` fallbacks are §2-sanctioned.
- **Tile chrome:** already `--surface-container` / `--radius-md` via the
  shared `.quick-settings-menu` rule — no change.

## 4. What stays the same

Two-column layout (§7 homogeneous + per-column fill), the tile set and
order, the tile-merge seams, the QuickActions footer (unchanged —
Settings included), the 860px width, `MenuWidget::Clock` and the
standalone clock menu.

## 5. Verification

Per DESIGN.md §11. The user builds via `~/.kod/margo_build/rebuild.sh`
and reports back; then: `mshellctl menu dashboard`, screenshot (grim +
crop), and check that — header reads as `[grid] Dashboard … (⚙)` with a
dim date and no pink underline; ⚙ opens Settings; tiles read calm with
metadata receding; no panic in `journalctl --user-unit mshell`.
Confirm `mshellctl menu clock` is unchanged.

## 6. DESIGN.md follow-up

§12 already specifies the header region visually. Once the dashboard
ships, note in §12 (or the reference-implementation line) that the
panel header is implemented by the reusable `MenuWidget::PanelHeader`,
so future panels reuse it rather than rebuilding.
