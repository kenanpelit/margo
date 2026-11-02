//! Rhai scripting: hook registration and firing.
//!
//! Hooks (`on_focus_change(...)` & co.) are registered while a script runs
//! and fired later from compositor events.

use super::fixture::Fixture;
use crate::layout::LayoutId;

fn fixture() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx
}

/// Write `src` to a fresh file and run it the way `mctl run` does.
fn run(fx: &mut Fixture, name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("margo-scripting-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.rhai"));
    std::fs::write(&path, src).expect("write script");
    crate::scripting::run_script_file(&mut fx.server.state, &path);
    path
}

fn layout(fx: &Fixture) -> LayoutId {
    fx.server.state.monitors[0].current_layout()
}

#[test]
fn a_script_registers_its_hook() {
    let mut fx = fixture();
    run(&mut fx, "register", "on_focus_change(|| ());");
    assert_eq!(
        fx.server.state.script_hooks.on_focus_change.len(),
        1,
        "on_focus_change from a script must register a hook"
    );
}

#[test]
fn a_registered_hook_fires() {
    let mut fx = fixture();
    assert_ne!(layout(&fx), LayoutId::Monocle);
    run(
        &mut fx,
        "fires",
        r#"on_focus_change(|| dispatch("setlayout", ["monocle"]));"#,
    );
    assert_ne!(
        layout(&fx),
        LayoutId::Monocle,
        "registering must not run it"
    );

    crate::scripting::fire_focus_change(&mut fx.server.state);

    assert_eq!(layout(&fx), LayoutId::Monocle, "the hook body must run");
}

#[test]
fn every_hook_kind_registers_and_fires() {
    let mut fx = fixture();
    run(
        &mut fx,
        "kinds",
        r#"
        on_tag_switch(|| dispatch("setlayout", ["monocle"]));
        on_window_open(|| dispatch("setlayout", ["grid"]));
        on_window_close(|app_id, title| dispatch("setlayout", [app_id]));
        on_output_change(|name| dispatch("setlayout", [name]));
        "#,
    );
    let hooks = &fx.server.state.script_hooks;
    assert_eq!(hooks.on_tag_switch.len(), 1);
    assert_eq!(hooks.on_window_open.len(), 1);
    assert_eq!(hooks.on_window_close.len(), 1);
    assert_eq!(hooks.on_output_change.len(), 1);

    crate::scripting::fire_tag_switch(&mut fx.server.state);
    assert_eq!(layout(&fx), LayoutId::Monocle);
    crate::scripting::fire_window_open(&mut fx.server.state);
    assert_eq!(layout(&fx), LayoutId::Grid);
    crate::scripting::fire_window_close(&mut fx.server.state, "scroller", "t");
    assert_eq!(
        layout(&fx),
        LayoutId::Scroller,
        "close hooks get (app_id, title)"
    );
    crate::scripting::fire_output_change(&mut fx.server.state, "monocle");
    assert_eq!(layout(&fx), LayoutId::Monocle, "output hooks get the name");
}

#[test]
fn a_named_function_hook_from_a_later_script_still_resolves() {
    // `mctl run` / plugins compile their own AST. A hook given as
    // `Fn("name")` has to be called against the AST that defines `name`,
    // not whichever AST happens to be parked on the engine.
    let mut fx = fixture();
    run(&mut fx, "first", "let unused = 1;");
    run(
        &mut fx,
        "second",
        r#"
        fn go_monocle() { dispatch("setlayout", ["monocle"]); }
        on_tag_switch(Fn("go_monocle"));
        "#,
    );

    crate::scripting::fire_tag_switch(&mut fx.server.state);

    assert_eq!(layout(&fx), LayoutId::Monocle);
}

#[test]
fn hooks_survive_being_fired() {
    let mut fx = fixture();
    run(&mut fx, "survive", "on_focus_change(|| ());");
    crate::scripting::fire_focus_change(&mut fx.server.state);
    crate::scripting::fire_focus_change(&mut fx.server.state);
    assert_eq!(fx.server.state.script_hooks.on_focus_change.len(), 1);
}

#[test]
fn a_hook_registered_by_a_hook_is_kept_alongside_it() {
    let mut fx = fixture();
    run(
        &mut fx,
        "nested-register",
        "on_focus_change(|| on_focus_change(|| ()));",
    );
    crate::scripting::fire_focus_change(&mut fx.server.state);
    assert_eq!(fx.server.state.script_hooks.on_focus_change.len(), 2);
}

#[test]
fn run_script_from_inside_a_hook_does_not_break_the_running_hook() {
    // The nested run used to install a fresh engine, clear the state
    // pointer when it finished (so the rest of the outer hook ran with
    // every binding ignored), and then have its engine overwritten.
    let mut fx = fixture();
    let inner = run(&mut fx, "inner", "let x = 1;");
    run(
        &mut fx,
        "outer",
        &format!(
            r#"on_focus_change(|| {{
                dispatch("run_script", [{inner:?}]);
                dispatch("setlayout", ["monocle"]);
            }});"#
        ),
    );

    crate::scripting::fire_focus_change(&mut fx.server.state);

    assert_eq!(
        layout(&fx),
        LayoutId::Monocle,
        "bindings after a nested run_script must still reach the state"
    );
    assert_eq!(fx.server.state.script_hooks.on_focus_change.len(), 1);
    // The engine is still usable afterwards.
    run(&mut fx, "after", "on_tag_switch(|| ());");
    assert_eq!(fx.server.state.script_hooks.on_tag_switch.len(), 1);
}
