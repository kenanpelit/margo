//! Embedded scripting engine for user-defined hooks.
//!
//! `~/.config/margo/init.rhai`, plugins and `mctl run` scripts run on
//! the compositor thread with the live state reachable through the
//! bindings below. Their `on_focus_change`, `on_tag_switch`,
//! `on_window_open`, `on_window_close` and `on_output_change` calls
//! register hooks that fire at the matching compositor events.
//!
//! The engine lives on `MargoState::scripting`; the hook lists live
//! on `MargoState::script_hooks`, outside the engine box. To run a
//! script or fire a hook, [`with_engine`] briefly *takes* the engine
//! out of `MargoState`, points the thread-local `STATE_PTR` at the
//! state so bindings can reach it, runs, restores the previous
//! pointer and puts the engine back. While it is checked out a nested
//! `fire_*` finds `scripting = None` and is a no-op — the recursion
//! guard for a hook whose `dispatch(...)` causes another event.
//!
//! Each hook remembers the AST of the script that registered it, so a
//! `Fn("name")` handler from a plugin or `mctl run` file is called
//! against the file that defines `name`.
//!
//! ## State flow
//!
//! ```text
//!  startup:
//!    main.rs → init_user_scripting(&mut state)
//!      ↪ build engine (parked on state), compile AST
//!      ↪ eval script body → on_*(...) pushes Hook{handler, ast}
//!        into state.script_hooks
//!  runtime:
//!    state.focus_surface(...)
//!      ↪ fire_focus_change(state)
//!         ↪ take the hook list + the engine out of state
//!         ↪ for h in hooks: h.handler.call(&engine, &h.ast, args)
//!            ↪ binding code accesses state via STATE_PTR
//!         ↪ put the engine back, merge hooks registered meanwhile
//! ```
//!
//! ## Why not store engine in an Rc<RefCell<…>>?
//!
//! Tried it. Rhai's `FnPtr::call(&engine, &ast, args)` takes plain
//! `&` references; wrapping in RefCell forces us to borrow on
//! every call and a hook that recursively triggers another hook
//! double-borrows. The take/restore dance gives us recursion
//! safety for free.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use rhai::{AST, Array, Dynamic, Engine, FnPtr, Scope};
use tracing::{error, info, warn};

use margo_config::Arg;

use crate::state::MargoState;

thread_local! {
    static STATE_PTR: Cell<*mut MargoState> = const { Cell::new(std::ptr::null_mut()) };
    /// AST of the script or hook running right now; new hooks are tied to it.
    static CURRENT_AST: RefCell<Option<Rc<AST>>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut MargoState) -> R) -> Option<R> {
    let ptr = STATE_PTR.with(|s| s.get());
    if ptr.is_null() {
        warn!("scripting binding called outside an eval context — ignoring");
        return None;
    }
    // Safety: STATE_PTR is non-null only inside `with_engine`, which
    // holds the unique mutable borrow of MargoState for the duration.
    // Rhai runs synchronously on the compositor thread.
    let state = unsafe { &mut *ptr };
    Some(f(state))
}

/// Per-MargoState scripting context: the running engine. Stored as
/// `Option<Box<ScriptingState>>` on `MargoState` so [`with_engine`]
/// can `take()` it out while a script or hook runs (see module docs).
pub struct ScriptingState {
    engine: Engine,
}

/// One registered hook: the handler plus the AST of the script that
/// registered it (needed to resolve `Fn("name")` handlers).
pub struct Hook {
    handler: FnPtr,
    ast: Rc<AST>,
}

/// Registered hooks, on `MargoState::script_hooks`.
#[derive(Default)]
pub struct ScriptingHooks {
    pub on_focus_change: Vec<Hook>,
    pub on_tag_switch: Vec<Hook>,
    pub on_window_open: Vec<Hook>,
    /// `on_window_close(|appid, title| { ... })` — handlers receive
    /// the closing window's identity as args (the window is about
    /// to be removed from `clients`, so `focused_*()` can't reach
    /// it; the focus has typically already moved to a sibling).
    pub on_window_close: Vec<Hook>,
    /// `on_output_change(|name| { ... })` — fires after udev's
    /// hotplug coalescer settles and `rescan_outputs` has updated
    /// the monitor list. The arg is the affected output's connector
    /// name when knowable (`"DP-3"`, `"eDP-1"`), or empty string
    /// when the rescan covered the whole topology and no single
    /// output dominated the change. Handlers can query
    /// `monitor_count()`, `output_geometry(name)`, etc. from the
    /// scripting API to react.
    pub on_output_change: Vec<Hook>,
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
    // Bound total operations so a runaway script (`loop {}`, an accidental
    // infinite recursion in a hook, `run_script` on a hostile file) can't
    // wedge the compositor event loop forever — the eval errors out instead
    // of freezing input + rendering until the session is killed. Scripts run
    // synchronously on the main loop, so this is a hard safety limit, not a
    // perf tunable; 5M is generous for any legitimate init/hook script.
    engine.set_max_operations(5_000_000);

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

