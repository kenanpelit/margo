# mshell UI Design Language

> **How to use this doc.** This is the binding spec for every mshell
> bar pill, menu, and dashboard tile. When a task says "build widget X"
> or "make a menu for Y", follow these rules without being re-told
> them. Point at this file (`mshell-crates/mshell-frame/DESIGN.md`) and
> it's the contract. If a request conflicts with a rule here, the
> request wins for that one widget — but flag the divergence.

The goal: every surface reads as one coherent system — same accent,
same card chrome, same severity colours, same interaction grammar —
so a new widget looks like it always belonged.

### Contents

- **§0** Design philosophy — the *intent* behind everything below.
- **§1** Design tokens — colours, surface tiers, radius, spacing, motion, fonts.
- **§2** Severity ladder — calm / warn / danger.
- **§3** Active-state tinting — live/on = `--primary`.
- **§4** Bar pills — the thin status-chip contract.
- **§5** Menus — layer-shell surfaces, card chrome, revealer-row, lists.
- **§6** Bar → menu wiring checklist — the 11 touch-points.
- **§7** Dashboard & container layout — homogeneous / fill.
- **§8** Settings registration — movable surfaces **and** sidebar pages.
- **§9** Config schema conventions — serde defaults, the two `Config`s.
- **§10** IPC verb convention.
- **§11** Build / SCSS / verify loop.
- **§12** Panel archetype — spacious browse-and-filter surfaces.
- **§13** Interaction philosophy — how the system should *feel*.
- **§14** Visual restraint & Margo identity.
- **§15** Design lint — the quality gate (CI-checkable rules + grep recipes).
- **§16** Component state matrix — rest/hover/pressed/focus/selected/disabled/loading/error per component.
- **§17** Async states — loading / empty / error / no-permission / no-service.
- **§18** Reusable component registry — what to reuse, where it lives, when (not) to.
- **§19** Surface decision tree — pill vs menu vs panel vs tile vs page.
- **Quick checklists** — condensed recipes per surface kind.

§0 and §1–§12 are the *visual* contract; §13 is the *behavioural*
contract; §15–§19 are the *enforcement & reuse* contract (how a rule is
checked, and which prebuilt part satisfies it). All are binding.

**Reorderable rows** (grip + drag standard) live in §5; **settings-page
layout** in §8b; **config migration** in §9; **performance budget**,
**state-continuity matrix**, **operational accessibility**, and
**focus/keyboard navigation** in §13.

---

## 0. Design philosophy

Margo should feel **calm, intelligent, adaptive, composable, and
keyboard-first** — workstation-grade, not flashy. Fast and breathable;
focused and distraction-free. It is uniquely Margo: not a clone of
GNOME, KDE, macOS, or Raycast, but it respects Linux/Wayland workflows.

Guiding rules (these set the *intent*; §1–§14 are the binding details):

1. **Surfaces over borders.** Express grouping and depth with layered
   tonal surfaces + subtle shadow, not hard outlines or boxed layouts.
   A border is a fallback for when two adjacent surface tones are too
   close to separate on their own (see §1 Surface elevation).
2. **Tonal elevation.** Raise an element by shifting it to a brighter
   surface container, not by stacking heavy drop shadows.
3. **Typography creates hierarchy.** Titles SemiBold; secondary text
   regular at reduced opacity; metadata dimmest. Don't lean on many
   font weights.
4. **Adaptive density.** Density scales with context — launcher medium,
   menus compact, settings relaxed, notifications comfortable (§1).
5. **Calm motion.** Motion explains spatial relationships and
   reinforces focus; it never bounces, never plays. Prefer fades, soft
   springs, shared-axis (§1 Motion).
6. **One icon family.** Symbolic, consistent optical size + stroke.
   Never mix flat / skeuomorphic / symbolic in one surface.
7. **State on every interactive element.** hover / focus / pressed /
   disabled, expressed via opacity overlays + tonal shifts, never a
   sudden colour swap (§3, §5).
8. **Spacing scale: 4 / 8 / 12 / 16 / 24 / 32 / 48.** Use it
   consistently; align content; avoid floating, uneven, crowded
   layouts.
9. **Accessibility:** ≥40px interactive targets, WCAG-AA text contrast,
   keyboard-first is mandatory.

The launcher is the heart of the desktop: it must open instantly, feel
lightweight, stay visually calm, and keep search primary.

These rules set the *visual* intent; §1–§12 are the binding detail. The
**behavioural** intent — how Margo should feel in use (cognitive load,
attention, spatial memory, responsiveness, restraint) — lives in §13–§14
and is **equally binding**: design for *mental efficiency*, not for the
screenshot.

---

## 1. Design tokens (never hardcode — always use the CSS variable)

All colours come from matugen at runtime (`:root` overridden by the
style manager). The baseline lives in
`mshell-style/scss/01-tokens/_colors.scss`. **Never hardcode a hex
colour** in a widget's SCSS — reference the variable so the widget
re-themes with the wallpaper.

### Colour roles
| Variable | Use |
|---|---|
| `--primary` | Accent. Active state, sliders, highlights, "this is live" tint. |
| `--on-primary` | Text/icon on a primary-filled surface. |
| `--primary-container` / `--on-primary-container` | Soft accent card (warn state hero). |
| `--surface` | Window/frame background. |
| `--surface-container` | Default card background. |
| `--surface-container-high` / `--highest` | Raised card / slider trough. |
| `--on-surface` | Primary text. |
| `--on-surface-variant` | Secondary text, captions, section labels. |
| `--outline` / `--outline-variant` | Borders, inset hairlines. |
| `--error` / `--on-error-container` / `--error-container` | Danger state only. |

### Surface elevation (tonal layering, not borders)
Express depth by stepping up the surface tier, darkest → brightest:

| Tier | Use |
|---|---|
| `--surface` | window / frame background |
| `--surface-container-lowest` / `--low` | inset "well" (result list card) |
| `--surface-container` | default card / tile |
| `--surface-container-high` | raised card, hover, **selected** row base |
| `--surface-container-highest` | slider trough, top-most chip |
| `--secondary-container` | category chips, soft toggles |

Prefer one tier of separation between a container and its parent. **Add
a border only when the runtime palette's two adjacent tiers are too
close to read** (the static Margo baseline collapses several tiers — at
runtime matugen separates them, so a border that looks needed in the
first-paint baseline is often redundant once themed; verify before
adding `--outline-variant`).

### Shape scale (`01-tokens/_sizing.scss`) — THE radius rule

There are **two corner systems and they don't overlap.** Don't mix them.

**1. Widget interiors → the fixed Material-3 scale.** Everything you see
*inside* a menu / dashboard / popup — buttons, rows, cards, tiles,
entries, the menu surface itself — picks one of these by component
kind. These are fixed (not config-driven); they are the design
language and never change at runtime:

| Token | px | Use |
|---|---|---|
| `--radius-xs` | 10 | nested image/thumb inside a card, inline badge |
| `--radius-sm` | 16 | **buttons, list rows, entries, spins, dropdowns, calendar cells** |
| `--radius-md` | 20 | cards, tiles, hero panels, generic surfaces |
| `--radius-lg` | 28 | launcher / large menu surfaces |
| `--radius-xl` | 32 | search field, hero toggles (a *panel* search is a pill — §12) |
| `--radius-pill` | 999 | toggles/switches, progress bars, category chips |

The scale is intentionally soft (GNOME / ashell-style rounding) — buttons
read as gently-rounded, cards/tiles clearly so. `button-base` is
`--radius-sm`, so **every `.ok-button-*` is 16 by default** — don't
re-declare a button radius per component.

**Nesting (concentric corners).** Two boxes are only stepped down a
notch when one is *physically inside* the other with padding (e.g. a
thumbnail in a card: card `--radius-md`, image `--radius-xs`/`sm`).
**Siblings stacked in a column share the same radius** — a hero card
and the button rows below it are siblings, so they match (see
`_power.scss`). Don't invent a step where there's no nesting.

**2. Frame / bar chrome → config-driven `--radius-widget` /
`--radius-window`.** These are the *only* runtime-tunable corners
(Settings → Theme → Sizing). They apply **exclusively** to the
window frame (`--radius-window`) and the bar pills
(`--radius-widget`, re-applied by the high-specificity `.ok-bar-widget`
rule). They must **never** appear inside a menu/widget — if you reach
for `--radius-widget` in a `04-components/*` or `03-primitives/*` rule
that isn't a bar pill, you've picked the wrong system; use the scale.

