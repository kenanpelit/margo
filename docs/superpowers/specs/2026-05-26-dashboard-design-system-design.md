# Dashboard Design-System Stabilization (Phase 1 Foundation + Phase 2 Motion) — Design

**Date:** 2026-05-26
**Status:** Approved (design); implementation pending
**Scope:** Stabilize the dashboard's visual design system — complete the token layer (spacing,
semantic colour), make surface elevation real, bring the shared dashboard widgets into token
conformance, fix the two weak widgets (media, calendar state model), and apply consistent motion.
One spec, one PR. Targets BOTH dashboards (mshelldash + the classic dashboard menu), which share
the same widget components.

## Goal

The design *system* (DESIGN.md, 819 lines) is already mature and specifies tokens, surface
elevation, the severity ladder, attention hierarchy, and Margo identity. The dashboard does not
fully *conform* to it, and two token gaps make conformance impossible. This work closes the gaps
and normalizes the dashboard widgets — design-system stabilization, not new features.

## Grounded findings (current state)

- **Tokens that exist:** radius (`--radius-xs/sm/md/lg/xl/pill`), font sizes
  (`--font-2xs … --font-xxxl`, all `* --font-scale`), motion (`--motion-fast/medium/slow`,
  `--ease-standard/decelerate/accelerate`), full Material You colour roles.
- **Gap 1 — no spacing tokens:** there is no `--space-*` scale; partials hardcode px → drift.
- **Gap 2 — no WARN/SUCCESS colour:** `_colors.scss` has `--error` (#FF5555) but no `--warning`
  or `--success`; line 46 maps `warning → error` and `success → primary`. The severity ladder's
  warn tier has no colour of its own, so metrics (CPU temp) can't be "semantic warning."
- **Gap 3 — collapsed surface tiers (static baseline):** `--surface-container-high` ==
  `--surface-container` == `--surface-variant` == `#44475A`; lows all `#282A36`. Only two real
  levels → "every card on the same layer." (matugen *runtime* steps them; the static baseline
  doesn't, and widgets don't pick the tier deliberately.)
- **Runtime colour generation:** `mshell-matugen/src/css_mapping.rs::to_css` re-declares the
  colour `:root` at runtime but emits NO `--warning`/`--success`/`--space` → tokens added to the
  static `_colors.scss`/`_sizing.scss` for those keys SURVIVE matugen regen (it never overwrites
  them). So semantic colours are stable/wallpaper-independent (GNOME-style) with zero matugen change.
- **Shared dashboard widgets** (used by BOTH dashboards): `calendar_grid.rs`, `panel_header.rs`,
  `compact_audio.rs`, `system_status.rs`, `overview_intel.rs`, `cpu_dashboard/`, `media_player/`,
  `weather/`, clock. SCSS: `_clock`, `_cpu_dashboard`, `_media_player`, `_mshelldash`,
  `_overview_intel`, `_system_status`, `_weather`, `_audio_dashboard`, `03-primitives/_calendar`.

## Decisions (locked)

- Targets: both dashboards (shared widgets ⇒ one pass covers both).
- Scope: Phase 1 Foundation + Phase 2 Motion.
- Delivery: one PR, subagent-built. **Aesthetic judgment is the user's** (visual work; verified by
  rebuild + render). Changes are token-driven/structural to minimize subjective guesswork.
- Identity: sharpen toward "Operational Softness" (terminal-rooted, technical, non-aggressive,
  soft) within DESIGN.md §14 — avoid GNOME/KDE/Material clone drift.

## A) Token layer

### Spacing scale (`01-tokens/_sizing.scss` — matugen never touches this file)
```css
--space-1: 4px;   --space-2: 8px;   --space-3: 12px;
--space-4: 16px;  --space-5: 24px;  --space-6: 32px;
```
Document in DESIGN.md §1: "spacing = `--space-N`, never raw px." Map: widget internal padding
`--space-4` (16), section gaps `--space-3` (12), icon↔text `--space-2` (8), micro `--space-1` (4).

### Semantic colour (`01-tokens/_colors.scss` static `:root` — survives matugen)
Add stable, muted, wallpaper-independent semantic tokens (Operational Softness palette):
```css
--warning: #e0af68;   /* amber  */
--success: #9ece6a;   /* green  */
/* --danger maps to existing --error (#FF5555 / matugen) */
```
Container tints derived in partials via `color-mix(in srgb, var(--warning) 18%, transparent)` etc.
Update DESIGN.md §2 severity ladder: calm = `--on-surface-variant`, warn = `--warning`,
danger = `--error`; positive = `--success`. Note these are stable (not matugen-tinted) by design.

### Surface tiers (`01-tokens/_colors.scss` static baseline → 3 distinct steps)
Re-step the Margo baseline so elevation reads even on the default theme:
```css
--surface-container-lowest: #232530;
--surface-container-low:    #2b2d39;
--surface-container:        #34374a;   /* NEW distinct mid */
--surface-container-high:   #3e4257;   /* NEW distinct, clearly above container */
--surface-container-highest:#44475A;
```
(Exact hexes may be nudged for harmony; the rule is low < container < high, visibly stepped. Keep
`mshell-matugen/src/static_themes/margo.rs` in sync if it mirrors these slots.)

## B) Conformance + elevation hierarchy (shared widgets)

