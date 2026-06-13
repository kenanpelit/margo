//! First-run config bootstrap (`ensure_default_config`).
//!
//! A brand-new `~/.config/margo` must come up with a complete, valid,
//! *parseable* config — `config.conf` + a starter `binds.conf` +
//! `conf.d/colors.conf` — so a fresh session works instead of falling back to
//! bare built-in defaults. And an existing file must never be clobbered. This
//! chain was hand-verified only; these guard it. Pure filesystem (the
//! `Some(path)` form takes no `HOME`), so no wayland fixture is needed.

use std::fs;
use std::path::PathBuf;

/// A fresh, unique temp dir (process id + nanos) so parallel test runs don't
/// collide on the filesystem.
fn unique_tmp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "margo-bootstrap-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn writes_complete_valid_config_on_empty_dir() {
    let dir = unique_tmp("empty");
    let config = dir.join("config.conf");
    assert!(!config.exists());

    crate::ensure_default_config(Some(&config));

    assert!(config.exists(), "config.conf written");
    assert!(
        dir.join("binds.conf").exists(),
        "starter binds.conf written"
    );
    assert!(
        dir.join("conf.d/colors.conf").exists(),
        "colors.conf placeholder written so config.conf's `source` resolves"
    );
    assert!(
        !fs::read_to_string(&config).unwrap().is_empty(),
        "config.conf is non-empty"
    );

    // The strongest guarantee: the first-run config actually parses, with its
    // sourced binds/colors fragments resolving — a broken default would ship a
    // broken first session.
    let parsed = margo_config::parse_config_with_defaults(Some(&config));
    assert!(
        parsed.is_ok(),
        "first-run config must parse cleanly: {:?}",
        parsed.err()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn never_clobbers_an_existing_config() {
    let dir = unique_tmp("existing");
    let config = dir.join("config.conf");
    let sentinel = "# the user's own config\nmodkey = alt\n";
    fs::write(&config, sentinel).unwrap();

    crate::ensure_default_config(Some(&config));

    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        sentinel,
        "an existing config.conf is left exactly as the user had it"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn writes_missing_config_but_preserves_existing_binds() {
    // config.conf absent, binds.conf already present (Settings → Keybinds /
    // the user manages it): the config is seeded, the binds are NOT touched.
    let dir = unique_tmp("binds");
    let config = dir.join("config.conf");
    let binds = dir.join("binds.conf");
    let user_binds = "# my own keybinds\n";
    fs::write(&binds, user_binds).unwrap();

    crate::ensure_default_config(Some(&config));

    assert!(config.exists(), "missing config.conf written");
    assert_eq!(
        fs::read_to_string(&binds).unwrap(),
        user_binds,
        "an existing binds.conf is never clobbered"
    );
    let _ = fs::remove_dir_all(&dir);
}