    engine.register_fn("tag", tag_mask_for);

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

    // ── Event hook registration ─────────────────────────────────────────
    //
    // The handler is appended to the matching list in
    // `MargoState::script_hooks` and invoked from the event site via
    // the `fire_*` helpers below.

    engine.register_fn("on_focus_change", |handler: FnPtr| {
        register_hook(HookKind::FocusChange, handler);
    });
    engine.register_fn("on_tag_switch", |handler: FnPtr| {
        register_hook(HookKind::TagSwitch, handler);
    });
    engine.register_fn("on_window_open", |handler: FnPtr| {
        register_hook(HookKind::WindowOpen, handler);
    });
    engine.register_fn("on_window_close", |handler: FnPtr| {
        register_hook(HookKind::WindowClose, handler);
    });
    engine.register_fn("on_output_change", |handler: FnPtr| {
        register_hook(HookKind::OutputChange, handler);
    });

    engine
}

/// Rhai `tag(n)`: the mask for 1-based tag `n`, or 0 (with a warning) when
/// `n` isn't one of margo's `MAX_TAGS` tags. Tag actions ignore a 0 mask.
fn tag_mask_for(n: rhai::INT) -> rhai::INT {
    if (1..=crate::MAX_TAGS as rhai::INT).contains(&n) {
        1 << (n - 1)
    } else {
        warn!(
            n = n,
            max = crate::MAX_TAGS,
            "init.rhai: tag() out of range [1, MAX_TAGS], returning 0"
        );
        0
    }
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
            // Saturate the signed slot (a wrapped sign would flip a
            // move/resize); `ui` keeps the low bits, it carries tag masks.
            let signed = i.clamp(rhai::INT::from(i32::MIN), rhai::INT::from(i32::MAX)) as i32;
            match int_slot {
                0 => {
                    arg.i = signed;
                    arg.ui = i as u32;
                }
                1 => {
                    arg.i2 = signed;
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
            // NaN / ±inf (or a finite f64 beyond f32) would pass through
            // f32::clamp into the layouts: leave the slot at its default.
            let f = v.as_float().unwrap_or(0.0) as f32;
            let f = if f.is_finite() { f } else { 0.0 };
            match float_slot {
                0 => arg.f = f,
                1 => arg.f2 = f,
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
/// statements populate `state.script_hooks`; non-hook statements
/// (like a top-level `dispatch("setlayout", ["scroller"])`) execute
/// immediately. After this returns, the engine stays parked on
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
    let Some(ast) = compile(state, Source::File(&path)) else {
        return;
    };
    match eval(state, ast) {
        Some(Ok(())) => info!("init.rhai: ran cleanly"),
        Some(Err(e)) => error!("init.rhai: runtime error — {e}"),
        None => {}
    }
}

/// Which hook list to fire or register into.
#[derive(Copy, Clone, Debug)]
enum HookKind {
    FocusChange,
    TagSwitch,
    WindowOpen,
    WindowClose,
    OutputChange,
}

impl ScriptingHooks {
    fn list_mut(&mut self, kind: HookKind) -> &mut Vec<Hook> {
        match kind {
            HookKind::FocusChange => &mut self.on_focus_change,
            HookKind::TagSwitch => &mut self.on_tag_switch,
            HookKind::WindowOpen => &mut self.on_window_open,
            HookKind::WindowClose => &mut self.on_window_close,
            HookKind::OutputChange => &mut self.on_output_change,
        }
    }
}

/// Invoke every registered `on_focus_change` hook.
pub fn fire_focus_change(state: &mut MargoState) {
    fire(state, HookKind::FocusChange, Vec::new());
}

/// Invoke every registered `on_tag_switch` hook.
pub fn fire_tag_switch(state: &mut MargoState) {
    fire(state, HookKind::TagSwitch, Vec::new());
}

/// Invoke every registered `on_window_open` hook. Called from
/// `finalize_initial_map` so handlers see the final, post-rule
/// app_id / title — not the empty initial values.
pub fn fire_window_open(state: &mut MargoState) {
    fire(state, HookKind::WindowOpen, Vec::new());
}

/// Invoke every registered `on_window_close` hook. Called from the
/// toplevel-destroyed path BEFORE the client is removed from
/// `clients`, so `client_count()` and `focused_*()` still reflect
/// the pre-close state. Handler receives `(app_id, title)` as Rhai
/// strings — the closing window is rarely the focused one (focus
/// has usually already shifted to a sibling), so the args are the
/// only reliable identity channel.
pub fn fire_window_close(state: &mut MargoState, app_id: &str, title: &str) {
    fire(
        state,
        HookKind::WindowClose,
        vec![
            Dynamic::from(app_id.to_string()),
            Dynamic::from(title.to_string()),
        ],
    );
}

/// Invoke every registered `on_output_change` hook. Called from the
/// hotplug coalescer's timer body after `rescan_outputs` settles, so
/// handlers see the final post-rescan monitor list. `output_name`
/// is the connector that prompted the rescan when knowable
/// (UdevEvent::Changed only gives us a device id, not the connector
/// per-se, so we pass the empty string for now; future refinement
/// can plumb the affected connector through `rescan_outputs`'s
/// diff).
pub fn fire_output_change(state: &mut MargoState, output_name: &str) {
    fire(
        state,
        HookKind::OutputChange,
        vec![Dynamic::from(output_name.to_string())],
    );
}

/// W3.3: load every discovered plugin under
/// `~/.config/margo/plugins/<name>/`. Called once at startup
/// after `init_user_scripting` so init.rhai sets up the engine
/// + state and plugins layer their hooks on top.
///
/// Each plugin's init.rhai is compiled + run with the same
/// engine the user's init.rhai uses and sees all the
/// hook-registration bindings; its hooks are called against its
/// own AST.
///
/// Plugins are trusted code, exactly like init.rhai: there is no
/// sandbox. The binding surface includes `spawn(cmd)` (a shell
/// command) and `dispatch(...)` (every compositor action), so a
/// plugin can do anything the user can. Only install plugins you
/// would run as a shell script.
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

        let Some(ast) = compile(state, Source::File(&plugin.script)) else {
            continue;
        };
        if let Some(Err(e)) = eval(state, ast) {
            warn!("plugin {}: runtime error — {e}", plugin.manifest.name);
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
///   * If init.rhai never set up the engine, we stand one up so
///     users can `mctl run` even without a config file.
///   * Compile / runtime errors log at error level and return —
///     the live compositor stays up.
///   * Hook registrations inside the script (e.g. an
///     `on_focus_change(...)` call) DO persist after this run,
///     same as init.rhai. Use case: live-edit your hook list.
///   * Called from inside a running script or hook (a hook that
///     dispatches `run_script`), it refuses with a warning: the
///     engine is checked out by the outer eval.
pub fn run_script_file(state: &mut MargoState, path: &std::path::Path) {
    if in_eval() {
        warn!(path = %path.display(), "run_script: ignored inside a running script or hook");
        return;
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            error!(path = %path.display(), error = ?e, "mctl run: read failed");
            return;
        }
    };
    let Some(ast) = compile(state, Source::Text(&src, path)) else {
        return;
    };
    match eval(state, ast) {
        Some(Ok(())) => info!(path = %path.display(), "mctl run: ran"),
        Some(Err(e)) => error!(path = %path.display(), "mctl run: runtime error — {e}"),
        None => {}
    }
}

/// Where a script's text comes from (for compiling and error messages).
enum Source<'a> {
    File(&'a std::path::Path),
    Text(&'a str, &'a std::path::Path),
}

/// Whether a script or hook is running right now (the engine is checked
/// out and `STATE_PTR` is set).
fn in_eval() -> bool {
    !STATE_PTR.with(|s| s.get()).is_null()
}

/// Compile `source` with the parked engine, standing the engine up first
/// if nothing has yet. `None` (logged) on a compile error, or inside a
/// running eval where the engine is checked out.
fn compile(state: &mut MargoState, source: Source<'_>) -> Option<Rc<AST>> {
    if in_eval() {
        return None;
    }
    let sc = state.scripting.get_or_insert_with(|| {
        Box::new(ScriptingState {
            engine: build_engine(),
        })
    });
    let (result, path) = match source {
        Source::File(path) => (
            sc.engine
                .compile_file(path.to_path_buf())
                .map_err(|e| e.to_string()),
            path,
        ),
        Source::Text(src, path) => (sc.engine.compile(src).map_err(|e| e.to_string()), path),
    };
    match result {
        Ok(ast) => Some(Rc::new(ast)),
        Err(e) => {
            error!(path = %path.display(), error = %e, "rhai: compile error");
            None
        }
    }
}

/// Run a compiled script's top-level statements once. Hooks it registers
/// are tied to `ast`. `None` when the engine is unavailable.
fn eval(state: &mut MargoState, ast: Rc<AST>) -> Option<Result<(), Box<rhai::EvalAltResult>>> {
    with_engine(state, |engine| {
        set_current_ast(Some(Rc::clone(&ast)));
        engine.run_ast_with_scope(&mut Scope::new(), &ast)
    })
}

fn set_current_ast(ast: Option<Rc<AST>>) -> Option<Rc<AST>> {
    CURRENT_AST.with(|c| c.replace(ast))
}

/// The one place that checks the engine out of `state` and points the
/// bindings at it. Runs `f` with `STATE_PTR` set to `state`, then restores
/// the previous `STATE_PTR` / current AST (not null: nesting-safe, and a
/// panic unwinding through `f` restores them too) and parks the engine
/// again.
///
/// Returns `None` without running `f` when the engine isn't set up or is
/// already checked out by an outer eval. That is the recursion guard: a
/// hook whose `dispatch(...)` causes another event (a focus change inside
/// `on_focus_change`) doesn't fire hooks re-entrantly.
fn with_engine<R>(state: &mut MargoState, f: impl FnOnce(&Engine) -> R) -> Option<R> {
    let sc = state.scripting.take()?;

    struct Restore {
        ptr: *mut MargoState,
        ast: Option<Rc<AST>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            STATE_PTR.with(|s| s.set(self.ptr));
            set_current_ast(self.ast.take());
        }
    }
    let restore = Restore {
        ptr: STATE_PTR.with(|s| s.replace(state as *mut _)),
        ast: set_current_ast(None),
    };
    let result = f(&sc.engine);
    drop(restore);

    state.scripting = Some(sc);
    Some(result)
}

/// Register `handler` for `kind`, tied to the AST of the script that is
/// running (so `Fn("name")` handlers resolve against the file that
/// defines `name`).
fn register_hook(kind: HookKind, handler: FnPtr) {
    let Some(ast) = CURRENT_AST.with(|c| c.borrow().clone()) else {
        warn!(
            ?kind,
            "rhai: hook registered outside a running script — ignoring"
        );
        return;
    };
    with_state(|s| {
        s.script_hooks.list_mut(kind).push(Hook { handler, ast });
    });
}

/// Fire every hook registered for `kind` with `args`.
///
/// The list lives on `state.script_hooks`, outside the engine box, so a
/// hook body can register more hooks while it runs; those are appended
/// after the existing ones.
fn fire(state: &mut MargoState, kind: HookKind, args: Vec<Dynamic>) {
    // Hot path: nothing registered, or no engine / engine checked out.
    if state.script_hooks.list_mut(kind).is_empty() || state.scripting.is_none() {
        return;
    }
    let hooks = std::mem::take(state.script_hooks.list_mut(kind));
    with_engine(state, |engine| {
        for h in &hooks {
            set_current_ast(Some(Rc::clone(&h.ast)));
            let res: Result<Dynamic, Box<rhai::EvalAltResult>> =
                h.handler.call(engine, &h.ast, args.clone());
            if let Err(e) = res {
                warn!(?kind, error = ?e, "rhai: hook error");
            }
        }
    });
    let dst = state.script_hooks.list_mut(kind);
    let added = std::mem::replace(dst, hooks);
    dst.extend(added);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_is_limited_to_real_tags() {
        assert_eq!(tag_mask_for(1), 1);
        assert_eq!(
            tag_mask_for(crate::MAX_TAGS as rhai::INT),
            1 << (crate::MAX_TAGS - 1)
        );
        assert_eq!(tag_mask_for(0), 0);
        assert_eq!(tag_mask_for(crate::MAX_TAGS as rhai::INT + 1), 0);
        assert_eq!(tag_mask_for(32), 0);
        assert_eq!(tag_mask_for(-3), 0);
    }

    #[test]
    fn numbers_from_scripts_are_saturated_and_finite() {
        let arg = args_to_arg(&vec![
            Dynamic::from(rhai::INT::MAX),
            Dynamic::from(rhai::INT::MIN),
            Dynamic::from(f64::NAN),
            Dynamic::from(f64::INFINITY),
        ]);
        assert_eq!((arg.i, arg.i2), (i32::MAX, i32::MIN));
        assert_eq!((arg.f, arg.f2), (0.0, 0.0));
        // A finite f64 too large for f32 becomes inf when cast.
        let arg = args_to_arg(&vec![Dynamic::from(1e300_f64)]);
        assert_eq!(arg.f, 0.0);
    }
}
