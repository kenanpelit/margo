//! The keymode (`setkeymode`) and the session lock.
//!
//! Over the lock screen only `force_unlock` and `l` (lock_apply) binds may
//! run. They used to be looked up in the current keymode only, so in a
//! "passthrough"/"resize" mode the volume keys and the `force_unlock`
//! recovery bind were dead on the lock screen. A keymode that has no binds
//! (a `setkeymode` typo, or a mode a reload removed) left only the
//! `common` binds live, with no key to get back to `default`.

use super::fixture::Fixture;
use crate::dispatch::dispatch_action;
use crate::input::find_lock_binding;
use margo_config::{Arg, Modifiers};

const BACKSPACE: u32 = 0xff08;
const VOLUME_UP: u32 = 0x1008_ff13;
const RETURN: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;

const BINDS: &str = "\
bind = super+ctrl+alt,BackSpace,force_unlock
bindl = none,XF86AudioRaiseVolume,spawn,volume-up
bind = super,Return,spawn,terminal
keymode = passthrough
bind = super,Escape,setkeymode,default
bind = none,XF86AudioRaiseVolume,spawn,vm-volume
";

fn config() -> margo_config::Config {
    margo_config::parse_config_str(BINDS, None).expect("test binds parse")
}

fn action(kb: Option<&margo_config::KeyBinding>) -> Option<(&str, Option<&str>)> {
    kb.map(|kb| (kb.action.as_str(), kb.arg.v.as_deref()))
}

#[test]
fn lock_binds_resolve_from_the_default_mode_in_another_keymode() {
    let cfg = config();
    let binds = &cfg.key_bindings;
    let hyper = Modifiers::LOGO | Modifiers::CTRL | Modifiers::ALT;
    for mode in ["default", "passthrough", "no-such-mode"] {
        assert_eq!(
            action(find_lock_binding(binds, hyper, &[BACKSPACE], 0, mode)),
            Some(("force_unlock", None)),
            "force_unlock in mode {mode}"
        );
        assert_eq!(
            action(find_lock_binding(
                binds,
                Modifiers::empty(),
                &[VOLUME_UP],
                0,
                mode
            )),
            Some(("spawn", Some("volume-up"))),
            "lock_apply volume bind in mode {mode}: the passthrough mode's \
             own (not lock_apply) bind must not shadow it"
        );
    }
}

#[test]
fn ordinary_binds_never_resolve_over_the_lock() {
    let cfg = config();
    let binds = &cfg.key_bindings;
    for mode in ["default", "passthrough"] {
        assert_eq!(
            action(find_lock_binding(
                binds,
                Modifiers::LOGO,
                &[RETURN],
                0,
                mode
            )),
            None
        );
        assert_eq!(
            action(find_lock_binding(
                binds,
                Modifiers::LOGO,
                &[ESCAPE],
                0,
                mode
            )),
            None
        );
    }
}

#[test]
fn a_later_keysym_candidate_still_matches() {
    // The handler passes the modified sym first, then the raw/latin syms.
    let cfg = config();
    let hyper = Modifiers::LOGO | Modifiers::CTRL | Modifiers::ALT;
    assert_eq!(
        action(find_lock_binding(
            &cfg.key_bindings,
            hyper,
            &[0x61, BACKSPACE],
            0,
            "passthrough"
        )),
        Some(("force_unlock", None))
    );
}

fn setkeymode(fx: &mut Fixture, mode: &str) {
    let arg = Arg {
        v: Some(mode.to_string()),
        ..Default::default()
    };
    dispatch_action(&mut fx.server.state, "setkeymode", &arg);
}

#[test]
fn setkeymode_refuses_a_mode_without_binds() {
    let mut fx = Fixture::with_config(config());
    setkeymode(&mut fx, "passthrough");
    assert_eq!(fx.server.state.input_keyboard.mode, "passthrough");
    fx.server.state.state_dirty.set(false);
    setkeymode(&mut fx, "pasthrough");
    assert_eq!(
        fx.server.state.input_keyboard.mode, "passthrough",
        "a typo must not strand the keyboard in a mode with no binds"
    );
    assert!(!fx.server.state.state_dirty.get());
    setkeymode(&mut fx, "default");
    assert_eq!(fx.server.state.input_keyboard.mode, "default");
}

#[test]
fn a_reload_that_drops_the_current_keymode_returns_to_default() {
    let dir = std::env::temp_dir().join(format!("margo-keymode-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.conf");
    std::fs::write(&path, BINDS).unwrap();

    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.server.state.config_path = Some(path.clone());
    fx.server.state.generations_dir = dir.join("generations");
    fx.server.state.reload_config().unwrap();
    setkeymode(&mut fx, "passthrough");
    assert_eq!(fx.server.state.input_keyboard.mode, "passthrough");

    // Same mode still defined: a reload keeps it.
    fx.server.state.reload_config().unwrap();
    assert_eq!(fx.server.state.input_keyboard.mode, "passthrough");

    std::fs::write(&path, "bind = super,Return,spawn,terminal\n").unwrap();
    fx.server.state.state_dirty.set(false);
    fx.server.state.reload_config().unwrap();
    assert_eq!(fx.server.state.input_keyboard.mode, "default");
    assert!(
        fx.server.state.state_dirty.get(),
        "the keymode pill must update"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
