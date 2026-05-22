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

---

## 0. Design philosophy

Margo should feel **calm, intelligent, adaptive, composable, and
keyboard-first** — workstation-grade, not flashy. Fast and breathable;
focused and distraction-free. It is uniquely Margo: not a clone of
GNOME, KDE, macOS, or Raycast, but it respects Linux/Wayland workflows.

Guiding rules (these set the *intent*; §1–§11 are the binding details):

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
| `--radius-xs` | 8 | nested image/thumb inside a card, inline badge |
| `--radius-sm` | 12 | **buttons, list rows, entries, spins, dropdowns, calendar cells** |
| `--radius-md` | 16 | cards, tiles, hero panels, generic surfaces |
| `--radius-lg` | 24 | launcher / large menu surfaces |
| `--radius-xl` | 28 | search field (compact menus / launcher; a *panel* search is a pill — §12) |
| `--radius-pill` | 999 | toggles/switches, progress bars, category chips |

`button-base` is `--radius-sm`, so **every `.ok-button-*` is 12 by
default** — don't re-declare a button radius per component.

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

- **calm** → no override (inherits `--on-surface`).
- **warn** → `var(--primary)`.
- **danger** → `var(--error, #ef4444)`.

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

A new surface must appear in Settings → so the user can move/resize
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

- **Title**: SemiBold (`font-weight: 600`), `--on-surface`, a bespoke
  **24px** display size — bigger than body, but `--font-xxl` (32) reads
  oversized in a panel header. This (and the clock hero) are the only
  literal-px display sizes §1 permits; a panel title is the one place a
  menu goes display-size at all (compact menus use a `…-section-label`).
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

## Quick checklists

**New bar pill (opens a menu):** §4 pill shape → §6 all 11 wiring
points → §8 register as Menu → §10 IPC verb → §11 build+verify.

**New menu content:** §5 reuse revealer-row / quick-settings card →
§3 active tint if stateful → §1 tokens only. Scrollable list? §5 keep
the ListBox + scroller transparent and size-to-content (no dark band).

**New dashboard tile:** drop into a column's widget list; if it's the
big "anchor" put it last (§7 `fill`); keep quiet tiles compact above;
always-on metrics use escalating severity wording (§7).

**New panel (browse + filter a surface):** §12 — `--surface` panel at
`--radius-lg` + `--padding-xl`; header with a SemiBold 24px title +
circular action buttons; segmented control (active =
`--secondary-container`, `--outline-variant` capsule hairline); pill
search (`--radius-pill`); lightweight `--surface-container` /
`--radius-md` rows with metadata at the dim `--outline` tier. Still a
layer-shell menu (§5); tokens only (§1); keep list density medium —
don't balloon rows.
