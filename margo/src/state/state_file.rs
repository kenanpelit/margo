//! State snapshot builder + the per-iteration IPC flush.
//!
//! `build_state_snapshot` produces the JSON document every IPC `get`/
//! `watch` reply is built from. `mark_state_dirty` / `flush_ipc_if_dirty`
//! coalesce a burst of changes into one pushed `watch` frame per
//! event-loop iteration, at most 60 a second, and a subscriber is never
//! sent the same frame twice in a row. There is no longer a state snapshot file — the
//! Unix socket is the only state egress.

use super::MargoState;
use smithay::reexports::wayland_server::protocol::wl_output::Transform as WlTransform;

use crate::MAX_TAGS;

/// Minimum spacing of `watch` pushes (60 Hz). The shell parses every
/// snapshot on its GTK main thread; a drag arranging every 8 ms used to
/// push ~125 full snapshots a second.
pub const IPC_PUSH_INTERVAL: std::time::Duration = std::time::Duration::from_micros(16_667);

/// How long a dirty flush at `now` must wait so pushes stay at least
/// `interval` apart: `None` = push now.
pub fn ipc_push_delay(
    last_push: Option<std::time::Instant>,
    now: std::time::Instant,
    interval: std::time::Duration,
) -> Option<std::time::Duration> {
    let since = now.saturating_duration_since(last_push?);
    interval.checked_sub(since).filter(|wait| !wait.is_zero())
}

impl MargoState {
    /// Mark compositor state as changed since the last flush. The
    /// actual fan-out (a fresh frame to every IPC `watch` subscriber)
    /// is **coalesced** in `flush_ipc_if_dirty`: once per event-loop
    /// iteration and at most every [`IPC_PUSH_INTERVAL`], so a burst of
    /// changes (one layout switch touches focus + windows + tags, each of
    /// which calls this; a drag arranges every few ms) pushes one
    /// snapshot instead of N.
    pub fn mark_state_dirty(&self) {
        self.state_dirty.set(true);
    }

    /// Once per event-loop iteration: if state changed, push a fresh
    /// snapshot frame to every IPC `watch` subscriber. This is the sole
    /// state-egress path — there is no state snapshot file anymore.
    ///
    /// Rate-capped: within [`IPC_PUSH_INTERVAL`] of the previous push the
    /// state stays dirty and a one-shot timer flushes it when the interval
    /// is up, so the last change always goes out even if nothing else
    /// wakes the loop. A change after a quiet spell is pushed at once.
    pub fn flush_ipc_if_dirty(&mut self) {
        if !self.state_dirty.get() {
            return;
        }
        if self.ipc.watches.watches.is_empty() {
            // Nobody listens: nothing to build or rate-limit.
            self.state_dirty.set(false);
            return;
        }
        let now = std::time::Instant::now();
        if let Some(wait) = ipc_push_delay(self.ipc.last_push, now, IPC_PUSH_INTERVAL) {
            if !self.ipc.push_timer_armed {
                let timer = calloop::timer::Timer::from_duration(wait);
                let armed = self.loop_handle.insert_source(timer, |_, _, state| {
                    state.ipc.push_timer_armed = false;
                    state.flush_ipc_if_dirty();
                    calloop::timer::TimeoutAction::Drop
                });
                match armed {
                    Ok(_) => self.ipc.push_timer_armed = true,
                    Err(e) => {
                        // Without the timer the frame could be held back
                        // indefinitely: push now rather than risk that.
                        tracing::warn!(error = %e, "ipc: push timer");
                        self.push_now(now);
                    }
                }
            }
            return;
        }
        self.push_now(now);
    }

    fn push_now(&mut self, now: std::time::Instant) {
        self.state_dirty.set(false);
        self.ipc.last_push = Some(now);
        self.ipc_push_watches();
    }

