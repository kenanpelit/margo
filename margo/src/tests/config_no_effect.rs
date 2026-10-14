//! Keeps `margo_config::validator::NO_EFFECT_KEYS` honest: the `Config`
//! fields behind those keys must stay unread by the compositor. When one
//! gets implemented this fails — drop it from both lists so `mctl
//! check-config` stops calling it a no-op.

/// `Config` fields set by the `NO_EFFECT_KEYS` keys.
const NO_EFFECT_FIELDS: &[&str] = &[
    "allow_lock_transparent",
    "allow_shortcuts_inhibit",
    "allow_tearing",
    "axis_bind_apply_timeout",
    "axis_bindings",
    "axis_scroll_factor",
    "blur_optimized",
    "button_map",
    "capslock",
    "enable_floating_snap",
    "enable_hotarea",
    "exchange_cross_monitor",
    "focus_cross_monitor",
    "focus_cross_tag",
    "focus_on_activate",
    "hotarea_corner",
    "hotarea_size",
    "new_is_master",
    "no_border_when_single",
    "no_radius_when_single",
    "numlock_on",
    "ov_tab_mode",
    "snap_distance",
    "switch_bindings",
    "syncobj_enable",
    "tablet_map_to_mon",
    "touch_degrees_leniency",
    "touch_distance_threshold",
    "touch_edge_size_bottom",
    "touch_edge_size_left",
    "touch_edge_size_right",
    "touch_edge_size_top",
    "touch_gesture_bindings",
    "touch_timeout_ms",
    "trackpad_scroll_factor",
    "wallpaper_fit",
    "warpcursor",
];

fn rust_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "tests") {
                rust_sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn reads_field(text: &str, field: &str) -> bool {
    let needle = format!(".{field}");
    text.match_indices(&needle).any(|(i, _)| {
        text[i + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

#[test]
fn no_effect_keys_stay_unread() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    let mut read = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for field in NO_EFFECT_FIELDS {
            if reads_field(&text, field) {
                read.push(format!("{field} ({})", file.display()));
            }
        }
    }
    assert!(
        read.is_empty(),
        "now read by the compositor — remove from NO_EFFECT_FIELDS and the \
         validator's NO_EFFECT_KEYS: {read:?}"
    );
}
