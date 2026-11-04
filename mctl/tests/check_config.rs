//! `mctl check-config` end to end: it reports exactly what the validator
//! finds, across `source`d files, with one code per meaning.

use std::path::Path;
use std::process::{Command, Output};

fn check(main: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mctl"))
        .arg("check-config")
        .arg("--config")
        .arg(main)
        .args(extra)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn duplicate_bind_is_reported_once_and_first_wins() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("config.conf");
    std::fs::write(&main, "bind = super,q,killclient\nbind = super,q,quit\n").unwrap();
    let out = check(&main, &[]);
    let err = stderr(&out);
    assert_eq!(err.matches("warning[").count(), 1, "{err}");
    assert!(err.contains("warning[W004]"), "{err}");
    assert!(err.contains("first match wins"), "{err}");
    assert!(!err.contains("this definition wins"), "{err}");
    assert!(out.status.success());
}

#[test]
fn same_key_in_two_keymodes_is_clean() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("config.conf");
    std::fs::write(
        &main,
        "keymode = resize\nbind = none,h,resizewin,-10,0\nkeymode = default\nbind = none,h,focusdir,left\n",
    )
    .unwrap();
    let out = check(&main, &[]);
    assert!(!stderr(&out).contains("warning["), "{}", stderr(&out));
}

#[test]
fn regex_errors_in_sourced_files_are_found() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("rules.conf"),
        "windowrule = isfloating:1, appid:^($[\n",
    )
    .unwrap();
    let main = dir.path().join("config.conf");
    std::fs::write(&main, "source = rules.conf\n").unwrap();
    let out = check(&main, &[]);
    let err = stderr(&out);
    assert!(err.contains("warning[W011]"), "{err}");
    assert!(err.contains("rules.conf:1"), "{err}");
    assert!(
        !err.contains("[E004]"),
        "regex errors no longer reuse E004: {err}"
    );
}

#[test]
fn strict_fails_on_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("config.conf");
    std::fs::write(&main, "bind = super,q,killclient\nbind = super,q,quit\n").unwrap();
    assert_eq!(check(&main, &[]).status.code(), Some(0));
    assert_eq!(check(&main, &["--strict"]).status.code(), Some(1));
    let clean = dir.path().join("clean.conf");
    std::fs::write(&clean, "borderpx = 2\n").unwrap();
    assert_eq!(check(&clean, &["--strict"]).status.code(), Some(0));
}
