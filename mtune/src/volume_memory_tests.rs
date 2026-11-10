// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn sanitize_clamps_and_rejects_non_finite() {
    assert_eq!(sanitize(-0.3), 0.0);
    assert_eq!(sanitize(1.7), 1.0);
    assert_eq!(sanitize(0.42), 0.42);
    assert_eq!(sanitize(f64::NAN), 1.0);
    assert_eq!(sanitize(f64::INFINITY), 1.0);
    assert_eq!(sanitize(f64::NEG_INFINITY), 1.0);
}

#[test]
fn burst_writes_only_the_settled_value() {
    let mut d = Debounce::new(1.0);
    let g1 = d.note(0.9);
    let g2 = d.note(0.7);
    let g3 = d.note(0.55);
    // Earlier timers are stale and write nothing.
    assert_eq!(d.fire(g1), None);
    assert_eq!(d.fire(g2), None);
    assert_eq!(d.fire(g3), Some(0.55));
    // Nothing left to write afterwards.
    assert_eq!(d.fire(g3), None);
    assert_eq!(d.flush(), None);
}

#[test]
fn unchanged_value_is_not_rewritten() {
    let mut d = Debounce::new(0.5);
    let g = d.note(0.502);
    assert_eq!(d.fire(g), None);
    let g = d.note(0.8);
    let g2 = d.note(0.5);
    assert_eq!(d.fire(g), None);
    assert_eq!(d.fire(g2), None, "settled back on the stored level");
}

#[test]
fn flush_writes_pending_regardless_of_timer() {
    let mut d = Debounce::new(1.0);
    let _ = d.note(0.25);
    assert_eq!(d.flush(), Some(0.25));
    assert_eq!(d.flush(), None);
}

#[test]
fn out_of_range_requests_are_clamped_before_write() {
    let mut d = Debounce::new(0.5);
    let g = d.note(3.0);
    assert_eq!(d.fire(g), Some(1.0));
    let g = d.note(-1.0);
    assert_eq!(d.fire(g), Some(0.0));
    let g = d.note(f64::NAN);
    assert_eq!(d.fire(g), Some(1.0));
}

#[test]
fn mute_is_remembered() {
    let mut d = Debounce::new(0.6);
    let g = d.note(0.0);
    assert_eq!(d.fire(g), Some(0.0));
}

/// Compile the shipped schema into a temp dir and open it on a memory
/// backend — never touches the user's dconf.
fn memory_settings() -> Option<(tempfile::TempDir, gio::Settings)> {
    if !margo_fs::path::on_path("glib-compile-schemas") {
        eprintln!("glib-compile-schemas not installed; skipping gsettings test");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/org.margo.Tune.gschema.xml"
        ),
        dir.path().join("org.margo.Tune.gschema.xml"),
    )
    .unwrap();
    let status = std::process::Command::new("glib-compile-schemas")
        .arg("--strict")
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "schema must compile with --strict");
    let source = gio::SettingsSchemaSource::from_directory(
        dir.path(),
        None::<&gio::SettingsSchemaSource>,
        false,
    )
    .unwrap();
    let schema = source.lookup("org.margo.Tune", false).unwrap();
    let backend = gio::functions::memory_settings_backend_new();
    let settings = gio::Settings::new_full(&schema, Some(&backend), None);
    Some((dir, settings))
}

#[test]
fn schema_key_defaults_to_full_and_enforces_range() {
    let Some((_dir, settings)) = memory_settings() else {
        return;
    };
    assert_eq!(load(&settings), 1.0);
    assert!(settings.set_double(KEY, 0.35).is_ok());
    assert_eq!(load(&settings), 0.35);
    // The schema's <range> refuses out-of-range values.
    let key = settings.settings_schema().unwrap().key(KEY);
    assert!(key.range_check(&0.0.to_variant()));
    assert!(key.range_check(&1.0.to_variant()));
    assert!(!key.range_check(&1.5.to_variant()));
    assert!(!key.range_check(&(-0.1).to_variant()));
}

#[test]
fn memory_debounces_writes_and_flushes() {
    let Some((_dir, settings)) = memory_settings() else {
        return;
    };
    let ctx = glib::MainContext::new();
    ctx.with_thread_default(|| {
        let mem = VolumeMemory::new(settings.clone());
        mem.note(0.9);
        mem.note(0.4);
        // Not written yet: the debounce window is still open.
        assert_eq!(load(&settings), 1.0);
        let deadline = std::time::Instant::now() + DEBOUNCE * 4;
        while load(&settings) != 0.4 && std::time::Instant::now() < deadline {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            load(&settings),
            0.4,
            "settled value written after the debounce"
        );

        // A pending change is written by flush() without waiting.
        mem.note(0.2);
        mem.flush();
        assert_eq!(load(&settings), 0.2);

        // A fresh memory restores the remembered level.
        assert_eq!(
            load(&settings),
            VolumeMemory::new(settings.clone()).state.borrow().saved
        );
    })
    .unwrap();
}
