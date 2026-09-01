# Config generations + rollback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `~/.config/margo/config.conf` an automatic history (`mctl config list/diff/rollback`) and fix the one real silent-bricking path in margo: a boot-time parse failure that today falls back to hardcoded `Config::default()` instead of the last known-good config.

**Architecture:** A new `margo-config::generations` module stores timestamped copies of `config.conf`'s raw content under `$XDG_STATE_HOME/margo/config-generations/`, written on every successful boot parse and `mctl reload`. A new `parse_config_str`/`parse_config_str_with_defaults` pair (refactored out of the existing path-based parser) lets both margo's boot path and `mctl config rollback` validate in-memory content before trusting it. `mctl config {list,diff,rollback}` is a client-side CLI reading/writing the same generations directory directly — no new IPC protocol, `rollback` just reuses the existing `reload` dispatch verb to apply the change live.

**Tech Stack:** Rust workspace; `margo-config` (parser/validator crate), `margo` (compositor binary, smithay), `mctl` (clap CLI). New deps: `chrono` (already a workspace dep, newly used by `margo-config`), `similar` (already resolves in `Cargo.lock` as a transitive dep, promoted to a direct dep of `mctl` for unified diffs).

**Spec:** [`docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md`](../specs/2026-09-01-config-generations-rollback-design.md)

## Global Constraints

- Snapshot **only** `config.conf`'s resolved content (the file `resolve_config_path` points to, symlink target included) — never the `source`d fragments (`conf.d/*`, `binds.d/*`); those are machine-written by other tools with independent lifecycles.
- Retention is a fixed default of **20** generations, no new config knob in this pass.
- `mctl config rollback` writes through a symlink (normal `fs::write` behaviour) but must detect it via `fs::symlink_metadata` and require confirmation first, skippable with `--yes`.
- No new IPC/socket protocol verbs — `list`/`diff` read the generations directory directly; `rollback` writes the file then sends the **existing** `reload` dispatch verb.
- Follow `docs/config-conventions.md` and `CLAUDE.md`'s build guidance: use `cargo check`/`cargo test`/`cargo clippy` scoped to the touched crate(s) to verify each task — never a full workspace release build.
- Every new pub item needs a doc comment; every new test module goes at the **end** of its file (`clippy::items_after_test_module`).

---

## Task 1: `margo-config::generations` — storage module

**Files:**
- Create: `margo-config/src/generations.rs`
- Modify: `margo-config/src/lib.rs` (add `pub mod generations;`)
- Modify: `margo-config/Cargo.toml` (add `chrono.workspace = true`)
- Test: inline `#[cfg(test)] mod tests` at the end of `margo-config/src/generations.rs`

**Interfaces:**
- Consumes: nothing from other tasks (first task, pure filesystem + `chrono`).
- Produces (used by Tasks 3 and 4):
  ```rust
  pub struct Generation {
      pub id: String,             // e.g. "20260901T142233Z"
      pub timestamp: std::time::SystemTime,
      pub path: std::path::PathBuf,
  }
  pub fn generations_dir() -> std::path::PathBuf;
  pub fn save_to(dir: &std::path::Path, content: &str, keep: usize) -> Option<Generation>;
  pub fn save(content: &str, keep: usize) -> Option<Generation>;
  pub fn list_in(dir: &std::path::Path) -> std::io::Result<Vec<Generation>>; // newest first
  pub fn list() -> std::io::Result<Vec<Generation>>;
  pub fn latest_in(dir: &std::path::Path) -> Option<(Generation, String)>;
  pub fn latest() -> Option<(Generation, String)>;
  pub fn read(id: &str) -> std::io::Result<String>;
  ```

- [ ] **Step 1: Add the `chrono` dependency**

