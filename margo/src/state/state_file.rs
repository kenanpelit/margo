//! State snapshot builder + the per-iteration IPC flush.
//!
//! `build_state_snapshot` produces the JSON document every IPC `get`/
//! `watch` reply is built from. `mark_state_dirty` / `flush_ipc_if_dirty`
//! coalesce a burst of changes into one pushed `watch` frame per
//! event-loop iteration. There is no longer a state snapshot file — the
//! Unix socket is the only state egress.

use super::MargoState;
use crate::MAX_TAGS;

impl MargoState {
    /// Mark compositor state as changed since the last flush. The
    /// actual fan-out (a fresh frame to every IPC `watch` subscriber)
    /// is **coalesced** to once per event-loop iteration in
    /// `flush_ipc_if_dirty`, so a burst of changes (one layout switch
    /// touches focus + windows + tags, each of which calls this) pushes
    /// the snapshot once instead of N times.
    pub fn mark_state_dirty(&self) {
        self.state_dirty.set(true);
    }

    /// Once per event-loop iteration: if state changed, push a fresh
    /// snapshot frame to every IPC `watch` subscriber. This is the sole
    /// state-egress path — there is no state snapshot file anymore.
    pub fn flush_ipc_if_dirty(&mut self) {
        if !self.state_dirty.replace(false) {
            return;
        }
        self.ipc_push_watches();
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
        let outputs: Vec<_> = self
            .monitors
            .iter()
            .enumerate()
            .map(|(i, mon)| {
                let mode = mon.output.current_mode();
                let phys_w = mode.map(|m| m.size.w).unwrap_or(0);
                let phys_h = mode.map(|m| m.size.h).unwrap_or(0);
                let refresh = mode.map(|m| m.refresh).unwrap_or(0);
                let active_tag = mon.tagset[mon.seltags];
                let prev_tag = mon.tagset[mon.seltags ^ 1];
                let active_output = i == focused_mon_idx;
                json!({
                    "name": mon.name,
                    "active": active_output,
                    "x": mon.monitor_area.x,
                    "y": mon.monitor_area.y,
                    "width": mon.monitor_area.width,
                    "height": mon.monitor_area.height,
                    "scale": mon.scale,
                    "transform": mon.transform,
                    "mode": {
                        "physical_width": phys_w,
                        "physical_height": phys_h,
                        "refresh_mhz": refresh,
                    },
                    "layout_idx": mon.pertag.ltidxs[mon.pertag.curtag] as u32,
                    "active_tag_mask": active_tag,
                    "prev_tag_mask": prev_tag,
                    "occupied_tag_mask": self.clients.iter()
                        .filter(|c| c.monitor == i)
                        .fold(0u32, |a, c| a | c.tags),
                    "is_overview": mon.is_overview,
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
                    // W3.4: scratchpad summary (counts of visible /
                    // hidden) and per-monitor focus history (MRU
                    // app_ids, most recent first). MRU widgets and
                    // dock indicators read these to render counts +
                    // recently-used app rings.
                    "scratchpad_visible": self.clients.iter()
                        .filter(|c| c.monitor == i
                            && c.is_in_scratchpad
                            && c.is_scratchpad_show)
                        .count(),
                    "scratchpad_hidden": self.clients.iter()
                        .filter(|c| c.monitor == i
                            && c.is_in_scratchpad
                            && !c.is_scratchpad_show)
                        .count(),
                    "focus_history": mon.focus_history.iter()
                        .filter_map(|&idx| self.clients.get(idx))
                        .map(|c| c.app_id.clone())
                        .collect::<Vec<_>>(),
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
                    "fullscreen": c.is_fullscreen,
                    "minimized": c.is_minimized,
                    "urgent": c.is_urgent,
                    "scratchpad": c.is_in_scratchpad,
                    "global": c.is_global,
                    "focused": Some(idx) == focused_idx,
                    "pid": c.pid,
                    "scanout": c.last_scanout,
                    // Tabbed-group membership (null for ungrouped windows).
                    // Lets the shell surface "tab 2/4" / a group pill later.
                    "group_id": c.group_id,
                    "group_active": c.group_active,
                })
            })
            .collect();

        // The canonical layouts list — same set the live status
        // bar shows.
        let all_layouts = [
            crate::layout::LayoutId::Tile,
            crate::layout::LayoutId::Scroller,
            crate::layout::LayoutId::Grid,
            crate::layout::LayoutId::Monocle,
            crate::layout::LayoutId::Deck,
            crate::layout::LayoutId::CenterTile,
            crate::layout::LayoutId::RightTile,
            crate::layout::LayoutId::VerticalScroller,
            crate::layout::LayoutId::VerticalTile,
            crate::layout::LayoutId::VerticalGrid,
            crate::layout::LayoutId::VerticalDeck,
            crate::layout::LayoutId::TgMix,
            crate::layout::LayoutId::Canvas,
            crate::layout::LayoutId::Dwindle,
        ];
        let layout_names: Vec<_> = all_layouts
            .iter()
            .map(|l| serde_json::Value::String(l.name().to_string()))
            .collect();

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
            "outputs": outputs,
            "clients": clients,
            "layouts": layout_names,
            "config_errors": config_errors,
            "twilight": {
                "enabled": self.config.twilight,
                "mode": match self.config.twilight_mode {
                    margo_config::TwilightMode::Geo => "geo",
                    margo_config::TwilightMode::Manual => "manual",
                    margo_config::TwilightMode::Static => "static",
                    margo_config::TwilightMode::Schedule => "schedule",
                },
                "current_temp_k": self.twilight.last_target.map(|t| t.temp_k),
                "current_gamma_pct": self.twilight.last_target.map(|t| t.gamma_pct),
                "phase": match self.twilight.last_phase {
                    Some(crate::twilight::schedule::Phase::Day) => "day",
                    Some(crate::twilight::schedule::Phase::Night) => "night",
                    Some(crate::twilight::schedule::Phase::TransitionToDay { .. }) => "transition_to_day",
                    Some(crate::twilight::schedule::Phase::TransitionToNight { .. }) => "transition_to_night",
                    None => "idle",
                },
                "source": match self.twilight.source {
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
