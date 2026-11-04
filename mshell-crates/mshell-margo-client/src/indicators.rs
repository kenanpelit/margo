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

/// Where a scroller column sits relative to the viewport. A column counts
/// as on-screen while its centre is inside the viewport, so one peeking in
/// by a few pixels at an edge still reads as hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnPlace {
    Left,
    Visible,
    Right,
}

/// Place a column `[x, x + width)` against a viewport `[0, viewport_width)`.
pub fn column_place(x: i32, width: i32, viewport_width: i32) -> ColumnPlace {
    let centre = i64::from(x) + i64::from(width.max(0)) / 2;
    if centre < 0 {
        ColumnPlace::Left
    } else if centre >= i64::from(viewport_width.max(0)) {
        ColumnPlace::Right
    } else {
        ColumnPlace::Visible
    }
}

/// One ribbon segment (a scroller column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RibbonSegment {
    pub id: u64,
    pub app_id: String,
    pub place: ColumnPlace,
    pub focused: bool,
}

/// What the scroller ribbon pill renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ribbon {
    pub hidden_left: usize,
    pub hidden_right: usize,
    pub segments: Vec<RibbonSegment>,
}

/// Build the ribbon for a scroller strip, or `None` to hide the pill:
/// outside the scroller layout, and with fewer than two columns (a lone
/// column has nothing to orient by).
pub fn ribbon(view: Option<&crate::state_json::RawScroller>) -> Option<Ribbon> {
    let view = view?;
    if view.columns.len() < 2 {
        return None;
    }
    let segments: Vec<RibbonSegment> = view
        .columns
        .iter()
        .map(|c| RibbonSegment {
            id: c.id,
            app_id: c.app_id.clone(),
            place: column_place(c.x, c.width, view.viewport_width),
            focused: c.focused,
        })
        .collect();
    let count = |p: ColumnPlace| segments.iter().filter(|s| s.place == p).count();
    Some(Ribbon {
        hidden_left: count(ColumnPlace::Left),
        hidden_right: count(ColumnPlace::Right),
        segments,
    })
}

/// One scratchpad chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchpadChip {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    /// On screen now (vs tucked away).
    pub shown: bool,
}

