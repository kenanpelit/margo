# Config generations + rollback — design

**Date:** 2026-09-01
**Status:** Approved (design); implementation plan to follow.
**Sub-project:** 1 of 3 (config rollback → screen-share redaction →
universal PiP — three independently brainstormed/planned features).

## Goal

Give `~/.config/margo/config.conf` a lightweight, automatic history so a
bad edit is a one-command undo (`mctl config rollback`), and close the one
real "silently bricked" path margo still has: a config that fails to
*parse at all* falls back to hardcoded `Config::default()` at boot,
wiping every keybind/rule/layout with no on-screen warning.

## Current state (what exists)

Two separate config-loading paths, with different safety levels:

- **Live reload** (`mctl reload` → `margo/src/state.rs::reload_config`,
  line 1890) — runs `margo_config::validator::validate_config` *first*.
  On any error it bails **before** touching `self.config`, sets
  `config_error_overlay_until` (a 10s on-screen banner), and the
  previously-running config (and its keybinds) stays live. This path is
  already safe — no rollback is needed here.
- **Cold boot** (`margo/src/main.rs`, line 457) — calls
  `margo_config::parse_config_with_defaults` directly, **no validator**.
  If it returns `Err` (the permissive parser couldn't even parse-with-defaults
  through the file — unreadable file, hard syntax break), margo falls back
  to `Config::default()` **entirely**: every custom keybind, window rule,
  and layout choice is gone for the session. Only a `tracing::error!` line
  records it — no banner, since `config_error_overlay_until` is only ever
  set from the reload path. **This is the real bricking scenario the
  Hyprland comparison was pointing at**, just at boot rather than at
  reload.

`config.conf` is frequently a symlink into a dotfiles repo
(`docs/config-conventions.md` §2); `resolve_config_path` (no explicit
`--config`) always resolves to the literal `~/.config/margo/config.conf`
path, and plain `fs::write`/`fs::read_to_string` on that path transparently
follow the symlink to whatever it points at — no special-casing needed to
read/write "through" it, only to *detect* it for the user-facing warning.

## Decisions (locked)

