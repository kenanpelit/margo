//! Pure decisions behind the shell's compositor-state bar indicators
//! (keymode pill, scroller ribbon, scratchpad chips, XWayland badge, tag
//! window icons). Kept here, next to the state schema they read, so they
//! are unit-tested without GTK; the widgets only render the result.

/// The keymode margo starts in and returns to (`setkeymode default`).
pub const DEFAULT_KEYMODE: &str = "default";

/// The keymode the pill should show, or `None` to hide it: hidden in the
/// default mode, before the first snapshot, and on a margo that doesn't
/// report keymodes.
pub fn visible_keymode(mode: Option<&str>) -> Option<&str> {
    let mode = mode?.trim();
    (!mode.is_empty() && mode != DEFAULT_KEYMODE).then_some(mode)
}

/// What a keymode change should announce: the new mode's name, or `None`
/// when nothing changed or `prev` is the first observation (a baseline,
/// so the shell starting up inside a mode doesn't toast).
pub fn keymode_change<'a>(prev: Option<&str>, cur: &'a str) -> Option<&'a str> {
    let prev = prev?;
    let cur_t = cur.trim();
    (prev.trim() != cur_t && !cur_t.is_empty()).then_some(cur_t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymode_pill_hides_in_default_and_unknown_modes() {
        assert_eq!(visible_keymode(None), None, "older margo");
        assert_eq!(visible_keymode(Some("default")), None);
        assert_eq!(visible_keymode(Some("")), None);
        assert_eq!(visible_keymode(Some("  ")), None);
        assert_eq!(visible_keymode(Some("resize")), Some("resize"));
        assert_eq!(visible_keymode(Some(" move ")), Some("move"));
    }

    #[test]
    fn keymode_change_baselines_then_reports_real_changes() {
        assert_eq!(
            keymode_change(None, "resize"),
            None,
            "first value baselines"
        );
        assert_eq!(keymode_change(Some("default"), "default"), None);
        assert_eq!(keymode_change(Some("default"), "resize"), Some("resize"));
        assert_eq!(keymode_change(Some("resize"), "default"), Some("default"));
        assert_eq!(keymode_change(Some("resize"), ""), None);
    }
}
