//! `state.json` serialization — the runtime side-channel mctl's
//! rich subcommands (`clients`, `outputs`, `status`) read to render
//! richer info than fits in dwl-ipc-v2 events. Extracted from
//! `state.rs` (roadmap Q1).
//!
//! Best-effort: write failures are logged at debug level, never
//! surfaced to the user — the file is tooling-only, not a hard
//! correctness requirement. Atomically replaced via tmp-rename so
//! readers never see a half-written file.

use super::{state_file_path, MargoState};
use crate::MAX_TAGS;

impl MargoState {
    /// Serialise the current state — outputs, clients, layouts —
    /// to `$XDG_RUNTIME_DIR/margo/state.json` (atomic rename).
    /// Read by `mctl clients` / `mctl outputs` / the improved
    /// `mctl status` so they can list richer info than what fits
    /// in the wire-level dwl-ipc-v2 events.
    ///
    /// Best-effort: failures are logged at debug level, never
    /// surfaced to the user — the file is a side-channel for
    /// tooling, not a hard correctness requirement.
    pub fn write_state_file(&self) {
        let path = state_file_path();
        if let Err(err) = self.write_state_file_inner(&path) {
            tracing::debug!(path = %path.display(), error = ?err, "write_state_file failed");
        }
    }

    fn write_state_file_inner(&self, path: &std::path::Path) -> anyhow::Result<()> {
        use std::io::Write as _;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let payload = self.build_state_snapshot();
        let json = serde_json::to_string(&payload)?;

        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn build_state_snapshot(&self) -> serde_json::Value {
        use serde_json::json;

        let focused_idx = self.focused_client_idx();
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
                let active_output = focused_idx
                    .and_then(|fc| self.clients.get(fc))
                    .map(|c| c.monitor == i)
                    .unwrap_or(false);
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
                    // tag. Wallpaper daemons watching state.json
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
                let mon_name = self.monitors.get(c.monitor)
                    .map(|m| m.name.clone()).unwrap_or_default();
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
                })
            })
            .collect();

        // Mirror dwl-ipc's layouts list — same set the live status
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

        // Active output: the one the focused client is on, else the
        // first monitor.
        let active_output = focused_idx
            .and_then(|idx| self.clients.get(idx))
            .and_then(|c| self.monitors.get(c.monitor))
            .map(|m| m.name.clone())
            .or_else(|| self.monitors.first().map(|m| m.name.clone()))
            .unwrap_or_default();

        // Diagnostics from the most recent reload (or initial parse).
        // Exposed in state.json so `mctl config-errors` can fetch
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
            "tag_count": MAX_TAGS,
            "active_output": active_output,
            "focused_idx": focused_idx,
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
