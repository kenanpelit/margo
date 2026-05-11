//! `MargoState::debug_dump` — triggered by `SIGUSR1` or the
//! `mctl debug-dump` IPC command so a user staring at a frozen / grey
//! screen can capture full compositor state to the journal without
//! attaching a debugger. Extracted from `state.rs` (roadmap Q1).
//!
//! Output is all `tracing::info!` lines so it lands in whatever
//! subscriber the user has configured (systemd journal, stderr, etc.)
//! and gets correlated with surrounding events via timestamps.

use super::MargoState;

impl MargoState {
    pub fn debug_dump(&self) {
        tracing::info!("─── margo debug dump ───");
        tracing::info!(
            "outputs: {} monitor(s); session_locked={} lock_surfaces={}",
            self.monitors.len(),
            self.session_locked,
            self.lock_surfaces.len()
        );
        for (i, mon) in self.monitors.iter().enumerate() {
            tracing::info!(
                "  mon[{i}] {} area={}x{}+{}+{} tagset[{}]={:#x} prev={:#x} layout={:?} selected={:?} prev_selected={:?}",
                mon.name,
                mon.monitor_area.width,
                mon.monitor_area.height,
                mon.monitor_area.x,
                mon.monitor_area.y,
                mon.seltags,
                mon.tagset[mon.seltags],
                mon.tagset[mon.seltags ^ 1],
                mon.current_layout(),
                mon.selected,
                mon.prev_selected,
            );
        }
        tracing::info!(
            "clients: {} total; focused={:?}",
            self.clients.len(),
            self.focused_client_idx()
        );
        for (i, c) in self.clients.iter().enumerate().take(32) {
            tracing::info!(
                "  client[{i}] mon={} tags={:#x} float={} fs={} app_id={:?} title={:?} geom={}x{}+{}+{}",
                c.monitor,
                c.tags,
                c.is_floating,
                c.is_fullscreen,
                c.app_id,
                c.title,
                c.geom.width,
                c.geom.height,
                c.geom.x,
                c.geom.y,
            );
        }
        if self.clients.len() > 32 {
            tracing::info!(more = self.clients.len() - 32, "client dump truncated");
        }
        tracing::info!(count = self.idle_inhibitors.len(), "idle inhibitors");
        let kbd = self.seat.get_keyboard();
        if let Some(kb) = kbd.as_ref() {
            tracing::info!(
                "keyboard focus: {}",
                kb.current_focus()
                    .map(|t| format!("{t:?}"))
                    .unwrap_or_else(|| "<none>".to_string())
            );
        }
        let layer_count: usize = self
            .space
            .outputs()
            .map(|o| smithay::desktop::layer_map_for_output(o).layers().count())
            .sum();
        tracing::info!(count = layer_count, "layer surfaces (all outputs)");
        tracing::info!("─── end debug dump ───");
    }
}
