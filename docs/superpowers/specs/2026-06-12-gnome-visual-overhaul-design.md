# GNOME-metrics visual overhaul — design spec

**Date:** 2026-06-12
**Status:** approved direction (user picked "full libadwaita metrics")
**Scope:** mshell visual design language — tokens, button anatomy, list
pattern, DESIGN.md rewrite, then per-surface application waves
(settings → clipboard → dashboard → control-center → full sweep).

## 1. Goal

Move mshell's visual language from the current soft Material-3/ashell
hybrid (button 16 px radius, card 20, menu 28) to **GNOME/libadwaita
metrics** — the proven, calm, tight scale (button 9, card/boxed-list 12,
window 15, pill reserved for prominent CTAs and switches). Buttons,
radii, sizes, lists, and Settings all follow Adwaita anatomy. Margo's
identity layers — matugen palette, severity ladder, keyboard-first,
layer-shell surfaces — are unchanged.

Approach is **token-led**: change the *values* behind the existing
semantic tokens so all ~130 `--radius-sm/md` call-sites shift in one
commit, then fix per-surface *anatomy* (boxed-lists, button heights,
pill demotion) wave by wave.

## 2. Token remap (`mshell-style/scss/01-tokens/_sizing.scss`)

Semantic names stay; only values change. The config-driven
`--radius-widget` / `--radius-window` (bar pills + window frame,
Settings → Theme → Sizing) are **not touched**.

| Token | Old | New | Use (unchanged semantics) |
|---|---|---|---|
| `--radius-xs` | 10 | **6** | nested thumb in a card, inline badge, inner segment |
| `--radius-sm` | 16 | **9** | buttons, entries, list rows, spins, dropdowns, calendar cells (= Adwaita `$button_radius`) |
| `--radius-md` | 20 | **12** | cards, tiles, boxed-lists, hero panels (= Adwaita `$card_radius`) |
| `--radius-lg` | 28 | **15** | menu / panel surfaces (= Adwaita `$window_radius`) |
| `--radius-xl` | 32 | **18** | the launcher window + any future largest-hero surface |
| `--radius-pill` | 999 | 999 | **only**: switches, progress bars, category chips, the panel search, prominent CTAs (wizard nav) |

Spacing scale (`--space-*`), motion tokens, font scale, icon scale:
**unchanged**. Density tiers (§1) unchanged.

### Search-field mapping (resolves the §12 vs Adwaita tension)
- Compact menu / launcher search: `--radius-sm` (9) like every Adwaita
  entry. (Old rule said `--radius-xl`; that dies.)
- Panel search (clipboard archetype): stays `--radius-pill` — GNOME
  Shell's overview search precedent; it is the one hero query surface.

## 3. Button anatomy (DESIGN.md §21 rewrite + `03-primitives/_buttons.scss`)

Adwaita button geometry:

- `button-base`: radius `--radius-sm` (9), **min-height 34 px**, inner
  padding `--space-1 --space-3` (4 × 12; Adwaita is 5 × 17 — nearest
  tokens win, L2 stays clean). Labelled buttons read as compact
  rounded rectangles, not pills or tiles.
- `.ok-button-cell` (menu action buttons — UFW footer, power, session,
  DNS, …): **demoted from pill** → `--radius-sm` (9), min-height 34,
  min-width 80. One family with every other button.
- `.wizard-button`: stays `--radius-pill` — it is the prominent-CTA
  exception (Adwaita `.pill`), used only for onboarding nav.
- `.ok-button-large` (72 px tiles): keep size, radius becomes
  `--radius-md` (12) — it's a tile, not a button.
- Hierarchy/state rules (§21 kind × hierarchy × state, one primary per
  surface, state-layer washes, 34 px+ targets) unchanged.
- Pill demotion triage: of ~75 `--radius-pill` uses, keep pill only for
  switches / progress / chips / panel-search / wizard CTA / segmented
  capsule track; every action button goes to 9 px. Each wave triages
  its own files.

Accessibility note: §0.9's "≥40px interactive targets" relaxes to
"≥34px (Adwaita standard), ≥40px for icon-only circular actions" —
matching GNOME's own metrics.

## 4. Boxed-list pattern (new shared primitive)

New `03-primitives/_boxed_list.scss` + DESIGN.md §5 subsection:

- A group of related rows sits in **one** `--surface-container` card at
  `--radius-md` (12).
- Rows inside are square-cornered, separated by 1 px
  `--outline-variant` hairlines; first/last row inherit the card's top/
  bottom corners (GTK: `.boxed-list` on the ListBox or per-row
  `:first-child`/`:last-child` on a Box column).
- Row anatomy = Adwaita ActionRow: title `--on-surface`, subtitle dim
  `--on-surface-variant`, control trailing, min-height ≈ 48 (relaxed
  density) / 40 (compact menus).
- Hover keeps the canonical 14 % primary wash; selected keeps
  `--surface-container-high` + check glyph (§5 unchanged).
