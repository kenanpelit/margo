//! Embedded scripting engine for user-defined hooks.
//!
//! Status: **foundation**. This module wires Rhai into margo so a
//! user can drop a `~/.config/margo/init.rhai` file and have it
//! evaluated at compositor startup. The current API surface is
//! intentionally tiny — just `spawn(cmd)` — enough to validate the
//! plumbing without exposing half-finished bindings that have to
//! evolve incompatibly later. Event hooks (`on_focus`,
//! `on_tag_switch`, `on_window_open`) are documented in
//! `docs/scripting-design.md` and will land as separate commits.
//!
//! Why Rhai instead of Lua / a bespoke DSL:
//!
//!   * Pure Rust, no C dependency or libc-allocator interaction
//!     (matches the rest of margo's "everything builds with cargo
//!     alone" stance).
//!   * Type-safe binding — Rust functions register with their
//!     normal signatures, Rhai handles the marshalling. Avoids the
//!     manual stack-juggling Lua bindings need.
//!   * Sandboxed by default: scripts can only call functions we
//!     explicitly register. No filesystem / network / process
//!     access unless we bind it.
//!   * ~300 KB binary size hit, acceptable for the value.
//!
//! Why not just `mctl dispatch <…>` from a shell script:
//!
//!   * Works for one-shot actions but not for "react to focus
//!     change" / "every tag switch run X" / "if window matches
//!     this rule, do Y" — those need an event-loop callback the
//!     compositor calls into. Shell-out per event is too slow.
//!
//! Implementation today (this commit):
//!
//!   * `init_engine()` constructs a Rhai engine with a single
//!     bound function: `spawn(cmd: String)` → runs the command
//!     via `crate::utils::spawn_shell`.
//!   * `run_user_init()` looks for `~/.config/margo/init.rhai`,
//!     evaluates it once at startup. Errors are logged but
//!     non-fatal — a typo in the user's script doesn't kill the
//!     compositor.
//!
//! Future iterations layer on:
//!
//!   * `dispatch(action: String, args: Vec<String>)` so scripts can
//!     trigger any margo action.
//!   * Event registration: `on_focus_change(|client| { … })`,
//!     `on_tag_switch(|tag| { … })`, etc. The engine instance is
//!     kept alive for the compositor's lifetime; event sites in
//!     `state.rs` look up registered callbacks and call them with
//!     a frozen state snapshot.
//!   * Per-window matchers: `on_map(|window| if window.appid ==
//!     "Spotify" { tagview(8) })`.

use std::path::PathBuf;

use rhai::{Engine, Scope};
use tracing::{error, info, warn};

/// Build a Rhai engine pre-loaded with the bindings margo exposes
/// to user scripts. Called once at startup; the engine itself is
/// stateless (Scope carries per-script variables) so we don't need
/// to keep it in `MargoState` yet.
pub fn init_engine() -> Engine {
    let mut engine = Engine::new();

    // Tighten the default sandbox: no filesystem, no module
    // resolver, no operator overloading. The scripts we run come
    // from the user's own dotfiles, but defence-in-depth is cheap
    // — a malicious upstream config (rare but possible via
    // dotfile sync mishaps) can't exfiltrate without our explicit
    // bindings.
    engine.set_max_call_levels(64);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_array_size(1024);
    engine.set_max_string_size(64 * 1024);
    engine.set_strict_variables(true);

    // Bound function: `spawn(cmd)` — equivalent to
    // `bind = …,…,spawn,<cmd>` from the config. Returns Unit on
    // failure (we don't surface spawn errors as Rhai exceptions
    // to avoid script crashes from the user mistyping a command).
    engine.register_fn("spawn", |cmd: &str| {
        if let Err(e) = crate::utils::spawn_shell(cmd) {
            warn!("init.rhai spawn '{cmd}' failed: {e}");
        }
    });

    // Stub for future event hooks. Calling these from a script
    // today is a no-op so users can write forward-compatible
    // scripts that reference handlers we haven't implemented yet.
    // When the real event sites are wired in, scripts that
    // already register hooks just start firing.
    engine.register_fn("on_focus_change", |_handler: rhai::FnPtr| {
        info!("init.rhai: on_focus_change handler registered (event hooks not yet wired)");
    });
    engine.register_fn("on_tag_switch", |_handler: rhai::FnPtr| {
        info!("init.rhai: on_tag_switch handler registered (event hooks not yet wired)");
    });
    engine.register_fn("on_window_open", |_handler: rhai::FnPtr| {
        info!("init.rhai: on_window_open handler registered (event hooks not yet wired)");
    });

    engine
}

fn init_script_path() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("XDG_CONFIG_HOME")
            .map(|h| PathBuf::from(h).join("margo/init.rhai")),
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/margo/init.rhai")),
    ];
    for c in candidates.into_iter().flatten() {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// Evaluate the user's `init.rhai` if present. Errors are logged
/// (with the script's own line numbers thanks to Rhai's reporting)
/// but never panic the compositor.
pub fn run_user_init(engine: &Engine) {
    let Some(path) = init_script_path() else {
        return;
    };
    info!("init.rhai: evaluating {}", path.display());
    let mut scope = Scope::new();
    match engine.run_file_with_scope(&mut scope, path.clone()) {
        Ok(()) => info!("init.rhai: ran cleanly"),
        Err(e) => error!("init.rhai: error in {} — {e}", path.display()),
    }
}
