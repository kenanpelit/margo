//! Embedded scripting engine for user-defined hooks.
//!
//! Status: **Phase 2** — `dispatch(action, args)` and read-only state
//! introspection are live. Scripts in `~/.config/margo/init.rhai` can
//! invoke any registered margo action and inspect focused-client /
//! current-tag / monitor-list state. Event hooks (`on_focus_change`,
//! `on_tag_switch`, `on_window_open`) are still forward-compat stubs;
//! they accept handler registrations without error so users can write
//! Phase-3-ready scripts today.
//!
//! Why Rhai instead of Lua / a bespoke DSL: pure Rust (no C build),
//! type-safe binding via `register_fn`, sandbox tight by default.
//! See `docs/scripting-design.md` for the rollout plan.
//!
//! ## State access pattern
//!
//! Phase 2 bindings need `&mut MargoState` to dispatch actions and
//! read state. Rather than thread the state through Rhai's
//! `NativeCallContext` plumbing, we use a thread-local raw pointer
//! set for the duration of `run_user_init` and cleared on return:
//!
//!   * Rhai runs synchronously on the compositor thread, so there's
//!     no aliasing window — the script can't escape to another
//!     thread or yield mid-call.
//!   * The pointer is set via a `Drop`-guarded scope so a Rhai panic
//!     (script syntax error caught by the engine, or a `panic!()`
//!     in a binding) cannot leave a dangling pointer behind.
//!   * Bindings call `with_state` which checks the pointer is non-
//!     null before dereferencing. If a script somehow ends up calling
//!     a binding outside the eval scope (a stored `FnPtr` invoked
//!     later), the call is a no-op with a warning, not a crash.
//!
//! Once Phase 3 lands and event hooks fire mid-event-loop, this
//! pattern still works — set the pointer at each event site, run
//! the registered handler, clear. Same eval contract.

use std::cell::Cell;
use std::path::PathBuf;

use rhai::{Array, Dynamic, Engine, Scope};
use tracing::{error, info, warn};

use margo_config::Arg;

use crate::state::MargoState;

thread_local! {
    /// Raw pointer to the live `MargoState` for the duration of a
    /// script eval. Set by `run_user_init`, cleared on return. See
    /// the module-level docs for the safety contract.
    static STATE_PTR: Cell<*mut MargoState> = const { Cell::new(std::ptr::null_mut()) };
}

/// Run `f` with a `&mut MargoState` if the script eval scope is
/// active. Returns `None` (with a warning) if a binding was somehow
/// invoked outside an eval — e.g. via a stored `FnPtr` retained past
/// the scope. The default value the caller provides ensures the
/// script keeps running rather than panicking.
fn with_state<R>(f: impl FnOnce(&mut MargoState) -> R) -> Option<R> {
    let ptr = STATE_PTR.with(|s| s.get());
    if ptr.is_null() {
        warn!("scripting binding called outside an eval context — ignoring");
        return None;
    }
    // Safety: STATE_PTR is non-null only inside `run_user_init`,
    // which holds the unique mutable borrow of `MargoState`. Rhai
    // runs synchronously on the compositor thread, so no second
    // reference can race.
    let state = unsafe { &mut *ptr };
    Some(f(state))
}

