//! `cursor_theme` / `cursor_size` apply on reload (the live cursor picker
//! path: Settings writes the keys, then runs `mctl reload`). The fixture
//! has no XWayland, so only margo's own cursor is checked here; the env /
//! XSETTINGS mapping is unit-tested in `state/reload/cursor.rs`.

use super::fixture::Fixture;
use crate::cursor::settings::{self, CursorSettings};

fn reload_with(fx: &mut Fixture, dir: &std::path::Path, body: &str) {
    let path = dir.join("config.conf");
    std::fs::write(&path, body).expect("write config");
    fx.server.state.config_path = Some(path);
    fx.server.state.generations_dir = dir.join("generations");
    fx.server.state.reload_config().expect("reload");
}

#[test]
fn reload_applies_cursor_theme_and_size_and_drops_them_again() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let dir = tempfile::tempdir().expect("tempdir");

    reload_with(
        &mut fx,
        dir.path(),
        "cursor_theme = margo-test-no-such-theme\ncursor_size = 40\n",
    );
    assert_eq!(
        fx.server.state.cursor_manager.settings(),
        &CursorSettings {
            theme: Some("margo-test-no-such-theme".into()),
            size: 40,
        }
    );

    // Keys removed: back to the launch environment / defaults.
    reload_with(&mut fx, dir.path(), "borderpx = 2\n");
    let fallback = settings::resolve(None, None, &[], settings::inherited_env());
    assert_eq!(fx.server.state.cursor_manager.settings(), &fallback);
}

#[test]
fn an_explicit_cursor_size_beats_an_env_line() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let dir = tempfile::tempdir().expect("tempdir");
    // 24 is also the built-in default: written out, it still wins.
    reload_with(
        &mut fx,
        dir.path(),
        "env = XCURSOR_SIZE,36\ncursor_size = 24\n",
    );
    assert_eq!(fx.server.state.cursor_manager.settings().size, 24);
}

#[test]
fn an_env_line_counts_when_the_keys_are_unset() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    let dir = tempfile::tempdir().expect("tempdir");
    reload_with(
        &mut fx,
        dir.path(),
        "env = XCURSOR_THEME,margo-test-env-theme\nenv = XCURSOR_SIZE,36\n",
    );
    assert_eq!(
        fx.server.state.cursor_manager.settings(),
        &CursorSettings {
            theme: Some("margo-test-env-theme".into()),
            size: 36,
        }
    );
}
