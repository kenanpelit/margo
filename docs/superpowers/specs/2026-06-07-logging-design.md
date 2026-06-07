# margo + mshell logging — design

**Date:** 2026-06-07
**Status:** Approved (design); implementation plan to follow.

## Goal

Give both **margo** (compositor) and **mshell** (shell) a real file-logging
mechanism: configurable from **Settings** and from the **command line**, on by
default at `info`, with selectable depth (`error`→`trace`) applied **live**, and
keeping the **last 3 sessions** of logs on disk. Purpose: catch and diagnose
bugs after the fact ("açıkken son 3 oturum diskte dursun").

## Current state (what exists)

- Both processes log via `tracing_subscriber::fmt()` to **stdout/stderr only**.
  - `mshell-crates/mshell-logging/src/lib.rs::init(app)` — `EnvFilter` from
    `RUST_LOG` else `"{app}=info"`, fmt to stdout.
  - `margo/src/main.rs` — its own `tracing_subscriber::fmt` + `EnvFilter`.
- No file sink, no rotation, no runtime level control. mshell's stdout is
  captured by journald (`systemctl --user`); margo's goes to the TTY/start-margo.
- Precedent for the location: margo already uses
  `~/.local/state/margo/session` via `XDG_STATE_HOME` (`margo/src/session.rs`).

## Decisions (locked)

| Question | Decision |
|---|---|
| Location | `$XDG_STATE_HOME/margo/logs/` (default `~/.local/state/margo/logs/`), **flat** — both apps write directly here, namespaced by the `{app}-` filename prefix (`margo-*.log`, `mshell-*.log`). No per-app subdir. |
| Default | **On**, level `info`. |
| Levels | Full ladder `error < warn < info < debug < trace`, selectable in Settings + command, applied **live** (no restart). |
| "Last 3 sessions" | **Each process start = a new session file**; keep the newest 3 per app, delete older. |
| Scope (YAGNI) | File sink + per-start rotation + Settings toggle/level + CLI command. **No** in-Settings live viewer, **no** tar diagnostic bundle (deferred). |
| Architecture | Shared `margo-logging` crate used by both processes (Approach A). |

## Architecture

### Component 1 — `margo-logging` (new top-level crate)

The shared engine. One clear job: stand up tracing with a stdout layer + a
rotating per-session file layer, and expose a live level handle.

**Public API**

```rust
pub struct LogInit {
    /// "margo" or "mshell" — used for the filter target and the file prefix.
    pub app_name: String,
    /// The shared log directory, ~/.local/state/margo/logs (flat; files are
    /// namespaced by the `{app_name}-` prefix). Both apps pass the same dir.
    pub dir: PathBuf,
    /// Initial level: "error"|"warn"|"info"|"debug"|"trace".
    pub level: String,
    /// Whether file logging is enabled (false ⇒ file filter starts "off").
    pub enabled: bool,
    /// How many session files to keep (3).
    pub keep_sessions: usize,
    /// Whether to also log to stdout (true for both processes today).
    pub to_stdout: bool,
}

pub struct LogHandle { /* reload handle + WorkerGuard + dir + app_name */ }

pub fn init(opts: LogInit) -> LogHandle;

impl LogHandle {
    /// Live: change the file layer's level ("error".."trace").
    pub fn set_level(&self, level: &str) -> Result<(), LogError>;
    /// Live: enable/disable file logging (maps to filter "off" when disabled).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), LogError>;
    /// The current session file path (for `log path` / "open log").
    pub fn current_file(&self) -> PathBuf;
    /// The app's log directory (for "open folder").
    pub fn dir(&self) -> &Path;
}
```

**Init flow**

1. `create_dir_all(dir)` — on failure, fall back to **stdout-only** (warn, no
   panic — this runs on the login-critical path).
2. **Rotate**: list `dir/{app_name}-*.log`, sort by name (timestamps sort
   lexicographically), delete the oldest until `keep_sessions - 1` remain, then
   create the new session file `dir/{app_name}-{YYYYMMDD-HHMMSS}.log`. Refresh a
   convenience symlink `dir/{app_name}-latest.log → <new file>` (best-effort).
3. Build the subscriber:
   - **stdout layer** (only if `to_stdout`) with its own `EnvFilter` — preserves
     today's behaviour.
   - **file layer**: `tracing_appender::non_blocking` writer over the session
     file, wrapped in a **`reload::Layer`-managed `EnvFilter`**. The `WorkerGuard`
     is stored in `LogHandle` so logs flush on drop.
   - Initial file filter = `RUST_LOG` if set, else `level_to_filter(app_name,
     level)`; if `!enabled`, file filter = `"off"`.
4. `.init()` the registry; return `LogHandle`.

**Level mapping** — `level_to_filter("margo", "debug")` ⇒ EnvFilter `"margo=debug"`
(plus a sane default for deps, e.g. `warn` baseline). `enabled=false` ⇒ `"off"`.

**Live changes** — `set_level`/`set_enabled` call `reload::Handle::modify` to
swap the file filter. Stdout filter is left at startup value. The handle is
`Send + Sync`, so it can be stored in a global `OnceLock` and driven from an IPC
handler thread in either process.

