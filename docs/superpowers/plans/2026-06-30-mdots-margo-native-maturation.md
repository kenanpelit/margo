# mdots — margo-native maturation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the freshly-vendored `mdots` crate from a "verbatim dcli port" into a first-class margo citizen: finish the half-built TUI, add operator-facing health/diff tooling, adopt margo's shared logging, add a regression safety net, document it, and pay down the panic/dead-code debt the vendoring introduced.

**Background:** `mdots/` (top-level crate, peer of `mctl`/`mplay`/`mvpn`) is the user's `dcli` declarative Arch package + dotfiles manager (~37k LOC), vendored 2026-06-30 (commits up to `517a8d58`). It builds, clippy is `-D warnings` clean, 85 tests pass, CI is green. But it carries debt: its own `env_logger` (not `margo-logging`), 59 `#[allow(dead_code)]` + a handful of clippy allows, a panic-baseline raised 329→380 for ~51 added `unwrap/expect`, three placeholder TUI screens ("Coming soon…"), no operator health tooling, and no user docs.

**Tech Stack:** Rust (edition 2021 for this crate), clap, mlua (lua54 vendored), ratatui + crossterm, serde_yaml, anyhow, `margo-logging` (workspace crate).

## Global Constraints

- **No `dcli` anywhere.** `grep -rni dcli mdots/src install.sh PKGBUILD justfile` MUST stay `0`. No compat symlink, no migration, no legacy read-tolerances. (Standing hard rule.)
- **CI gate per task:** every task ends with `just check` green = `cargo fmt --check` + `cargo clippy --all-targets -D warnings` + `scripts/panic-ratchet.sh` + design-lint + `cargo test`. A plain `cargo check` is NOT sufficient.
- **No new non-test panics.** New/changed non-test code MUST NOT add `.unwrap()` / `.expect(` / `panic!(` — use `Result` + `?` + `anyhow::Context`. The panic baseline is at its ceiling (380); a single new panic breaks `just check`. (Test code is exempt — it lives after `#[cfg(test)]`.)
- **panic-baseline is down-only.** `scripts/panic-baseline.txt` may only be lowered, and only to the exact measured count, never raised.
- **English in the code.** mdots CLI strings, comments, commit messages, changelog stay English (margo convention). Turkish is for the human-facing chat only.
- **Conventional commits**, one per task, footer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- **No behaviour regressions** in existing subcommands. New subcommands are additive.
- **Reuse existing patterns:** TUI screens follow `mdots/src/tui/screens/overview.rs`; any mshell work follows `mshell-crates/mshell-frame/DESIGN.md` + `docs/config-conventions.md`.
- **Branch:** work on `main` is authorized (standing permission); commit + push freely.

---

### Task 1: Finish the three placeholder TUI dashboard screens

The `mdots tui` dashboard (`mdots/src/tui/`) has four sidebar screens. `Overview` is fully implemented (`screens/overview.rs`); `Modules`, `Packages`, and `Sync` are stubs that render a `"Coming soon…"` paragraph (`screens/{modules,packages,sync}.rs`). Implement all three for real, following the `OverviewScreenState` pattern (lazy `load_data` on first render guarded by a `loaded` flag, `render` with ratatui widgets, `handle_key` for in-screen interaction + `Esc → ScreenAction::Back`).

**Files:**
- Modify: `mdots/src/tui/screens/modules.rs`, `mdots/src/tui/screens/packages.rs`, `mdots/src/tui/screens/sync.rs`
- Read for patterns: `mdots/src/tui/screens/overview.rs`, `mdots/src/tui/app.rs`, `mdots/src/tui/screens/mod.rs`
- Read for data sources: `mdots/src/commands/module.rs` (module listing), `mdots/src/backend/` (installed packages), `mdots/src/config/mod.rs` (declared packages/modules)

