# Dashboard Design-System Stabilization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the dashboard token layer (spacing + semantic colour), make surface elevation real, bring the shared dashboard widgets into token + 3-tier-elevation conformance, fix the media layout + calendar state model, and apply consistent motion — across both dashboards.

**Architecture:** Pure design-system work. Token additions go in `mshell-style/scss/01-tokens/` (matugen never overwrites `--space-*` or new colour keys). Shared widget partials in `04-components/` + `03-primitives/_calendar.scss` are normalized to tokens and a shared `.dash-card` contract. A few Rust widgets (`system_status.rs`, `cpu_dashboard`, `media_player`, `calendar_grid.rs`) toggle state CSS classes. SCSS compiles at build via `grass`.

**Tech Stack:** SCSS (grass), GTK4 CSS, relm4 widgets, matugen colour pipeline.

**Verification:** No automated visual tests. Each SCSS task ends with `cargo build -p mshell-style` (grass must compile); Rust-touching tasks add `cargo clippy -p mshell-frame` + `cargo build -p mshell`. A conformance grep checks no raw spacing px remain. **The user's render review after rebuild is the acceptance gate.**

**Spec:** `docs/superpowers/specs/2026-05-26-dashboard-design-system-design.md`

**Token mapping (apply consistently in every conformance task):**
| Raw px (old) | Token |
|---|---|
| 4 | `var(--space-1)` |
| 8 | `var(--space-2)` |
| 12 | `var(--space-3)` |
| 16 | `var(--space-4)` |
| 24 | `var(--space-5)` |
| 32 | `var(--space-6)` |
| 11px font | `var(--font-2xs)` |
| 12px | `var(--font-xs)` |
| 13–14px | `var(--font-sm)` |
| 16px | `var(--font-md)` |
| 18px | `var(--font-lg)` |
| 22–26px | `var(--font-xl)` |
(Border widths, 1–2px hairlines, and `--radius-*` are NOT spacing — leave them. Snap odd values, e.g. 10px→`--space-3`/12, 6px→`--space-2`/8, to the nearest scale step.)

---

## Task 1: Token foundation (spacing + semantic colour + surface tiers + DESIGN.md)

**Files:**
- Modify: `mshell-crates/mshell-style/scss/01-tokens/_sizing.scss`
- Modify: `mshell-crates/mshell-style/scss/01-tokens/_colors.scss`
- Modify: `mshell-crates/mshell-frame/DESIGN.md`
- Modify: `mshell-crates/mshell-matugen/src/static_themes/margo.rs` (only if it mirrors the surface slots — verify)

- [ ] **Step 1: Add the spacing scale to `_sizing.scss`**

Inside the existing `:root { … }` block (near the radius tokens), add:
```css
  /* Spacing scale (DESIGN.md §1) — never hardcode raw px for gaps/padding. */
  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 24px;
  --space-6: 32px;
```

- [ ] **Step 2: Add semantic colours to `_colors.scss`**

Inside the static `:root { … }` block (after `--error-container`), add:
```css
  /* Semantic accents — stable, wallpaper-independent (matugen never
     re-declares these keys, so they persist through theme regen). */
  --warning: #e0af68;   /* amber — caution / warn tier */
  --success: #9ece6a;   /* green — positive / success */
```
Also update the icon palette line so warning is distinct:
```css
  -gtk-icon-palette: warning var(--warning), error var(--error), success var(--success);
```

- [ ] **Step 3: Re-step the surface tiers in `_colors.scss`**

Replace the collapsed surface-container values with three distinct steps:
```css
  --surface-container-highest: #6272A4;
  --surface-container-high: #3e4257;
  --surface-container: #34374a;
  --surface-container-low: #2b2d39;
  --surface-container-lowest: #232530;
```
(Keep `--surface: #282A36` and `--surface-variant: #44475A` as-is. The rule: low < container < high are visibly stepped.)

- [ ] **Step 4: Sync the static theme mirror (if present)**

