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

/// What a built-in keyboard change should announce: the new `disabled`
/// value, or `None` when nothing changed or `prev` is the first
/// observation (a baseline, so the shell starting with it off doesn't
/// toast). `cur = None` (an older margo) never announces.
pub fn internal_keyboard_change(
    prev: Option<bool>,
    cur: Option<&crate::state_json::RawInternalKeyboard>,
) -> Option<bool> {
    let (prev, cur) = (prev?, cur?.disabled);
    (prev != cur).then_some(cur)
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

/// One minimized window, for the Scratchpads pill's restore chips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinimizedChip {
    pub id: u64,
    pub app_id: String,
    pub title: String,
}

/// The minimized windows (not hidden scratchpads, which have their own
/// chips), most recently minimized first — the order `restore_last` walks.
/// A margo without `minimized_order` reports 0 for all; they then keep
/// window-id order.
pub fn minimized_chips(clients: &[crate::state_json::RawClient]) -> Vec<MinimizedChip> {
    let mut windows: Vec<&crate::state_json::RawClient> = clients
        .iter()
        .filter(|c| c.minimized && !c.scratchpad)
        .collect();
    windows.sort_by(|a, b| {
        b.minimized_order
            .cmp(&a.minimized_order)
            .then(a.id.cmp(&b.id))
    });
    windows
        .into_iter()
        .map(|c| MinimizedChip {
            id: c.id,
            app_id: c.app_id.clone(),
            title: c.title.clone(),
        })
        .collect()
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

/// One window of the active tag, for the tag row's app icons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagWindow {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
}

/// The windows on the focused output's active tag(s), in margo's client
/// order (the order windows were opened / the layout strip). Minimised
/// windows and tucked-away scratchpads are left out — clicking an icon
/// focuses a window you can see come up; a shown scratchpad is included.
pub fn active_tag_windows(state: &crate::state_json::StateJson) -> Vec<TagWindow> {
    let Some(output) = state.outputs.iter().find(|o| o.name == state.active_output) else {
        return Vec::new();
    };
    let mask = output.active_tag_mask;
    let focused_idx = state.focused_idx.and_then(|i| usize::try_from(i).ok());
    state
        .clients
        .iter()
        .enumerate()
        .filter(|(_, c)| c.monitor == output.name && c.tags & mask != 0)
        .filter(|(_, c)| !c.minimized && (!c.scratchpad || c.scratchpad_shown))
        .map(|(i, c)| TagWindow {
            id: c.id,
            app_id: c.app_id.clone(),
            title: c.title.clone(),
            focused: Some(i) == focused_idx,
        })
        .collect()
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
    fn internal_keyboard_change_baselines_then_reports_flips() {
        let state = |disabled| crate::state_json::RawInternalKeyboard {
            disabled,
            ..Default::default()
        };
        assert_eq!(internal_keyboard_change(None, Some(&state(true))), None);
        assert_eq!(
            internal_keyboard_change(Some(false), Some(&state(true))),
            Some(true)
        );
        assert_eq!(
            internal_keyboard_change(Some(true), Some(&state(false))),
            Some(false)
        );
        assert_eq!(
            internal_keyboard_change(Some(true), Some(&state(true))),
            None
        );
        assert_eq!(internal_keyboard_change(Some(true), None), None);
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
    fn minimized_chips_are_newest_first_and_skip_scratchpads() {
        let min = |id: u64, order: u64| crate::state_json::RawClient {
            minimized: true,
            minimized_order: order,
            ..raw(id, "app", false, false)
        };
        let mut hidden_pad = raw(7, "pad", true, false);
        hidden_pad.minimized = true;
        let clients = [
            min(1, 2),
            raw(2, "open", false, false),
            min(3, 5),
            hidden_pad,
            min(4, 1),
        ];
        let ids: Vec<u64> = minimized_chips(&clients).iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [3, 1, 4],
            "restore_last order; no scratchpad, no open window"
        );
        // An older margo without the order field: window-id order.
        let ids: Vec<u64> = minimized_chips(&[min(9, 0), min(2, 0)])
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, [2, 9]);
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

    #[test]
    fn active_tag_windows_follow_the_focused_output_and_its_tag() {
        use crate::state_json::{RawOutput, StateJson};
        let out = |name: &str, mask: u32| RawOutput {
            name: name.into(),
            active: false,
            active_tag_mask: mask,
            occupied_tag_mask: mask,
            focus_history: Vec::new(),
            focus_history_v2: Vec::new(),
            layout_idx: 0,
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            scale: 1.0,
            logical_width: None,
            logical_height: None,
            exclusive_fullscreen: false,
            mode: None,
            wallpaper: String::new(),
            wallpapers_by_tag: Vec::new(),
            scroller: None,
        };
        let win = |id: u64, mon: &str, tags: u32| crate::state_json::RawClient {
            id,
            app_id: format!("app{id}"),
            monitor: mon.into(),
            tags,
            ..Default::default()
        };
        let mut minimized = win(4, "DP-1", 0b1);
        minimized.minimized = true;
        let mut hidden_pad = win(5, "DP-1", 0b1);
        hidden_pad.scratchpad = true;
        let mut shown_pad = win(6, "DP-1", 0b1);
        shown_pad.scratchpad = true;
        shown_pad.scratchpad_shown = true;
        let state = StateJson {
            active_output: "DP-1".into(),
            focused_idx: Some(2),
            layouts: Vec::new(),
            circle_layouts: Vec::new(),
            outputs: vec![out("DP-1", 0b1), out("HDMI-A-1", 0b1)],
            clients: vec![
                win(1, "DP-1", 0b1),
                win(2, "DP-1", 0b10),
                win(3, "DP-1", 0b11),
                win(7, "HDMI-A-1", 0b1),
                minimized,
                hidden_pad,
                shown_pad,
            ],
            tag_count: 9,
            keyboard_layout: String::new(),
            session_locked: None,
            lock_keys: None,
            keymode: None,
            config_errors: Vec::new(),
            internal_keyboard: None,
        };
        let wins = active_tag_windows(&state);
        let ids: Vec<u64> = wins.iter().map(|w| w.id).collect();
        assert_eq!(ids, [1, 3, 6], "tag 1 of DP-1, visible windows only");
        assert_eq!(
            wins.iter()
                .filter(|w| w.focused)
                .map(|w| w.id)
                .collect::<Vec<_>>(),
            [3],
            "focused_idx 2 is window 3"
        );
        let elsewhere = StateJson {
            active_output: "nope".into(),
            ..state
        };
        assert!(active_tag_windows(&elsewhere).is_empty());
    }
}
