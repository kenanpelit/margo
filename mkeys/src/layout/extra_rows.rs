//! Optional function row (Esc, F1-F12) and numeric keypad, toggled from
//! Settings independently of the base `en`/`tr` layout — the same rows
//! regardless of language, since Esc/F-keys/digits aren't translated.
//!
//! Must run AFTER `live_legends::apply()`: these rows' single-character
//! digit legends ("7", "8", …) are static labels, not keysyms to derive
//! from the active xkb layout, and `live_legends` can't tell the
//! difference — it would try to look up a "shifted" level for a keypad
//! scan code, which is meaningless. Appending after live_legends has
//! already run means it never sees them.

use super::parse::{KeyDefinition, LayoutDefinition};
use crate::config::Config;

pub fn apply(layout: &mut LayoutDefinition, config: &Config) {
    if config.function_row {
        layout.layout.insert(0, function_row());
    }
    if config.numpad {
        layout.layout.extend(numpad_rows());
    }
    if config.function_row || config.numpad {
        layout.recompute_geometry();
    }
}

fn key(scan_code: u16, label: &str, width: Option<f32>) -> KeyDefinition {
    KeyDefinition {
        top_legend: Some(label.to_string()),
        bottom_legend: None,
        scan_code,
        width,
    }
}

fn function_row() -> Vec<KeyDefinition> {
    vec![
        key(1, "Esc", None),
        key(59, "F1", None),
        key(60, "F2", None),
        key(61, "F3", None),
        key(62, "F4", None),
        key(63, "F5", None),
        key(64, "F6", None),
        key(65, "F7", None),
        key(66, "F8", None),
        key(67, "F9", None),
        key(68, "F10", None),
        key(87, "F11", None),
        key(88, "F12", None),
    ]
}

/// A plain 4x4 grid rather than a real numpad's plus/enter-spans-two-rows
/// layout — this is a tap keyboard, not a physical calculator; every key
/// stays reachable at one tap either way.
fn numpad_rows() -> Vec<Vec<KeyDefinition>> {
    vec![
        vec![
            key(71, "7", None),
            key(72, "8", None),
            key(73, "9", None),
            key(74, "-", None),
        ],
        vec![
            key(75, "4", None),
            key(76, "5", None),
            key(77, "6", None),
            key(78, "+", None),
        ],
        vec![
            key(79, "1", None),
            key(80, "2", None),
            key(81, "3", None),
            key(96, "Enter", None),
        ],
        vec![
            key(82, "0", Some(2.0)),
            key(83, ".", None),
            key(69, "Num", None),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_layout() -> LayoutDefinition {
        LayoutDefinition::from_toml(r#"layout = [[ { top_legend = "q", scan_code = 16 } ]]"#)
            .unwrap()
    }

    #[test]
    fn neither_toggle_leaves_layout_untouched() {
        let mut layout = base_layout();
        let config = Config {
            function_row: false,
            numpad: false,
            ..Config::default()
        };
        apply(&mut layout, &config);
        assert_eq!(layout.layout.len(), 1);
        assert_eq!(layout.height, 1);
    }

    #[test]
    fn function_row_is_prepended() {
        let mut layout = base_layout();
        let config = Config {
            function_row: true,
            numpad: false,
            ..Config::default()
        };
        apply(&mut layout, &config);
        assert_eq!(layout.layout.len(), 2);
        assert_eq!(layout.layout[0][0].top_legend.as_deref(), Some("Esc"));
        assert_eq!(layout.layout[0].len(), 13);
        assert_eq!(layout.height, 2);
    }

    #[test]
    fn numpad_is_appended_and_widens_the_layout() {
        let mut layout = base_layout();
        let config = Config {
            function_row: false,
            numpad: true,
            ..Config::default()
        };
        apply(&mut layout, &config);
        assert_eq!(layout.layout.len(), 5);
        assert_eq!(layout.layout[4][0].top_legend.as_deref(), Some("0"));
        assert_eq!(layout.height, 5);
    }

    #[test]
    fn both_toggles_compose() {
        let mut layout = base_layout();
        let config = Config {
            function_row: true,
            numpad: true,
            ..Config::default()
        };
        apply(&mut layout, &config);
        assert_eq!(layout.layout.len(), 6);
        assert_eq!(layout.layout[0][0].top_legend.as_deref(), Some("Esc"));
        assert_eq!(layout.layout[5][0].top_legend.as_deref(), Some("0"));
    }
}
