//! Embedded scripting engine for user-defined hooks.
//!
//! Status: **Phase 3** — event hooks fire mid-event-loop.
//! `on_focus_change`, `on_tag_switch`, and `on_window_open`
//! registered from `~/.config/margo/init.rhai` are now invoked
//! at the matching compositor event sites with the live state
//! reachable through the same binding surface as Phase 2.
//!
//! The engine, the compiled AST, and the per-event hook lists
//! all live on `MargoState::scripting`, so callbacks survive
//! past startup. To fire a hook we briefly *take* the
//! `ScriptingState` out of `MargoState`, run the FnPtr on the
//! still-owned engine + AST, then put it back. This keeps the
//! borrow checker happy without introducing unsafe pointer juggling
//! beyond the thread-local state pointer that was already in
//! place for Phase 2 — and gives us a free recursion guard: if a
//! hook calls `dispatch(...)` and that triggers a re-entrant
//! focus change, the inner `fire_*` finds `scripting = None` and
//! is a no-op rather than a stack overflow.
//!
//! ## State flow
//!
//! ```text
//!  startup:
//!    main.rs → init_user_scripting(&mut state)
//!      ↪ build engine, compile AST, store ScriptingState on state
//!      ↪ eval script body → registers FnPtrs into hooks vecs
//!  runtime:
//!    state.focus_surface(...)
//!      ↪ fire_focus_change(state)
//!         ↪ take scripting out of state (now None)
//!         ↪ for h in hooks: h.call(&engine, &ast, ())
//!            ↪ binding code accesses state via STATE_PTR
//!         ↪ put scripting back
//! ```
//!
//! ## Why not store engine in an Rc<RefCell<…>>?
//!
//! Tried it. Rhai's `FnPtr::call(&engine, &ast, args)` takes plain
//! `&` references; wrapping in RefCell forces us to borrow on
//! every call and a hook that recursively triggers another hook
//! double-borrows. The take/restore dance gives us recursion
//! safety for free.

use std::cell::Cell;
use std::path::PathBuf;

use rhai::{Array, Dynamic, Engine, FnPtr, Scope, AST};
use tracing::{error, info, warn};

use margo_config::Arg;

use crate::state::MargoState;

thread_local! {
    static STATE_PTR: Cell<*mut MargoState> = const { Cell::new(std::ptr::null_mut()) };
}

fn with_state<R>(f: impl FnOnce(&mut MargoState) -> R) -> Option<R> {
    let ptr = STATE_PTR.with(|s| s.get());
    if ptr.is_null() {
        warn!("scripting binding called outside an eval context — ignoring");
        return None;
    }
    // Safety: STATE_PTR is non-null only during init or hook eval,
    // both of which hold the unique mutable borrow of MargoState.
    // Rhai runs synchronously on the compositor thread.
    let state = unsafe { &mut *ptr };
    Some(f(state))
}

/// Per-MargoState scripting context. Holds the running engine,
/// the compiled init.rhai AST, and the lists of hook closures the
/// user registered. Stored as `Option<Box<ScriptingState>>` on
/// `MargoState` so we can `take()` the value out during hook
/// invocation (see module docs).
pub struct ScriptingState {
    engine: Engine,
    ast: AST,
    pub hooks: ScriptingHooks,
}

#[derive(Default)]
pub struct ScriptingHooks {
    pub on_focus_change: Vec<FnPtr>,
    pub on_tag_switch: Vec<FnPtr>,
    pub on_window_open: Vec<FnPtr>,
    /// `on_window_close(|appid, title| { ... })` — handlers receive
    /// the closing window's identity as args (the window is about
    /// to be removed from `clients`, so `focused_*()` can't reach
    /// it; the focus has typically already moved to a sibling).
    pub on_window_close: Vec<FnPtr>,
    /// `on_output_change(|name| { ... })` — fires after udev's
    /// hotplug coalescer settles and `rescan_outputs` has updated
    /// the monitor list. The arg is the affected output's connector
    /// name when knowable (`"DP-3"`, `"eDP-1"`), or empty string
    /// when the rescan covered the whole topology and no single
    /// output dominated the change. Handlers can query
    /// `monitor_count()`, `output_geometry(name)`, etc. from the
    /// scripting API to react.
    pub on_output_change: Vec<FnPtr>,
}