- Adoption: Settings form/switch rows (§8b table gains "rows live in
  boxed-lists, not free-floating cards"), menu device lists, revealer
  detail lists. RevealerRow keeps its component; its container and the
  rows it reveals take the boxed-list chrome.

## 5. DESIGN.md overhaul

The doc is the binding spec; it is rewritten to the new language in the
same commit as the tokens:

1. **§1 Shape scale table** → new values; the "intentionally soft"
   paragraph becomes "Adwaita-tight: buttons read compact, cards
   clearly rounded, large surfaces calm".
2. **Doc↔token drift fixed** — §12 currently cites stale numbers
   (`--radius-lg (24)`, `--radius-sm (12)`, `--radius-md (16)`); every
   parenthesised px in the doc is re-checked against `_sizing.scss`.
3. **§12 panel archetype** → new radii; panel search pill rule kept and
   marked as the deliberate exception; compact search drops to
   `--radius-sm`.
4. **§21 button taxonomy** → rewritten per §3 above (cell ≠ pill
   anymore; pill = CTA/switch/chip only).
5. **§22 control-center tile** → `--radius-md` (12), GNOME QS fill
   behaviour unchanged.
6. **New §5 boxed-list subsection** + §18 registry row + quick-checklist
   entries.
7. **§0 identity sentence** → "not a clone of GNOME" becomes: Margo
   *adopts GNOME HIG metrics and component anatomy* (a proven, calm
   system); identity lives in matugen adaptivity, severity ladder,
   keyboard-first, and layer-shell surfaces — not in bespoke geometry.
8. **§15 lint table unchanged** (greps are value-agnostic;
   `scripts/design-lint.sh` needs no edit). §0.9 target-size note per
   §3.

## 6. Application waves

Each wave = one commit, pushed; the user verifies on-device via
`rebuild.sh` and reports back (no auto-build of the live shell). A wave
only starts after the previous one is visually accepted.

| Wave | Scope | Key work |
|---|---|---|
| **W1** | tokens + primitives + DESIGN.md | §2 values, `_buttons.scss` anatomy, `_boxed_list.scss`, `_entries`/`_dropdown`/`_spin`/`_switch` audit to 9 px/34 px, doc rewrite. Whole shell tightens at once. |
| **W2** | **Settings** (`mshell-settings` + `_settings.scss`) | boxed-list adoption for form/switch rows across all pages, button cleanup (compact 9 px standard already close), hero/section spacing per Adwaita preference-page rhythm. |
| **W3** | **Clipboard** (`_clipboard.scss`, clipboard menu) | panel archetype on new scale: 15 px panel, 12 px content rows, pill search kept, segmented capsule (track pill, active segment `--radius-sm` 9 → reads like Adwaita ToggleGroup). |
| **W4** | **Dashboard** (`_dashboard*`, panel_header, tiles) | 12 px tiles/cards, boxed-list where rows group, panel header circular actions stay ≥40. |
| **W5** | **Control-center** (`control_center/`, tile.rs SCSS) | 12 px tiles, slider rows, inline-expand detail surfaces on new scale. |
| **W6** | **Full sweep** — every remaining `04-components/*.scss` + bar widget menus (network, bluetooth, power, session, UFW, podman, VPN, weather, media, notifications, OSD, launcher, plugins, …) | pill-demotion triage, boxed-list adoption where rows group, stale per-component radius overrides deleted (most components inherit the new primitives for free). `scripts/design-lint.sh` + fmt/clippy green. |

W1 is deliberately global: because values live in tokens, the first
rebuild already shows the new language everywhere; W2–W6 are anatomy
(structure) work, not re-theming.

## 7. What does NOT change

- matugen colour roles, surface-elevation tiers, severity ladder (§2),
  active-tint rule (§3).
- Spacing scale, motion tokens, font tokens, density tiers.
- `--radius-widget` / `--radius-window` config knobs + their defaults.
- Bar pill contract (§4), menu wiring (§6), Settings registration (§8),
  IPC (§10), panel archetype roles (§13.5).
- `scripts/design-lint.sh` rules L1–L7.

## 8. Verification

- Per wave: `cargo build --release -p mshell` compiles;
  `scripts/design-lint.sh` exits 0; `cargo fmt --all -- --check`
  (un-piped) + clippy clean.
- On-device: user runs `rebuild.sh`, opens Settings / clipboard /
  dashboard / control-center, reports back. Screenshot review for each
  wave (`grim` + crop) before the next wave starts.
- Saved-profile caveat (§9): no config schema changes are planned, so
  no migration; but verify the user's profile doesn't pin sizes that
  mask token changes (menu min-width/max-height are config — fine).

## 9. Risks

- **Pill demotion churn (75 uses):** mitigated by triage table in §3 —
  primitives flip most of them in W1; per-component overrides are
  deleted, not edited, in W6.
- **Tight radii on big translucent menus** may read sharp against the
  compositor's rounded frame (`--radius-window` default 8): acceptable —
  Adwaita windows are 15 px and the frame knob remains user-tunable.
- **GTK hairline rendering** of boxed-list separators at fractional
  scales: use `border-top: 1px solid var(--outline-variant)` (L2 allows
  1 px), verify at 1.25/1.5 per §5 positioning rules.