**Interfaces / decisions:**
- `ScreenTrait` = `handle_key(&mut self, KeyEvent) -> Result<Option<ScreenAction>>`, `render(&mut self, &ConfigPaths, &Config, &mut Frame, Rect) -> Result<()>`, `on_activate`. Use the existing `ScreenAction::{Back, Refresh}`.
- **Modules screen:** a selectable `List`/`Table` of modules (name, enabled?, package count). Reuse whatever listing logic `commands::module` already exposes — do not duplicate discovery. `j/k`/arrows move selection, `r` refreshes. A detail pane (right or bottom) showing the selected module's packages/description is a plus, not required.
- **Packages screen:** show declared packages (`config.packages` + module packages) with an installed/missing indicator, and a `/`-to-filter text field over the list. Reuse `backend::create_backend(&config).get_installed_packages()` for the installed set (already used in `overview.rs:149`). Filtering is in-memory over the loaded list.
- **Sync screen:** a read-only **plan preview** — what a real `mdots sync` would add/remove/leave (packages to install, to prune, modules enabled). This MUST NOT mutate the system; it is a dry-run summary view, computed from the same data the existing dry-run path uses. Do NOT trigger an actual sync from the TUI in this task.
- Wire `App::show_message` / `MessageLevel` for transient errors instead of `eprintln!` where a screen needs to report a load failure (these are currently `#[allow(dead_code)]` scaffolding — using them is intended).
- Keep all data loading lazy + cached (the `loaded` flag) so opening the TUI stays instant; expensive backend queries only run on screen activation/refresh.

**Acceptance:** `mdots tui` shows real content on all four screens; no "Coming soon" string remains; `grep -rn "Coming soon" mdots/src` is empty. No new non-test panics. `just check` green.

- [ ] Implement modules screen
- [ ] Implement packages screen
- [ ] Implement sync (plan-preview) screen
- [ ] Remove now-dead placeholder code; `just check` green

---

### Task 2: `mdots doctor` — environment health check

Add a new top-level subcommand `mdots doctor` that runs a series of read-only checks and prints a pass/warn/fail report, exiting non-zero if any hard check fails. This is the operator's "is my setup sane?" command.

**Files:**
- Create: `mdots/src/commands/doctor.rs`
- Modify: `mdots/src/commands/mod.rs` (add `pub mod doctor;`), `mdots/src/main.rs` (add `Doctor` to the `Commands` enum + dispatch arm)

**Interfaces / decisions:**
- Checks (each → `Ok`/`Warn`/`Fail` with a one-line message):
  - config dir exists + `config.yaml` resolves + `Config` parses (reuse `config::load_config`).
  - package backend resolves (`backend::create_backend`) and the AUR helper / pacman binary is on `PATH`.
  - `flatpak` present iff any flatpak packages are declared (Warn if declared but missing).
  - secrets: for each declared secret, is `sops` installed, the age key resolvable, the encrypted source present? (Reuse `secrets::` status logic — do NOT duplicate it; expose a helper if needed.)
  - Lua: the config's Lua manifests evaluate without error (reuse the existing validate/eval path).
  - nix: if `nix.home_manager_enabled`, is `nix` / `home-manager` on `PATH`? (Warn otherwise.)
- Output: aligned `✓ / ! / ✗` lines grouped by area, a summary count, exit code `1` if any `Fail`.
- Read-only. No writes, no network beyond what a `which`/version probe needs. No new non-test panics.

**Acceptance:** `mdots doctor` runs against the real config and prints a grouped report; exits 0 when healthy. Unit test the pure check-aggregation/exit-code logic with synthetic check results. `just check` green.

- [ ] Implement check framework + checks
- [ ] Wire subcommand + dispatch
- [ ] Unit-test aggregation/exit logic

---

### Task 3: `mdots diff` + drift summary in `mdots status`

Give the operator a clear "what does declared-vs-installed look like?" view. Add `mdots diff` (detailed) and a one-line drift summary to the existing `mdots status` output.

**Files:**
- Create: `mdots/src/commands/diff.rs`
- Modify: `mdots/src/commands/mod.rs`, `mdots/src/main.rs` (add `Diff` subcommand + dispatch), and the existing `status` command source (find via `grep -rn "fn .*status" mdots/src/commands`).

**Interfaces / decisions:**
- `mdots diff` computes, read-only: packages **declared but not installed** (would-install), **installed but not declared** and prunable (would-remove iff `auto_prune`/explicit), and modules enabled-vs-available. Reuse the existing sync-planning/backend logic — factor a shared `fn compute_drift(&Config, &dyn Backend) -> Drift` helper rather than duplicating the sync planner. If the sync planner is not cleanly reusable, extract the minimal package-set diff it already performs.
- Output: colorized `+ pkg` (to add) / `- pkg` (to remove) sections with counts; empty sections collapse to "in sync".
- `mdots status`: append one line, e.g. `drift: 3 to install, 1 to remove (run 'mdots diff')`, or `drift: in sync`. Keep it cheap — if computing drift requires an expensive backend query, gate the line behind the query already performed by status, or compute it (status is not a hot path).
- Read-only; no new non-test panics.