| Question | Decision |
|---|---|
| Scope | Both: manual `mctl config {list,diff,rollback}` **and** fixing the boot-time `Config::default()` fallback to prefer the last-good generation. |
| What gets snapshotted | `config.conf`'s resolved content only (the literal file at `resolve_config_path`, symlink target included). **Not** the `source`d fragments (`conf.d/colors.conf`, `conf.d/taglayouts.conf`, `conf.d/mlayout.conf`, `binds.d/*.conf`) — those are machine-written by their own tools (matugen / mshell / plugin manager / mlayout) with independent lifecycles per `config-conventions.md` §1; rolling them back independently of their owner would fight the owner's next write. |
| Trigger points | (1) Cold boot, right after `parse_config_with_defaults` returns `Ok` in `main.rs`. (2) `reload_config`, right after validate+parse both succeed, just before `self.config = new_config`. Identical-to-last-saved content is skipped (no bloat from repeated no-op reloads). |
| Storage | `$XDG_STATE_HOME/margo/config-generations/` (mirrors `margo_logging::logs_dir()`'s `margo/logs` sibling pattern), one file per generation, name = sortable UTC timestamp (`20260901T142233Z.conf`). |
| Retention | Fixed default **20**, prune-oldest. No new config knob for v1 (YAGNI — add `config_generations_keep` later only if asked). |
| Symlink handling on rollback | Write through the symlink (plain `fs::write`, normal Unix behaviour — mutates the dotfiles-tracked file). Before writing, detect via `fs::symlink_metadata` and print "`config.conf` bir symlink, `<target>` dosyasına yazılacak" + require confirmation, skippable with `--yes`. |
| Rollback safety | The candidate generation's content is parsed in-memory (`margo_config::parse_config_str`, see the parsing-in-memory row below) before it's written to disk, so a rollback can't write a config that itself fails to parse. |
| Boot fallback | On `parse_config_with_defaults` returning `Err`: try `generations::latest()`; if that content parses, use it (log which generation, and arm the same on-screen banner reload uses) instead of silently falling to `Config::default()`. Only fall through to `Config::default()` if no generation exists or it also fails to parse. |
| Parsing in-memory content | Add a public `margo_config::parse_config_str(content: &str) -> Result<Config>` (and `_with_defaults` variant) that `parse_config`/`parse_config_with_defaults` delegate to after reading the file. `parser.rs` already has a private `#[cfg(test)]` `parse_conf(unique, content: &str)` helper and `validator.rs` a private `#[cfg(test)]` `validate_str(text: &str)` — both prove the internals already separate "read the file" from "parse/validate content", so promoting a string entry point to `pub` is a small, low-risk refactor, not new logic. Boot fallback and rollback's pre-write check both call this instead of round-tripping through a temp file. |

## Architecture

### Component 1 — `margo-config::generations` (new module)

Lives beside `validator`/`diagnostics` as a third `pub mod` in
`margo-config/src/lib.rs`. Pure filesystem + string operations, no I/O
surprises beyond the generations directory itself — so both `margo`
(server-side, on boot/reload) and `mctl` (client-side CLI) link the same
crate and call the same functions; **no new IPC/socket protocol verbs are
needed** — `mctl config list`/`diff` read the generations directory
directly, and `mctl config rollback` writes `config.conf` directly and
then reuses the *existing* `reload` dispatch verb to apply it live.

```rust
pub struct Generation {
    pub id: String,        // the filename stem, e.g. "20260901T142233Z"
    pub timestamp: SystemTime,
    pub path: PathBuf,
}

/// $XDG_STATE_HOME/margo/config-generations (default ~/.local/state/margo/config-generations)
pub fn generations_dir() -> PathBuf;

/// Save `content` as a new generation unless it's byte-identical to the
/// most recent one. Prunes down to `keep` (20) afterward. Best-effort:
/// logs a warning and returns Ok(None) on I/O failure rather than
/// bubbling an error into the boot/reload path.
pub fn save(content: &str, keep: usize) -> std::io::Result<Option<Generation>>;

/// Newest-first.
pub fn list() -> std::io::Result<Vec<Generation>>;

/// Most recent generation's content, if any exist and it's readable.
pub fn latest() -> Option<(Generation, String)>;

/// Read one generation's content by id (as listed by `list()`).
pub fn read(id: &str) -> std::io::Result<String>;
```

### Component 2 — margo wiring (server-side triggers)

- **`margo/src/main.rs`** (boot): after
  `parse_config_with_defaults(args.config.as_deref())`:
  - `Ok(cfg)` → `margo_config::generations::save(&raw_content, 20)` (needs
    the raw file bytes, not the parsed `Config` — read once via
    `std::fs::read_to_string` on the resolved path before/alongside
    parsing; the parser doesn't currently expose the raw string it read).
  - `Err(_)` → **before** falling to `Config::default()`, try
    `generations::latest()`; if `margo_config::parse_config_str_with_defaults`
    can parse that saved content, use it and log which
    generation string was restored + why. Only then fall to
    `Config::default()`.
- **`margo/src/state.rs::reload_config`**: after `parse_config_with_defaults`
  succeeds (line ~1920) and before `self.config = new_config` (line 1992),
  call `generations::save(&raw_content, 20)` the same way.
- **On-screen banner reuse**: boot-time fallback arms
  `config_error_overlay_until` exactly like `reload_config` already does,
  so the first rendered frame carries "config bozuktu, generation X'e
  döndüm — `mctl check-config` çalıştır" instead of a silent log line.

### Component 3 — `mctl config` CLI (client-side, no IPC changes)

Nested subcommand, matching the existing `Twilight { #[command(subcommand)]
action }` pattern in `mctl/src/bin/mctl.rs` (not a new flat verb — `config`
groups three actions the way `twilight` groups its five):

```
mctl config list                 # index / timestamp / short diff-stat vs current file
mctl config diff [N]             # unified diff: generation N (default: most recent) vs live file
mctl config rollback [N]         # default N=1 (previous generation)
  --yes                          # skip the symlink/confirmation prompt
```

`rollback`:
1. Resolve `N` → generation via `generations::list()`.
2. Parse its content in-memory; abort with an error if it doesn't parse
   (should be unreachable given save-only-on-success, but cheap insurance).
3. `fs::symlink_metadata` the live path; if it's a symlink, print the
   target + prompt for confirmation (unless `--yes`).
4. `fs::write` the generation's content to the live path.
5. Send the existing `reload` dispatch verb over the IPC socket — same
   code path `mctl reload` already uses — so the change is live
   immediately, matching "anında öncekine dön".

### Data flow

```
boot:    main.rs ─Ok(cfg)─> generations::save(raw)
         main.rs ─Err─────> generations::latest() ─parses?─> use it (+ arm banner)
                                                  └─no/none─> Config::default() (unchanged fallback)

reload:  reload_config ─validate+parse OK─> generations::save(raw) ─> self.config = new_config

CLI:     mctl config list/diff  ──reads config-generations/ directly (no socket)
         mctl config rollback   ──fs::write(config.conf, generation) ──"reload" dispatch──> margo re-parses
```

## Error handling

- `generations::save` never fails the caller — I/O errors are logged and
  swallowed (this runs on the boot- and reload-critical path; a full disk
  must not prevent config from applying).
- `generations::latest()`/`read()` return `None`/`Err` cleanly when the
  directory is empty or missing (first run, or after a fresh install) —
  boot falls through to `Config::default()` exactly as it does today.
- `mctl config rollback` on a generation that fails the in-memory parse
  check aborts with a clear error and writes nothing.
- Prune failures (can't delete an old generation file) log a warning and
  continue — never blocks a save.

## Testing

- `margo-config::generations` unit tests (tempdir-based, following the
  `margo-logging` rotation-test pattern): save/list ordering, prune keeps
  exactly `keep`, identical-content save is a no-op, `latest()` on an
  empty dir returns `None`.
- `main.rs` boot-fallback logic extracted into a small pure/testable
  helper (e.g. `fn resolve_boot_config(parse_result, generations_dir) ->
  Config`) so the "Err + a parseable generation exists → use it" and "Err
  + no generation → `Config::default()`" branches get a regression test
  without booting a real compositor.
- `mctl config` CLI: clap parsing tests for `list`/`diff`/`rollback`
  (including the `N` default and `--yes`), following the existing
  `mctl`/`mshellctl` test-module placement convention (end of file, per
  `clippy::items_after_test_module`).

## Out of scope (deferred)

- `mshell-config` (YAML) generations — this spec covers `margo-config`
  only, per the user's original framing (compositor keybinds/rules).
  Could follow the same module shape later if wanted.
- A `config_generations_keep` knob — ship the fixed default of 20 first.
- Snapshotting the `source`d fragments (`conf.d/*`, `binds.d/*`) — owned
  by other tools, out of scope (see Decisions table).
- A TUI/interactive rollback picker — `list` + `diff` + `rollback N` covers
  the CLI workflow; a picker can follow if requested.
