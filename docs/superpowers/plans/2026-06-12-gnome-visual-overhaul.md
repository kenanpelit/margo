# GNOME-Metrics Visual Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move mshell's whole visual language to GNOME/libadwaita metrics (button 9px/34px, card 12px, surface 15px, boxed-lists, uniform button sizing) driven from ONE central component-token layer, then sweep every surface.

**Architecture:** Token-led. A new `01-tokens/_components.scss` defines *component* geometry (`--button-*`, `--row-*`, `--card-*`) derived from the remapped shape scale in `_sizing.scss`. Primitives (`_buttons.scss` etc.) read ONLY those tokens; components inherit primitives. Waves: W1 central layer + DESIGN.md → W2 Settings → W3 Clipboard → W4 Dashboard → W5 Control-center → W6 full component sweep.

**Tech Stack:** SCSS (grass, baked via `mshell-style/build.rs`), GTK4/relm4 Rust for class changes, `scripts/design-lint.sh` + `cargo fmt/clippy` as gates.

**Spec:** `docs/superpowers/specs/2026-06-12-gnome-visual-overhaul-design.md`

**Verification commands (every wave):**
```bash
cargo build --release -p mshell        # full compile incl. style bake
./scripts/design-lint.sh               # must print "design-lint OK"
cargo fmt --all -- --check             # un-piped, exit 0
```
On-device verification is the USER's (rebuild.sh); never restart their shell.

---

## Wave 1 — central tokens + primitives + DESIGN.md

### Task 1.1: Remap shape scale in `_sizing.scss`

**Files:** Modify `mshell-crates/mshell-style/scss/01-tokens/_sizing.scss:14-19`

- [ ] Replace the radius block with Adwaita values:

```scss
  // Semantic shape scale — GNOME/libadwaita metrics (DESIGN.md §1).
  // Adwaita: $button_radius 9, $card_radius 12, $window_radius 15.
  // Fixed (not config-driven): corner geometry is the design language.
  --radius-xs: 6px;     // nested thumb in a card, inline badge, inner segment
  --radius-sm: 9px;     // buttons, entries, list rows, dropdowns (Adwaita button)
  --radius-md: 12px;    // cards, tiles, boxed-lists (Adwaita card)
  --radius-lg: 15px;    // menu / panel surfaces (Adwaita window)
  --radius-xl: 18px;    // launcher window / largest hero only
  --radius-pill: 999px; // switches, progress, chips, panel search, CTAs
```

### Task 1.2: Central component-token layer (NEW — the single source)

**Files:** Create `mshell-crates/mshell-style/scss/01-tokens/_components.scss`; Modify `01-tokens/_index.scss`

- [ ] Create the file. Every button/row/card in the shell pulls shape,
size and colour roles from here — change a value here, every surface
follows:

```scss
// Component tokens — THE central geometry + colour-role source
// (DESIGN.md §21/§5). Primitives read these; widgets read primitives.
// Tune a component here, never per-widget.
:root {
  // ── Buttons (Adwaita anatomy) ──────────────────────────────
  --button-radius: var(--radius-sm);      // 9px
  --button-min-height: 34px;              // Adwaita standard target
  --button-min-width: 80px;               // labelled action buttons match in a row
  --button-padding: var(--space-1) var(--space-3);
  --button-bg: var(--surface-container-high); // resting fill on page surfaces
  --button-fg: var(--on-surface);
  // Icon-only circular actions (panel header ⌫/⚙, media controls)
  --button-circle-size: 40px;

  // ── Rows / boxed-lists ─────────────────────────────────────
  --row-min-height: 48px;                 // relaxed (Settings)
  --row-min-height-compact: 40px;         // compact (menus)
  --row-padding: var(--space-2) var(--space-3);
  --list-radius: var(--radius-md);        // the boxed-list card corner
  --list-separator: var(--outline-variant);

  // ── Cards / tiles ──────────────────────────────────────────
  --card-radius: var(--radius-md);
  --card-bg: var(--surface-container);
  --card-padding: var(--space-4);

  // ── Entries ────────────────────────────────────────────────
  --entry-radius: var(--radius-sm);
  --entry-min-height: 34px;
}
```

- [ ] Add `@use "components";` to `01-tokens/_index.scss` (after sizing).

### Task 1.3: Rewrite `_buttons.scss` on central tokens

**Files:** Modify `mshell-crates/mshell-style/scss/03-primitives/_buttons.scss`

- [ ] `button-base`: `border-radius: var(--button-radius); min-height: var(--button-min-height);` and inner `> * { padding: var(--button-padding); }`.
- [ ] `.ok-button-cell` (menu action family) — demote from pill, pin the
uniform size so a row of them reads as ONE family (the user's
"boyut eşitliği" requirement):