**Rotation rule** — "session" = one process lifetime. Crash/restart ⇒ clean new
file; the previous session's file is preserved (one of the kept 3). Empty files
(disabled-at-start) still count toward the 3; acceptable.

### Component 2 — margo wiring

- **`margo-config`** (`.conf`, text key=value): add knobs (parser + `KNOWN` +
  validator, per `docs/config-conventions.md`):
  - `log_to_file` (bool, default `true`)
  - `log_level` (string, default `"info"`; validate against the ladder)
  - `log_keep_sessions` (u32, default `3`)
- **`margo/src/main.rs`**: replace the inline tracing init with
  `margo_logging::init(LogInit{ app_name:"margo", dir: state_logs,
  level, enabled, keep_sessions, to_stdout:true })` (where `state_logs =
  ~/.local/state/margo/logs`). Store the `LogHandle` in a
  `static OnceLock<LogHandle>` (the guard must outlive the process).
- **`reload_config`**: when the parsed config's `log_level`/`log_to_file`
  change, call `handle.set_level` / `set_enabled` so a `mctl config reload`
  applies them live (per the config-conventions "re-apply to be live" rule).
- **`mctl` + socket IPC**: new `log` subcommand →
  - `mctl log level <error|warn|info|debug|trace>` — live `set_level`.
  - `mctl log path` — print dir + current file.
  - `mctl log open` — `xdg-open` the dir.
  - Dispatch verbs added to margo's socket handler; they reach the global handle.

### Component 3 — mshell wiring

- **`mshell-config`** (YAML, serde): add to `Config`:
  ```rust
  #[serde(default)]
  pub logging: LoggingConfig,
  // LoggingConfig { enabled: bool=true, level: String="info", keep_sessions: u32=3 }
  ```
  with a manual `Default` (serde-default caveat per config-conventions) and a
  `.logging()` store accessor.
- **`mshell-logging::init`**: delegate to `margo_logging::init` (app
  `"mshell"`, dir `~/.local/state/margo/logs`), reading the config. Store the
  `LogHandle` in a global `OnceLock` so `mshell-core` IPC can reach it. Keep the
  `init(app_name)` signature working for any other callers.
- **`mshellctl` + IPC** (`mshell-core/src/ipc.rs`): new `log` verbs mirroring
  margo's — `mshellctl log level <lvl>`, `mshellctl log path`, `mshellctl log
  open`. The IPC handler calls `handle.set_level` / `set_enabled`.

### Component 4 — Settings → Logging page (`mshell-settings`)

One page, two sections (one card each):

- **Shell (mshell)**: enable `Switch` + level `DropDown`
  (Error/Warn/Info/Debug/Trace). On change → patch the mshell YAML via
  `config_manager` **and** apply live through the shell `LogHandle`.
- **Compositor (margo)**: enable `Switch` + level `DropDown`. On change → patch
  margo's `.conf` via the existing `compositor_conf`
  (`read_raw`/`patch_conf`/`set_and_reload` → `mctl reload`) **and** apply live
  via `mctl log level`.
- **Footer**: "Open log folder" button → `xdg-open ~/.local/state/margo/logs`.

Registration (per the Settings sidebar pattern): `mod logging_settings;` +
controller + bump the `stack_pages` array length + a sidebar `Page` entry +
search keywords/aliases ("log", "logging", "debug", "trace", "diagnostics").

Follow `mshell-frame/DESIGN.md` for switch/dropdown/button styling; buttons are
the **compact** Settings kind (no `.ok-button-cell`), matching the recent
Settings button pass.

## Data flow

```
Settings page ──patch──> mshell YAML (.logging)   ──live──> shell LogHandle.set_*
            └─patch──> margo .conf (log_*) ─mctl reload─> margo LogHandle.set_*
mshellctl log level ─IPC─> shell LogHandle.set_level
mctl log level      ─sock─> margo LogHandle.set_level
process start ─> margo_logging::init ─> rotate(keep 3) + open session file + tracing layers
tracing events ─> stdout layer (unchanged) + file layer (EnvFilter, reloadable) ─> session .log
```

## Error handling

- Dir create / file open failure ⇒ stdout-only fallback, one warning, **never
  panic** (margo init is login-critical).
- Bad level string from IPC/config ⇒ reject with an error to the caller
  (CLI prints it); keep the previous filter.
- Rotation delete failure ⇒ log a warning, continue (don't block startup).

## Testing

- **`margo-logging`** unit tests:
  - rotation keeps exactly `keep_sessions`, deletes the oldest, ordering by
    timestamped name;
  - `level_to_filter` maps each ladder rung; `enabled=false` ⇒ `"off"`;
  - init into a `tempfile::tempdir()` creates a session file + symlink.
- **`margo-config`**: parse `log_to_file`/`log_level`/`log_keep_sessions`
  (valid + invalid level rejected by validator).
- **`mshell-config`**: `LoggingConfig` serde defaults when the key is absent.
- IPC level-set verb covered where the existing IPC test harness allows.

## Out of scope (deferred)

- In-Settings live log viewer/tail panel ("Open folder" suffices for now).
- One-command diagnostic tarball (last 3 sessions + config) — revisit later,
  reusing the Backup bundle pattern.
- Per-target/module filtering UI (the `EnvFilter` supports it; not exposed yet).