/// Build an engine pre-loaded with the binding surface (Phase 2)
/// plus the hook registration functions (Phase 3). The engine is
/// stateless w.r.t. user code — all per-script data ends up in
/// `ScriptingHooks` on MargoState — so we hand a fresh one back.
fn build_engine() -> Engine {
    let mut engine = Engine::new();

    engine.set_max_call_levels(64);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_array_size(1024);
    engine.set_max_string_size(64 * 1024);
    engine.set_strict_variables(true);

    engine.on_print(|s| info!(target: "init.rhai", "{s}"));
    engine.on_debug(|s, src, pos| {
        let src = src.unwrap_or("init.rhai");
        info!(target: "init.rhai", "[{src}:{pos:?}] {s}");
    });

    // ── Action invocation ───────────────────────────────────────────────

    engine.register_fn("dispatch", |action: &str, args: Array| {
        let arg = args_to_arg(&args);
        with_state(|state| {
            crate::dispatch::dispatch_action(state, action, &arg);
        });
    });
    engine.register_fn("dispatch", |action: &str| {
        with_state(|state| {
            crate::dispatch::dispatch_action(state, action, &Arg::default());
        });
    });

    engine.register_fn("spawn", |cmd: &str| {
        if let Err(e) = crate::utils::spawn_shell(cmd) {
            warn!(cmd = %cmd, error = ?e, "init.rhai: spawn failed");
        }
    });

    engine.register_fn("tag", |n: rhai::INT| -> rhai::INT {
        if (1..=32).contains(&n) {
            1 << (n - 1)
        } else {
            warn!(n = n, "init.rhai: tag() out of range [1, 32], returning 0");
            0
        }
    });

    // ── Read-only state introspection ───────────────────────────────────

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

    engine.register_fn("focused_appid", || -> String {
        with_state(|state| {
            state
                .focused_client_idx()
                .map(|i| state.clients[i].app_id.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    });

    engine.register_fn("focused_title", || -> String {
        with_state(|state| {
            state
                .focused_client_idx()
                .map(|i| state.clients[i].title.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    });

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

    engine.register_fn("monitor_count", || -> rhai::INT {
        with_state(|state| state.monitors.len() as rhai::INT).unwrap_or(0)
    });

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

    engine.register_fn("client_count", || -> rhai::INT {
        with_state(|state| state.clients.len() as rhai::INT).unwrap_or(0)
    });

    // ── Event hook registration (Phase 3) ───────────────────────────────
    //
    // These now actually fire — the registered FnPtr is appended to
    // the matching list in `ScriptingHooks` and invoked from the
    // event site via `fire_*` helpers below.

    engine.register_fn("on_focus_change", |handler: FnPtr| {
        with_state(|s| {
            if let Some(sc) = s.scripting.as_mut() {
                sc.hooks.on_focus_change.push(handler);
            }
        });
    });
    engine.register_fn("on_tag_switch", |handler: FnPtr| {
        with_state(|s| {
            if let Some(sc) = s.scripting.as_mut() {
                sc.hooks.on_tag_switch.push(handler);
            }
        });
    });
    engine.register_fn("on_window_open", |handler: FnPtr| {
        with_state(|s| {
            if let Some(sc) = s.scripting.as_mut() {
                sc.hooks.on_window_open.push(handler);
            }
        });
    });
    engine.register_fn("on_window_close", |handler: FnPtr| {
        with_state(|s| {
            if let Some(sc) = s.scripting.as_mut() {
                sc.hooks.on_window_close.push(handler);
            }
        });
    });
    engine.register_fn("on_output_change", |handler: FnPtr| {
        with_state(|s| {
            if let Some(sc) = s.scripting.as_mut() {
                sc.hooks.on_output_change.push(handler);
            }
        });
    });

    engine
}

fn args_to_arg(args: &Array) -> Arg {
    let mut arg = Arg::default();
    let mut string_slot = 0;
    let mut int_slot = 0;
    let mut float_slot = 0;
    for v in args.iter() {
        if v.is_string() {
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
    candidates.into_iter().flatten().find(|c| c.is_file())
}

/// Stand up the scripting engine on `state`, compile init.rhai if
/// present, and run its top-level statements once. Hook-registration
/// statements populate `state.scripting.hooks`; non-hook statements
/// (like a top-level `dispatch("setlayout", ["scroller"])`) execute
/// immediately. After this returns, the engine + AST stay parked on
/// MargoState ready for the `fire_*` helpers to invoke registered
/// hooks at runtime.
///
/// No-op if no init.rhai exists — the user runs un-scripted by
/// default.
pub fn init_user_scripting(state: &mut MargoState) {
    let Some(path) = init_script_path() else {
        return;
    };
    info!(path = %path.display(), "init.rhai: evaluating");

    let engine = build_engine();
    let ast = match engine.compile_file(path.clone()) {
        Ok(ast) => ast,
        Err(e) => {
            error!(path = %path.display(), error = ?e, "init.rhai: compile error");
            return;
        }
    };

    state.scripting = Some(Box::new(ScriptingState {
        engine,
        ast,
        hooks: ScriptingHooks::default(),
    }));

    // Eval top-level statements. The hook registration functions
    // need state.scripting to exist (they push into hooks vecs), so
    // we install ScriptingState first and run after.
    let Some(sc) = state.scripting.take() else {
        return;
    };

    struct StateGuard;
    impl Drop for StateGuard {
        fn drop(&mut self) {
            STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
        }
    }
    STATE_PTR.with(|s| s.set(state as *mut _));
    let _guard = StateGuard;

    let mut scope = Scope::new();
    let result = sc.engine.run_ast_with_scope(&mut scope, &sc.ast);

    // Drop the state pointer before the explicit re-assignment so
    // observers that count refcounts via STATE_PTR don't see two
    // active refs.
    STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
    state.scripting = Some(sc);

    match result {
        Ok(()) => info!("init.rhai: ran cleanly"),
        Err(e) => error!("init.rhai: runtime error — {e}"),
    }
}

/// Which hook list to fire. Used by `fire_hook` so it knows which
/// vec to drain + which to restore — encoded explicitly instead of
/// inferred so re-registration during a hook body (a hook calls
/// `on_tag_switch(...)` while running an `on_focus_change`)
/// doesn't make us put back the wrong list.
#[derive(Copy, Clone)]
enum HookKind {
    FocusChange,
    TagSwitch,
    WindowOpen,
}

/// Invoke every registered `on_focus_change` hook.
pub fn fire_focus_change(state: &mut MargoState) {
    fire_hook(state, HookKind::FocusChange);
}

/// Invoke every registered `on_tag_switch` hook.
pub fn fire_tag_switch(state: &mut MargoState) {
    fire_hook(state, HookKind::TagSwitch);
}

/// Invoke every registered `on_window_open` hook. Called from
/// `finalize_initial_map` so handlers see the final, post-rule
/// app_id / title — not the empty initial values.
pub fn fire_window_open(state: &mut MargoState) {
    fire_hook(state, HookKind::WindowOpen);
}

/// W3.3: load every discovered plugin under
/// `~/.config/margo/plugins/<name>/`. Called once at startup
/// after `init_user_scripting` so init.rhai sets up the engine
/// + state and plugins layer their hooks on top.
///
/// Each plugin's init.rhai is compiled + run with the same
/// engine the user's init.rhai uses, so plugins can share
/// helpers via `import` (Rhai's module system) and see all the
/// hook-registration bindings. The compositor doesn't sandbox
/// per-plugin — Rhai's host-side sandboxing already prevents
/// FFI / fs access; cross-plugin interference is the user's
/// responsibility (don't install plugins that fight each
/// other).
///
/// Plugins with `enabled = false` in their manifest are skipped.
/// Compile / runtime errors don't abort the loader — the rest
/// of the plugins continue. The compositor stays up regardless.
pub fn init_plugins(state: &mut MargoState) {
    let mut plugins = crate::plugin::discover();
    if plugins.is_empty() {
        return;
    }
    info!(count = plugins.len(), "loading plugins");

    // Stand up the engine if init.rhai didn't (no init.rhai on
    // disk — plugins-only setup). Mirrors the same fallback as
    // run_script_file.
    if state.scripting.is_none() {
        state.scripting = Some(Box::new(ScriptingState {
            engine: build_engine(),
            ast: rhai::AST::empty(),
            hooks: ScriptingHooks::default(),
        }));
    }

    for plugin in plugins.iter_mut() {
        if !plugin.manifest.enabled {
            info!(
                "plugin: {} (disabled in manifest, skipping)",
                plugin.manifest.name
            );
            continue;
        }
        if !plugin.script.is_file() {
            warn!(
                "plugin: {} has no init.rhai at {} — skipping",
                plugin.manifest.name,
                plugin.script.display()
            );
            continue;
        }

        // Compile against the live engine.
        let Some(mut sc) = state.scripting.take() else {
            return;
        };
        let ast = match sc.engine.compile_file(plugin.script.clone()) {
            Ok(a) => a,
            Err(e) => {
                warn!(
                    "plugin {}: compile error in {} — {e}",
                    plugin.manifest.name,
                    plugin.script.display()
                );
                state.scripting = Some(sc);
                continue;
            }
        };
        let original_ast = std::mem::replace(&mut sc.ast, ast);
        state.scripting = Some(sc);
        run_compiled(state);
        if let Some(sc) = state.scripting.as_mut() {
            sc.ast = original_ast;
        }
        plugin.loaded = true;
        info!(
            "plugin loaded: {} v{} from {} — {}",
            plugin.manifest.name,
            plugin.manifest.version,
            plugin.dir.display(),
            plugin.manifest.description
        );
    }

    state.plugins = plugins;
}

/// W3.2: one-shot script eval. Reads a Rhai script from `path`,
/// compiles it, and runs it once against the live MargoState.
/// Used by `mctl run <file>` for ad-hoc scripting without
/// reloading init.rhai.
///
/// Falls back gracefully:
///   * If init.rhai never set up the engine (no scripting state
///     parked on MargoState), we stand up a fresh one for this
///     call so users can `mctl run` even without a config file.
///   * Compile / runtime errors log at error level and return —
///     the live compositor stays up.
///   * Hook registrations inside the script (e.g. an
///     `on_focus_change(...)` call) DO persist after this run,
///     same as init.rhai. Use case: live-edit your hook list.
pub fn run_script_file(state: &mut MargoState, path: &std::path::Path) {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            error!(path = %path.display(), error = ?e, "mctl run: read failed");
            return;
        }
    };

    // Stand up the engine if init.rhai never ran (no init.rhai on
    // disk, or it errored out). This lets users `mctl run` from a
    // fresh session without forcing them to author an init script.
    if state.scripting.is_none() {
        let engine = build_engine();
        let ast = match engine.compile(&src) {
            Ok(a) => a,
            Err(e) => {
                error!(path = %path.display(), error = ?e, "mctl run: compile failed");
                return;
            }
        };
        state.scripting = Some(Box::new(ScriptingState {
            engine,
            ast,
            hooks: ScriptingHooks::default(),
        }));
        // Run the freshly-compiled script via the same eval path
        // init_user_scripting uses.
        run_compiled(state);
        return;
    }

    // Engine already running. Compile against the same engine so
    // any registered fns / consts from init.rhai are visible.
    let Some(mut sc) = state.scripting.take() else {
        return;
    };
    let ast = match sc.engine.compile(&src) {
        Ok(a) => a,
        Err(e) => {
            error!(path = %path.display(), error = ?e, "mctl run: compile failed");
            state.scripting = Some(sc);
            return;
        }
    };
    // Replace the stored AST temporarily so hook registrations
    // resolve against the new file's symbols. Keep the old AST
    // saved so the engine doesn't lose init.rhai's definitions.
    let original_ast = std::mem::replace(&mut sc.ast, ast);
    state.scripting = Some(sc);
    run_compiled(state);
    // Swap original AST back so the next hook fire still sees
    // init.rhai's body.
    if let Some(sc) = state.scripting.as_mut() {
        sc.ast = original_ast;
    }
    info!(path = %path.display(), "mctl run: ran");
}

fn run_compiled(state: &mut MargoState) {
    let Some(sc) = state.scripting.take() else {
        return;
    };
    struct StateGuard;
    impl Drop for StateGuard {
        fn drop(&mut self) {
            STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
        }
    }
    STATE_PTR.with(|s| s.set(state as *mut _));
    let _guard = StateGuard;
    let mut scope = Scope::new();
    if let Err(e) = sc.engine.run_ast_with_scope(&mut scope, &sc.ast) {
        error!(error = ?e, "mctl run: runtime error");
    }
    STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
    state.scripting = Some(sc);
}

/// Invoke every registered `on_window_close` hook. Called from the
/// toplevel-destroyed path BEFORE the client is removed from
/// `clients`, so `client_count()` and `focused_*()` still reflect
/// the pre-close state. Handler receives `(app_id, title)` as Rhai
/// strings — the closing window is rarely the focused one (focus
/// has usually already shifted to a sibling), so the args are the
/// only reliable identity channel.
/// Invoke every registered `on_output_change` hook. Called from the
/// hotplug coalescer's timer body after `rescan_outputs` settles, so
/// handlers see the final post-rescan monitor list. `output_name`
/// is the connector that prompted the rescan when knowable
/// (UdevEvent::Changed only gives us a device id, not the connector
/// per-se, so we pass the empty string for now; future refinement
/// can plumb the affected connector through `rescan_outputs`'s
/// diff).
pub fn fire_output_change(state: &mut MargoState, output_name: &str) {
    let Some(mut sc) = state.scripting.take() else {
        return;
    };
    let hooks = std::mem::take(&mut sc.hooks.on_output_change);
    if hooks.is_empty() {
        state.scripting = Some(sc);
        return;
    }

    {
        struct StateGuard;
        impl Drop for StateGuard {
            fn drop(&mut self) {
                STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
            }
        }
        STATE_PTR.with(|s| s.set(state as *mut _));
        let _guard = StateGuard;

        let name = output_name.to_string();
        for h in &hooks {
            let res: Result<Dynamic, Box<rhai::EvalAltResult>> =
                h.call(&sc.engine, &sc.ast, (name.clone(),));
            if let Err(e) = res {
                warn!(error = ?e, "init.rhai: on_output_change handler error");
            }
        }
    }

    if sc.hooks.on_output_change.is_empty() {
        sc.hooks.on_output_change = hooks;
    } else {
        let mut combined = hooks;
        combined.extend(std::mem::take(&mut sc.hooks.on_output_change));
        sc.hooks.on_output_change = combined;
    }
    state.scripting = Some(sc);
}

pub fn fire_window_close(state: &mut MargoState, app_id: &str, title: &str) {
    let Some(mut sc) = state.scripting.take() else {
        return;
    };
    let hooks = std::mem::take(&mut sc.hooks.on_window_close);
    if hooks.is_empty() {
        state.scripting = Some(sc);
        return;
    }

    {
        struct StateGuard;
        impl Drop for StateGuard {
            fn drop(&mut self) {
                STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
            }
        }
        STATE_PTR.with(|s| s.set(state as *mut _));
        let _guard = StateGuard;

        let app_id = app_id.to_string();
        let title = title.to_string();
        for h in &hooks {
            let res: Result<Dynamic, Box<rhai::EvalAltResult>> =
                h.call(&sc.engine, &sc.ast, (app_id.clone(), title.clone()));
            if let Err(e) = res {
                warn!(error = ?e, "init.rhai: on_window_close handler error");
            }
        }
    }

    if sc.hooks.on_window_close.is_empty() {
        sc.hooks.on_window_close = hooks;
    } else {
        let mut combined = hooks;
        combined.extend(std::mem::take(&mut sc.hooks.on_window_close));
        sc.hooks.on_window_close = combined;
    }
    state.scripting = Some(sc);
}

fn fire_hook(state: &mut MargoState, kind: HookKind) {
    // Take scripting out of state. A nested fire_hook (a hook calls
    // dispatch which causes another event) finds None and is a
    // no-op — recursion guard for free.
    let Some(mut sc) = state.scripting.take() else {
        return;
    };
    let hooks = match kind {
        HookKind::FocusChange => std::mem::take(&mut sc.hooks.on_focus_change),
        HookKind::TagSwitch => std::mem::take(&mut sc.hooks.on_tag_switch),
        HookKind::WindowOpen => std::mem::take(&mut sc.hooks.on_window_open),
    };

    // Hot path: nothing registered. Restore + return without
    // touching the state pointer.
    if hooks.is_empty() {
        // The list was empty before too, so nothing to restore.
        state.scripting = Some(sc);
        return;
    }

    {
        // RAII state-pointer guard so a panic inside Rhai (caught by
        // the engine but theoretically possible) clears the pointer.
        struct StateGuard;
        impl Drop for StateGuard {
            fn drop(&mut self) {
                STATE_PTR.with(|s| s.set(std::ptr::null_mut()));
            }
        }
        STATE_PTR.with(|s| s.set(state as *mut _));
        let _guard = StateGuard;

        for h in &hooks {
            let res: Result<Dynamic, Box<rhai::EvalAltResult>> =
                h.call(&sc.engine, &sc.ast, ());
            if let Err(e) = res {
                warn!(error = ?e, "init.rhai: hook error");
            }
        }
    }

    // Restore the drained list — but if a hook body called
    // `on_focus_change(...)` etc. while running, the matching
    // vec is now non-empty; append rather than overwrite so we
    // keep both old + new handlers.
    let dst = match kind {
        HookKind::FocusChange => &mut sc.hooks.on_focus_change,
        HookKind::TagSwitch => &mut sc.hooks.on_tag_switch,
        HookKind::WindowOpen => &mut sc.hooks.on_window_open,
    };
    if dst.is_empty() {
        *dst = hooks;
    } else {
        // Newly-registered handlers came in during the run; keep
        // them at the front and put the previous set behind.
        let mut combined = hooks;
        combined.extend(std::mem::take(dst));
        *dst = combined;
    }
    state.scripting = Some(sc);
}
