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

### Shape scale (`01-tokens/_sizing.scss`)
Material-3 corner language — pick by component class, don't reuse
`--radius-widget` everywhere:

| Token | px | Use |
|---|---|---|
| `--radius-xs` | 8 | small chips, inline badges |
| `--radius-sm` | 12 | buttons, list items |
| `--radius-md` | 16 | cards, tiles |
| `--radius-lg` | 24 | launcher / menu surfaces |
| `--radius-xl` | 28 | search field |
| `--radius-pill` | 999 | fully-rounded category chips |

(`--radius-widget` / `--radius-window` remain config-driven for the
frame; the scale above is the fixed design language.)

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
- Bar pill text: **`--font-bar`** (single knob for every status pill —
  always use it for bar labels so they stay in step).
- Section labels in menus: `11px`, `font-weight: 600`,
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

## Quick checklists

**New bar pill (opens a menu):** §4 pill shape → §6 all 11 wiring
points → §8 register as Menu → §10 IPC verb → §11 build+verify.

**New menu content:** §5 reuse revealer-row / quick-settings card →
§3 active tint if stateful → §1 tokens only.

**New dashboard tile:** drop into a column's widget list; if it's the
big "anchor" put it last (§7 `fill`); keep quiet tiles compact above;
always-on metrics use escalating severity wording (§7).