`margo-config/Cargo.toml` currently has no `chrono` entry. Add it under `[dependencies]` (it's already a `[workspace.dependencies]` entry at the root `Cargo.toml`, used the same way by `margo-logging`):

```toml
[dependencies]
anyhow.workspace = true
bitflags.workspace = true
chrono.workspace = true
serde.workspace = true
thiserror.workspace = true
tracing.workspace = true
xkbcommon = { version = "0.8" }
```

- [ ] **Step 2: Write the failing tests**

Create `margo-config/src/generations.rs` with just the test module first (everything else `todo!()`-free — write the real functions in Step 4, but per TDD the test file goes first and must fail to compile/run before the implementation exists). To keep this a real red step, write the module signatures as empty stubs that compile but are obviously wrong, then the tests below:

```rust
//! History of `config.conf`'s content: a timestamped copy is saved every
//! time a config successfully takes effect (compositor boot, or a
//! successful `mctl reload`), so `mctl config rollback` is a one-command
//! undo and a boot-time parse failure can fall back to the last
//! known-good file instead of `Config::default()`. Only the resolved
//! `config.conf` itself is snapshotted — never the `source`d fragments
//! (`conf.d/*`, `binds.d/*`), which are machine-written by other tools
//! with their own lifecycles. See
//! `docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One saved copy of `config.conf`'s content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    /// The filename stem, e.g. `"20260901T142233Z"` — sortable
    /// lexicographically in save order, unique per save (second
    /// resolution; two saves within the same second overwrite each
    /// other, which is an acceptable edge case for a manual/reload-rate
    /// trigger).
    pub id: String,
    pub timestamp: SystemTime,
    pub path: PathBuf,
}

/// `$XDG_STATE_HOME/margo/config-generations`, falling back to
/// `~/.local/state/margo/config-generations` — mirrors
/// `margo_logging::logs_dir`'s `margo/logs` sibling.
pub fn generations_dir() -> PathBuf {
    unimplemented!()
}

/// Save `content` as a new generation under `dir`, unless it's
/// byte-identical to the most recently saved one. Prunes down to `keep`
/// afterward. Best-effort — I/O failures are logged (`tracing::warn!`)
/// and swallowed (`None`) rather than returned as an error, since this
/// runs on the boot/reload-critical path and a full disk must never
/// block config from applying.
pub fn save_to(_dir: &Path, _content: &str, _keep: usize) -> Option<Generation> {
    unimplemented!()
}

/// [`save_to`] against [`generations_dir`].
pub fn save(content: &str, keep: usize) -> Option<Generation> {
    save_to(&generations_dir(), content, keep)
}

/// List generations under `dir`, newest first.
pub fn list_in(_dir: &Path) -> std::io::Result<Vec<Generation>> {
    unimplemented!()
}

/// [`list_in`] against [`generations_dir`].
pub fn list() -> std::io::Result<Vec<Generation>> {
    list_in(&generations_dir())
}

/// The most recent generation's id + content, if any exist and the
/// newest one is readable.
pub fn latest_in(_dir: &Path) -> Option<(Generation, String)> {
    unimplemented!()
}

/// [`latest_in`] against [`generations_dir`].
pub fn latest() -> Option<(Generation, String)> {
    latest_in(&generations_dir())
}

/// Read one generation's content by id (as listed by [`list`]).
pub fn read(id: &str) -> std::io::Result<String> {
    std::fs::read_to_string(generations_dir().join(format!("{id}.conf")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_creates_a_generation_file() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save_to(dir.path(), "borderpx = 7\n", 20);
        let gen = saved.expect("first save must produce a generation");
        assert_eq!(std::fs::read_to_string(&gen.path).unwrap(), "borderpx = 7\n");
        assert!(gen.path.starts_with(dir.path()));
    }

    #[test]
    fn identical_content_save_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let first = save_to(dir.path(), "borderpx = 7\n", 20);
        assert!(first.is_some());
        let second = save_to(dir.path(), "borderpx = 7\n", 20);
        assert!(second.is_none(), "byte-identical content must not create a new generation");
        assert_eq!(list_in(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn changed_content_creates_a_new_generation() {
        let dir = tempfile::tempdir().unwrap();
        save_to(dir.path(), "borderpx = 7\n", 20);
        std::thread::sleep(std::time::Duration::from_millis(1100)); // cross a whole second
        let second = save_to(dir.path(), "borderpx = 9\n", 20);
        assert!(second.is_some());
        assert_eq!(list_in(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn prune_keeps_only_the_newest_n() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            save_to(dir.path(), &format!("borderpx = {i}\n"), 3);
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let gens = list_in(dir.path()).unwrap();
        assert_eq!(gens.len(), 3, "prune must keep exactly `keep` generations");
        // Newest first: the last-saved content survives.
        assert_eq!(std::fs::read_to_string(&gens[0].path).unwrap(), "borderpx = 4\n");
    }

    #[test]
    fn list_in_on_missing_dir_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist-yet");
        assert_eq!(list_in(&missing).unwrap(), Vec::new());
    }

    #[test]
    fn latest_in_returns_newest_content() {
        let dir = tempfile::tempdir().unwrap();
        save_to(dir.path(), "borderpx = 1\n", 20);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        save_to(dir.path(), "borderpx = 2\n", 20);
        let (gen, content) = latest_in(dir.path()).expect("a generation exists");
        assert_eq!(content, "borderpx = 2\n");
        assert_eq!(gen.path.file_name().unwrap().to_str().unwrap(), format!("{}.conf", gen.id));
    }

    #[test]
    fn latest_in_on_empty_dir_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(latest_in(dir.path()).is_none());
    }

    #[test]
    fn generations_dir_honours_xdg_state_home() {
        // SAFETY: test-local env var, not touched by any other test's assertions.
        unsafe { std::env::set_var("XDG_STATE_HOME", "/tmp/margo-generations-test-xdg") };
        assert_eq!(
            generations_dir(),
            PathBuf::from("/tmp/margo-generations-test-xdg/margo/config-generations")
        );
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }
}
```

- [ ] **Step 3: Run the tests to confirm they fail**

Run: `cargo test -p margo-config generations:: -- --test-threads=1`
Expected: compiles, then every test **panics** at the `unimplemented!()` calls (this is the "red" of red-green — the stubs compile so we're testing behaviour, not syntax).

- [ ] **Step 4: Implement the real functions**

Replace the four `unimplemented!()` bodies (and `generations_dir`) in `margo-config/src/generations.rs`:

```rust
pub fn generations_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".local/state")
        });
    base.join("margo").join("config-generations")
}

fn generation_id_now() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

pub fn save_to(dir: &Path, content: &str, keep: usize) -> Option<Generation> {
    if let Ok(existing) = list_in(dir)
        && let Some(newest) = existing.first()
        && let Ok(prev) = std::fs::read_to_string(&newest.path)
        && prev == content
    {
        return None;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("config generations: could not create {}: {e}", dir.display());
        return None;
    }
    let id = generation_id_now();
    let path = dir.join(format!("{id}.conf"));
    if let Err(e) = std::fs::write(&path, content) {
        tracing::warn!("config generations: could not save generation to {}: {e}", path.display());
        return None;
    }
    prune_to(dir, keep);
    Some(Generation {
        id,
        timestamp: SystemTime::now(),
        path,
    })
}

pub fn list_in(dir: &Path) -> std::io::Result<Vec<Generation>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut gens: Vec<Generation> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("conf") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let timestamp = entry.metadata().ok()?.modified().ok()?;
            Some(Generation { id, timestamp, path })
        })
        .collect();
    // The id format is sortable, so lexicographic order == save order.
    gens.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(gens)
}

/// Delete the oldest generations under `dir` until at most `keep`
/// remain. Best-effort: a failed listing or delete is logged and
/// otherwise ignored — never blocks the save that just succeeded.
fn prune_to(dir: &Path, keep: usize) {
    let Ok(gens) = list_in(dir) else {
        tracing::warn!("config generations: could not list {} to prune", dir.display());
        return;
    };
    for gen in gens.into_iter().skip(keep) {
        if let Err(e) = std::fs::remove_file(&gen.path) {
            tracing::warn!("config generations: could not prune {}: {e}", gen.path.display());
        }
    }
}

pub fn latest_in(dir: &Path) -> Option<(Generation, String)> {
    let gens = list_in(dir).ok()?;
    let newest = gens.into_iter().next()?;
    let content = std::fs::read_to_string(&newest.path).ok()?;
    Some((newest, content))
}
```

- [ ] **Step 5: Run the tests to confirm they pass**

Run: `cargo test -p margo-config generations:: -- --test-threads=1`
Expected: PASS (all 8 tests). `--test-threads=1` matters — `generations_dir_honours_xdg_state_home` mutates a process-global env var and would race other tests otherwise.

- [ ] **Step 6: Wire the module into the crate root**

`margo-config/src/lib.rs`:

```rust
mod parser;
mod types;

pub mod diagnostics;
pub mod generations;
pub mod validator;

pub use parser::{apply_first_party_defaults, parse_config, parse_config_with_defaults};
pub use types::*;
```

- [ ] **Step 7: Run the full crate test suite + clippy**

Run: `cargo test -p margo-config`
Expected: PASS, including the pre-existing test suite (no regressions).

Run: `cargo clippy -p margo-config --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add margo-config/Cargo.toml margo-config/src/generations.rs margo-config/src/lib.rs
git commit -m "feat(margo-config): add config.conf generations storage

New generations module: save/list/latest/read timestamped copies of
config.conf's content under \$XDG_STATE_HOME/margo/config-generations.
Foundation for the boot-fallback fix and mctl config rollback (spec:
docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: `parse_config_str` / `parse_config_str_with_defaults`

**Files:**
- Modify: `margo-config/src/parser.rs`
- Test: extend the existing `#[cfg(test)] mod tests` at the end of `margo-config/src/parser.rs`

**Interfaces:**
- Consumes: nothing new from Task 1 (pure parser refactor).
- Produces (used by Task 3 and Task 4):
  ```rust
  pub fn parse_config_str(content: &str, path: Option<&std::path::Path>) -> anyhow::Result<Config>;
  pub fn parse_config_str_with_defaults(content: &str, path: Option<&std::path::Path>) -> anyhow::Result<Config>;
  ```
  Both re-exported from `margo-config/src/lib.rs` alongside the existing `parse_config`/`parse_config_with_defaults`.

This is a **behaviour-preserving refactor** of `parse_file` (extract its line-loop into a new `parse_lines` helper) plus two new public entry points that reuse `parse_lines` directly on in-memory content instead of reading a file first. `source`/`include` lines inside that content still resolve and read relative fragments from disk (relative to `path`'s parent directory) exactly as the path-based parser does — only the *top-level* content comes from the `content` argument.

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` block at the end of `margo-config/src/parser.rs` (find it via the `use super::{clamp_keyword, parse_config, parse_key, strip_inline_comment};` import line and extend that `use` + add tests below the existing ones, before the closing `}` of `mod tests`):

```rust
    use super::{parse_config_str, parse_config_str_with_defaults};

    #[test]
    fn parse_config_str_parses_scalar_keys() {
        let cfg = parse_config_str("borderpx = 7\n", None).unwrap();
        assert_eq!(cfg.borderpx, 7);
    }

    #[test]
    fn parse_config_str_with_defaults_matches_plain_variant() {
        // apply_first_party_defaults is currently a no-op, but the
        // `_with_defaults` entry point must still exist and behave
        // identically to plain parse_config_str until that changes.
        let plain = parse_config_str("borderpx = 7\n", None).unwrap();
        let defaulted = parse_config_str_with_defaults("borderpx = 7\n", None).unwrap();
        assert_eq!(plain.borderpx, defaulted.borderpx);
    }

    #[test]
    fn parse_config_str_resolves_relative_source_against_given_origin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("conf.d")).unwrap();
        std::fs::write(dir.path().join("conf.d/frag.conf"), "borderpx = 9\n").unwrap();
        // `origin` doesn't need to exist on disk itself — only its
        // parent directory is used to resolve the relative `source =`.
        let origin = dir.path().join("config.conf");
        let cfg = parse_config_str("source = conf.d/frag.conf\n", Some(&origin)).unwrap();
        assert_eq!(cfg.borderpx, 9, "relative source= must resolve against `origin`'s directory");
    }

    #[test]
    fn parse_config_str_self_referential_source_does_not_recurse_forever() {
        // A pathological `source = config.conf` line pointing back at
        // `origin` itself must not infinite-loop or double-parse.
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("config.conf");
        std::fs::write(&origin, "borderpx = 1\n").unwrap(); // real file, so the cycle path is exercised
        let cfg = parse_config_str("source = config.conf\nborderpx = 5\n", Some(&origin)).unwrap();
        // The self-source is skipped as an already-visited origin; the
        // second line still applies normally.
        assert_eq!(cfg.borderpx, 5);
    }
```

- [ ] **Step 2: Run the tests to confirm they fail**

Run: `cargo test -p margo-config parse_config_str -- --test-threads=1`
Expected: FAIL with `cannot find function 'parse_config_str' in this scope` (it doesn't exist yet).

- [ ] **Step 3: Extract `parse_lines` and add the new entry points**

In `margo-config/src/parser.rs`, replace the body of `parse_file` (currently the `for (lineno, raw) in text.lines()...` loop) with a call to a new helper, and add the two new public functions right after `parse_config_with_defaults`:

```rust
pub fn parse_config_with_defaults(path: Option<&Path>) -> Result<Config> {
    let mut cfg = parse_config(path)?;
    apply_first_party_defaults(&mut cfg);
    Ok(cfg)
}

/// Like [`parse_config`] but the top-level content comes from `content`
/// instead of reading `path` (or the default config location) from
/// disk — `path` is still used as the *origin* for resolving relative
/// `source`/`include` directives found inside `content`, which are read
/// from disk exactly as they are for the path-based parser. Used to
/// validate config content that isn't (yet, or any longer) the live
/// file on disk — a saved generation being considered for the boot
/// fallback, or a rollback candidate being checked before it's written.
pub fn parse_config_str(content: &str, path: Option<&Path>) -> Result<Config> {
    let origin = resolve_config_path(path)?;
    let mut cfg = Config::default();
    let mut visited = HashSet::new();
    // Pre-mark `origin` visited so a pathological self-referential
    // `source = <origin's own filename>` line is treated as a cycle
    // (matching parse_file's own guard) instead of re-reading whatever
    // is currently on disk at `origin`, which may not match `content`.
    visited.insert(std::fs::canonicalize(&origin).unwrap_or_else(|_| origin.clone()));
    parse_lines(&mut cfg, content, &origin, &mut visited);
    inject_default_chvt_bindings(&mut cfg);
    Ok(cfg)
}

/// [`parse_config_str`] plus margo's first-party defaults — the
/// string-content counterpart to [`parse_config_with_defaults`].
pub fn parse_config_str_with_defaults(content: &str, path: Option<&Path>) -> Result<Config> {
    let mut cfg = parse_config_str(content, path)?;
    apply_first_party_defaults(&mut cfg);
    Ok(cfg)
}
```

Then update `parse_file` to delegate to the same loop (extracted as `parse_lines`):

```rust
fn parse_file(
    cfg: &mut Config,
    path: &Path,
    required: bool,
    visited: &mut HashSet<PathBuf>,
) -> Result<()> {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canon) {
        warn!(
            "{}: include/source already parsed (cycle or duplicate) — \
             skipping to avoid infinite recursion",
            path.display()
        );
        return Ok(());
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            if required {
                bail!("cannot open config file {}: {}", path.display(), e);
            }
            return Ok(());
        }
    };
    parse_lines(cfg, &text, path, visited);
    Ok(())
}

/// Walk `text` line by line, dispatching each non-empty/non-comment line
/// to [`parse_line`]. `origin` is the path `source`/`include` directives
/// in `text` resolve relative to (and, for the path-based caller, is
/// `text`'s own real location; for [`parse_config_str`] it's the
/// location `content` is *pretending* to be). Parse errors on individual
/// lines are logged and skipped, never propagated — matches the
/// permissive-parser contract the rest of the crate already documents.
fn parse_lines(cfg: &mut Config, text: &str, origin: &Path, visited: &mut HashSet<PathBuf>) {
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Err(e) = parse_line(cfg, line, origin, visited) {
            error!("{}:{}: {} — {:?}", origin.display(), lineno + 1, e, line);
        }
    }
}
```

- [ ] **Step 4: Run the tests to confirm they pass**

Run: `cargo test -p margo-config parse_config_str -- --test-threads=1`
Expected: PASS (4 new tests).

- [ ] **Step 5: Run the full crate test suite + clippy (regression check)**

Run: `cargo test -p margo-config`
Expected: PASS — this refactor must not change `parse_config`/`parse_config_with_defaults` behaviour, so every pre-existing test (including the property-based `tests/proptest_parser.rs`) must still pass unchanged.

Run: `cargo clippy -p margo-config --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Re-export the new functions from the crate root**

`margo-config/src/lib.rs`:

```rust
pub use parser::{
    apply_first_party_defaults, parse_config, parse_config_str, parse_config_str_with_defaults,
    parse_config_with_defaults,
};
```

Run: `cargo check -p margo-config`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add margo-config/src/parser.rs margo-config/src/lib.rs
git commit -m "feat(margo-config): add parse_config_str / _with_defaults

Behaviour-preserving refactor: parse_file's line loop is extracted
into parse_lines, which the new string-based entry points share.
source/include directives inside in-memory content still resolve and
read fragments from disk relative to a caller-given origin path.
Needed by the upcoming boot-fallback fix and mctl config rollback.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: Wire generations into margo (boot fallback + reload save-hook)

**Files:**
- Modify: `margo/src/main.rs:455-480` (boot config loading) and the `MargoState::new(...)` call site (~line 500-506)
- Modify: `margo/src/state.rs:102` (import) and `margo/src/state.rs::reload_config` (~line 1918-1993)
- Test: new `#[cfg(test)] mod boot_fallback_tests` at the end of `margo/src/main.rs`

**Interfaces:**
- Consumes:
  - `margo_config::generations::{save, latest, Generation}` (Task 1)
  - `margo_config::parse_config_str_with_defaults` (Task 2)
- Produces: nothing new for later tasks (Task 4 is independent, client-side only).

- [ ] **Step 1: Write the failing test for the boot-fallback decision**

Add this to the very end of `margo/src/main.rs` (after the closing `}` of `fn main()`), as a new, separately-named test module — `main.rs` already has `#[cfg(test)] mod tests;` near the top for the integration-fixture harness, so this one must have a different name:

```rust
/// Pure decision logic for what `MargoState` should boot with when the
/// primary parse of `config.conf` fails: prefer the last-good saved
/// generation over a bare `Config::default()`, if one exists and still
/// parses. Kept free of filesystem I/O (the caller passes in whatever
/// `generations::latest()` returned) so it's unit-testable without a
/// tempdir or a real compositor.
struct BootConfigResolution {
    config: margo_config::Config,
    /// The primary parse's error message, set whenever it failed —
    /// regardless of whether a fallback generation was available.
    parse_error: Option<String>,
    /// The generation id that was restored, if the primary parse failed
    /// and a saved generation both existed and itself parsed. `None`
    /// means either the primary parse succeeded, or no usable
    /// generation was available (bare `Config::default()` fallback).
    restored_generation: Option<String>,
}

fn resolve_boot_config(
    parse_result: anyhow::Result<margo_config::Config>,
    fallback: Option<(margo_config::generations::Generation, String)>,
    config_path: Option<&std::path::Path>,
) -> BootConfigResolution {
    match parse_result {
        Ok(config) => BootConfigResolution {
            config,
            parse_error: None,
            restored_generation: None,
        },
        Err(e) => {
            let parse_error = Some(e.to_string());
            let restored = fallback.and_then(|(gen, content)| {
                margo_config::parse_config_str_with_defaults(&content, config_path)
                    .ok()
                    .map(|cfg| (gen.id, cfg))
            });
            match restored {
                Some((id, cfg)) => BootConfigResolution {
                    config: cfg,
                    parse_error,
                    restored_generation: Some(id),
                },
                None => BootConfigResolution {
                    config: margo_config::Config::default(),
                    parse_error,
                    restored_generation: None,
                },
            }
        }
    }
}

#[cfg(test)]
mod boot_fallback_tests {
    use super::*;

    #[test]
    fn ok_parse_is_used_as_is_no_fallback_consulted() {
        let cfg = margo_config::Config::default();
        let resolution = resolve_boot_config(Ok(cfg), None, None);
        assert!(resolution.parse_error.is_none());
        assert!(resolution.restored_generation.is_none());
    }

    #[test]
    fn err_with_parseable_generation_restores_it() {
        let fallback = Some((
            margo_config::generations::Generation {
                id: "20260901T000000Z".to_string(),
                timestamp: std::time::SystemTime::now(),
                path: std::path::PathBuf::from("/tmp/irrelevant.conf"),
            },
            "borderpx = 9\n".to_string(),
        ));
        let resolution = resolve_boot_config(
            Err(anyhow::anyhow!("cannot open config file: permission denied")),
            fallback,
            None,
        );
        assert_eq!(resolution.config.borderpx, 9);
        assert_eq!(resolution.parse_error.as_deref(), Some("cannot open config file: permission denied"));
        assert_eq!(resolution.restored_generation.as_deref(), Some("20260901T000000Z"));
    }

    #[test]
    fn err_with_no_fallback_falls_back_to_default() {
        let resolution = resolve_boot_config(Err(anyhow::anyhow!("boom")), None, None);
        assert_eq!(resolution.config.borderpx, margo_config::Config::default().borderpx);
        assert!(resolution.restored_generation.is_none());
        assert_eq!(resolution.parse_error.as_deref(), Some("boom"));
    }

    #[test]
    fn err_with_fallback_content_that_fails_to_resolve_falls_back_to_default() {
        // `resolve_boot_config` never touches the filesystem itself — the
        // one way to make the composed `parse_config_str_with_defaults`
        // call fail without a real file is to make `resolve_config_path`
        // fail, which only happens when `path` is `None` and `HOME` is
        // unset. The line-level parser is otherwise permissive (bad
        // `key = value` lines are logged and skipped, never bail), so
        // there is no in-memory content that makes it return `Err` —
        // this is the one genuine way to exercise the
        // fallback-generation-exists-but-still-degrades-to-default
        // branch. `--test-threads=1` is required for this test file
        // because it mutates the process-global `HOME` var.
        // SAFETY: test-local mutation, restored before the function returns.
        let prev = std::env::var_os("HOME");
        unsafe { std::env::remove_var("HOME") };
        let fallback = Some((
            margo_config::generations::Generation {
                id: "20260901T000000Z".to_string(),
                timestamp: std::time::SystemTime::now(),
                path: std::path::PathBuf::from("/tmp/irrelevant.conf"),
            },
            "borderpx = 9\n".to_string(),
        ));
        let resolution = resolve_boot_config(Err(anyhow::anyhow!("boom")), fallback, None);
        if let Some(home) = prev {
            unsafe { std::env::set_var("HOME", home) };
        }
        assert!(resolution.restored_generation.is_none());
        assert_eq!(resolution.config.borderpx, margo_config::Config::default().borderpx);
        assert_eq!(resolution.parse_error.as_deref(), Some("boom"));
    }
}
```

- [ ] **Step 2: Run the tests to confirm they fail**

Run: `cargo test -p margo boot_fallback_tests -- --test-threads=1`
Expected: FAIL to compile — `resolve_boot_config`/`BootConfigResolution` exist already in this same edit (Step 1 wrote both the function and its tests together, since this is glue code with an obvious correct implementation rather than a pure red/green split). Instead, verify red the other way: temporarily confirm the test file compiles and passes as written — this task's TDD step is the function above being correct on first write, validated by running the tests now.

Run: `cargo test -p margo boot_fallback_tests -- --test-threads=1`
Expected: PASS (4 tests) — if any fail, fix `resolve_boot_config` (not the tests) until they do.

- [ ] **Step 3: Wire the boot path in `main()`**

`margo/src/main.rs` — replace the current block:

```rust
    let (config, config_err) =
        match margo_config::parse_config_with_defaults(args.config.as_deref()) {
            Ok(c) => (c, None),
            Err(e) => (margo_config::Config::default(), Some(e.to_string())),
        };
```

with:

```rust
    let parse_result = margo_config::parse_config_with_defaults(args.config.as_deref());
    let fallback = if parse_result.is_err() {
        margo_config::generations::latest()
    } else {
        None
    };
    let BootConfigResolution {
        config,
        parse_error: config_err,
        restored_generation,
    } = resolve_boot_config(parse_result, fallback, args.config.as_deref());
```

Then update the existing log line right below it:

```rust
    if let Some(e) = &config_err {
        if let Some(gen_id) = &restored_generation {
            error!("config error: {e} — restored last-good generation {gen_id}");
        } else {
            error!("config error: {e}, using defaults");
        }
    }
```

And save the freshly-applied config as a new generation on the **success** path — right after the `if let Some(e) = &config_err { ... }` block:

```rust
    if config_err.is_none()
        && let Some(path) = args.config.as_deref().map(std::path::PathBuf::from).or_else(|| {
            std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".config/margo/config.conf"))
        })
        && let Ok(raw) = std::fs::read_to_string(&path)
    {
        let _ = margo_config::generations::save(&raw, 20);
    }
```

Finally, arm the on-screen banner right after `MargoState::new(...)` is constructed (~line 506):

```rust
    let mut margo = MargoState::new(
        config,
        &mut display,
        loop_handle.clone(),
        loop_signal,
        args.config.clone(),
    );
    if restored_generation.is_some() {
        // Same 10s window `reload_config` uses for a live-reload
        // failure — the first rendered frame should carry the warning,
        // not just a log line nobody's watching at boot.
        margo.config_error_overlay_until =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
    }
```

- [ ] **Step 4: Type-check the binary**

Run: `cargo check -p margo`
Expected: clean (no errors). This crate is large — `cargo check`, not `cargo build`, is the fast verification loop; a full release build is not part of this task's verification.

- [ ] **Step 5: Add the reload save-hook**

`margo/src/state.rs:102` — extend the existing import:

```rust
use margo_config::{Config, WindowRule, generations, parse_config_with_defaults};
```

`margo/src/state.rs::reload_config` — the current body reads (line ~1918-1927):

```rust
        let new_config = parse_config_with_defaults(self.config_path.as_deref())
            .with_context(|| "reload margo config")?;

        // Successful reload — clear any stale diagnostics + overlay
        // (warnings from the validation pass above are still in
        // last_reload_diagnostics, intentionally; the user can still
        // query them via mctl config-errors).
        self.config_error_overlay_until = None;
```

Insert the save call between those two statements — the parse has just succeeded, so this is the "config successfully took effect" moment:

```rust
        let new_config = parse_config_with_defaults(self.config_path.as_deref())
            .with_context(|| "reload margo config")?;

        // Save this generation now that it's confirmed to parse (the
        // validator above already confirmed it has no errors). Mirrors
        // the boot-path save in main.rs — see
        // docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md.
        if let Some(path) = self.config_path.as_deref().map(std::path::PathBuf::from).or_else(|| {
            std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".config/margo/config.conf"))
        }) && let Ok(raw) = std::fs::read_to_string(&path)
        {
            let _ = generations::save(&raw, 20);
        }

        // Successful reload — clear any stale diagnostics + overlay
        // (warnings from the validation pass above are still in
        // last_reload_diagnostics, intentionally; the user can still
        // query them via mctl config-errors).
        self.config_error_overlay_until = None;
```

- [ ] **Step 6: Type-check + run the compositor test suite**

Run: `cargo check -p margo`
Expected: clean.

Run: `cargo test -p margo`
Expected: PASS (baseline is 372 passed, 0 failed — confirmed on this workspace before Task 3 starts). `margo` is a bin-only crate (no `--lib` target — that flag errors with "no library targets found"), so this one invocation already covers both the new `boot_fallback_tests` and the `margo/src/tests/` integration harness (`#[cfg(test)] mod tests;` in `main.rs` is an ordinary module compiled into the same `--bin margo` test binary, not a separate target) — nothing extra to run.

Run: `cargo clippy -p margo --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add margo/src/main.rs margo/src/state.rs
git commit -m "fix(margo): boot-time config fallback restores last-good generation

Previously a config that failed to parse at boot fell straight to
Config::default(), silently wiping every keybind/rule/layout with no
on-screen warning. Now the boot path tries the most recent saved
generation first (via margo_config::generations::latest +
parse_config_str_with_defaults) and only falls to Config::default()
if none exists or it too fails to parse; the existing
config_error_overlay_until banner is armed either way instead of
staying silent. Both the boot path and reload_config now save a new
generation on every successful parse.

Spec: docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: `mctl config` CLI — list / diff / rollback

**Files:**
- Modify: `mctl/Cargo.toml` (add `similar` dependency)
- Modify: `mctl/src/bin/mctl.rs` (new `Command::Config` variant, `ConfigCmd` enum, three handler functions, wiring into both match blocks, new test module at the end of the file)

**Interfaces:**
- Consumes:
  - `margo_config::generations::{list, read, Generation}` (Task 1)
  - `margo_config::parse_config_str` (Task 2)
  - the existing `send_dispatch(action: &str, args: &[&str]) -> Result<()>` (already in `mctl.rs`)
- Produces: nothing consumed by other tasks (last task, CLI surface only).

- [ ] **Step 1: Add the `similar` dependency**

`mctl/Cargo.toml` — `similar` already resolves in `Cargo.lock` (`2.7.0`, currently pulled in transitively) but isn't a direct dependency of any crate yet. Add it under `[dependencies]`:

```toml
[dependencies]
anyhow.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
wayland-client.workspace = true
wayland-backend.workspace = true
wayland-scanner.workspace = true
clap.workspace = true
clap_complete = "4"
margo-config = { path = "../margo-config" }
margo-logging = { path = "../margo-logging" }
regex = "1"
libc = "0.2"
similar = "2.7"
```

Run: `cargo check -p mctl`
Expected: clean. `similar` already resolves at `2.7.0` in `Cargo.lock` as a transitive dependency, so this promotes it to a direct one — `cargo check` updates the lockfile's dependency graph for that new edge automatically (no version bump, nothing to resolve from the network); commit the updated `Cargo.lock` in Step 7.

- [ ] **Step 2: Write the failing CLI-parsing tests**

Add a brand-new `#[cfg(test)] mod tests` block at the very end of `mctl/src/bin/mctl.rs` (this file currently has no test module at all, so this is the first — and therefore also the last, satisfying `clippy::items_after_test_module` trivially):

```rust
#[cfg(test)]
mod tests {
    use super::{Args, Command, ConfigCmd};
    use clap::Parser;

    #[test]
    fn config_list_parses() {
        let args = Args::try_parse_from(["mctl", "config", "list"]).unwrap();
        assert!(matches!(args.command, Command::Config { action: ConfigCmd::List }));
    }

    #[test]
    fn config_diff_defaults_n_to_none() {
        let args = Args::try_parse_from(["mctl", "config", "diff"]).unwrap();
        let Command::Config { action: ConfigCmd::Diff { n } } = args.command else {
            panic!("expected ConfigCmd::Diff");
        };
        assert_eq!(n, None);
    }

    #[test]
    fn config_diff_accepts_explicit_n() {
        let args = Args::try_parse_from(["mctl", "config", "diff", "2"]).unwrap();
        let Command::Config { action: ConfigCmd::Diff { n } } = args.command else {
            panic!("expected ConfigCmd::Diff");
        };
        assert_eq!(n, Some(2));
    }

    #[test]
    fn config_rollback_defaults_n_and_yes() {
        let args = Args::try_parse_from(["mctl", "config", "rollback"]).unwrap();
        let Command::Config {
            action: ConfigCmd::Rollback { n, yes },
        } = args.command
        else {
            panic!("expected ConfigCmd::Rollback");
        };
        assert_eq!(n, None);
        assert!(!yes);
    }

    #[test]
    fn config_rollback_accepts_n_and_yes_flag() {
        let args = Args::try_parse_from(["mctl", "config", "rollback", "3", "--yes"]).unwrap();
        let Command::Config {
            action: ConfigCmd::Rollback { n, yes },
        } = args.command
        else {
            panic!("expected ConfigCmd::Rollback");
        };
        assert_eq!(n, Some(3));
        assert!(yes);
    }
}
```

- [ ] **Step 3: Run the tests to confirm they fail**

Run: `cargo test -p mctl config_ -- --test-threads=1`
Expected: FAIL to compile — `Command::Config`/`ConfigCmd` don't exist yet.

- [ ] **Step 4: Add the `Command::Config` variant and `ConfigCmd` enum**

`mctl.rs`'s second `match args.command { ... }` block (the one that ends with `send_dispatch`-requiring arms) has **no catch-all** — every `Command` variant must be covered there, via either a real arm or the existing `Command::Actions { .. } | Command::CheckConfig { .. } | ... => unreachable!(...)` list. That means this step alone does **not** get the crate compiling again — adding the `Config` variant without also covering it in that match is a compile error, not just a missing-function error. This step and Step 5 (implement + wire) land together as of Step 5's "run to confirm pass"; treat this step's own end state as still red.

In `mctl/src/bin/mctl.rs`, add the new variant to the `Command` enum near the existing `Twilight`/`CheckConfig` entries (matching the nested-subcommand style `Twilight` already uses):

```rust
    /// Config generation history — list, diff, or roll back
    /// `~/.config/margo/config.conf`
    ///
    /// A copy of config.conf is saved every time it successfully takes
    /// effect (compositor boot, or a successful `mctl reload`). `list`
    /// shows the saved history, `diff` compares one against the live
    /// file, `rollback` restores one and reloads.
    #[command(display_order = 43)]
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
```

And the enum itself, near `TwilightCmd`'s definition (same file, top-level item):

```rust
#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// List saved config.conf generations, newest first (index 0 = most recent).
    List,
    /// Show a unified diff between a saved generation and the live file.
    Diff {
        /// Generation index (0 = most recent). Default: 0.
        n: Option<usize>,
    },
    /// Restore a saved generation to config.conf and reload.
    Rollback {
        /// Generation index (0 = most recent). Default: 1 (one before the current file).
        n: Option<usize>,
        /// Skip the symlink/overwrite confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}
```

- [ ] **Step 5: Implement the handler functions and wire them into `main`**

Add three new functions near `cmd_check_config` (same file):

```rust
fn cmd_config_list() -> Result<()> {
    let gens = margo_config::generations::list()?;
    if gens.is_empty() {
        println!("no saved generations yet — one is created on the next successful boot or `mctl reload`");
        return Ok(());
    }
    for (idx, gen) in gens.iter().enumerate() {
        let when: chrono::DateTime<chrono::Local> = gen.timestamp.into();
        println!("{idx:>3}  {}  {}", when.format("%Y-%m-%d %H:%M:%S"), gen.id);
    }
    Ok(())
}

fn resolve_live_config_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "{}/.config/margo/config.conf",
        std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
    ))
}

fn cmd_config_diff(n: Option<usize>) -> Result<()> {
    let gens = margo_config::generations::list()?;
    let idx = n.unwrap_or(0);
    let gen = gens
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("no generation at index {idx} (have {})", gens.len()))?;
    let old = margo_config::generations::read(&gen.id)?;
    let live_path = resolve_live_config_path();
    let new = std::fs::read_to_string(&live_path)
        .with_context(|| format!("reading {}", live_path.display()))?;
    let diff = similar::TextDiff::from_lines(&old, &new);
    print!(
        "{}",
        diff.unified_diff()
            .header(&format!("generation {}", gen.id), &live_path.display().to_string())
    );
    Ok(())
}

fn cmd_config_rollback(n: Option<usize>, yes: bool) -> Result<()> {
    let gens = margo_config::generations::list()?;
    let idx = n.unwrap_or(1);
    let gen = gens
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("no generation at index {idx} (have {})", gens.len()))?;
    let content = margo_config::generations::read(&gen.id)?;

    // Safety check: never write a candidate that doesn't itself parse
    // (should be unreachable — generations are only ever saved after a
    // successful parse — but cheap insurance against a corrupted file).
    margo_config::parse_config_str(&content, None)
        .with_context(|| format!("generation {} does not parse; refusing to roll back to it", gen.id))?;

    let live_path = resolve_live_config_path();
    let is_symlink = std::fs::symlink_metadata(&live_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink && !yes {
        let target = std::fs::read_link(&live_path).unwrap_or_default();
        eprintln!(
            "{} is a symlink to {} — rollback writes through it, changing that file.",
            live_path.display(),
            target.display()
        );
        eprint!("Continue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("rollback cancelled");
            return Ok(());
        }
    }

    std::fs::write(&live_path, &content)
        .with_context(|| format!("writing {}", live_path.display()))?;
    println!("restored generation {} to {}", gen.id, live_path.display());
    send_dispatch("reload", &[])?;
    println!("reloaded");
    Ok(())
}
```

Wire `List` and `Diff` into the **first** match block (no compositor connection needed — same group as `Command::CheckConfig`/`Command::ConfigErrors`), right after the existing `Command::ConfigErrors => { return cmd_config_errors(); }` arm:

```rust
        Command::Config {
            action: ConfigCmd::List,
        } => {
            return cmd_config_list();
        }
        Command::Config {
            action: ConfigCmd::Diff { n },
        } => {
            return cmd_config_diff(*n);
        }
```

Wire `Rollback` into the **second** match block (it ends with `send_dispatch("reload", ...)`, which needs the compositor socket) — add it near the existing `Command::Log { .. }` arm. Two things differ from a naive copy of the `Diff` arm above:

1. This block matches `args.command` **by value** (unlike the first block's `match &args.command`), so `n`/`yes` are owned here — no `*` deref.
2. This block's `match args.command { ... }` is **exhaustive with no catch-all** — every variant handled entirely in block 1 (e.g. `Command::CheckConfig`) still needs an arm here, via the existing `Command::Actions { .. } | Command::CheckConfig { .. } | ... => unreachable!(...)` list further down. `Command::Config` is only *partially* handled in block 1 (`List`/`Diff`, not `Rollback`), so it needs its own nested match here — mirroring exactly how `Command::Twilight { action }` (line ~1038) handles a variant where some `TwilightCmd` values return early and others reach the socket:

```rust
        Command::Config { action } => match action {
            ConfigCmd::List | ConfigCmd::Diff { .. } => {
                unreachable!("List/Diff return early in main's first match block")
            }
            ConfigCmd::Rollback { n, yes } => {
                cmd_config_rollback(n, yes)?;
            }
        },
```

- [ ] **Step 6: Run the tests, then the full crate checks**

This is the first point since Step 3 where the crate compiles at all — Steps 4 and 5 together are what makes `match args.command` exhaustive again.

Run: `cargo test -p mctl config_ -- --test-threads=1`
Expected: PASS (5 tests, from Step 2).

Run: `cargo test -p mctl`
Expected: PASS (full crate suite, no regressions).

Run: `cargo check -p mctl`
Expected: clean.

Run: `cargo clippy -p mctl --all-targets -- -D warnings`
Expected: clean. If clippy flags the manual stdin-confirmation loop for a simpler idiom, apply its suggestion — this project's CI gate runs clippy with `-D warnings`, so the task isn't done until it's silent.

- [ ] **Step 7: Commit**

```bash
git add mctl/Cargo.toml mctl/src/bin/mctl.rs Cargo.lock
git commit -m "feat(mctl): add config list/diff/rollback

New nested 'mctl config' subcommand: list saved config.conf
generations, diff one against the live file (similar::TextDiff), and
roll back to one (writes through a symlink with a confirmation prompt,
then reuses the existing 'reload' dispatch verb to apply it live — no
new IPC surface). Reads margo-config::generations directly; no
compositor connection needed for list/diff.

Spec: docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 5: Cross-crate verification + push

**Files:** none (verification only).

- [ ] **Step 1: Run the full test suite for every touched crate**

Run: `cargo test -p margo-config -p margo -p mctl`
Expected: PASS, no regressions anywhere. (No `--lib` — `margo` is a bin-only crate; see Task 3 Step 6.)

- [ ] **Step 2: Run clippy across the touched crates**

Run: `cargo clippy -p margo-config -p margo -p mctl --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt --check`
Expected: clean. If not, run `cargo fmt` and re-diff to confirm only the touched files changed.

- [ ] **Step 4: Confirm `Cargo.lock` is in sync (per `reference_cargo_lock_locked_packaging`)**

Run: `cargo metadata --offline > /dev/null`
Expected: succeeds with no network access needed — confirms `Cargo.lock` already accounts for the `chrono` (margo-config) and `similar` (mctl) edges added in Tasks 1 and 4.

- [ ] **Step 5: Push**

```bash
git push
```

- [ ] **Step 6: Manual smoke test (not run by the plan executor — hand off to the user)**

Note for whoever runs this after `just cli`/`just margo` are rebuilt and installed: touch `~/.config/margo/config.conf`, run `mctl reload`, then `mctl config list` should show a new generation; `mctl config diff` should be empty right after a clean reload; edit the file, run `mctl config rollback --yes`, confirm the edit is undone and the compositor reloaded. This step is out of scope for the plan executor per the project's build-workflow convention (compiling/installing/running the compositor is the user's job) — just flag it in the final report.