### Widget card contract (SCSS)
A shared dashboard-card shape (a SCSS mixin or base class `.dash-card`): `--radius-md` corners,
`--space-4` internal padding, an optional header region (title + `--space-3` below) and a content
region. Every dashboard tile derives from it — kills the outline-vs-filled inconsistency (§6 of
critique). `_overview_intel.scss` / `_mshelldash.scss` own the container/grid; tiles consume `.dash-card`.

### Elevation hierarchy (3-tier surface model)
- **Primary tiles** (clock, media, weather) → `--surface-container-high` (elevated).
- **Secondary tiles** (battery, temp, bluetooth, updates, system metrics) → `--surface-container`.
- **Background / grid** → `--surface` / `--surface-container-low`.
Active/focused state adds the §3 active-tint (primary-container) on top.

### Spacing + font conformance
Replace hardcoded px in every dashboard partial with `--space-*` and the existing `--font-*`
tokens. No raw 11/12/13/14 px font sizes; no raw 6/10/14/20 px gaps.

### Semantic application
CPU temperature (and any threshold metric): a Rust-toggled CSS class —
`.metric-warning` (uses `--warning`) above a warn threshold, `.metric-critical` (uses `--error`)
above a critical threshold; default uses normal text colour, NOT `--primary`.

## C) Weak-widget fixes

### Media widget (`media_player/` + `_media_player.scss`)
Relayout to:
```
[ cover ]  title (1 line, ellipsized)
           artist (1 line, dimmed)
           progress bar (full width)
────────────────────────────────────
[ controls: prev  play/pause  next ]  (centred row)
```
Fix typography clipping (line-height + ellipsize), progress-bar proportion (full content width,
consistent height), and controls density (`--space-*` gaps). Cover uses `--radius-sm`.

### Calendar state model (`calendar_grid.rs` + `03-primitives/_calendar.scss`)
Distinct visual states (Rust toggles the class, SCSS styles it):
| State          | Visual |
|----------------|--------|
| today          | filled `--primary` / `--on-primary` |
| selected       | outlined (`--primary` border, transparent fill) |
| hovered        | soft glow (primary state-layer via color-mix) |
| inactive month | faded (`--on-surface-variant` at reduced opacity) |

## D) Motion (Phase 2)

Apply the existing motion tokens consistently across dashboard widgets for **state continuity**:
- hover / state-layer: `transition: … var(--motion-fast) var(--ease-standard)`.
- selection / focus glow: `--motion-medium`.
- reveal / expand (calendar month change, widget open): `--motion-slow` with `--ease-decelerate`
  (enter) / `--ease-accelerate` (exit).
Audit each dashboard partial; add the missing transitions; ensure no widget snaps without one.
**Honest limit:** GTK CSS transitions are not true spring physics; libadwaita-level smoothness is
approximated with the decelerate/accelerate eases (+ existing relm4/scoped_effects choreography
where a widget already animates in Rust). No new physics engine.

## Decomposition (one PR, ~8 tasks)

1. **Token foundation** — `--space-*` (sizing), `--warning`/`--success` (colors), re-stepped
   surface tiers; DESIGN.md §1/§2 updates; sync `static_themes/margo.rs`. Build `mshell-style`.
2. **Widget card contract + elevation** — `.dash-card` mixin/class; assign primary/secondary tiers;
   apply in `_overview_intel`/`_mshelldash`.
3. **System/CPU conformance + semantic** — `_system_status`, `_cpu_dashboard` to tokens; CPU temp
   `.metric-warning`/`.metric-critical` (Rust class toggle in `system_status.rs`/`cpu_dashboard`).
4. **Media redesign** — `media_player/` layout + `_media_player.scss`.
5. **Calendar states** — `calendar_grid.rs` + `_calendar.scss`.
6. **Weather/clock/audio conformance** — `_weather`, `_clock`, `_audio_dashboard` to tokens + tier.
7. **Motion pass** — consistent `--motion-*`/`--ease-*` transitions across all dashboard partials.
8. **Final** — `cargo build -p mshell-style` + `cargo build -p mshell`; clippy any touched Rust;
   Cargo.lock check; push.

## Verification

- `cargo build -p mshell-style` after each SCSS task (grass compiles); `cargo build -p mshell` +
  `cargo clippy` for tasks touching Rust (system_status, cpu_dashboard, media_player, calendar_grid).
- No automated visual test exists. **User verifies the render after rebuild** — the acceptance gate.
- Token discipline check: `grep -rnE '[^-]([0-9]{1,2})px' <touched dashboard partials>` should find
  no raw spacing/font px (radius/border exceptions noted) — a cheap conformance check.

## Risks / honest limits

- **Aesthetic outcome is subjective** — subagents apply tokens/structure objectively but can't see
  the render; the user's eyeball is the gate. Some tiles may need a follow-up nudge.
- **Surface-tier hexes** are a judgment call; the rule (3 distinct steps) matters more than exact values.
- **Motion** is CSS-transition-based, not spring physics (stated above).
- Re-stepping the static baseline only affects first-paint / default-theme users; matugen users
  already get stepped tiers — verify the change doesn't fight matugen output (it won't; different
  declaration source, matugen wins for its keys).

## Out of scope

Adaptive density modes (Phase 4), responsive/breakpoint dashboard, blur pipeline, GPU rendering
changes, accessibility scaling, new widgets, a true physics-spring animation engine.
