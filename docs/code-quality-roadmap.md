# Code-quality roadmap

A grounded review of margo (compositor + mshell shell) as a codebase, with
prioritised improvement work. Snapshot metrics (2026-05-31): ~186k LOC Rust,
58 crates, 503 test fns across 100 files, 123 `unsafe`, 27 files >1000 LOC,
20 TODO/FIXME, ~563 `unwrap`/`expect`/`panic`.

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

- [ ] **Data-drive the menu/settings boilerplate.** `menus/menu.rs` carries
  ~32 hand-written per-menu reactive effect blocks; `widget_menu_settings.rs`
  repeats the same ~27-arm `MenuKind` match for *every* field
  (read/tracked/write × position/min_width/max_height). Adding one field means
  editing 30+ sites — error-prone (a single missed multi-line block caused a
  real bug). Collapse the per-menu match into one macro / registry so each menu
  is a single record and the dispatch + effects are generated.
  - First slice (bounded, this file only): a `for_menu!` macro that emits the
    MenuKind match once; rewrite the 6 read/tracked/write fns through it.
- [ ] **Data-drive Settings-page registration.** Adding a sidebar page is a
  manual 9-point wiring (mod + use + field + sidebar button + builder + route
  + ComponentParts + add_titled + ActivateSection). Easy to get wrong (the
  Tiling Layout button landed in the wrong alphabetical slot). Drive it from a
  table of `(SettingsPage, route, icon, title, builder)`.
- [ ] **Split the god files.** `state.rs` 3559, `udev/mod.rs` 3344,
  `mctl.rs` 2883, `frame.rs` 2708. Split `impl MargoState` across submodule
  `impl` blocks (input / render / layout / ipc) — no behaviour change, just
  smaller units that fit in context.
- [ ] **Unify overlapping mechanisms.** Per-tag layout exists twice
  (`tagrule layout_name` *and* `taglayout`) with previously-undefined
  precedence (caused a bug, fixed in e42c0bb). Menu sizing had min/max +
  plugin panel min/max + auto (since reverted) on the same axis. Make
  "is there already a mechanism for this, and how does it compose?" a design
  step before adding new knobs.

## Medium priority

- [ ] **Config migration / versioning.** `#[serde(default)]` fills a *missing*
  field with the type default, not the intended value, so new fields don't
  reach existing saved profiles. Add a `config_version` + stepwise migration
  fns. (This is why a default-value change couldn't reach existing users.)
- [ ] **Tame reactive-store granularity.** A write to any field wakes every
  effect bound to that store, so menus carry a manual `widget_kinds` guard to
  avoid destructive rebuilds (which re-run dns/ufw/podman probes). Extract the
  guard pattern into one helper, or move to finer-grained signals.
- [ ] **Close shell-side test gaps on testable logic.** GTK is hard to test
  (fair), but `mplugin-sdk`, `mshell-plugin-host` (capability/path-sandbox —
  a security boundary!), `mshell-core` (D-Bus/IPC), `mshell-services` are pure
  logic and currently have zero tests.
- [ ] **Reduce `unwrap`/`expect`/`panic` on external input.** ~563 total. In a
  compositor a panic kills the whole desktop; in mshell it kills the bar.
  Sweep hot paths (render, input, IPC handlers, config/file I/O) toward
  `Result` + graceful degrade.

## Low priority / quick wins

- [ ] **Build/deploy ergonomics.** No single "build+install+restart" target;
  recurring confusion between compositor vs shell rebuilds and "`mctl reload`
  doesn't pick up new binary code (needs relogin)". Add an `xtask`/`just`
  flow + document the split.
- [ ] **CI gate** (if absent): `cargo clippy` + `fmt --check` + `test` on push.
- [ ] **TODO/FIXME triage** (20): link to issues or remove; "temporary" markers
  outlive everything.

## One-line summary

Highest ROI: move the menu/settings definitions to a **data-driven registry** —
it kills the class of bug hit this cycle and turns "edit 30 files" into "add one
record."