Read `mshell-crates/mshell-matugen/src/static_themes/margo.rs`. If it declares the same
surface-container slots with the old collapsed values, update them to match Step 3 (so the Rust
static-theme path and the SCSS baseline agree — the `_colors.scss` comment says they're slot-for-slot).
If `margo.rs` does not carry these slots, skip this step and note it.

- [ ] **Step 5: Document in DESIGN.md**

In §1: under the existing token docs, add a "Spacing" subsection listing `--space-1..6` and the
rule "use `--space-N` for all padding/gaps, never raw px." In §2 (severity ladder): state the colour
mapping — calm = `--on-surface-variant`, **warn = `--warning`**, danger = `--error`, positive =
`--success`; note these semantic colours are intentionally stable (not matugen-tinted) for
recognizability. Keep edits surgical (don't rewrite the sections).

- [ ] **Step 6: Build + commit**

Run: `cargo build -p mshell-style` → must succeed (grass compiles).
```bash
git add mshell-crates/mshell-style/scss/01-tokens/_sizing.scss mshell-crates/mshell-style/scss/01-tokens/_colors.scss mshell-crates/mshell-frame/DESIGN.md mshell-crates/mshell-matugen/src/static_themes/margo.rs
git commit -m "feat(style): spacing scale + semantic warning/success colours + stepped surface tiers"
```
(If `margo.rs` was untouched, drop it from the `git add`.) Do NOT push.

---

## Task 2: Dashboard card contract + elevation hierarchy

**Files:**
- Modify: `mshell-crates/mshell-style/scss/04-components/_overview_intel.scss`
- Modify: `mshell-crates/mshell-style/scss/04-components/_mshelldash.scss`
- (Read first to see the current tile structure + class names.)

- [ ] **Step 1: Read the two partials**

Read `_overview_intel.scss` and `_mshelldash.scss` fully. Note the existing tile/card class names
(e.g. whatever wraps clock/media/weather/system tiles) and which element is the grid container.

- [ ] **Step 2: Add a shared `.dash-card` contract**

In `_overview_intel.scss` (or wherever the dashboard tiles are defined), add a base rule the tiles
share (add the class to the tiles, or `@extend`/apply the rule to the existing tile selector):
```scss
%dash-card {
  border-radius: var(--radius-md);
  padding: var(--space-4);
  background: var(--surface-container);   // default = secondary tier
  transition: background var(--motion-fast) var(--ease-standard);
}
```
Apply it to every dashboard tile selector (`@extend %dash-card;` on each tile rule, or convert
tiles to use a `.dash-card` class). Header region inside a card: title + `margin-bottom: var(--space-3)`.

- [ ] **Step 3: Assign the 3-tier elevation**

- Primary tiles — clock, media, weather — get `background: var(--surface-container-high);`
  (override the `%dash-card` default).
- Secondary tiles — system/CPU/battery/bluetooth/updates/audio — keep `var(--surface-container)`.
- The dashboard grid/background uses `var(--surface)` or `var(--surface-container-low)`.
Add a comment noting the primary/secondary split.

- [ ] **Step 4: Normalize spacing in these two partials**

Replace raw px gaps/padding with `--space-*` per the mapping table at the top of this plan.

- [ ] **Step 5: Build + commit**

Run: `cargo build -p mshell-style` → succeeds.
```bash
git add mshell-crates/mshell-style/scss/04-components/_overview_intel.scss mshell-crates/mshell-style/scss/04-components/_mshelldash.scss
git commit -m "feat(style): dashboard card contract + 3-tier surface elevation"
```

---

## Task 3: System + CPU conformance + semantic temperature

**Files:**
- Modify: `mshell-crates/mshell-style/scss/04-components/_system_status.scss`, `_cpu_dashboard.scss`
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/system_status.rs` and/or `cpu_dashboard/*.rs`

- [ ] **Step 1: Normalize the two partials to tokens**

Read `_system_status.scss` + `_cpu_dashboard.scss`; replace raw px spacing/font with `--space-*` /
`--font-*` per the mapping table. Ensure cards use `--radius-md` (or `%dash-card` if applied).

- [ ] **Step 2: Add semantic temperature classes (SCSS)**

In `_cpu_dashboard.scss` (or `_system_status.scss`, wherever the temp label lives):
```scss
.metric-warning { color: var(--warning); }
.metric-critical { color: var(--error); }
```

- [ ] **Step 3: Toggle the classes from Rust by threshold**

Read the Rust widget that renders the CPU/temperature value (`system_status.rs` or
`cpu_dashboard/cpu_dashboard_menu_widget.rs`). Where it sets the temperature label, toggle the
class based on thresholds (warn ≥ 75 °C, critical ≥ 85 °C — adjust if the widget already has
constants):
```rust
let label = /* the temp gtk::Label */;
label.remove_css_class("metric-warning");
label.remove_css_class("metric-critical");
if temp_c >= 85.0 {
    label.add_css_class("metric-critical");
} else if temp_c >= 75.0 {
    label.add_css_class("metric-warning");
}
```
(Match the widget's existing update flow — apply on each refresh. If temperature is rendered in
more than one place, do the highest-visibility one.)

- [ ] **Step 4: Build, clippy, commit**

Run: `cargo build -p mshell-style` → succeeds; `cargo clippy -p mshell-frame` → clean.
```bash
git add mshell-crates/mshell-style/scss/04-components/_system_status.scss mshell-crates/mshell-style/scss/04-components/_cpu_dashboard.scss mshell-crates/mshell-frame/src/menus/menu_widgets/system_status.rs mshell-crates/mshell-frame/src/menus/menu_widgets/cpu_dashboard
git commit -m "feat(dashboard): system/CPU token conformance + semantic temperature colour"
```
(Only `git add` the Rust files you actually changed.)

---

## Task 4: Media widget redesign

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/media_player/*.rs`
- Modify: `mshell-crates/mshell-style/scss/04-components/_media_player.scss`

- [ ] **Step 1: Read the current media widget + SCSS**

Read `media_player/media_players.rs` (+ siblings) and `_media_player.scss`. Identify the current
layout (cover, title, artist, progress, controls) and the class names.

- [ ] **Step 2: Relayout to the target structure**

Target (adjust to the existing widget tree minimally — don't rebuild from scratch if a small
reorder achieves it):
```
row:  [ cover ]   col( title, artist, progress )
row:           controls( prev, play/pause, next )  // centred
```
- Title: 1 line, `ellipsize: End`, `--font-md`.
- Artist: 1 line, `ellipsize: End`, `--font-sm`, dimmed (`--on-surface-variant`).
- Progress: full content width, fixed height (e.g. `min-height` via the progress primitive),
  consistent margins (`--space-2`).
- Controls: centred horizontal row, `--space-3` gaps.
- Cover: `--radius-sm`, fixed size.

- [ ] **Step 3: Fix clipping + proportion in SCSS**

In `_media_player.scss`: give the title/artist labels proper line-height so glyphs don't clip;
normalize all spacing to `--space-*`; ensure the progress bar spans the content column; controls
use `--space-*` gaps. Tile background = `--surface-container-high` (primary tier).

- [ ] **Step 4: Build, clippy, commit**

Run: `cargo build -p mshell-style`; `cargo clippy -p mshell-frame` → clean; `cargo build -p mshell`.
```bash
git add mshell-crates/mshell-frame/src/menus/menu_widgets/media_player mshell-crates/mshell-style/scss/04-components/_media_player.scss
git commit -m "feat(dashboard): media widget relayout — cover/title/artist/progress + centred controls"
```

---

## Task 5: Calendar state model

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/calendar_grid.rs`
- Modify: `mshell-crates/mshell-style/scss/03-primitives/_calendar.scss`

- [ ] **Step 1: Read the current calendar grid + SCSS**

Read `calendar_grid.rs` + `_calendar.scss`. Note how day cells are built and which classes mark
today / other-month / selected (if any).

- [ ] **Step 2: Define the four states in SCSS**

In `_calendar.scss`:
```scss
.cal-day {
  border-radius: var(--radius-sm);
  transition: background var(--motion-fast) var(--ease-standard),
              box-shadow var(--motion-fast) var(--ease-standard);
}
.cal-day.today {
  background: var(--primary);
  color: var(--on-primary);
}
.cal-day.selected {
  background: transparent;
  box-shadow: inset 0 0 0 1px var(--primary);
}
.cal-day:hover {
  background: color-mix(in srgb, var(--primary) 14%, transparent);
}
.cal-day.inactive {
  color: color-mix(in srgb, var(--on-surface-variant) 55%, transparent);
}
```
(Keep `today` winning over `selected` if both apply — order rules so `today` colour holds.)

- [ ] **Step 3: Wire the classes from Rust**

In `calendar_grid.rs`, ensure each day cell gets `.cal-day`, plus `.today` for the current date,
`.inactive` for days outside the displayed month, and `.selected` for a user-selected day (if the
widget supports selection; if it doesn't, omit `.selected` — don't add selection behaviour). Match
the widget's existing cell-building loop; add/remove the classes there.

- [ ] **Step 4: Build, clippy, commit**

Run: `cargo build -p mshell-style`; `cargo clippy -p mshell-frame` → clean.
```bash
git add mshell-crates/mshell-frame/src/menus/menu_widgets/calendar_grid.rs mshell-crates/mshell-style/scss/03-primitives/_calendar.scss
git commit -m "feat(dashboard): calendar state model — today/selected/hovered/inactive"
```

---

## Task 6: Weather + clock + audio conformance

**Files:**
- Modify: `mshell-crates/mshell-style/scss/04-components/_weather.scss`, `_clock.scss`, `_audio_dashboard.scss`

- [ ] **Step 1: Normalize the three partials**

Read each; replace raw px spacing/font with `--space-*` / `--font-*` per the mapping table; cards
use `--radius-md`. Apply elevation tiers: weather + clock = `--surface-container-high` (primary);
audio = `--surface-container` (secondary). Add the `%dash-card` transition if not inherited.

- [ ] **Step 2: Build + commit**

Run: `cargo build -p mshell-style` → succeeds.
```bash
git add mshell-crates/mshell-style/scss/04-components/_weather.scss mshell-crates/mshell-style/scss/04-components/_clock.scss mshell-crates/mshell-style/scss/04-components/_audio_dashboard.scss
git commit -m "feat(dashboard): weather/clock/audio token conformance + elevation"
```

---

## Task 7: Motion pass (state continuity)

**Files:**
- Modify: dashboard partials touched above that still lack transitions: `_overview_intel.scss`,
  `_system_status.scss`, `_cpu_dashboard.scss`, `_media_player.scss`, `_weather.scss`, `_clock.scss`,
  `_audio_dashboard.scss`, `03-primitives/_calendar.scss`.

- [ ] **Step 1: Audit + add transitions**

For each partial, ensure interactive/state-changing elements have a transition using the motion
tokens (don't snap). Patterns:
- hover / state-layer / active-tint: `transition: background var(--motion-fast) var(--ease-standard);`
- selection / focus glow: add `box-shadow` to the transition list with `var(--motion-medium)`.
- reveal / expand (calendar month swap, any expander): `var(--motion-slow) var(--ease-decelerate)`.
Where a `:hover` or `.active`/`.selected` rule changes background/box-shadow but the base rule has
no `transition`, add one to the base selector. Do not add transitions to text colour changes that
should be instant (e.g. the metric-warning colour) unless subtle.

- [ ] **Step 2: Build + commit**

Run: `cargo build -p mshell-style` → succeeds.
```bash
git add mshell-crates/mshell-style/scss/04-components mshell-crates/mshell-style/scss/03-primitives/_calendar.scss
git commit -m "style(dashboard): consistent motion tokens for hover/focus/selection/reveal"
```

---

## Task 8: Conformance check + full build + push

- [ ] **Step 1: Spacing conformance grep**

Run (touched dashboard partials should have no raw spacing px left — radius/border/min-size px are OK):
```
grep -rnE '(padding|margin|gap|spacing)[^;]*[0-9]+px' mshell-crates/mshell-style/scss/04-components/_overview_intel.scss mshell-crates/mshell-style/scss/04-components/_mshelldash.scss mshell-crates/mshell-style/scss/04-components/_system_status.scss mshell-crates/mshell-style/scss/04-components/_cpu_dashboard.scss mshell-crates/mshell-style/scss/04-components/_media_player.scss mshell-crates/mshell-style/scss/04-components/_weather.scss mshell-crates/mshell-style/scss/04-components/_clock.scss mshell-crates/mshell-style/scss/04-components/_audio_dashboard.scss
```
Expected: no matches (or only intentional ones, noted). Fix stragglers, amend the relevant commit
or add a small follow-up commit.

- [ ] **Step 2: Full build**

Run: `cargo build -p mshell` → succeeds (links the shell with the new styles + Rust class toggles).
`cargo clippy -p mshell-frame` → clean.

- [ ] **Step 3: Cargo.lock + push**

`git status` clean (no new deps expected → no Cargo.lock change). Then `git push origin main`.

**Manual verification (user, post-rebuild):** rebuild + restart mshell; open both dashboards;
confirm — primary tiles (clock/media/weather) read as elevated vs secondary; CPU temp turns amber
(warn) / red (critical), not purple; media widget no longer clips and progress/controls are clean;
calendar today/selected/hovered/inactive are distinct; spacing feels uniform; hover/selection
transitions are smooth.

---

## Self-review

- **Spec coverage:** spacing tokens (T1) ✓; semantic warn/success (T1 + applied T3) ✓; surface
  tiers re-stepped (T1) + elevation applied (T2/T6/T4) ✓; widget card contract (T2) ✓; spacing/font
  conformance (T2/T3/T6) ✓; media redesign (T4) ✓; calendar states (T5) ✓; motion (T7) ✓; DESIGN.md
  §1/§2 (T1) ✓; both dashboards via shared widgets ✓; verification (T8) ✓.
- **Placeholders:** none — token values, semantic colours, surface hexes, `.dash-card`/`.metric-*`/
  `.cal-day` rules, media layout, and the threshold class-toggle are concrete. Conformance tasks give
  a literal px→token mapping table rather than re-listing each line (the files are read in-task).
- **Type/name consistency:** token names (`--space-1..6`, `--warning`, `--success`,
  `--surface-container-low/container/high`) identical across T1→T7; CSS classes `.dash-card`/
  `%dash-card`, `.metric-warning`/`.metric-critical`, `.cal-day`/`.today`/`.selected`/`.inactive`
  consistent between the SCSS task that defines them and the Rust task that toggles them.