```scss
// Uniform menu action button — every text button in a menu row shares
// EXACTLY this geometry (size equality across a row is the contract).
.ok-button-cell {
  padding: var(--button-padding);
  min-width: var(--button-min-width);
  min-height: var(--button-min-height);
  border-radius: var(--button-radius);
}
```

- [ ] `.ok-button-large`: `border-radius: var(--card-radius);` (it's a tile), keep 72/116 sizes.
- [ ] `.wizard-button`: stays `--radius-pill` (the prominent-CTA exception), `min-height: var(--button-min-height);`.
- [ ] `.ok-button-medium` / `.ok-button-medium-thin`: keep sizes (icon buttons), no radius change needed (inherit base 9px).

### Task 1.4: New boxed-list primitive

**Files:** Create `mshell-crates/mshell-style/scss/03-primitives/_boxed_list.scss`; Modify `03-primitives/_index.scss`

- [ ] Create:

```scss
// Boxed-list (Adwaita .boxed-list): related rows grouped in ONE card,
// hairline-separated; first/last row inherit the card corners.
// Apply .ok-boxed-list to the container (ListBox or Box column);
// rows are direct children (`row`, `.ok-boxed-row`, or plain buttons).
.ok-boxed-list {
  background-color: var(--card-bg);
  border-radius: var(--list-radius);

  > * {
    border-radius: 0;
    min-height: var(--row-min-height);
    padding: var(--row-padding);
    border-bottom: 1px solid var(--list-separator);
    background-color: transparent;
    transition: background-color var(--motion-fast) var(--ease-standard);
  }
  > *:first-child { border-top-left-radius: var(--list-radius); border-top-right-radius: var(--list-radius); }
  > *:last-child  { border-bottom-left-radius: var(--list-radius); border-bottom-right-radius: var(--list-radius); border-bottom: none; }

  > *:hover {
    background-color: color-mix(in srgb, var(--primary) 14%, transparent);
  }
}
.ok-boxed-list.compact > * { min-height: var(--row-min-height-compact); }
```

- [ ] Add `@use "boxed_list";` to `03-primitives/_index.scss`.

### Task 1.5: Primitive audit (entries / dropdown / spin / switch / calendar)

**Files:** Modify `03-primitives/_entries.scss`, `_spin.scss`, `_calendar.scss`

- [ ] `_entries.scss`: `.ok-entry-with-border` → `border-radius: var(--entry-radius); min-height: var(--entry-min-height);`. `.ok-entry` gains the same min-height.
- [ ] `_spin.scss`: `border-radius: var(--entry-radius);`.
- [ ] `_dropdown.scss`: inherits button-base — no change (verify only).
- [ ] `_switch.scss` / `_progress_bar.scss`: pill — correct, no change.
- [ ] `_calendar.scss`: day cells keep `--radius-sm` (now 9, fine).

### Task 1.6: DESIGN.md rewrite

**Files:** Modify `mshell-crates/mshell-frame/DESIGN.md`

- [ ] §1 shape table → new values + "Adwaita-tight" wording; document the
central `01-tokens/_components.scss` layer as THE source ("tune a
component there, never per-widget") with the token list.
- [ ] Fix every stale parenthesised px in §12 (`--radius-lg (24)` → `(15)`, `--radius-sm (12)` → `(9)`, `--radius-md (16)` → `(12)`, search-pill notes) and §21/§22.
- [ ] §21 rewrite: cell ≠ pill; pill = CTA/switch/chip/panel-search only; uniform-size contract ("buttons sharing a row share `--button-min-width`/`-height` exactly").
- [ ] New §5 "Boxed-list" subsection + §18 registry row + quick-checklist line.
- [ ] §0.9 target note: ≥34px standard, ≥40px icon-only circular.
- [ ] §0 identity sentence per spec §5.7.
- [ ] Compact-menu search rule: `--radius-sm`, panel search stays pill (mark as deliberate exception).

### Task 1.7: Verify + commit W1

- [ ] `cargo build --release -p mshell` → compiles.
- [ ] `./scripts/design-lint.sh` → "design-lint OK".
- [ ] `cargo fmt --all -- --check` → exit 0.
- [ ] Commit: `feat(style): GNOME/libadwaita metrics — central component tokens + boxed-list primitive (W1)` + push.

## Wave 2 — Settings (`mshell-settings` + `_settings.scss`)

**Files:** Modify `mshell-crates/mshell-style/scss/04-components/_settings.scss`, settings page `.rs` files as triaged.

- [ ] Inventory: `grep -rn 'radius-pill\|ok-button-cell\|min-height' mshell-crates/mshell-style/scss/04-components/_settings.scss` + the per-page scss (`_network_settings.scss`, `_power_settings.scss`, `_privacy_settings.scss`, `_keybinds.scss`, `_twilight.scss`, `_wallpaper.scss` …).
- [ ] Delete per-component radius/size overrides that now duplicate the primitives (inherit instead).
- [ ] Boxed-list adoption: pages with grouped form/switch rows get `.ok-boxed-list` on the group container (Rust: `add_css_class("ok-boxed-list")` or `set_css_classes`), rows lose their individual card chrome. Start with `idle_settings.rs` (the §8b reference), then sweep pages.
- [ ] Button equality pass: every action row's buttons → plain `.ok-button-surface`/`.ok-button-primary` (geometry now from central tokens; no bespoke min-widths unless a label-swap pin per §5).
- [ ] Verify (build + lint + fmt), commit `feat(settings): boxed-list rows + central button geometry (W2)`, push.

## Wave 3 — Clipboard panel

**Files:** `04-components/_clipboard.scss`, `menus/menu_widgets/clipboard/`.

- [ ] Panel surface 15 (`--radius-lg` — value already moved), content rows `--card-radius` (12), metadata tiers unchanged.
- [ ] Segmented control: track pill + 4px inset stays; active segment radius `--radius-sm` (9) → Adwaita ToggleGroup look.
- [ ] Search stays pill (deliberate exception, comment it).
- [ ] Delete stale local radius/size overrides; verify, commit `feat(clipboard): panel on GNOME metrics (W3)`, push.

## Wave 4 — Dashboard

**Files:** `04-components/_dashboard*.scss` (and `_clock.scss`, `_calendar*`, `_weather.scss`, `_media_player.scss` dashboard parts), `_panel_header.scss`.

- [ ] Tiles/cards → inherit `--card-radius`; delete local 16/20px-era overrides.
- [ ] Panel-header circular actions: `--button-circle-size` (40), still pill.
- [ ] Grouped row stacks → `.ok-boxed-list.compact` where they are lists (not tiles).
- [ ] Verify, commit `feat(dashboard): GNOME metrics + boxed-lists (W4)`, push.

## Wave 5 — Control-center

**Files:** `04-components/_control_center*.scss`, `control_center/tile.rs` classes if needed.

- [ ] Tiles `--card-radius` (12); active whole-tile `--primary` fill behaviour unchanged.
- [ ] Slider rows + inline detail sub-pages on new scale; detail device lists → `.ok-boxed-list.compact`.
- [ ] Verify, commit `feat(control-center): GNOME metrics (W5)`, push.

## Wave 6 — full sweep (every remaining component)

**Files:** all remaining `04-components/*.scss` + their widgets.

- [ ] Triage recipe per file:
  1. `grep -n 'border-radius\|min-height\|min-width\|radius-pill' <file>`
  2. Override duplicates a primitive value → DELETE the line (inherit).
  3. `--radius-pill` on an action button → `var(--button-radius)`; on a switch/progress/chip/search-hero → keep.
  4. Bespoke min-height/width on text buttons → `var(--button-min-height)`/`--button-min-width` (equality contract); icon-button sizes stay.
  5. Row groups → `.ok-boxed-list(.compact)` where rows are a related set.
- [ ] Sweep order: `_network.scss`, `_bluetooth*`, `_power.scss`, `_session.scss`, `_ufw.scss`, `_podman.scss`, `_vpn.scss`, `_dns*`, `_weather.scss`, `_media_player.scss`, `_notification.scss`, `_notes.scss`, `_ssh_sessions.scss`, `_system_update.scss`, `_privacy.scss`, `_plugins.scss`, `_launcher*`, `_alarm*`, `_ai*`, `_twilight.scss`, `_keybinds.scss`, remaining small files, then `05-surfaces`/`06-windows`.
- [ ] Final gates: build + `./scripts/design-lint.sh` + fmt + clippy workspace.
- [ ] Commit `feat(style): full-shell GNOME-metrics sweep (W6)`, push.
- [ ] Update CHANGELOG under the user-controlled release flow (no version bump).

## Self-review notes

- Spec coverage: spec §2→T1.1/1.2, §3→T1.3, §4→T1.4, §5→T1.6, §6 waves→W2–W6, §8 gates→each wave's verify step. Central-structure directive (user, post-spec) → T1.2 (the spec's token remap is implemented *through* the component layer).
- W2–W6 are triage recipes by design: the content is "delete stale overrides / apply listed classes" with explicit decision rules — no hidden TBDs.
