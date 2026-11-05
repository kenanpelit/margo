//! Dispatch-action methods on `MargoState`. Extracted from `state.rs`
//! (roadmap Q1). These are the entry points every keybind and IPC
//! command lands in: focus stack navigation, tag mask flips, layout
//! switches, sticky/floating/fullscreen toggles, gap and master
//! tuning, monitor warps. "User typed Super+J, make it happen"
//! plumbing — pure compositor commands, none of which belong in the
//! central state.rs translation unit.
//!
//! Everything in here is an inherent method on `MargoState`; the lift
//! is signature-preserving so call sites (binds, mctl, scripting) are
//! unchanged. They share the same "mutate state then arrange +
//! broadcast" idiom. This file keeps the focus-stack and kill actions;
//! the rest live by subject in `dispatch/`: `tags` (tag views, carousel,
//! edge scroller), `layout` (layouts, master, gaps), `window` (sticky,
//! floating, fullscreen, move/resize), `monitor` (monitor focus and
//! moves) and `initial_map` (a new toplevel's deferred first map).

use smithay::desktop::WindowSurface;

use super::{FocusTarget, MargoState};

mod initial_map;
mod layout;
mod monitor;
mod tags;
mod window;

/// Floor for a floating window's width/height (px). `resize_focused`
/// already clamped its own deltas here; `reseed_float_geom_if_stale`
/// reuses it as "small enough that this remembered geometry can't be
/// trusted" — see its doc comment.
pub(super) const MIN_FLOAT_SIZE: i32 = 50;
/// Bound for floating positions and sizes moved/resized by `movewin` /
/// `resizewin` (same ±2^20 as output positions): IPC and Rhai deltas are
/// arbitrary i32s, and layout/render math adds `x + width` in i32.
const MAX_FLOAT_COORD: i32 = crate::protocols::output_management::MAX_POSITION;
/// Upper bound for `incgaps`; far beyond any output, small enough that gap
/// sums in the layouts can't overflow.
const MAX_GAP: i32 = 10_000;
/// Upper bound for `incnmaster`; a master count above the window count
/// already means "all masters".
const MAX_NMASTER: u32 = 1_000;

impl MargoState {
    // ── Actions ───────────────────────────────────────────────────────────────

    pub fn kill_focused(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            if let WindowSurface::Wayland(toplevel) = self.clients[idx].window.underlying_surface()
            {
                toplevel.send_close();
            }
        }
    }

    /// mango's `smartkillclient` (`ac1420a0`): if the focused window is on
    /// more than one tag, remove it from just the currently-viewed tag(s)
    /// instead of closing it outright — it's still open on its other
    /// tags. Falls back to [`kill_focused`](Self::kill_focused) when it's
    /// on a single tag, or when removing the viewed tag(s) would leave it
    /// on none.
    pub fn smart_kill_client(&mut self) {
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let mon_idx = self.clients[idx].monitor;
        let client_tags = self.clients[idx].tags;
        if client_tags.count_ones() <= 1 || mon_idx >= self.monitors.len() {
            self.kill_focused();
            return;
        }

        let view_tags = self.monitors[mon_idx].current_tagset();
        let remaining_tags = client_tags & !view_tags;
        if remaining_tags == 0 {
            self.kill_focused();
            return;
        }

        self.clients[idx].tags = remaining_tags;
        self.focus_most_recent_visible_or_clear(mon_idx);
        self.arrange_monitor(mon_idx);
    }

    pub fn focus_stack(&mut self, direction: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();

        let visible: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset))
            .map(|(i, _)| i)
            .collect();

        if visible.is_empty() {
            return;
        }

        let len = visible.len();
        let current_pos = self
            .focused_client_idx()
            .and_then(|ci| visible.iter().position(|&vi| vi == ci))
            .unwrap_or(0);

        let new_pos = if direction > 0 {
            (current_pos + 1) % len
        } else {
            (current_pos + len - 1) % len
        };

        let new_idx = visible[new_pos];
        self.monitors[mon_idx].prev_selected = self.monitors[mon_idx].selected;
        self.monitors[mon_idx].selected = Some(new_idx);
        let window = self.clients[new_idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window)));
        self.arrange_monitor(mon_idx);
    }

    /// Toggle focus back to the previously-focused window on this
    /// monitor (dwl's `focuslast`). `prev_selected` is kept current by
    /// every focus change, and `focus_surface` swaps `selected` /
    /// `prev_selected` as it moves focus — so pressing the bind again
    /// returns you to where you were. No-op when there's no prior
    /// window, or it has since closed or moved off the current tagset.
    pub fn focus_last(&mut self) {
        let mon_idx = self.focused_monitor();
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        let tagset = mon.current_tagset();
        let Some(prev) = mon.prev_selected else {
            return;
        };
        if prev >= self.clients.len()
            || Some(prev) == mon.selected
            || !self.clients[prev].is_visible_on(mon_idx, tagset)
        {
            return;
        }
        let window = self.clients[prev].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window)));
        self.arrange_monitor(mon_idx);
    }

    pub fn exchange_stack(&mut self, direction: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();

        let visible: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset))
            .map(|(i, _)| i)
            .collect();

        if visible.len() < 2 {
            return;
        }

        let Some(current_idx) = self.focused_client_idx() else {
            return;
        };
        let Some(current_pos) = visible.iter().position(|&idx| idx == current_idx) else {
            return;
        };

        let len = visible.len();
        let target_pos = if direction > 0 {
            (current_pos + 1) % len
        } else {
            (current_pos + len - 1) % len
        };
        let target_idx = visible[target_pos];
        let window = self.clients[current_idx].window.clone();
        self.swap_clients(current_idx, target_idx);
        self.arrange_monitor(mon_idx);
        self.focus_surface(Some(FocusTarget::Window(window)));
    }
}

/// Edge-scroller pointer-focus decision. The enabled/allow guard and the
/// monitor lookup stay in the caller; this is the pure edge-detection +
/// speed-gated debounce. `armed` in → `armed` out; fires `focus` once per
/// edge dwell (only while armed and moving slower than `allow`).
/// Deliver dispatch feedback as an mshell OSD toast — ephemeral, no
/// notification-history pollution. Falls back to `notify-send` when
/// mshell isn't up (standalone margo + external bar setups). The
/// `$1`-style positional args keep arbitrary bodies quoting-safe.
fn osd_toast(title: &str, icon: &str, body: &str) {
    let _ = crate::utils::spawn([
        "sh",
        "-c",
        "mshellctl toast \"$1\" \"$2\" --icon \"$3\" 2>/dev/null \
         || notify-send -a margo -i \"$3\" -t 1200 \"$1\" \"$2\"",
        "sh",
        title,
        body,
        icon,
    ]);
}

fn layout_toast(body: &str) {
    osd_toast("Margo Layout", "view-grid-symbolic", body);
}