/// Build a Rhai engine pre-loaded with margo's binding surface.
pub fn init_engine() -> Engine {
    let mut engine = Engine::new();

    // Sandbox limits. Defence-in-depth: script comes from the user's
    // own dotfiles, but a typo'd recursive function shouldn't take
    // the compositor down.
    engine.set_max_call_levels(64);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_array_size(1024);
    engine.set_max_string_size(64 * 1024);
    engine.set_strict_variables(true);

    // Wire Rhai's `print()` and `debug()` into tracing so script
    // output lands in `journalctl -u margo` rather than stdout —
    // which is closed in a real DRM session and would otherwise
    // silently swallow everything.
    engine.on_print(|s| info!(target: "init.rhai", "{s}"));
    engine.on_debug(|s, src, pos| {
        let src = src.unwrap_or("init.rhai");
        info!(target: "init.rhai", "[{src}:{pos:?}] {s}");
    });

    // ── Action invocation ───────────────────────────────────────────────
    //
    // `dispatch(action, args_array)` invokes any registered margo
    // action. The args array maps positionally onto the `Arg` struct:
    //   - First / second / third strings → v / v2 / v3
    //   - First / second integers        → ui & i / ui2 & i2
    //   - First / second floats          → f / f2
    //
    // Tags are bitmasks — use `tag(n)` to convert a 1-based tag
    // number to its mask.
    //
    // Examples:
    //   dispatch("spawn", ["kitty"])
    //   dispatch("setlayout", ["scroller"])
    //   dispatch("view", [tag(5)])         // switch to tag 5
    //   dispatch("focusstack", [1])        // direction = +1 (next)
    //   dispatch("tagview", [tag(8)])      // move + follow to tag 8
    engine.register_fn("dispatch", |action: &str, args: Array| {
        let arg = args_to_arg(&args);
        with_state(|state| {
            crate::dispatch::dispatch_action(state, action, &arg);
        });
    });
    // Zero-arg overload for actions like "killclient" / "switch_layout"
    // / "togglefloating" / "togglefullscreen" / "reload".
    engine.register_fn("dispatch", |action: &str| {
        with_state(|state| {
            crate::dispatch::dispatch_action(state, action, &Arg::default());
        });
    });

    // `spawn(cmd)` — convenience equivalent to `dispatch("spawn", [cmd])`
    // but without the dispatch-table indirection. Identical semantics
    // to the config-file `spawn` action.
    engine.register_fn("spawn", |cmd: &str| {
        if let Err(e) = crate::utils::spawn_shell(cmd) {
            warn!("init.rhai spawn '{cmd}' failed: {e}");
        }
    });

    // `tag(n)` — convert 1-based tag number to bitmask. Example:
    //   tag(1) == 1, tag(5) == 16, tag(9) == 256.
    // Out-of-range (n < 1 or n > 32) returns 0 — dispatch with mask 0
    // is a no-op rather than a crash, so a typo is recoverable.
    engine.register_fn("tag", |n: rhai::INT| -> rhai::INT {
        if (1..=32).contains(&n) {
            1 << (n - 1)
        } else {
            warn!("init.rhai: tag({n}) out of range [1, 32], returning 0");
            0
        }
    });

    // ── Read-only state introspection ───────────────────────────────────
    //
    // None of these mutate. Mutation goes through `dispatch(...)`.

    // Returns the 1-based bit position of the lowest selected tag on
    // the focused monitor. Useful for `if current_tag() == 8 { … }`.
    // Returns 0 if no monitor or no tag is selected.
    engine.register_fn("current_tag", || -> rhai::INT {
        with_state(|state| {
            let mon = state.focused_monitor();
            let mask = state
                .monitors
                .get(mon)
                .map(|m| m.current_tagset())
                .unwrap_or(0);
            if mask == 0 {
                0
            } else {
                (mask.trailing_zeros() as rhai::INT) + 1
            }
        })
        .unwrap_or(0)
    });

    // Raw bitmask of all currently-visible tags on the focused monitor.
    // Useful for "is tag 4 visible alongside tag 8" checks via `&`.
    engine.register_fn("current_tagmask", || -> rhai::INT {
        with_state(|state| {
            let mon = state.focused_monitor();
            state
                .monitors
                .get(mon)
                .map(|m| m.current_tagset() as rhai::INT)
                .unwrap_or(0)
        })
        .unwrap_or(0)
    });

    // app_id of the focused client, empty string if no focus.
    engine.register_fn("focused_appid", || -> String {
        with_state(|state| {
            state
                .focused_client_idx()
                .map(|i| state.clients[i].app_id.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    });

    // title of the focused client, empty string if no focus.
    engine.register_fn("focused_title", || -> String {
        with_state(|state| {
            state
                .focused_client_idx()
                .map(|i| state.clients[i].title.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    });

    // Output name of the focused monitor (e.g. "DP-3", "eDP-1").
    // Empty string if no monitor is focused.
    engine.register_fn("focused_monitor_name", || -> String {
        with_state(|state| {
            let mon = state.focused_monitor();
            state
                .monitors
                .get(mon)
                .map(|m| m.name.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    });

    // Number of connected outputs.
    engine.register_fn("monitor_count", || -> rhai::INT {
        with_state(|state| state.monitors.len() as rhai::INT).unwrap_or(0)
    });

    // Names of all connected outputs as an array of strings.
    engine.register_fn("monitor_names", || -> Array {
        with_state(|state| {
            state
                .monitors
                .iter()
                .map(|m| Dynamic::from(m.name.clone()))
                .collect()
        })
        .unwrap_or_default()
    });

    // Number of mapped clients.
    engine.register_fn("client_count", || -> rhai::INT {
        with_state(|state| state.clients.len() as rhai::INT).unwrap_or(0)
    });

    // ── Forward-compat event-hook stubs (Phase 3) ───────────────────────
    //
    // Accept handler registrations today — log them — fire nothing
    // yet. When Phase 3 wires the event sites in `state.rs`, scripts
    // that already register hooks here just start firing.
    engine.register_fn("on_focus_change", |_handler: rhai::FnPtr| {
        info!("init.rhai: on_focus_change registered (Phase 3 — not yet wired)");
    });
    engine.register_fn("on_tag_switch", |_handler: rhai::FnPtr| {
        info!("init.rhai: on_tag_switch registered (Phase 3 — not yet wired)");
    });
    engine.register_fn("on_window_open", |_handler: rhai::FnPtr| {
        info!("init.rhai: on_window_open registered (Phase 3 — not yet wired)");
    });

    engine
}

/// Map a Rhai `Array` of args to the `Arg` struct the dispatch table
/// consumes. Strings populate `v` / `v2` / `v3` in declaration order;
/// integers populate `(i, ui)` / `(i2, ui2)` (one integer fills both
/// the signed and unsigned slots — actions read whichever they need
/// via `tag_arg` / `direction_arg`); floats populate `f` / `f2`.
///
/// Booleans are coerced to integers (`true → 1`, `false → 0`) so a
/// script written `dispatch("togglefloating", [true])` doesn't fail.
fn args_to_arg(args: &Array) -> Arg {
    let mut arg = Arg::default();
    let mut string_slot = 0;
    let mut int_slot = 0;
    let mut float_slot = 0;
    for v in args.iter() {
        if v.is_string() {
            // Cloning is cheap relative to the dispatch call itself
            // and keeps the `args` param non-`&mut` so multiple
            // bindings can read the same array shape.
            let s = v.clone().into_string().unwrap_or_default();
            match string_slot {
                0 => arg.v = Some(s),
                1 => arg.v2 = Some(s),
                2 => arg.v3 = Some(s),
                _ => {}
            }
            string_slot += 1;
        } else if v.is_int() {
            let i = v.as_int().unwrap_or(0);
            match int_slot {
                0 => {
                    arg.i = i as i32;
                    arg.ui = i as u32;
                }
                1 => {
                    arg.i2 = i as i32;
                    arg.ui2 = i as u32;
                }
                _ => {}
            }
            int_slot += 1;
        } else if v.is_bool() {
            let b = v.as_bool().unwrap_or(false);
            let i = if b { 1 } else { 0 };
            match int_slot {
                0 => {
                    arg.i = i;
                    arg.ui = i as u32;
                }
                1 => {
                    arg.i2 = i;
                    arg.ui2 = i as u32;
                }
                _ => {}
            }
            int_slot += 1;
        } else if v.is_float() {
            let f = v.as_float().unwrap_or(0.0);
            match float_slot {
                0 => arg.f = f as f32,
                1 => arg.f2 = f as f32,
                _ => {}
            }
            float_slot += 1;
        }
    }
    arg
}

fn init_script_path() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("XDG_CONFIG_HOME").map(|h| PathBuf::from(h).join("margo/init.rhai")),
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/margo/init.rhai")),
    ];
    for c in candidates.into_iter().flatten() {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// Evaluate the user's `init.rhai` if present, with the live
/// `MargoState` reachable through the bound functions. Errors are
/// logged with Rhai's own line/col reporting; never panic.
pub fn run_user_init(engine: &Engine, state: &mut MargoState) {
    let Some(path) = init_script_path() else {
        return;
    };
    info!("init.rhai: evaluating {}", path.display());

    // RAII guard so the pointer is cleared even if Rhai unwinds.
    struct StateGuard;
    impl Drop for StateGuard {
        fn drop(&mut self) {
            STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
        }
    }
    STATE_PTR.with(|s| s.set(state as *mut _));
    let _guard = StateGuard;

    let mut scope = Scope::new();
    match engine.run_file_with_scope(&mut scope, path.clone()) {
        Ok(()) => info!("init.rhai: ran cleanly"),
        Err(e) => error!("init.rhai: error in {} — {e}", path.display()),
    }
}
