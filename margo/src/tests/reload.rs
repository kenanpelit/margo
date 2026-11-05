//! `reload_config` commits nothing until every fallible step has passed.
//! An xkb layout typo (the validator doesn't check xkb) used to fail the
//! reload only after the file had been saved as the newest "last good"
//! generation (the one boot falls back to), its fingerprint recorded as
//! applied (so the auto-reload watcher skipped it) and the windowrule-once
//! bookkeeping cleared.

use super::fixture::Fixture;

#[test]
fn a_reload_failing_on_the_keyboard_layout_commits_nothing() {
    let dir = std::env::temp_dir().join(format!("margo-reload-xkb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.conf");
    std::fs::write(&config, "xkb_rules_layout = no-such-layout-zz\n").unwrap();
    let generations = dir.join("generations");

    let mut fx = Fixture::new();
    fx.add_keyboard();
    let state = &mut fx.server.state;
    state.config_path = Some(config.clone());
    state.generations_dir = generations.clone();
    state.rule_once_consumed.insert(3);
    let fingerprint = state.config_fingerprint;

    assert!(
        state.reload_config().is_err(),
        "an unknown layout must fail the reload"
    );

    let saved = margo_config::generations::list_in(&generations).unwrap();
    assert!(saved.is_empty(), "saved as a good generation: {saved:?}");
    assert_eq!(state.config_fingerprint, fingerprint);
    assert!(state.rule_once_consumed.contains(&3));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_good_reload_saves_its_generation() {
    let dir = std::env::temp_dir().join(format!("margo-reload-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.conf");
    std::fs::write(&config, "borderpx = 3\n").unwrap();
    let generations = dir.join("generations");

    let mut fx = Fixture::new();
    fx.add_keyboard();
    let state = &mut fx.server.state;
    state.config_path = Some(config);
    state.generations_dir = generations.clone();

    state.reload_config().unwrap();

    assert_eq!(
        margo_config::generations::list_in(&generations)
            .unwrap()
            .len(),
        1
    );
    assert!(state.config_fingerprint.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A reload replaces the seat keymap with the configured one, so the
/// "virtual keyboard keymap is already loaded" marker must go too: left
/// set, the next key from the same wtype/mkeys/wayvnc device skipped
/// re-installing its own keymap and was decoded with the user's layout.
#[test]
fn a_reload_forgets_the_active_virtual_keymap() {
    let dir = std::env::temp_dir().join(format!("margo-reload-vkbd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.conf");
    std::fs::write(&config, "borderpx = 3\n").unwrap();

    let mut fx = Fixture::new();
    fx.add_keyboard();
    let state = &mut fx.server.state;
    state.config_path = Some(config);
    state.generations_dir = dir.join("generations");
    state.active_virtual_keymap = Some(std::sync::Arc::from("xkb_keymap { virtual }"));

    state.reload_config().unwrap();

    assert_eq!(state.active_virtual_keymap, None);
    std::fs::remove_dir_all(dir).unwrap();
}

/// `wallpaper = …` applies on reload. The configured wallpaper was decoded
/// once at startup and never again, though the field's doc said reload
/// re-decoded it: a new path needed a compositor restart.
#[test]
fn a_reload_swaps_the_configured_wallpaper() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("config.conf");
    let a = dir.path().join("a.png");
    let b = dir.path().join("b.png");
    image::RgbaImage::new(3, 2).save(&a).expect("png");
    image::RgbaImage::new(5, 1).save(&b).expect("png");

    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.server.state.config_path = Some(config.clone());
    fx.server.state.generations_dir = dir.path().join("generations");
    let size = |fx: &Fixture| {
        fx.server
            .state
            .wallpaper
            .as_ref()
            .map(crate::wallpaper::WallpaperState::natural_size)
    };
    for (path, want) in [(&a, (3, 2)), (&b, (5, 1))] {
        std::fs::write(&config, format!("wallpaper = {}\n", path.display())).expect("write");
        fx.server.state.reload_config().expect("reload");
        for _ in 0..200 {
            if size(&fx) == Some(want) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            fx.server.dispatch();
        }
        assert_eq!(size(&fx), Some(want), "{}", path.display());
    }
}
