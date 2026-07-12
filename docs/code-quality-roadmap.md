# Code-quality roadmap

A grounded review of margo (compositor + mshell shell) as a codebase, with
prioritised improvement work. Snapshot metrics (2026-07-12, v1.1.8): ~298k
LOC Rust, 62 workspace crates, 1173 test fns, 249 `unsafe` sites,
panic-ratchet baseline 320 (non-test `unwrap`/`expect`/`panic`). These figures
come from [`scripts/metrics.sh`](https://github.com/kenanpelit/margo/blob/main/scripts/metrics.sh),
which is the canonical live source — run it for the current numbers rather than
trusting this line, since the snapshot drifts between refreshes. (Earlier
snapshots for trend: 2026-06-12 v1.0.3 was ~180k LOC / 53 crates / 765 tests.)

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
- [x] **Unify overlapping mechanisms.** ✅ The per-tag layout ambiguity is
  resolved: `state/window_rules.rs` builds a `taglayout_tags` set and gates
  `tagrule layout_name` behind `!taglayout_tags.contains(tag)`, with the
  documented precedence `taglayout > tagrule > default` (`state.rs`) plus a
  `taglayout_force` snapshot override. Menu sizing collapsed to min/max (the
  `auto` axis was reverted). Keep the design step for future knobs: "is there
  already a mechanism for this, and how does it compose?"

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
- [~] **Close shell-side test gaps on testable logic.** *Mostly done.*
  `mshell-plugin-host` now covers both boundaries — the path sandbox
  (`sandbox.rs`, 11 tests) **and** the new capability model (deny-by-default
  process/network/clipboard gating, with a `denies_network_without_capability`
  integration test); `mshell-core` and `mshell-services` have tests too. The
  security story also hardened: plugins declare `[capabilities]` in their
  manifest and the WASM host refuses ungated `run`/`http`/`clipboard` calls (it
  was previously FS-sandbox-only — the "capability sandbox" name was aspirational).
  Still open: `mplugin-sdk` (out-of-tree) has zero tests, and the auth/lock
  boundary crates (`mlock`, `mshell-auth`, `mshell-polkit`) are thin.
- [ ] **Reduce `unwrap`/`expect`/`panic` on external input.** In a compositor a
  panic kills the whole desktop; in mshell it kills the bar. Sweep hot paths
  (render, input, IPC handlers, config/file I/O) toward `Result` + graceful
  degrade. **CI-ratcheted** via `scripts/panic-ratchet.sh` +
  `scripts/panic-baseline.txt` (baseline **370**) — the count can only go down.
  The ratchet now also counts `unreachable!`/`todo!`/`unimplemented!` and skips
  test code by brace depth (so a mid-file `#[cfg(test)]` no longer hides the
  production code after it). The screencast pod-parsing path (`pw_utils.rs`,
  `mutter_screen_cast.rs`) was swept to `stop_cast()`/warn instead of panicking
  the compositor on malformed PipeWire input.

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