/// The scratchpad chips, in a stable order: every scratchpad window (named
/// or anonymous, shown or hidden), sorted by app id then window id, so a
/// chip never jumps when its scratchpad is toggled.
pub fn scratchpad_chips(clients: &[crate::state_json::RawClient]) -> Vec<ScratchpadChip> {
    let mut chips: Vec<ScratchpadChip> = clients
        .iter()
        .filter(|c| c.scratchpad)
        .map(|c| ScratchpadChip {
            id: c.id,
            app_id: c.app_id.clone(),
            title: c.title.clone(),
            shown: c.scratchpad_shown,
        })
        .collect();
    chips.sort_by(|a, b| {
        a.app_id
            .to_lowercase()
            .cmp(&b.app_id.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    chips
}

/// Most characters a chip label shows before it is cut with "…".
pub const CHIP_LABEL_MAX: usize = 12;

/// A short chip label for an app id: the last reverse-DNS segment
/// (`org.wezfurlong.wezterm` → `wezterm`), cut to [`CHIP_LABEL_MAX`]
/// characters. An empty app id falls back to the window title.
pub fn chip_label(app_id: &str, title: &str) -> String {
    let base = app_id
        .rsplit('.')
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
        .trim();
    let base = if base.is_empty() { title.trim() } else { base };
    let base = if base.is_empty() { "scratchpad" } else { base };
    if base.chars().count() > CHIP_LABEL_MAX {
        let cut: String = base.chars().take(CHIP_LABEL_MAX - 1).collect();
        format!("{cut}…")
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_json::{RawScroller, RawScrollerColumn};

    fn col(id: u64, x: i32, width: i32, focused: bool) -> RawScrollerColumn {
        RawScrollerColumn {
            id,
            app_id: format!("app{id}"),
            x,
            width,
            focused,
        }
    }

    #[test]
    fn column_place_uses_the_column_centre() {
        assert_eq!(column_place(-1500, 1400, 1920), ColumnPlace::Left);
        assert_eq!(
            column_place(-400, 1000, 1920),
            ColumnPlace::Visible,
            "mostly in"
        );
        assert_eq!(
            column_place(-900, 1000, 1920),
            ColumnPlace::Left,
            "mostly out"
        );
        assert_eq!(column_place(0, 1920, 1920), ColumnPlace::Visible);
        assert_eq!(
            column_place(1800, 1000, 1920),
            ColumnPlace::Right,
            "peeking in"
        );
        assert_eq!(column_place(1920, 10, 1920), ColumnPlace::Right);
        assert_eq!(
            column_place(10, 10, 0),
            ColumnPlace::Right,
            "degenerate viewport"
        );
    }

    #[test]
    fn ribbon_counts_hidden_columns_per_side() {
        // A 1080p strip scrolled to its third column: one off the left,
        // one in view (focused), two off the right.
        let view = RawScroller {
            viewport_width: 1920,
            columns: vec![
                col(1, -1500, 1500, false),
                col(2, 210, 1500, true),
                col(3, 1730, 1500, false),
                col(4, 3250, 1500, false),
            ],
        };
        let r = ribbon(Some(&view)).expect("shown");
        assert_eq!((r.hidden_left, r.hidden_right), (1, 2));
        let places: Vec<_> = r.segments.iter().map(|s| s.place).collect();
        assert_eq!(
            places,
            [
                ColumnPlace::Left,
                ColumnPlace::Visible,
                ColumnPlace::Right,
                ColumnPlace::Right
            ]
        );
        assert_eq!(r.segments.iter().filter(|s| s.focused).count(), 1);
    }

    #[test]
    fn ribbon_hides_outside_the_scroller_and_for_a_lone_column() {
        assert_eq!(ribbon(None), None);
        let lone = RawScroller {
            viewport_width: 1920,
            columns: vec![col(1, 0, 1920, true)],
        };
        assert_eq!(ribbon(Some(&lone)), None);
        let empty = RawScroller::default();
        assert_eq!(ribbon(Some(&empty)), None);
    }

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

    fn raw(id: u64, app: &str, scratchpad: bool, shown: bool) -> crate::state_json::RawClient {
        crate::state_json::RawClient {
            id,
            app_id: app.into(),
            title: format!("{app} title"),
            scratchpad,
            scratchpad_shown: shown,
            ..Default::default()
        }
    }

    #[test]
    fn scratchpad_chips_list_only_scratchpads_in_a_stable_order() {
        let clients = [
            raw(5, "kitty", false, false),
            raw(9, "Wiremix", true, false),
            raw(3, "dropterm", true, true),
            raw(4, "dropterm", true, false),
        ];
        let chips = scratchpad_chips(&clients);
        let ids: Vec<u64> = chips.iter().map(|c| c.id).collect();
        assert_eq!(ids, [3, 4, 9], "by app id (case-insensitive), then id");
        assert!(chips[0].shown && !chips[1].shown);
        assert!(scratchpad_chips(&[raw(1, "a", false, false)]).is_empty());
    }

    #[test]
    fn chip_labels_are_short_and_never_empty() {
        assert_eq!(chip_label("org.wezfurlong.wezterm", ""), "wezterm");
        assert_eq!(chip_label("dropterm", ""), "dropterm");
        assert_eq!(chip_label("", "Notes"), "Notes");
        assert_eq!(chip_label("", ""), "scratchpad");
        assert_eq!(chip_label("trailing.", ""), "trailing");
        assert_eq!(chip_label("averyveryverylongappname", ""), "averyveryve…");
        assert_eq!(
            chip_label("averyveryverylongappname", "").chars().count(),
            CHIP_LABEL_MAX
        );
    }
}