**Acceptance:** `mdots diff` prints add/remove sets against the real config; `mdots status` shows the drift line. Unit-test `compute_drift` with synthetic declared/installed sets. `just check` green.

- [ ] Extract/implement `compute_drift`
- [ ] `mdots diff` subcommand + output
- [ ] `status` drift line
- [ ] Unit-test `compute_drift`

---

### Task 4: Adopt `margo-logging` (retire `env_logger`)

Replace mdots's standalone `env_logger` init with the workspace `margo-logging` engine so mdots logs land in margo's shared, rotated, level-reloadable file sink alongside margo + mshell, while preserving the current stderr behaviour for an interactive CLI.

**Files:**
- Modify: `mdots/Cargo.toml` (drop `env_logger`; add `margo-logging` path dep — check how `mshell` depends on it), `mdots/src/main.rs:511` (the `env_logger::Builder…init()` call), any module that configures logging.
- Read for the integration contract: `margo-logging/src/lib.rs` (public init API), and a current consumer (`mshell/src/main.rs` or `margo/src/main.rs`) for the exact call pattern + env knobs.

**Interfaces / decisions:**
- Use `margo-logging`'s public init exactly as an existing consumer does (file sink + per-start rotation keep-N + reloadable level). Match its log directory convention (`~/.local/state/margo/logs` per the project docs) and its level env/flag.
- mdots is a foreground CLI, not a daemon: keep human-readable progress on stdout/stderr (the `indicatif`/`println!` UX is unchanged — this task is about the `log`/`tracing` diagnostic stream only, not the user-facing CLI output).
- If `margo-logging` is `tracing`-based and mdots uses the `log` facade, bridge with whatever the other consumers use (e.g. a `tracing-log` layer) rather than rewriting every `log::` call — but if call sites are few, migrating them is acceptable. Verify the chosen approach compiles and that `log::info!`/`debug!` still reach the sink.
- No new non-test panics; init failure degrades gracefully (log to stderr), never aborts the CLI.

**Acceptance:** `env_logger` is gone from `mdots/Cargo.toml` and `Cargo.lock`’s mdots deps; running an mdots command writes diagnostics into the margo log dir; `RUST_LOG`/the margo level knob still controls verbosity. `just check` green.

- [ ] Add `margo-logging`, drop `env_logger`
- [ ] Replace init; verify logs reach the sink
- [ ] Confirm level control + graceful degradation

---

### Task 5: Regression safety net — integration tests

Add CI-safe tests covering the paths that were never exercised end-to-end: a `sync --dry-run` smoke test, a SOPS/age secrets round-trip, and a Lua-API surface golden test. Tests must be hermetic (use a temp `MDOTS_CONFIG_DIR`) and must skip — not fail — when an external binary (`sops`, `age-keygen`, `pacman`) is absent, so CI stays green on any host.

**Files:**
- Create: `mdots/tests/dry_run_smoke.rs`, `mdots/tests/secrets_roundtrip.rs`, `mdots/tests/lua_api.rs` (or co-locate as `#[cfg(test)]` modules if the code is not reachable as a lib — mdots is a binary crate; check whether it exposes a `lib.rs`. If it is bin-only, drive via `assert_cmd`/`Command` against the built binary, or add a thin `lib.rs` re-export. Prefer the smallest change that makes the logic testable.)

**Interfaces / decisions:**
- **Dry-run smoke:** build a minimal config tree in a tempdir (one host yaml, one trivial module), point `MDOTS_CONFIG_DIR` at it, run `sync --dry-run`, assert exit 0 and that no system mutation is attempted (no real pacman call — the dry-run path must not shell out destructively; if it would, this test documents that boundary).
- **Secrets round-trip:** `#[test]` gated on `which::which("age-keygen").is_ok() && sops_available()`; generate a key, write a `.sops.yaml`, encrypt a known plaintext, declare it in a temp config, run `secrets sync`, assert the decrypted target matches the plaintext and is mode `0600`. Skip with an eprintln when binaries are missing.
- **Lua API golden:** evaluate a manifest that touches each registered global table (`mdots.hardware`, `mdots.security`, …) and assert it loads without "nil value" errors — locks in the `dcli→mdots` global rename so it can never silently regress.
- These are tests (after `#[cfg(test)]` / in `tests/`), so they are panic-ratchet-exempt; `unwrap`/`expect` in test bodies is fine.