**Not a radius at all:** `general.screen_corner_radius` ("Corner
radius (px)" in Settings → General) only sizes the *screen-edge*
rounded-corner overlay mask (and only when `show_screen_corners` is
on). It has zero effect on any widget, button, or menu.

### Spacing scale (`01-tokens/_sizing.scss`)
Use `--space-N` for all padding and gaps — **never a raw `px` value**
(the one exception: a 1–2px hairline/sub-grid micro-inset, since no
token is finer than `--space-1` = 4px — see §15 L2). Component
*dimensions* — `min-width`/`min-height`/icon size — are not on this
scale and take a px value directly.

| Token | px | Use |
|---|---|---|
| `--space-1` | 4 | tightest inset (badge padding, row gap) |
| `--space-2` | 8 | compact padding (dense row, icon gap) |
| `--space-3` | 12 | moderate gap / inset |
| `--space-4` | 16 | default card padding (`≈ --padding-lg`) |
| `--space-5` | 24 | generous padding (`≈ --padding-xl`) |
| `--space-6` | 32 | section gap / outer margin |

### Sizing / padding / icons
- Padding scale: `--padding-sm` 4, `--padding-md` 8, `--padding-lg` 16,
  `--padding-xl` 24. Spacing always snaps to **4 / 8 / 12 / 16 / 24 /
  32 / 48**.
- Icon scale: `--icon-sm` 16, `--icon-md` 24, `--icon-lg` 32.

### Density tiers
Density adapts to context (philosophy §4). Tune row/control padding,
not a magic height: keep the vertical rhythm legible without inflating
a results-heavy surface.

| Surface | Density |
|---|---|
| Launcher / clipboard | **medium** (compact rows, scannable; do not balloon list-item height — many results must stay visible) |
| Menus (QS, dashboards) | compact |
| Settings | relaxed |
| Notifications | comfortable |

### Motion (`01-tokens/_sizing.scss`)
Calm, responsive motion only (philosophy §5). CSS transitions use:
- Durations: `--motion-fast` 120ms (hover/state-layer),
  `--motion-medium` 200ms (selection/focus glow), `--motion-slow` 320ms
  (surface expand/reveal).
- Easing: `--ease-standard` (most), `--ease-decelerate` (enter/expand),
  `--ease-accelerate` (exit/collapse).

Always animate `background-color` / `color` / `border-color` / `opacity`
on state change — never a hard swap. Heavier choreography (staggered
list fade, shared-axis open) belongs in Rust `scoped_effects`, kept
minimal; the bar/menu must never bounce or feel playful.

### Fonts (`01-tokens/_font.scss`)
- **Hierarchy via weight + opacity, not size soup** (philosophy §3):
  a row/card **title** is SemiBold `--on-surface`; its **subtitle /
  description** is *regular* `--on-surface-variant` (often slightly
  reduced opacity); **metadata** (counts, hints, keycaps) is the
  dimmest tier (`--outline`). Titles and subtitles should read as
  clearly different ranks, not two near-equal lines.
- **Size scale — use the token, never a raw `px`.** `--font-2xs` 11 /
  `--font-xs` 12 / `--font-sm` 14 / `--font-md` 16 / `--font-lg` 18 /
  `--font-xl` 26 / `--font-xxl` 32 / `--font-xxxl` 48. Caption/value/chip
  text in dense menus lives at `--font-2xs`/`--font-xs`; body at
  `--font-sm`/`md`. Bespoke hero display sizes (e.g. the clock hero) are
  the only place a literal px is acceptable.
- Bar pill text: **`--font-bar`** (single knob for every status pill —
  always use it for bar labels so they stay in step).
- Section labels in menus: `var(--font-2xs)`, `font-weight: 600`,
  `letter-spacing: 0.05em`, `text-transform: uppercase`,
  `color: var(--on-surface-variant)`. Class: `…-section-label`.
- Numeric readouts: `font-variant-numeric: tabular-nums` so digits
  don't jitter as values change.

---

## 2. Severity ladder (calm / warn / danger)

Any metric with thresholds (CPU, temp, battery, RAM) uses the same
three-step ladder. The Rust side computes the class name; SCSS owns
the colour — the Rust side never knows hex values.

```rust
fn severity_class(value: u32) -> &'static str {
    if value >= DANGER { "danger" } else if value >= WARN { "warn" } else { "calm" }
}
```

- **calm** → no override (inherits `--on-surface-variant`).
- **warn** → `var(--warning)` — amber; intentionally stable (not matugen-tinted) for instant recognition.
- **danger** → `var(--error, #ef4444)`.
- **positive** → `var(--success)` — green; same stability guarantee as `--warning`.

`--warning` (#e0af68) and `--success` (#9ece6a) are declared in `_colors.scss` and are **wallpaper-independent** — matugen never re-declares these keys, so they survive every theme regeneration. This keeps status signals recognisable regardless of the current palette.

Thresholds are tuned high so the UI reads calm at idle (e.g. CPU
warn 70 / danger 90; temp warn 80 / danger 90; battery low 25 /
critical 10). Apply the class to the label/icon **and** the hero
card so tint + chrome escalate together.

---

## 3. Active-state tinting

A widget that can be "live / connected / on" tints its icon (and,
in menus, its name) with `--primary` when active. The Rust side
toggles a CSS class; SCSS paints it. Precedent: bar Bluetooth pill
adds `.connected` to its root → `.bluetooth-bar-widget.connected
image { color: var(--primary); }`; the Bluetooth menu row adds
`.bt-connected` to the label → primary + `font-weight: 600`.

Rule: **active = primary tint, never a separate badge or a size
change.** Inactive falls through to default styling.

---

## 4. Bar pills

A bar pill is a thin status chip that lives in a bar slot. It must:

1. Root `gtk::Box` with classes
   `&["<name>-bar-widget", "ok-button-surface", "ok-bar-widget"]`,
   `set_hexpand: false`, `set_vexpand: false`.
2. Wrap the clickable content in an inner
   `gtk::Button { set_css_classes: &["ok-button-flat"], … }` — the
   bar's transparent-surface + 14%-primary-hover wash comes from
   `.ok-bar-widget` automatically (`_bar_widget.scss`). Do **not**
   paint your own pill background/outline.
3. Left-click → emit a `…Output::Clicked` (opens the widget's menu).
   Right-click → a `gtk::GestureClick` on `BUTTON_SECONDARY` that
   cycles a display mode or toggles a detail (e.g. CPU pill toggles
   RAM%, Audio pill cycles Both/Out/In). Right-click is **ephemeral
   in-memory state**, not persisted.
4. Multi-metric clusters carry the `severity_class` on the inner
   cluster Box (so label + icon tint together), not the outer pill.
5. Tooltip = at-a-glance summary + the interaction hints
   ("Click: open …  ·  Right-click: …").
6. Icons are symbolic (`*-symbolic`). Volume/mic/bluetooth/battery
   pick the icon by level/state via the `mshell_utils` helpers.

Settling on whether a pill *opens a menu*: if yes it's registered as
a **Menu** in Settings (see §8), not a bar-only Pill.

---

## 5. Menus (layer-shell, anchored to the bar)

Menus are `MenuType` surfaces in the frame's menu stack — they open
**contiguous with the bar**, never as a free-floating `gtk::Popover`.
(If you find a Popover inside a bar widget, that's legacy — convert
it to a menu.)

### Card chrome
Reuse the `quick-settings-menu` CSS class on the menu's `css_class`
(in `menus/menu.rs`) to get the shared surface-variant card stack.
Example: `css_class = "quick-settings-menu <name>-menu".to_string();`.
A menu that omits `quick-settings-menu` will look flat/unstyled —
only do that if it has a fully custom surface (e.g. cpu-dashboard
hero).

### The revealer-row pattern (the default row shape)
Collapsible rows are the house style for "status + expand to
details/devices". Built from
`common_widgets/revealer_row/revealer_row.rs`:

- **action button** (left): icon, often a mute/power toggle.
- **content** (middle): a label (`RevealerRowLabelModel`) or a
  slider+% (`RevealerRowSliderModel`).
- **reveal chevron** (right): expands a `gtk::Revealer` holding the
  detail (device list, etc.).

Bluetooth, AudioOutput, AudioInput menu widgets all use this. **When
asked for a menu with "rows that open to show devices/details", reuse
these components — don't rebuild the row.** The AudioDashboard menu,
for instance, is just AudioOut + AudioIn revealer rows stacked.

### Device / list rows
- A row is a flat `gtk::Button` with `…-device-row` class, hover =
  14% primary wash, `all: unset` base.
- The **active** entry shows a `check-symbolic` in `--primary`.
  Clicking a row makes it the active one.

### Scrollable lists & footers (the "dark band" rule)
A `gtk::ListBox` inside a `gtk::ScrolledWindow` (ufw rules, podman
rows, …) has TWO traps. Both must be handled or a short list paints a
dark block of slack between the last card and the footer:

- **Keep the list + its scroller transparent.** A bare `GtkListBox`
  paints the theme's default opaque `list` / view background. Any
  reserved scroll height the rows don't fill then reads as a dark band.
  Always add `.<name>-list, .<name>-menu scrolledwindow {
  background-color: transparent; }` so slack reads as the panel
  surface (the way the DNS menu's plain card column already ends
  cleanly). The row cards keep their own `--surface-container-high`.
- **Size to content, don't reserve a floor.** `set_min_content_height:
  0` + `set_propagate_natural_height: true` + a `set_max_content_height`
  cap. The list then ends right after the last card (≈16px before the
  footer) and only scrolls once it passes the max. A fixed
  `min_content_height` (180/240/…) reserves empty height = the band.
- **Footer is its own region.** Refresh / man-page / action buttons
  live in a row *below* the list on the panel surface (≈16px
  `margin_top`), never inside the scroller — so the list visibly ends
  and the footer reads as separate.

### Buttons that toggle their own label
If a button swaps its label between states (`Apply` ⇄ `Active`,
`Connect` ⇄ `Connected`), pin a fixed `min-width` (CSS class) so it
doesn't reflow when the text length changes — otherwise a column of
them ends up ragged. See `.dns-preset-apply`.

### Read / unread markers
Bar status that has a "new vs. seen" distinction (notifications) uses a
three-state corner dot via `gtk::Overlay`, not just an icon swap:
unread → solid `--error` dot; seen-history → small dim
`--on-surface-variant` dot; empty → no dot. Track `total` vs an
acknowledged `seen` count (bump `seen = total` when the user opens the
surface). See `bars/bar_widgets/notifications.rs`.

### Reorderable rows (the drag-to-reorder standard)
Any list the user can reorder (bar-widget sections, menu-widget lists,
quick-actions, control-center tiles) follows ONE pattern — use
`mshell-settings/src/reorder_dnd.rs`, don't hand-roll DnD per list:

- **Grip handle, always present.** A leading `list-drag-handle-symbolic`
  ≡ icon with class `.reorder-grip`. The gesture binds to the grip, so
  it needs a real hit area: `.reorder-grip` is **≥28×28 px** with a
  `cursor: grab`. A bare symbolic icon (~16 px) is too small to grab —
  pad it, don't ship the raw icon.
- **Gesture, not GTK DnD.** Use a `gtk::GestureDrag` (primary button,
  Capture phase). **Do not use `GtkDragSource`/`GtkDropTarget` for rows
  inside a `GtkListBox`** — the ListBox swallows DnD motion/drop before
  the row sees it, so the drag starts but never lands. (Plain
  `gtk::Box`-hosted rows don't have this problem, but use the gesture
  everywhere for consistency.)
- **Threshold is a fixed pixel step, never the row's own height.** Rows
  with an expanded inline config area are tall; dividing travel by row
  height rounds every normal drag to zero "and nothing moves". Use a
  constant (`STEP_PX`, 32) so one step ≈ one position regardless of row
  height.
- **Visible feedback on `drag-begin`.** Add `.dragging` (a **global**
  rule: `opacity: 0.4`) to the row root so the grabbed row dims
  immediately — feedback within a frame (§13.4), not only on release.
- **Keyboard stays.** The ↑/↓ (and remove) buttons remain for
  keyboard/accessibility (§13.8) — drag is additive, never the only way.
- **Apply through the existing path.** The gesture only reports a signed
  delta; the owning list turns it into one move and clamps to its
  length (bar rewrites config directly; relm4 lists use a `Reorder`
  input → `FactoryVecDeque::move_to`).

### Positioning (multi-monitor, scale, overflow)
Menus open **contiguous with the bar pill that owns them** and must
behave under real multi-head setups:

- **Same output as the pill.** A menu opens on the monitor whose bar was
  clicked / targeted by IPC — never always on the primary.
- **Slide direction follows the bar edge.** Top bar → menu slides *down*;
  bottom bar → *up*; a vertical edge slides inward. The revealer
  transition is chosen from the bar position, not hardcoded.
- **Fractional scale aware.** Sizes come from §1 tokens (logical px); never
  bake a device-pixel size. Verify at 1.0, 1.25, 1.5, 2.0.
- **Edge overflow clamps, never clips.** A menu taller/wider than the
  output is capped to the work area and scrolls internally (§5 scrollable
  lists), staying fully on-screen — it does not run under the screen edge.
- **One menu at a time** per frame; opening another closes the previous
  (the stack is single-occupant).

---

## 6. Bar → menu wiring checklist

Adding a pill that opens its own menu touches these, in order. Miss
one and it silently routes to the wrong menu or doesn't build.

1. **`bars/bar_widgets/<name>.rs`** — pill emits `…Output::Clicked`.
2. **`bars/bar.rs`** — `BarOutput::<Name>Clicked` variant + dispatch
   `.forward(sender.output_sender(), |msg| …)` (not `.detach()`).
3. **`menus/menu.rs`** — `MenuType::<Name>` variant + a match arm
   that sets `css_class` and pushes the widgets / minimum_width /
   maximum_height effects from `config.menus().<name>_menu()`.
4. **`menus/menu_widgets/<name>/…`** — the menu content component
   (+ `mod.rs`, + `menu_widgets/mod.rs` entry).
5. **`menus/builder.rs`** — `MenuWidget::<Name>` → build the widget.
6. **`mshell-config/.../menu_widgets.rs`** — `MenuWidget::<Name>`
   enum variant + `display_name()` + `all_defaults()`.
7. **`mshell-config/.../config.rs`** — `<name>_menu: Menu` field on
   `Menus` with `#[serde(default = "default_<name>_menu")]` + the
   default fn + an entry in `Default for Menus`.
8. **`frame.rs`** — `<NAME>_MENU` const, `Controller<MenuModel>`
   field, `Toggle<Name>Menu` FrameInput, `build_menu(…)` call,
   struct-init entry, the toggle handler, `add_to_stack(…)` with the
   position read from config, and the
   `BarOutput::<Name>Clicked => FrameInput::Toggle<Name>Menu` map.
9. **`mshell-core/.../relm_app.rs`** — `Toggle<Name>Menu(Option<String>)`
   ShellInput + handler that emits `FrameInput::Toggle<Name>Menu`.
10. **`mshell-core/.../ipc.rs`** — `IPCCommand::<Name>` + dispatch +
    the `async fn <name>` interface method.
11. **`mshellctl/.../subcommands/menu.rs`** — `MenuCommands::<Name>`
    + `bus_command("<Name>")`.

Use bluetooth / cpu_dashboard / audio_dashboard as the reference
implementation — copy that shape exactly.

---

## 7. Dashboard & container layout

The dashboard is a `Container` tree (`menu_widgets/container.rs`,
config `ContainerConfig`). Layout flags:

- **`homogeneous: true`** on a horizontal container → children get
  identical widths regardless of natural content. Use it on any
  multi-column body so columns are symmetric.
- **`fill: true`** → the **last** child stretches to claim the
  container's remaining space; children above keep natural sizes and
  stack from the top. Use it on each column so the bottom anchor card
  fills down to a shared bottom edge.

**The two-column dashboard rule:** equal width (`homogeneous` on the
horizontal parent) + equal length (`fill` on each vertical column, so
each column's bottom card — Weather left, MediaPlayer right — grows to
the same bottom edge). The "anchor" card (the big visual one: weather,
media) goes **last** in its column; quiet status tiles stack above.

**Always-on readouts vs alerts:** a persistent metric (e.g. CPU temp)
is always visible with severity wording that escalates ("CPU
temperature 52°C" → "CPU running hot (85°C)"). Don't hide a readout
the user asked to always see behind a threshold — only true *alerts*
(notifications, low battery) are conditionally shown.

---

## 8. Settings registration

### 8a. Movable surfaces (pills & menus)
A new *surface* must appear in Settings → so the user can move/resize
it. Two kinds, mutually exclusive:

- **Opens a menu** → `WidgetEntry::Menu` + a `MenuKind` variant.
  Add the variant to `widget_menu_settings.rs` (`MenuKind` enum,
  `display_name`, `all()`, and **all 12 dispatch match arms**:
  read/tracked/write × position/min_width/max_height + read/tracked/
  write widgets) and a `WidgetEntry::Menu { … }` row in `settings.rs`.
- **Bar-only pill, no menu** → `WidgetEntry::Pill` + a `BarPillKind`
  variant with a `display_name` + a `description`.

Never register the same surface as both. When a former bar-only pill
gains a menu, move it from Pill → Menu and delete the dead
`BarPillKind` variant.

### 8b. Settings *pages* (a new sidebar entry)
A page is a section in the Settings window itself (Idle, Power,
Tiling Layout, …) — distinct from §8a, which registers a *bar/menu
surface* for placement. A page is a standard relm4 component
(`#[relm4::component(pub)] impl Component`) with a `…-page` root box;
**copy `idle_settings.rs` for the shape** — same tokens (§1), a
`settings-hero` header, `.ok-button-primary` actions.

Wiring touches **9 points** — miss one and the page won't build or
won't route:

1. **`mshell-settings/src/lib.rs`** — `mod <page>_settings;`.
2. **`settings.rs`** — `use crate::<page>_settings::{<Page>Init, <Page>Model};`.
3. **`settings.rs`** — `<page>_controller: Controller<<Page>Model>` field
   on `SettingsWindowModel`.
4. **`settings.rs` sidebar** — a `#[name = "<page>_btn"]
   gtk::ToggleButton` in the right sidebar group
   (`set_group: Some(&general_btn)`) whose `connect_toggled[stack]`
   does `stack.set_visible_child_name("<route>")`; symbolic icon +
   `label-medium` title.
5. **`settings.rs` `init`** — build it:
   `let <page>_controller = <Page>Model::builder().launch(<Page>Init {}).detach();`.
6. **`settings.rs` section table** — a `("<lowercased label>", "<route>")`
   row (plus any aliases) in the `(label, route)` search/section list,
   so `mshellctl`/the launcher can jump straight to it.
7. **`settings.rs` `ComponentParts`** — assign the `<page>_controller`
   field in the returned model.
8. **`settings.rs`** — `widgets.stack.add_titled(model.<page>_controller.widget(),
   Some("<route>"), "<Title>");`.
9. **`settings.rs` `ActivateSection`** — a `"<route>" => Some(&widgets.<page>_btn)`
   match arm.

**Where the data lives — pick the right backend:**

- **Shell-owned setting** (a `mshell-config` YAML-profile field) → read
  and write through the **`config_manager()` reactive store**; the live
  profile updates and every surface re-reads. This is the default
  (`idle_settings`, animations, …).
- **Compositor-owned setting** (a margo `.conf` directive) → mshell
  **cannot** reach margo's config through the store; they are separate
  worlds (§9, CLAUDE.md). The page instead **writes a managed fragment**
  `~/.config/margo/<name>.conf`, ensures the user's `config.conf`
  `source`s it (append-once, whitespace-tolerant check), then runs
  **`mctl reload`**. margo seeds itself from the fragment on (re)start.
  Reference: the Tiling Layout page (`tag_layout_settings.rs` →
  `taglayouts.conf`); same shape as the plugin-binds and Keybinds-editor
  fragments. Caveat: mshell runs under `systemd --user` and does **not**
  inherit shell-rc env — `mctl` lives in `/usr/bin` so it still resolves,
  but any env-dependent shell-out from a page must set its env explicitly.

**Settings-page layout standard.** Every page is built from the same
vertical vocabulary, top to bottom — reuse these, don't invent per-page
chrome:

| Region | Shape |
|---|---|
| **Hero** | `settings-hero` header — icon + SemiBold title + dim subtitle. One per page, at the top. |
| **Section** | A labelled group: a `…-section-label` (§1 caption style) over a `--surface-container` / `--radius-md` card. Group related rows; don't free-float controls. |
| **Form row** | Label left (`--on-surface`), control right, baseline-aligned. A dim helper line under it when needed (`--on-surface-variant`). |
| **Switch row** | Form row whose control is a `gtk::Switch`. Title + optional helper left, switch right-aligned, vertically centered. |
| **Reorder row** | The §5 reorderable-row standard (grip + ↑/↓). |
| **Danger zone** | Destructive actions (reset, delete profile) in their own section at the **bottom**, visually separated; buttons use the danger ladder (§2), and irreversible ones confirm first. |
| **Action / footer bar** | Page-level Apply/Save/Reset as `.ok-button-primary` in a trailing row, not scattered inline. Prefer live-apply (write through the store) over an explicit Save where possible. |

Relaxed density (§1). Copy `idle_settings.rs` for the hero + section
shape; copy a Net/BT page for form/switch rows.

---

## 9. Config schema conventions

- Every **new** field on an existing config struct gets
  `#[serde(default)]` (or `#[serde(default = "fn")]`) so older saved
  YAML profiles still parse. **Critical caveat:** `#[serde(default)]`
  defaults a *missing* field to its type default (e.g. `bool` →
  `false`), **not** to the value in the struct's `Default` impl. So a
  user with a saved profile won't pick up a new flag's intended value
  by changing the Rust `Default` alone — their YAML must be migrated
  (add the key) or the block removed to fall back to defaults.
- Both `margo-config` (compositor) and `mshell-config` (shell) export
  types named `Config`/`Menu`/`Position` — verify the import path.

**Versioned migration (when a serde default is the wrong answer).** A
`#[serde(default)]` only protects *parsing*; it can't give an existing
user the *intended* value of a new field (see the caveat above) or
rename/reshape an existing one. When the correct value isn't the type
default, treat it as a migration, not a default:

1. Add the field with `#[serde(default = "fn")]` returning the intended
   value, so fresh installs and missing keys both land on it.
2. If the desired value differs from the type default for *existing*
   profiles (e.g. a new `bool` that should be `true`), run a one-shot
   migration on load that fills the key when absent and rewrites the
   profile — don't rely on the `Default` impl reaching saved YAML.
3. For a rename/reshape, read the old key, write the new one, drop the
   old — keep the old alias readable for one release.
4. **Write the round-trip test:** an old-shape fixture YAML parses, the
   migration applies, and re-serialising yields the new shape with the
   intended values. A field added without this test is a latent
   "works-on-my-fresh-config" bug.

---

## 10. IPC verb convention

`mshellctl menu <name>` toggles a menu; the verb name matches the
`IPCCommand`/`MenuType` name where possible (kebab-case on the CLI,
PascalCase on the bus: `mshellctl menu cpu-dashboard` →
`bus_command("CpuDashboard")`).

---

## 11. Build / SCSS / verify loop

- **SCSS is baked at compile time** (`mshell-style/build.rs` →
  `include_str!`). Editing `.scss` requires a **recompile + restart**
  of mshell to show — it is *not* hot-loaded.
- Build & install the shell:
  ```bash
  cargo build --release -p mshell && sudo install -m755 target/release/mshell /usr/bin/mshell
  systemctl --user restart mshell
  ```
  (or `~/.kod/margo_build/rebuild.sh`).
- **Verify before claiming done:** after restart, confirm mshell is
  healthy with *instantaneous* CPU (`top -bn2 -d 1 -p $(pgrep -x
  mshell)` — the second sample; `ps`'s %CPU is a cumulative average
  and reads high right after a heavy startup, which is **not** a
  hang). Then open the surface (`mshellctl menu <name>`, toggle
  twice) and check `journalctl --user-unit mshell` for panics. For
  layout/visual changes, screenshot it (`grim` + crop with
  `magick`) and actually look — config in the user's saved profile
  can silently override code defaults (see §9).

---

## 12. Panel archetype (spacious surfaces)

Most menus are *compact* status surfaces (§5). A **panel** is the
roomier sibling — a self-contained, app-like surface you scan and
search rather than glance at: clipboard history, and any future
"browse + filter a list" surface. It is still a layer-shell menu
anchored to the bar (§5 holds — never a free-floating popover);
"panel" is its *visual* self-containment (own header, generous
padding, soft elevation), not a detached window.

Everything in §0–§11 still applies — same tokens, same spacing /
radius / motion scales (§1), same severity (§2), same active-tint rule
(§3). This section only adds the patterns a panel layers on top.

### Panel surface
- Background: `--surface` — the panel reads as one calm tonal sheet.
  Never pure black, never a hardcoded hex (§1).
- Outer radius: `--radius-lg` (24), the large-menu corner (§1).
- Outer padding: `--padding-xl` (24) on the content box.
- Elevation: one soft, wide, low-opacity shadow ("hovering surface").
  Never a hard drop shadow, glow, or neumorphism (§0, §14).
- Depth *inside* the panel is tonal, not bordered (§0.1 / §1):
  `--surface` (panel) → `--surface-container` (rows) →
  `--surface-container-high` (hover / selected).

### Header region
A panel opens with a real header, not just a section label:

```
[icon]  Clipboard History                    ( ⌫ )  ( ⚙ )
 leading   SemiBold title (hexpand)            circular actions
```

- **Title**: SemiBold (`font-weight: 600`), `--on-surface`, **`--font-md`
  (16)** — a touch above body weight, *not* a display banner. A bigger
  size (`--font-lg`/`xl`/`xxl`) reads oversized once the header lands on
  the narrow menus (UFW, Bluetooth, …), so the title stays calm and the
  leading icon + tonal weight carry the "this is a header" cue.
- **Leading icon**: symbolic, outline family, same stroke as the rest
  (§0.6) — never a filled glyph.
- **Action buttons** (trailing): icon-only, **perfect circle**
  (`--radius-pill`, equal padding, ≥40×40 — §9 target), resting
  transparent, hover = the canonical 14% primary state-layer (§4/§5).
  Never a naked floating icon, never raised button chrome.
- **Reuse the widget, don't rebuild it.** This header ships as the
  reusable `MenuWidget::PanelHeader { title }` config widget
  (`menu_widgets/panel_header.rs`, `.panel-header` /
  `.panel-action-btn`): leading glyph + title + a live date as dim
  `--outline` metadata + the ⚙ gear. Drop it at the top of any panel's
  widget list (the dashboard does — in place of its old Clock hero).
  The gear calls `open_settings()` with **no** CloseMenu emit — the
  frame's `toggle_menu` already hides the panel, so a CloseMenu after
  would slam Settings shut.

### Segmented control
A single unified capsule that switches between list categories
(clipboard: All · Text · Images · Files · ★) — it must read as **one
track**, not N separate buttons.

- **Track**: `--radius-pill` (999) capsule with a `--surface-container-low`
  fill and `--padding-sm` (4) inset around the segments. That fill
  collapses into `--surface` on palettes where the two tiers coincide
  (the Margo baseline does — both `#282A36`), so the capsule also carries
  a `--outline-variant` hairline (`--border-width`) to read as one unit
  on any palette. This is the §1-sanctioned "border when adjacent tiers
  are too close" case. The 4px inset keeps the active fill clear of the
  ring.
- **Active segment**: `--secondary-container` fill +
  `--on-secondary-container` text, inner radius `--radius-sm` (12).
  This is the soft-toggle tier §1 already sanctions — a calm filled
  selection, **not** a `--primary` flood (controlled saturation, no
  neon) and **not** the §3 live-state icon tint (that's for
  "connected / on"; a segmented choice is a *selection*).
- **Inactive segments**: transparent, `--on-surface-variant` text;
  hover = 14% primary wash.
- **No font-weight bump between states** — a weight change reflows the
  segment widths and makes the strip twitch on switch. Carry state on
  fill + colour only.
- Border: the capsule's `--outline-variant` hairline (above) is the
  *only* resting outline a panel carries — the segments themselves never
  get borders; inactive vs. active is fill + colour alone.

### Panel search (pill query surface)
A panel's search reads as a calm query surface, not a utility input.

- Shape: **`--radius-pill`** (999) — *panel-scoped*. Compact menu /
  launcher search keeps `--radius-xl` (28) per §1; the pill is the
  panel archetype's larger, more inviting field.
- Height: ~52px (`min-height`) with `--padding-lg` (16) horizontal
  inner padding — tune the control's padding, don't chase a magic
  height (§1 density).
- Background: `--surface-container` (one calm tier above the panel
  `--surface`), no resting border.
- Focus: the ring appears **on focus only** — inset 2px `--primary`,
  faded in over `--motion-medium`. Idle reads borderless.
- Leading search icon + placeholder share the dim `--on-surface-variant`
  tier (§1) — present but low-emphasis, never unreadably faint.

### Content rows (lightweight cards, not full cards)
Panel list rows are **content surfaces**, between a plain list-row and
a full card:

- Tier: `--surface-container` — one step above the panel `--surface`,
  *not* the `--surface-container-high` "raised card" tier (reserved
  for hover / selected lift).
- Radius: `--radius-md` (16) — content cards take the card corner, not
  the `--radius-sm` list-row corner.
- Hover: the canonical 14% primary state-layer (tonal lift, never a
  bright swap). **Selected** = inset 2px `--primary` ring (the
  keyboard cursor), never a background flood.
- Typography (§1): the entry's copy text is the content, medium-weight
  `--on-surface`. Metadata around it — the relative-time line, the
  per-row trash / pin affordances — drops to the dim `--outline` tier
  (the same tier as the keyboard-hint strip), **not**
  `--on-surface-variant` (which collapses into `--on-surface` on this
  palette and would read identical to the copy text). Metadata recedes;
  only content and the selection ring carry weight.
- **Density still wins.** Clipboard is **medium** density (§1) — many
  rows must stay on screen. Do *not* inflate rows to a fixed 64–72px;
  a panel's roominess lives in its header / search / padding, not in
  ballooned list items. 64–72px is the comfortable ceiling for a
  *sparse* panel, never a clipboard target.

### Emotional tone
A panel should feel **composed, intentional, fast, readable,
cohesive** — never playful, flashy, "riced", toy-like, or
gamer-themed. (This is §0 + §14 restated for the archetype.) Accent
shows up only on the live / selected / active element, never as
decoration.

**Reference implementation:** the clipboard menu
(`menu_widgets/clipboard/`, `04-components/_clipboard.scss`) and the
dashboard, whose header is the reusable `MenuWidget::PanelHeader`
(`menu_widgets/panel_header.rs`, `04-components/_panel_header.scss`).

---

## 13. Interaction philosophy (how the system should *feel*)

§0–§12 define how Margo *looks*; this section defines how it should
*behave in the user's mind*. Tokens and components are the "how"; this is
the "why" — and it is **binding intent**, checkable in review. When a
component rule and a principle here conflict, the component rule is the
one that's wrong.

Margo's benchmark is not "more components". It is the quiet competence of
GNOME HIG + Material 3, whose real strength is not how they look but that
**they do not tax the user's attention.** Margo is designed to be *used*,
for hours, not admired for a screenshot.

**Precedence (resolve tensions in this order):**

1. **Clarity of feedback** — the user must always know what just happened.
2. **Calm** — never raise visual intensity to win attention.
3. **Continuity** — preserve the user's place and orientation.
4. **Polish** — refinement comes last; it never costs the three above.

A "smoother" animation that delays feedback (1) or breaks calm (2) is a
regression, not an improvement.

### 13.1 Cognitive load

Margo prioritises sustained focus and low mental overhead: minimise the
interpretation an interface demands. The user should rarely have to ask
*where do I look? · what changed? · which element is active? · is this
interactive? · why is this moving? · why is this highlighted?*

1. Prefer **recognition over interpretation** — reuse known shapes (§5
   revealer-row, §12 panel); don't invent a per-surface idiom.
2. Prefer **consistency over novelty**.
3. Prefer **calm over density**.
4. Prefer **structure over decoration**.
5. Prefer **predictable interaction over clever interaction**.
6. **Visual emphasis is a finite budget.** Every accent, animation, and
   highlight must justify itself or be removed.

### 13.2 Attention hierarchy

Attention is directed on purpose. Every surface must make plain what
matters most, what is secondary, what is contextual, and what can be
ignored.

- **One dominant focus region per surface** — no two regions compete.
- **One accent region at a time.** Accent (`--primary` /
  `--secondary-container`) marks only the live / selected / active element
  (§3); it never decorates space and never appears twice on one surface.
  Any escalation beyond that single accent goes through the severity
  ladder (§2) — wording + a glyph — not more colour.
- **Motion reinforces hierarchy, never competes with it.**
- Secondary metadata recedes to the dim `--outline` tier (§1 Fonts).
- Decoration disappears before content does: when space is tight, drop
  ornament first, data last.

> If everything is emphasised, nothing is.

### 13.3 Spatial logic

Surfaces must be spatially predictable so the user builds spatial memory.

- Similar surfaces open from similar places; a menu inherits its slide
  direction from its anchor pill (§5).
- Expanded content stays visually connected to its trigger — the
  revealer-row (§5) is the canonical example.
- **Surfaces never teleport**; large layout shifts are avoided.
- Context stays anchored *during* interaction: opening a menu must not
  reflow the bar, and a dashboard column must not jump when one tile
  updates (the left/right columns stay height-matched — §7).

The interface should feel physically coherent.

### 13.4 Responsiveness

Perceived responsiveness beats decorative smoothness. A fast interface
feels lighter than an impressive one.

- **Feedback begins within one frame.** A hover/click registers
  immediately; never gate the *first* response on an animation or on data
  arriving.
- **Menus appear at once** — the surface and its layout paint together.
  Lazily built content (content is built on first reveal) must never leave
  a menu visibly empty or reflowing: paint the frame, then fill it.
- **Motion is a budget**, bound to the §1 Motion tokens:

  | Interaction | Token | Budget |
  |---|---|---|
  | hover / state layer | `--motion-fast` | ≤ 120 ms |
  | selection / focus | `--motion-medium` | ≤ 200 ms |
  | surface reveal / expand | `--motion-slow` | ≤ 320 ms |

  The more frequent the interaction, the *shorter* the duration — never
  lengthen a hover for drama.
- **Heavy work never blocks input.** Polling, decoding, and IPC run off
  the interaction path (the menu pollers' lazy/visible gating is the
  pattern: a closed menu does nothing).

**Measurable budget (treat as acceptance criteria, not vibes):**

| Rule | Target | How to check |
|---|---|---|
| Menu first paint | frame painted on the open frame; no empty/reflow | open the menu; it never flashes blank then fills |
| Hover/press feedback | ≤ 1 frame (state layer is CSS, not data-gated) | hover never waits on a poll |
| Closed surface is idle | a hidden menu/poller does **no** work | `top`/tracy shows no per-tick CPU while closed; pollers gate on visible/reveal |
| No blocking call on the GTK main thread | never | grep the interaction path for sync `recv()` / `block_on` / `.output()` of a blocking shell-out; all must be off-thread (`spawn` / `command`) and delivered via channel |
| No synchronous shell-out / socket read on the main loop | never | a sync IPC/socket read on the main thread is a freeze risk — bound it with a timeout or move it off-thread |
| Config writes | coalesced | a burst of edits collapses to one profile write, not one per keystroke |

The launcher is the strictest case: it must open instantly and stay
calm under rapid typing (§0).

### 13.5 Surface ownership

Each surface type owns exactly one interaction role; they must not compete
for it.

| Surface | Owns | Interaction |
|---|---|---|
| **Bar pill** (§4) | glanceable status, immediate state | minimal — a click toggles its menu |
| **Menu** (§5) | quick, transient controls | short-lived focus; closes on outside-click |
| **Panel** (§12) | browsing + filtering | extended; search + lists |
| **Dashboard** (§7) | ambient overview | persistent; multi-widget coordination |

When a surface starts doing another's job — a pill growing panel-grade
controls, a menu becoming a dashboard — split it instead.

### 13.6 Density (deepening §1)

Density optimises **scanning speed**, not information count.

- Compact ≠ cramped; spacious ≠ oversized.
- Density emerges from the **4/8/12/16/24 rhythm** (§0.8), not arbitrary
  padding.
- Frequently scanned surfaces (launcher, clipboard) prioritise alignment
  consistency; browsing surfaces (panels) prioritise breathing room.
- **Long-lived surfaces** (dashboard, clipboard history) must stay
  visually sustainable after hours of continuous use — no shimmer, no
  per-update churn, no fatigue.

### 13.7 State continuity

State changes preserve orientation; continuity reduces fatigue.

- Preserve context across interactions — **filters, selections, scroll
  position, and the active tab survive a refresh.**
- Avoid unnecessary resets: a data update repaints the *values that
  changed*, not the whole surface. (This is why the compositor→shell
  mirror is set-if-changed and `state.json` writes are coalesced — the
  shell sees stable updates, not thrash.)
- Layout changes feel progressive, never abrupt. The user never loses
  their place without cause.

**What survives what** (the continuity contract; preserve unless the
row below says otherwise):

| State | Survives a data refresh | Survives close→reopen |
|---|---|---|
| Scroll position | yes | menu: reset to top; panel/dashboard: keep |
| Selected row / active tab | yes | yes |
| Active filter / segmented choice | yes | yes |
| Expanded revealer rows | yes | menu: collapse; panel: keep |
| Search query + results | yes | launcher: clear on close; panel: keep while session lives |

A transient menu may reset scroll/expansion on reopen (it's a glance);
a panel/dashboard is a workspace and keeps it. Never reset a *filter*
or *selection* on a mere value update.

### 13.8 Accessibility (deepening §0.9)

Accessibility is part of interaction quality, not an optional compliance
layer — accessible systems are more usable for *everyone*.

- **Never rely on colour alone.** Pair an accent with a glyph, label, or
  position (the severity ladder §2 already does this).
- **Focus is always visible**; keyboard navigation is mandatory — dense
  layouts included.
- Motion stays comfortable over long sessions; a **reduced-motion**
  preference collapses motion to opacity-only cross-fades while keeping
  every state legible — drop the *animation*, never the *information* it
  carried.
- The UI stays understandable under reduced attention: a glance suffices.

**Operational rules (per interactive widget — not aspirational):**

- **Accessible name on every control.** An icon-only button MUST set an
  `accessible_label` / `set_tooltip_text` that says the *action*
  ("Move up", "Remove"), not the glyph. A tooltip is **not** a
  substitute for the accessible name — set both when they differ.
- **Accessible description for state** the label doesn't carry (e.g. a
  toggle's on/off, a slider's value/unit).
- **Tab order matches reading order** (top→bottom, left→right). Don't let
  GTK's default focus chain wander; group related controls.
- **Don't make decorative widgets focusable.** The `.reorder-grip`, pure
  icons, and separators set `can_target=false`/non-focusable so they
  don't pollute the Tab chain.
- **Screen-reader label ≠ visual label** only when necessary; keep them
  in sync otherwise.

### 13.9 Focus & keyboard navigation

Keyboard-first (§0) is a concrete contract, not a slogan:

- **On open, focus the primary affordance.** A search-first panel
  (launcher, panel §12) focuses the search field; a controls menu
  focuses the first actionable row.
- **`Esc` closes** the current surface and **returns focus** to the bar
  pill / element that opened it (focus never gets stranded).
- **Arrow keys move within a list**; `Tab`/`Shift+Tab` move between
  regions (search → list → footer). `Enter` activates the focused row.
- **Search-first panels** keep typing routed to the query even when a
  result is focused — typing refines, arrows select.
- **Every drag/reorder has a keyboard equivalent** (§5: the ↑/↓ buttons
  stay). Nothing is mouse-only.

## 14. Visual restraint & Margo identity

(This is the tone §12's "Emotional tone" refers back to.)

### 14.1 Visual restraint

Margo avoids unnecessary visual intensity. The interface should feel
**composed and intentional**, not expressive or decorative. Restraint is
what keeps a desktop usable for years — and Margo's defence against the
slow entropy of rice culture.

**Avoid:** excessive saturation · glow · oversized headers · ornamental
animation · gratuitous borders · multiple accent regions · floating
decorative elements · exaggerated shadows · decorative gradients · visual
gimmicks.

**Prefer:** tonal layering (§1) · spacing rhythm · typographic hierarchy ·
subtle motion · predictable alignment · restrained colour.

> Restraint creates longevity.

### 14.2 Margo identity

Margo is not built to impress at first glance. It is built to get *better
over time, through prolonged use.* It should feel **calm · reliable ·
focused · lightweight · intentional · efficient · spatially coherent.**

Margo favours sustained usability over novelty. **The desktop supports
work; it never competes with it.**

---

## 15. Design lint — the quality gate

A rule no one checks is a suggestion. These are the mechanically
**checkable** invariants of §1–§14; wire them into CI (a small
grep/script step over `mshell-crates/`) so a violation fails the build
instead of waiting for review. Each is phrased as "this pattern must
NOT appear" with the grep that finds it.

| # | Rule (§) | Forbidden pattern → grep |
|---|---|---|
| # | Conf | Rule (§) | Forbidden pattern → grep, with the exceptions that keep it false-positive-free |
|---|---|---|---|
| L1 | hard | No hardcoded hex (§1) | `grep -rnE '#[0-9a-fA-F]{3,8}\b' …/scss/0[34]-*` — a hex in a *rule*. Exceptions: comments, and **no `var(--x, #fallback)`** (drop the hex fallback — matugen always defines the token). Only `01-tokens/_colors.scss` may hold hex. |
| L2 | hard | No raw `px` for **spacing / radius / font** (§1) | `grep -rnE '(padding|margin|gap|border-radius|font-size)[^;{]*:\s*-?[0-9]+px'`. Exceptions: **`0px`, `1px`, `2px`** hairline/sub-grid micro-insets are allowed (the spacing scale starts at `--space-1` = 4 px, so there is no sub-4 token). A raw `px ≥ 4` here is a real miss → use `--space-*` / `--radius-*` / `--font-*`. **Not** linted: `min-width`/`min-height`/`-gtk-icon-size`/`min-content-*` — those are component dimensions with no token scale (a bespoke offset like a `30px` avatar inset is fine). |
| L3 | hard | `--radius-widget` / `--radius-window` only on bar/frame (§1) | `grep -rn 'radius-widget\|radius-window' …/scss/04-components` then **exclude `_bar_widget.scss`, `*_bar_widget.scss` (those *are* the `.ok-bar-widget` rules) and comments**. A hit in any other component file = wrong system. |
| L4 | hard | No literal motion **duration** (§1) | `grep -rnE 'transition:[^;]*\bvar\(--motion' must be present; a duration like `200ms` in the duration slot is forbidden. A trailing stagger **delay** (`… var(--ease) 50ms`) is allowed — it's a delay, not a duration, and has no token. |
| L5 | hard | No `gtk::Popover` as a bar widget's **primary** surface (§5) | `grep -rn 'Popover::new\|set_popover' …/bars/bar_widgets` (a click-to-open panel must be a layer-shell menu). **Allowed:** a **right-click context menu** via `PopoverMenu::from_model(_full)` fed a `gio::Menu` (dock item, system-tray item) — that's the correct GTK pattern, not a primary surface. |
| L6 | hard | No `add_css_class("")` (empty class) | `grep -rnE 'add_css_class\(\s*""' …/src` — use `set_css_classes(&[])` for the empty case. |
| L7 | hard | No `GtkDragSource`/`DropTarget` for ListBox row reorder (§5) | `grep -rn 'DragSource\|DropTarget' …/mshell-settings/src` (comments excluded) — reorder uses `reorder_dnd` (GestureDrag). |
| L8 | warn | No sync shell-out / blocking read on the GTK main thread (§13.4) | grep-assisted **manual** review (`block_on` / `.recv()` / `Stdio::inherit` / sync socket read reachable from a widget callback). Not a hard gate — grep can't prove the thread; keep it advisory. |
| L9 | hard | fmt/clippy clean (§11) | `cargo fmt --all -- --check` (exit 0, **not** piped) + `cargo clippy --workspace --all-targets -- -D warnings`. |

"hard" = fails CI; "warn" = reported, doesn't block. Every hard rule's
grep is written so a clean tree returns **zero** matches — if it
false-positives on legitimate code, tighten the grep (or carve the
exception here), don't loosen the rule. An audit of the current tree
against these passes (the one real hit, an `--error` hex fallback, was
removed).

Run `cargo fmt --all -- --check` **without a pipe** and check its exit
code directly — `cargo fmt … | tail` masks the failure behind `tail`'s
exit 0 (this has bitten CI here). Same for any `&&`-chained gate.

**Do / Don't (the rules that bite most):**

| Do | Don't |
|---|---|
| `background-color: var(--surface-container)` | `background-color: #1e1e2e` |
| `border-radius: var(--radius-sm)` on a button | `border-radius: 12px` / re-declaring a button radius |
| `set_css_classes(&[])` for "no class" | `add_css_class("")` |
| `--radius-md` for a card inside a menu | `--radius-widget` inside a menu |
| reuse `revealer_row` / `reorder_dnd` | a bespoke row / per-list DnD copy |
| poll only while the menu is revealed | a timer that ticks while closed |
| selected row = `--surface-container-high` + glyph | selected row = raw colour swap |

## 16. Component state matrix

Every interactive component defines the same state set; "I only styled
hover" is the usual gap. A blank cell = inherits the row above / no
distinct treatment. Build with these in mind and verify each visually.

| Component | rest | hover | pressed | focus | selected / active | disabled | loading | error |
|---|---|---|---|---|---|---|---|---|
| **Button** (`.ok-button-*`) | tonal surface | +`--motion-fast` state-layer wash | brief darker wash | visible focus ring | n/a | 38% opacity, no hover | spinner in place of label, width pinned | danger ladder (§2) if it's a failed action |
| **Bar pill** (§4) | glanceable status | subtle wash | — | ring | live = `--primary` tint (§3) | dim | — | warn/danger glyph + tint (§2) |
| **List / device row** (§5) | transparent | 14% primary wash | — | ring | `--surface-container-high` + `check-symbolic` in `--primary` | dim, non-activatable | skeleton/placeholder row | inline error line (§17) |
| **Reorder row** (§5) | row + grip | grip → grab cursor, full opacity | `.dragging` (0.4) | ring on row | — | ↑/↓ disabled at ends | — | — |
| **Segmented control** (§12) | capsule, dim segments | wash | — | ring | active = `--secondary-container` + hairline | dim | — | — |
| **Search field** (§12) | pill, dim placeholder | — | — | ring + slightly raised | has-query state | — | inline "searching…" if async | "no results" empty state (§17) |
| **Card / tile** (§5/§7) | `--surface-container` | raise to `-high` if interactive | — | ring if focusable | `-high` base | reduced opacity | placeholder content | severity tint (§2) |
| **Toggle / switch** | pill track | wash | — | ring | on = `--primary` | dim | — | — |
| **Notification** | comfortable card | wash | — | ring | unread dot (§5) | — | — | error = danger card |

Focus ring + disabled (≥ here, 38% opacity, pointer-inert) are
**mandatory** on every interactive component, not optional polish.

## 17. Async states (loading / empty / error / no-permission / no-service)

Any surface that reads async data (network, bluetooth, weather, podman,
ufw, plugins) must define all five non-happy states — a blank or
reflowing menu (§13.4) is a bug. Standard visuals:

| State | Looks like |
|---|---|
| **Loading** | Frame paints immediately (§13.4); content area shows a calm placeholder (skeleton rows or a single centered spinner) — **never** an empty box that fills late. |
| **Empty** (loaded, nothing to show) | Centered dim line stating the situation ("No devices found", "No updates"), `--on-surface-variant`, optional small icon. No error styling. |
| **Error** (operation failed) | Inline message in the content area using the danger ladder (§2) + a Retry affordance where it makes sense. Don't blank the surface. |
| **No permission** (auth/polkit needed) | A neutral prompt explaining what's needed + the action that grants it ("Authenticate to view rules"), not an error. (UFW's read-only fallback is the reference.) |
| **No service** (daemon/binary absent) | Calm "not available" line naming what's missing ("UFW not installed") — informational, dim, not red. |

Pick the right tone: *error* (red, something broke) vs *empty/no-service*
(dim, nothing to show) vs *no-permission* (neutral, action available).
Mislabelling "nothing here" as an error is itself a defect.

## 18. Reusable component registry

Before building a row/header/control, check this — most "new" UI is an
existing part. Reuse keeps the system coherent (§0) and is faster.

| Component | Lives in | Use when | Don't use when |
|---|---|---|---|
| **RevealerRow** | `common_widgets/revealer_row/` | status row that expands to details/devices (§5) | a plain non-expanding row (use a device row) |
| **PanelHeader** | `menus/menu_widgets/.../panel_header` (§12) | spacious panel title + circular actions | a compact menu (no hero header) |
| **Segmented control** | §12 pattern | 2–4 mutually exclusive views in a panel | many options (use a list/dropdown) |
| **Panel search** | §12 pill search | browse-and-filter panel | a compact menu (use `--radius-xl` search) |
| **Reorder DnD** | `mshell-settings/src/reorder_dnd.rs` | any user-reorderable list (§5) | non-reorderable list |
| **Device / list row** | §5 row pattern | selectable item in a list | expandable row (use RevealerRow) |
| **Dynamic box** | `mshell-common/.../dynamic_box` | animated add/remove/reorder in a `Box` (dock) | a static or relm4-factory list |
| **Severity class** | Rust `severity_class` + §2 SCSS | any thresholded metric | binary on/off (use §3 active tint) |
| **Managed `.conf` fragment** | §8b pattern (`tag_layout_settings`) | a Settings page writing compositor config | a shell-owned setting (use the store) |

If a needed part doesn't exist, build it as a shared component here, not
inline in one widget.

## 19. Surface decision tree

"Is this new thing a bar pill, a menu, a panel, a dashboard tile, or a
Settings page?" Answer once, up front — picking the wrong surface is
expensive to undo. Walk it top to bottom; first match wins:

1. **Is it a persistent system preference** (not glance-or-toggle, lives
   in its own configuration screen)? → **Settings page** (§8b).
2. **Is it always-on ambient overview**, coordinating several widgets at
   once (clock + calendar + QS)? → **Dashboard tile/section** (§7).
3. **Is its main job browsing + filtering** a list (search, scroll,
   segmented views)? → **Panel** (§12).
4. **Does it need transient controls** beyond a glance (sliders,
   toggles, a device list) shown on demand? → **Menu** (§5), with a bar
   pill to open it (§6).
5. **Is it pure glanceable status / a one-click toggle**, no surface of
   its own? → **Bar pill** (§4).

Cross-checks: a pill that grows sliders/lists wants a menu (§13.5). A
menu that gains search + long lists wants the panel archetype (§12). A
menu doing multi-widget coordination wants a dashboard (§7). When in
doubt, the smaller surface that still fits the role wins — split before
you overload one.

---

## 20. Translucency (`--surface-opacity`)

The painted shell surfaces — the bar and every menu/panel — can frost so the
wallpaper shows through, the way ashell's `opacity` does. One user knob drives
it: **Settings → Theme → Surface opacity** (`theme.attributes.sizing.surface_opacity`,
a percentage 60–100, default 100 = fully opaque).

**The matugen-alpha rule.** Translucency only ever scales a surface's **alpha**
— never recolours it. Colours stay matugen tokens; opacity is a separate
multiplier so the palette still tracks the wallpaper. Two application points,
because margo paints surfaces two ways:

- **Framed (default):** the single `FrameDrawWidget` fills the bar+menu shape;
  it reads `--surface-opacity` and scales its fill alpha (border stays at full
  alpha for definition). So bar and menus frost together as one connected
  surface.
- **Frameless (`.frame-disabled`):** each bar/menu paints its own background —
  `color-mix(in srgb, var(--surface-container) var(--surface-opacity), transparent)`.

`--surface-opacity` is injected on `:root` by the style manager
(`attributes_css_provider`, alongside `--radius-widget` etc.), so it is live
and reactive — no rebuild. **Never** frost a surface that must stay legible
over arbitrary content: the lock screen, modal dialogs, and notification toasts
stay opaque regardless of the knob.

## 21. Button taxonomy

Every button resolves on three axes — don't invent one-off button styles:

- **Kind** — `solid` (filled, the primary affordance), `outline` (hairline
  `--outline-variant`, transparent fill, secondary actions), `ghost`/`flat`
  (no chrome until hover, in-row icon actions; add `flat`).
- **Hierarchy** — `primary` (`--primary` / `--on-primary`), `secondary`
  (`--secondary-container`), `danger` (`--error` / `--on-error`, destructive
  only). At most **one** primary per surface (§13.2).
- **State** — rest → hover (canonical state-layer wash, `@include state-layer`)
  → active/pressed → disabled (drop to ~30 % alpha, never a new grey). All four
  transition with `--motion-fast` `--ease-standard`; never snap.

Radius + size: a **menu action button** (the `.ok-button-cell` family — power
profile/control, session, DNS, UFW footer, network, …) is a **slim fully-rounded
pill**: `--radius-pill`, `--space-2 × --space-4` padding, 84 px min-width, 40 px
min-height. Thin so it never reads as a chunky block; fully rounded for the
GNOME/ashell look. Do **not** reach for `--radius-xl` here — a big radius only
reads as a rounded *square* on a tall, content-filled toggle **tile** (the
control center, §22), not on a slim button, where it just forces an ugly chunky
height. Compact secondary selectors (e.g. the charge-limit % presets) are a
notch slimmer (≈30 px) but the same pill. Pill toggles / category chips also use
`--radius-pill`; tiny inline icon buttons and bar pills (§4) keep their own
radii. One pill family across menus + plugin selectors; tall toggle tiles are a
separate kind (§22).

## 22. Control-center tile anatomy

The quick-settings tile (`control_center/tile.rs`) is the canonical toggle.
Three shapes, one grammar:

- **Normal** — flat `--surface-container` card (`--radius-md`): leading icon,
  then a `title` + a **live `subtitle`** (the current state — "Wi-Fi · MyNet",
  "Twilight · 4000 K", "Bluetooth · 2 devices"; keep it current via
  `set_subtitle`, never leave it static).
- **Expandable** — same, plus a trailing `>` chevron (`go-next-symbolic`) that
  opens the detail/sub-page inline.
- **Small** — icon-only (`.small`), for dense rows.

**Active = whole-tile fill** (GNOME quick-settings, not a coloured chip): the
button fills with `--primary` and icon/title/subtitle flip to `--on-primary`.
The fill **animates** (`transition: background --motion-fast`) so toggling
morphs rather than snaps. Right-click a tile to jump to its Settings page where
one exists. Reuse this tile for any new quick toggle — don't hand-roll a button.

---

## Quick checklists

**New bar pill (opens a menu):** §4 pill shape → §6 all 11 wiring
points → §8 register as Menu → §10 IPC verb → §11 build+verify.

**New menu content:** §5 reuse revealer-row / quick-settings card →
§3 active tint if stateful → §1 tokens only. Scrollable list? §5 keep
the ListBox + scroller transparent and size-to-content (no dark band).
Async data? define all five §17 states (loading/empty/error/no-perm/
no-service). Define every §16 state (esp. focus + disabled).

**Reorderable list (any surface):** §5 reorderable-row standard — reuse
`reorder_dnd` (GestureDrag, fixed `STEP_PX`), `.reorder-grip` ≥28px grip,
`.dragging` feedback, keep ↑/↓ for keyboard (§13.9). Never `DragSource`/
`DropTarget` on `GtkListBox` rows (L7).

**New dashboard tile:** drop into a column's widget list; if it's the
big "anchor" put it last (§7 `fill`); keep quiet tiles compact above;
always-on metrics use escalating severity wording (§7).

**New panel (browse + filter a surface):** §12 — `--surface` panel at
`--radius-lg` + `--padding-xl`; header with a SemiBold `--font-md` title +
circular action buttons; segmented control (active =
`--secondary-container`, `--outline-variant` capsule hairline); pill
search (`--radius-pill`); lightweight `--surface-container` /
`--radius-md` rows with metadata at the dim `--outline` tier. Still a
layer-shell menu (§5); tokens only (§1); keep list density medium —
don't balloon rows.

**New Settings page (a sidebar entry):** §8b — copy `idle_settings.rs`
for the component shape (`settings-hero` header, §1 tokens); wire all 9
points (`mod` in `lib.rs` + use / field / sidebar `ToggleButton` /
builder / section-table row / `ComponentParts` assign / `add_titled` /
`ActivateSection` arm in `settings.rs`). Backend: shell-owned setting →
`config_manager()` store; compositor-owned `.conf` → write a managed
`~/.config/margo/<name>.conf`, `source` it from `config.conf`, run
`mctl reload`.

**Philosophy self-check (every new surface):** §13 — at a glance, can the
user answer *where to look · what changed · what's active · what's
interactive*? Exactly **one** accent region (§13.2)? Feedback within a
frame and motion inside the §13.4 budget (§1 tokens)? Does it own a single
role (§13.5) and keep the user's place across updates (§13.7)? If a "cool"
idea fails any of these it is noise — cut it (§14).

**Pre-merge lint (every PR):** §15 — no hardcoded hex / raw px / literal
motion (L1/L2/L4), `--radius-widget` only on bar/frame (L3), no Popover
in a bar widget (L5), no empty css class (L6), no `DragSource`/
`DropTarget` reorder (L7), no blocking call on the main thread (L8),
`cargo fmt --all -- --check` (un-piped, exit 0) + clippy clean (L9).
