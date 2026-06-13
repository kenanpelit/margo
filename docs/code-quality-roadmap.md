# Code-quality roadmap

A grounded review of margo (compositor + mshell shell) as a codebase, with
prioritised improvement work. Snapshot metrics (2026-06-12, v1.0.3): ~180k
LOC Rust, 53 workspace crates, 765 test fns, 23 TODO/FIXME, ~581
`unwrap`/`expect`/`panic` (non-test). Trend note vs the 2026-05-31
snapshot: tests grew 503 → 765 ✅, but `state.rs` regrew past its Phase-2
target (2944 → 4045 lines) and the unwrap count crept up 563 → 581 —
the two ratchets to watch.

> **Update (1.0.6 internals pass, 2026-06-13):** the unwrap ratchet is now
> a CI hard gate (`scripts/panic-ratchet.sh`, baseline 334 — can only drop);
> `state.rs` was split back **under the <3k bar (4045 → 2441)** into
> `state/{window_rules,focus_methods,dpms,arrange}.rs`; profile **config
> versioning + stepped migration** shipped (`mshell-config/src/migration.rs`).
> See the per-item checkmarks below.

The architecture and discipline are strong (especially `DESIGN.md` and the
parser test coverage). The real debt is **repeated boilerplate, a few god
files, and overlapping feature mechanisms.**

## Strengths (keep these)

- `mshell-frame/DESIGN.md` — a binding design-system spec (tokens, severity
  ladder, wiring checklists). Rare and genuinely valuable.
- Crate-level decomposition — 39+ focused crates with clean responsibility
  boundaries.
- Test weight is on the riskiest layer: `margo-config` parser, session
  snapshot, nmcli parser.
- WASM plugin tier with capability sandboxing — ambitious, well-scoped.
- Perf + panic audit already done (2026-05-23).

## High priority

- [x] **Data-drive the menu/settings boilerplate.** ✅ Done.
  - [x] `widget_menu_settings.rs`: `menu_read!` / `menu_write!` macros hold the
    MenuKind→accessor map once; all 12 dispatch helpers (position / min_width /
    max_height / widgets × read/tracked/write) are now one-liners. ~300 lines
    removed. (ab18f46, + widgets in a follow-up)
  - [x] `menus/menu.rs`: `effect_widgets!` / `effect_min_width!` /
    `effect_max_height!` macros replace the hand-written per-menu reactive
    effect blocks across ~28 MenuType arms. ~492 lines removed. (6ea8f67)
  - Remaining (optional): a single `menu` registry that also drives the
    `MenuType`↔`MenuKind`↔config-accessor relationship so the mapping lives in
    exactly one table rather than two macros.
- [~] **Data-drive Settings-page registration.** *Mostly done (1.0.6).* The
  page stack (`stack_pages` loop) and sidebar (`SIDEBAR` const) were already
  table-driven; the `build_pages!` macro now collapses the 47 controller
  builds into one declarative list. Remaining (optional): a true single-source
  registry that also emits the struct fields + `ComponentParts` assigns so the
  page-list lives in exactly one table (blocked on the heterogeneous typed
  `Controller<…>` fields + the `#[relm4::component]` struct context).
- [x] **Split the god files — `state.rs`.** ✅ Done (1.0.6): 4045 → **2441**,
  back under the Phase-2 <3k bar. Lifted into sibling `impl MargoState` blocks:
  `state/window_rules.rs` (window/tag rules + placement), `state/arrange.rs`
  (the tiling-arrange cluster incl. the ~526-line `arrange_monitor`),
  `state/focus_methods.rs` (keyboard-focus + pointer-monitor), `state/dpms.rs`,
  and `apply_theme_preset` beside its `ThemeBaseline` in `state/theme.rs`.
  Still oversized (optional next): `udev/mod.rs` 3868, `frame.rs` 2885,
  `mctl.rs` 2620, `settings.rs` ~2480.
- [ ] **Unify overlapping mechanisms.** Per-tag layout exists twice
  (`tagrule layout_name` *and* `taglayout`) with previously-undefined
  precedence (caused a bug, fixed in e42c0bb). Menu sizing had min/max +
  plugin panel min/max + auto (since reverted) on the same axis. Make
  "is there already a mechanism for this, and how does it compose?" a design
  step before adding new knobs.

## Medium priority

- [x] **Config migration / versioning.** ✅ Done (1.0.6).
  `mshell-config/src/migration.rs`: `CONFIG_VERSION` + a stepped `migrate_yaml`
  load pre-pass (rewrites an older profile up to current, once) + `stamp_version`
  on save. `config_version` is a file-format meta key (serde ignores it on read)
  so `Config`'s `Store`/`Patch`/`JsonSchema` derives are untouched. v0→v1 is the
  versioning baseline; the next real reshape is a one-arm + one round-trip-test
  change. 7 round-trip tests, incl. bundled-profile parse-after-migrate.
- [~] **Tame reactive-store granularity.** *Partially addressed (1.0.6).* The
  three bar-slot rebuild guards collapsed into one `BarModel::rebuild_slot`
  helper (the distinct-until-changed defence lives in one place now). The
  deeper fix — field-level signals so a write doesn't wake root-bound effects —
  is still open. Original note: a write to any field wakes every
  effect bound to that store, so menus carry a manual `widget_kinds` guard to
  avoid destructive rebuilds (which re-run dns/ufw/podman probes). Extract the
  guard pattern into one helper, or move to finer-grained signals.
- [ ] **Close shell-side test gaps on testable logic.** GTK is hard to test
  (fair), but `mplugin-sdk`, `mshell-plugin-host` (capability/path-sandbox —
  a security boundary!), `mshell-core` (D-Bus/IPC), `mshell-services` are pure
  logic and currently have zero tests.
- [ ] **Reduce `unwrap`/`expect`/`panic` on external input.** 334 in
  non-test code (2026-06-12). In a compositor a panic kills the whole
  desktop; in mshell it kills the bar. Sweep hot paths (render, input, IPC
  handlers, config/file I/O) toward `Result` + graceful degrade.
  **The count is now CI-ratcheted**: `scripts/panic-ratchet.sh` +
  `scripts/panic-baseline.txt` gate every push — the number can only go
  down (raising it needs an explicit baseline bump with rationale; lowering
  it requires locking the new floor in).

## Low priority / quick wins

- [ ] **Build/deploy ergonomics.** No single "build+install+restart" target;
  recurring confusion between compositor vs shell rebuilds and "`mctl reload`
  doesn't pick up new binary code (needs relogin)". Add an `xtask`/`just`
  flow + document the split.
- [x] **CI gate**: shipped — `ci.yml` runs build/test/clippy (`-D warnings`) +
  `mctl check-config`; `smoke.yml` runs the winit smoke under Xvfb.
- [ ] **TODO/FIXME triage** (20): link to issues or remove; "temporary" markers
  outlive everything.

## One-line summary

Highest ROI: move the menu/settings definitions to a **data-driven registry** —
it kills the class of bug hit this cycle and turns "edit 30 files" into "add one
record."
