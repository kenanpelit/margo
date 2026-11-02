//! Pointer/input dwell and throttle timers, grouped out of `MargoState`:
//! hot-corner dwell, cursor auto-hide, `axisbind` throttling, and the
//! move/resize grab re-arrange throttle.

use super::HotCorner;

pub struct InputTimers {
    /// Which hot corner the pointer is currently dwelling in (if any).
    /// `None` while pointer is anywhere else; set on entry, cleared on
    /// exit. Together with [`hot_corner_armed_at`] drives the dwell
    /// threshold before the corner's action fires.
    pub hot_corner_dwelling: Option<HotCorner>,
    /// `Instant` the pointer entered the current dwell corner; `None`
    /// once the corner has fired (until the pointer leaves and re-enters).
    pub hot_corner_armed_at: Option<std::time::Instant>,
    /// One-shot timer that fires the dwell for a cursor parked in a
    /// corner (no further motion needed). Cleared by its own callback
    /// before the source drops.
    pub hot_corner_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    /// When a move/resize grab last re-arranged (see
    /// `input::grabs::throttled_grab_arrange`), and the timer that
    /// applies a throttled update once the interval is over.
    pub grab_arrange_last: Option<std::time::Instant>,
    pub grab_arrange_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    /// `cursor_hide_timeout`: the pointer sprite is hidden after that many
    /// seconds without pointer activity. `pointer_activity_at` is the last
    /// motion / button / scroll; the timer re-arms itself until the
    /// timeout has really elapsed since then.
    pub cursor_hidden: bool,
    pub pointer_activity_at: std::time::Instant,
    pub cursor_hide_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    /// Last `axisbind` scroll (ms) and its direction sign, for
    /// `axis_bind_apply_timeout` throttling.
    pub axis_bind_time: u32,
    pub axis_bind_dir: i8,
}

impl InputTimers {
    pub fn new() -> Self {
        Self {
            hot_corner_dwelling: None,
            hot_corner_armed_at: None,
            hot_corner_timer: None,
            grab_arrange_last: None,
            grab_arrange_timer: None,
            cursor_hidden: false,
            pointer_activity_at: std::time::Instant::now(),
            cursor_hide_timer: None,
            axis_bind_time: 0,
            axis_bind_dir: 0,
        }
    }
}

impl Default for InputTimers {
    fn default() -> Self {
        Self::new()
    }
}