    /// The scroller strip of monitor `mon_idx` as the shell's ribbon reads
    /// it: one entry per tiled column in strip order, each column's *target*
    /// rect (an in-flight move animation reports where it is heading, not
    /// where it is mid-flight) with `x` relative to the work area, plus the
    /// viewport width. `null` outside the scroller layout and in overview.
    fn scroller_viewport(&self, mon_idx: usize, focused_idx: Option<usize>) -> serde_json::Value {
        use serde_json::json;
        let Some(mon) = self.monitors.get(mon_idx) else {
            return serde_json::Value::Null;
        };
        if mon.is_overview || mon.current_layout() != crate::layout::LayoutId::Scroller {
            return serde_json::Value::Null;
        }
        let tagset = mon.current_tagset();
        let wa = mon.work_area;
        let columns: Vec<_> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                !c.is_initial_map_pending && c.is_visible_on(mon_idx, tagset) && c.is_tiled()
            })
            .map(|(idx, c)| {
                let rect = if c.animation.running {
                    c.animation.current
                } else {
                    c.geom
                };
                json!({
                    "id": c.id,
                    "app_id": c.app_id,
                    "x": rect.x - wa.x,
                    "width": rect.width,
                    "focused": Some(idx) == focused_idx,
                })
            })
            .collect();
        json!({ "viewport_width": wa.width, "columns": columns })
    }

    pub(crate) fn build_state_snapshot(&self) -> serde_json::Value {
        use serde_json::json;

        let focused_idx = self.focused_client_idx();
        // The compositor's "where is the user" signal that mshell needs
        // for menu placement (launcher, settings, every pill menu open
        // routed through `active_monitor_name()`). Driven by
        // `active_output_source`, a last-writer-wins choice between the
        // two input signals (see `ActiveOutputSource` for the why):
        //
        // * `Focus` — the user's most recent action was a keyboard
        //   keybind on the focused monitor (tag switch, focus move, …),
        //   so follow keyboard focus. This is what fixes "I'm working on
        //   monitor A but the launcher opens on B because the cursor is
        //   parked there".
        // * `Pointer` — the user's most recent action was moving the
        //   cursor into a (possibly empty) monitor, so follow the
        //   pointer. This preserves "mouse onto empty monitor B →
        //   Super+Space opens the launcher there".
        //
        // Both arms fall back through the other signal, then to the first
        // enumerated monitor (initial startup, no input yet).
        let focused_mon_idx = match self.active_output_source {
            crate::state::ActiveOutputSource::Focus => self.focused_monitor(),
            crate::state::ActiveOutputSource::Pointer => self
                .input_pointer
                .last_monitor
                .or_else(|| {
                    focused_idx
                        .and_then(|i| self.clients.get(i))
                        .map(|c| c.monitor)
                })
                .unwrap_or(0),
        };
        // Single pass over clients → per-monitor aggregates. The monitor
        // loop below used to re-scan all of `self.clients` three times
        // *per monitor* (occupied tag mask + the two scratchpad counts),
        // i.e. O(monitors × clients × 3); this folds them in one O(clients)
        // sweep so the loop just indexes the result.
        let mon_count = self.monitors.len();
        let mut occupied_mask = vec![0u32; mon_count];
        let mut scratchpad_visible = vec![0usize; mon_count];
        let mut scratchpad_hidden = vec![0usize; mon_count];
        for c in &self.clients {
            let m = c.monitor;
            if m >= mon_count {
                continue;
            }
            occupied_mask[m] |= c.tags;
            if c.is_in_scratchpad {
                if c.is_scratchpad_show {
                    scratchpad_visible[m] += 1;
                } else {
                    scratchpad_hidden[m] += 1;
                }
            }
        }

        let outputs: Vec<_> = self
            .monitors
            .iter()
            .enumerate()
            .map(|(i, mon)| {
                let mode = mon.output.current_mode();
                let phys_w = mode.map(|m| m.size.w).unwrap_or(0);
                let phys_h = mode.map(|m| m.size.h).unwrap_or(0);
                let refresh = mode.map(|m| m.refresh).unwrap_or(0);
                // `monitor_area` is the output's live layout-space rect
                // (mode → transform → scale), the same rect the renderer
                // and layer maps use; it is reported as `logical_*`.
                // Scale and transform are read from the live `Output`,
                // the one source of truth wlr-output-management changes.
                let live_scale = mon.output.current_scale().fractional_scale();
                let transform = u32::from(WlTransform::from(mon.output.current_transform()));
                let logical = (mon.monitor_area.width, mon.monitor_area.height);
                let active_tag = mon.tagset[mon.seltags];
                let prev_tag = mon.tagset[mon.seltags ^ 1];
                let active_output = i == focused_mon_idx;
                let focus_history: Vec<_> = mon
                    .focus_history
                    .iter()
                    .filter_map(|&id| self.clients.iter().find(|c| c.id == id && c.monitor == i))
                    .collect();
                json!({
                    "name": mon.name,
                    "active": active_output,
                    "x": mon.monitor_area.x,
                    "y": mon.monitor_area.y,
                    // Current mode size in physical pixels (pre-transform,
                    // pre-scale), as older margo reported it — clients
                    // (mlock, mlayout, mscreenshot, mshell's mode / scale
                    // fallback) read it as such. Layout-space size is
                    // `logical_width` / `logical_height`.
                    "width": phys_w,
                    "height": phys_h,
                    "logical_width": logical.0,
                    "logical_height": logical.1,
                    "scale": live_scale,
                    "transform": transform,
                    "mode": {
                        "physical_width": phys_w,
                        "physical_height": phys_h,
                        "refresh_mhz": refresh,
                    },
                    "layout_idx": mon.pertag.ltidxs[mon.pertag.curtag] as u32,
                    "active_tag_mask": active_tag,
                    "prev_tag_mask": prev_tag,
                    "occupied_tag_mask": occupied_mask[i],
                    // An exclusive-fullscreen client covers this output and
                    // its layer surfaces (bar) are not rendered — see
                    // `layer_renders_on_output`. (`togglefullscreen` /
                    // xdg fullscreen is the work-area mode, which keeps the
                    // bar, so a client's `fullscreen` flag alone can't tell.)
                    "exclusive_fullscreen": !mon.is_overview
                        && self.monitor_has_exclusive_fullscreen(i),
                    "is_overview": mon.is_overview,
                    // Out of the layout to show another output's picture
                    // (`enabled` reads false in wlr-output-management).
                    "mirroring": self.mirror.is_target(&mon.name),
                    // W3.6: per-tag wallpaper hint of the *active*
                    // tag. Wallpaper daemons watching state snapshot
                    // can swap on tag change. Empty string = "use
                    // session default". Per-tag map is in
                    // `wallpapers_by_tag` below for daemons that
                    // want to pre-cache.
                    "wallpaper": mon.pertag.wallpapers
                        .get(mon.pertag.curtag).cloned().unwrap_or_default(),
                    "wallpapers_by_tag": (1..=crate::MAX_TAGS)
                        .map(|t| mon.pertag.wallpapers
                            .get(t).cloned().unwrap_or_default())
                        .collect::<Vec<_>>(),
                    // Keep the legacy app-id array for rolling upgrades. V2
                    // preserves per-window identity when one app owns several
                    // toplevels.
                    "scratchpad_visible": scratchpad_visible[i],
                    "scratchpad_hidden": scratchpad_hidden[i],
                    "focus_history": focus_history.iter()
                        .map(|c| c.app_id.clone())
                        .collect::<Vec<_>>(),
                    "focus_history_v2": focus_history.iter()
                        .map(|c| json!({ "id": c.id, "app_id": c.app_id }))
                        .collect::<Vec<_>>(),
                    // The scroller strip's columns and viewport (null in
                    // any other layout) — the shell's scroller ribbon.
                    "scroller": self.scroller_viewport(i, focused_idx),
                })
            })
            .collect();

        let clients: Vec<_> = self
            .clients
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                let mon_name = self
                    .monitors
                    .get(c.monitor)
                    .map(|m| m.name.clone())
                    .unwrap_or_default();
                json!({
                    // Stable, monotonic per-window id (see `MargoClient::id`) —
                    // the shell keys client identity on this so a window's
                    // address survives another window closing (which shifts the
                    // positional `idx`). `focuswindowid` targets it race-free.
                    "id": c.id,
                    "idx": idx,
                    "monitor": mon_name,
                    "monitor_idx": c.monitor,
                    "tags": c.tags,
                    "app_id": c.app_id,
                    "title": c.title,
                    "x": c.geom.x,
                    "y": c.geom.y,
                    "width": c.geom.width,
                    "height": c.geom.height,
                    "floating": c.is_floating,
                    "floated_by_layout": c.floated_by_layout,
                    "fullscreen": c.is_fullscreen,
                    "minimized": c.is_minimized,
                    // Minimize order of a minimized (non-scratchpad) window,
                    // larger = later; 0 otherwise. The Scratchpads pill
                    // lists minimized windows newest first by it.
                    "minimized_order": if c.is_user_minimized() { c.minimized_seq } else { 0 },
                    "urgent": c.is_urgent,
                    "scratchpad": c.is_in_scratchpad,
                    // Scratchpad on screen (vs tucked away), and whether it
                    // is a named one — the shell's scratchpad chips.
                    "scratchpad_shown": c.is_in_scratchpad && c.is_scratchpad_show,
                    "named_scratchpad": c.is_named_scratchpad,
                    "global": c.is_global,
                    "focused": Some(idx) == focused_idx,
                    "pid": c.pid,
                    // An X11 client (XWayland) — the shell's active-window
                    // XWayland badge.
                    "xwayland": c.window.x11_surface().is_some(),
                    "scanout": c.last_scanout,
                    // Tabbed-group membership (null for ungrouped windows).
                    // Lets the shell surface "tab 2/4" / a group pill later.
                    "group_id": c.group_id,
                    "group_active": c.group_active,
                })
            })
            .collect();

        // The canonical layouts list — same set the live status bar
        // shows. It's compile-time constant, so build the JSON name
        // array once and clone it instead of rebuilding the 11-element
        // `LayoutId` array + re-running `name()` on every snapshot.
        static LAYOUT_NAMES: std::sync::OnceLock<Vec<serde_json::Value>> =
            std::sync::OnceLock::new();
        let layout_names = LAYOUT_NAMES
            .get_or_init(|| {
                [
                    crate::layout::LayoutId::Tile,
                    crate::layout::LayoutId::Scroller,
                    crate::layout::LayoutId::Grid,
                    crate::layout::LayoutId::Monocle,
                    crate::layout::LayoutId::Deck,
                    crate::layout::LayoutId::CenterTile,
                    crate::layout::LayoutId::RightTile,
                    crate::layout::LayoutId::TgMix,
                    crate::layout::LayoutId::Dwindle,
                    crate::layout::LayoutId::Floating,
                    crate::layout::LayoutId::Mosaic,
                ]
                .iter()
                .map(|l| serde_json::Value::String(l.name().to_string()))
                .collect()
            })
            .clone();

        // Active output: the monitor `focused_monitor()` resolves
        // to. Includes pointer-monitor fallback, so cursor-only
        // crossings and `focusmon` to an empty output update the
        // field — mshell's `active_monitor_name()` is then able to
        // route menus to that output.
        let active_output = self
            .monitors
            .get(focused_mon_idx)
            .map(|m| m.name.clone())
            .unwrap_or_default();

        // Diagnostics from the most recent reload (or initial parse).
        // Exposed in state snapshot so `mctl config-errors` can fetch
        // them without a dedicated IPC roundtrip.
        let config_errors: Vec<_> = self
            .last_reload_diagnostics
            .iter()
            .map(|d| {
                json!({
                    "path": d.path.display().to_string(),
                    "line": d.line,
                    "col": d.col,
                    "end_col": d.end_col,
                    "severity": match d.severity {
                        margo_config::diagnostics::Severity::Error => "error",
                        margo_config::diagnostics::Severity::Warning => "warning",
                    },
                    "code": d.code,
                    "message": d.message,
                    "line_text": d.line_text,
                })
            })
            .collect();

        json!({
            "version": 1,
            // The compositor binary's own version (workspace version at
            // build time). `mctl doctor` compares this against its own
            // build version to catch "installed a new margo but haven't
            // re-logged into it yet". Absent on margo builds predating
            // this field → doctor degrades to a soft warning.
            "margo_version": env!("CARGO_PKG_VERSION"),
            "tag_count": MAX_TAGS,
            "active_output": active_output,
            "focused_idx": focused_idx,
            "keyboard_layout": self.current_kb_layout,
            // Active keymode (`setkeymode`); "default" outside any mode.
            // The shell's keymode pill hides in "default".
            "keymode": self.input_keyboard.mode,
            // True from the moment an ext-session-lock is confirmed (the
            // compositor stops showing the desktop) until unlock. mshell
            // gates lock-before-sleep and the lock sidecar on this instead
            // of guessing from process names.
            "session_locked": self.session_locked,
            "lock_keys": {
                "caps": self.lock_keys.caps,
                "num": self.lock_keys.num,
                "scroll": self.lock_keys.scroll,
            },
            "outputs": outputs,
            "clients": clients,
            "layouts": layout_names,
            // The user's `circle_layout` order, verbatim (empty when the
            // knob is unset). `layouts` above stays in canonical
            // `LayoutId` order — the index `mctl layout <N>` wants — while
            // this is purely a *display/cycle* ordering hint for clients
            // like mshell's layout menu.
            "circle_layouts": self.config.circle_layouts,
            "config_errors": config_errors,
            // Output mirroring: the running session (null when idle) and
            // why the last request was refused (null when it wasn't).
            "mirror": {
                "active": self.mirror.active().map(|m| json!({
                    "source": m.source,
                    "target": m.target,
                })),
                "error": self.mirror.last_error,
            },
            "twilight": {
                "enabled": self.config.twilight,
                "mode": match self.config.twilight_mode {
                    margo_config::TwilightMode::Geo => "geo",
                    margo_config::TwilightMode::Manual => "manual",
                    margo_config::TwilightMode::Static => "static",
                    margo_config::TwilightMode::Schedule => "schedule",
                },
                "current_temp_k": self.twilight.engine.last_target.map(|t| t.temp_k),
                "current_gamma_pct": self.twilight.engine.last_target.map(|t| t.gamma_pct),
                "phase": match self.twilight.engine.last_phase {
                    Some(crate::twilight::schedule::Phase::Day) => "day",
                    Some(crate::twilight::schedule::Phase::Night) => "night",
                    Some(crate::twilight::schedule::Phase::TransitionToDay { .. }) => "transition_to_day",
                    Some(crate::twilight::schedule::Phase::TransitionToNight { .. }) => "transition_to_night",
                    None => "idle",
                },
                "source": match self.twilight.engine.source {
                    crate::twilight::Source::Scheduled => "scheduled",
                    crate::twilight::Source::Preview => "preview",
                    crate::twilight::Source::Test { .. } => "test",
                },
                "day_temp_k": self.config.twilight_day_temp,
                "night_temp_k": self.config.twilight_night_temp,
                "day_gamma_pct": self.config.twilight_day_gamma,
                "night_gamma_pct": self.config.twilight_night_gamma,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ipc_push_delay;
    use std::time::{Duration, Instant};

    #[test]
    fn push_delay_spaces_pushes_by_the_interval() {
        let t0 = Instant::now();
        let iv = Duration::from_millis(16);
        assert_eq!(ipc_push_delay(None, t0, iv), None, "first push goes out");
        assert_eq!(
            ipc_push_delay(Some(t0), t0 + Duration::from_millis(4), iv),
            Some(Duration::from_millis(12))
        );
        assert_eq!(ipc_push_delay(Some(t0), t0 + iv, iv), None);
        assert_eq!(
            ipc_push_delay(Some(t0), t0 + Duration::from_secs(5), iv),
            None
        );
        // A clock reading before the last push (no monotonic guarantee
        // across the call sites) waits the full interval, never panics.
        assert_eq!(ipc_push_delay(Some(t0 + iv), t0, iv), Some(iv));
    }
}