**Acceptance:** `cargo test -p mdots` runs the new tests; they pass on this host and self-skip where `sops`/`age` are unavailable. `just check` green.

- [ ] Dry-run smoke test (+ testability shim if needed)
- [ ] Secrets round-trip test (binary-gated)
- [ ] Lua API golden test

---

### Task 6: User documentation — `docs/mdots.md`

mdots has a generated man page + completions but no prose user doc. Write one covering the config model and the workflows that are easy to get wrong.

**Files:**
- Create: `docs/mdots.md`
- Modify: `docs/` index/README if one links the doc set (check `grep -rn "config-conventions" docs/*.md README.md`); link the new doc where the others are listed.

**Interfaces / decisions:**
- Sections: what mdots is + config home (`~/.config/mdots`, `MDOTS_CONFIG_DIR`); the YAML + Lua config model (host file, modules, packages, the injected `mdots.*` Lua API globals); the **secrets workflow** (age keygen → `.sops.yaml` recipient → declare `secrets:` entry → `secrets edit`/`status`/`sync`, with the "target must be outside the repo" guard); the **nix/home-manager** integration knobs; the operator commands (`status`, `doctor`, `diff`, `validate`, `sync [--dry-run]`, `tui`). Reflect the real subcommand surface — verify against `mdots --help`.
- No `dcli` references. Doc-only; no code. (Touches no Rust, so panic-ratchet is unaffected, but still run `just check` for the design-lint/fmt gate.)

**Acceptance:** `docs/mdots.md` exists, accurately describes the current CLI (cross-checked against `mdots --help`), contains zero `dcli`. `just check` green.

- [ ] Write `docs/mdots.md`
- [ ] Link from the docs index

---

### Task 7: Pay down debt — dead code, clippy allows, panic-ratchet down

The final pass, after features have consumed scaffolding. Remove genuinely-unused code, resolve the non-`deprecated` clippy allows properly, convert reasonable non-test `unwrap/expect` to `Result`/`?`, then lower `scripts/panic-baseline.txt` to the exact new measured count.

**Files:**
- Modify: across `mdots/src/**` (targeted), `scripts/panic-baseline.txt`.

**Interfaces / decisions:**
- **Dead code:** for each `#[allow(dead_code)]` remaining after Tasks 1–6, either delete the unused item or wire it. Do NOT delete anything still referenced (recheck after the feature tasks landed). Keep the 16 `#[allow(deprecated)]` that guard intentional config back-compat (`backup_tool`, `snapper_config`, etc.) — those are deliberate; leave them and their reading code untouched.
- **Clippy allows:** resolve `clippy::wrong_self_convention` (3), `clippy::needless_range_loop` (2), `clippy::too_many_arguments` (1) by fixing the code (rename to `to_*`/`as_*`/`is_*`, use iterators, group args into a struct) rather than keeping the allow — unless a fix would harm clarity, in which case keep the allow with a one-line justification comment.
- **Panics:** convert non-test `unwrap()/expect()` that have a sensible error path to `?` + `anyhow::Context`. Do not force-convert ones where failure is a genuine invariant (document those). The goal is a real reduction, not churn.
- **Baseline:** after the above, run `scripts/panic-ratchet.sh` to get the measured count and set `scripts/panic-baseline.txt` to exactly that number (must be `< 380`). The ratchet must pass.

**Acceptance:** fewer `#[allow(dead_code)]`; the three named clippy allows resolved or justified; `panic-baseline.txt` lowered to the exact measured count `< 380`; `grep -rni dcli mdots/src` still `0`. `just check` green.

- [ ] Dead-code sweep (delete or wire)
- [ ] Resolve clippy allows
- [ ] Convert reasonable panics
- [ ] Lower panic-baseline to measured count

---

## Out of scope (separate follow-up)

- **mshell desktop integration** (a Settings → mdots status page / drift indicator). This is a genuine multi-file shell feature gated by `mshell-frame/DESIGN.md` + `docs/config-conventions.md` and deserves its own spec/plan rather than being appended here. Flagged to the user as the remaining "nice-to-have" after this plan lands.
