//! Overrides the bundled per-language TOML legends with the keysyms
//! margo's ACTUAL active xkb layout produces, so the on-screen labels
//! match what pressing the key really types instead of a static
//! `en`/`tr` guess (the scan codes mkeys sends are layout-independent —
//! only the printed legend was ever wrong).
//!
//! Only single-character legends are touched — "Tab", "Backspace",
//! "Space", "Caps Lock", … stay exactly as authored, since those name
//! the key rather than quote a keysym.

use xkbcommon::xkb;

use super::parse::{KeyDefinition, LayoutDefinition};
use crate::xkb_config;

/// Takes the caller's already-compiled keymap (shared with the keymap
/// the virtual keyboard submits — see `service::host::run` — instead of
/// each compiling its own copy). No-op if `keymap` is `None`: the
/// bundled TOML legends are always a safe fallback.
pub fn apply(layout: &mut LayoutDefinition, keymap: Option<&xkb::Keymap>) {
    let Some(keymap) = keymap else {
        return;
    };
    let group = xkb_config::active_layout_group(keymap);

    for row in &mut layout.layout {
        for key in row {
            apply_key(keymap, group, key);
        }
    }
}

fn apply_key(keymap: &xkb::Keymap, group: u32, key: &mut KeyDefinition) {
    let is_single_char = |s: &Option<String>| s.as_deref().is_some_and(|s| s.chars().count() == 1);
    // Only override keys whose bundled legend is a single character —
    // exactly the letters/digits/punctuation row, never a named key.
    if !is_single_char(&key.top_legend) {
        return;
    }

    let keycode = xkb::Keycode::new(key.scan_code as u32 + 8);
    let base = level_char(keymap, keycode, group, 0);
    let shifted = level_char(keymap, keycode, group, 1);

    if is_single_char(&key.bottom_legend) {
        // Symbol-row key: physical keycaps print the shifted glyph on
        // top, the base glyph below (e.g. "!" over "1").
        if let (Some(top), Some(bottom)) = (shifted, base) {
            key.top_legend = Some(top);
            key.bottom_legend = Some(bottom);
        }
    } else if let Some(top) = shifted.or(base) {
        // Letter key: keycaps are always printed uppercase regardless
        // of the actual case Shift/Caps Lock will produce.
        key.top_legend = Some(top);
    }
}

/// The printable character xkb produces for `keycode` at `(group,
/// level)`, or `None` for a dead/non-printable/unmapped level.
fn level_char(
    keymap: &xkb::Keymap,
    keycode: xkb::Keycode,
    group: u32,
    level: u32,
) -> Option<String> {
    let syms = keymap.key_get_syms_by_level(keycode, group, level);
    let sym = *syms.first()?;
    let text = xkb::keysym_to_utf8(sym);
    (!text.is_empty() && text.chars().all(|c| !c.is_control())).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us_keymap() -> xkb::Keymap {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(
            &ctx,
            "evdev",
            "pc105",
            "us",
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("us keymap compiles")
    }

    fn apply_row(keymap: &xkb::Keymap, group: u32, def: &mut LayoutDefinition) {
        for row in &mut def.layout {
            for key in row {
                apply_key(keymap, group, key);
            }
        }
    }

    #[test]
    fn letter_key_becomes_uppercase_from_the_live_keymap() {
        let keymap = us_keymap();
        let mut def =
            LayoutDefinition::from_toml(r#"layout = [[ { top_legend = "q", scan_code = 16 } ]]"#)
                .unwrap();
        apply_row(&keymap, 0, &mut def);
        assert_eq!(def.layout[0][0].top_legend.as_deref(), Some("Q"));
    }

    #[test]
    fn symbol_key_top_is_shifted_bottom_is_base() {
        let keymap = us_keymap();
        // evdev KEY_1 — "!" over "1" on a US keyboard.
        let mut def = LayoutDefinition::from_toml(
            r#"layout = [[ { top_legend = "?", bottom_legend = "?", scan_code = 2 } ]]"#,
        )
        .unwrap();
        apply_row(&keymap, 0, &mut def);
        assert_eq!(def.layout[0][0].top_legend.as_deref(), Some("!"));
        assert_eq!(def.layout[0][0].bottom_legend.as_deref(), Some("1"));
    }

    #[test]
    fn named_keys_are_left_alone() {
        let keymap = us_keymap();
        let mut def = LayoutDefinition::from_toml(
            r#"layout = [[ { top_legend = "Backspace", scan_code = 14, width = 2 } ]]"#,
        )
        .unwrap();
        apply_row(&keymap, 0, &mut def);
        assert_eq!(def.layout[0][0].top_legend.as_deref(), Some("Backspace"));
    }
}
