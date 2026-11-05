//! `wlr-output-management-v1` handler — runtime topology / scale /
//! transform / mode / disable changes via `wlr-randr` and `kanshi`.
//!
//! Mode changes are deferred onto `output_pending.mode_changes`
//! because the actual DRM re-modeset happens in the udev backend
//! (which holds the DrmCompositor); doing it here would require
//! plumbing a backend handle onto MargoState. Each carries a part of
//! the configuration's reply, so the client hears `succeeded` only
//! after the mode is really set. Disable refuses to
//! kill the last active output (would strand the user with a dark
//! screen and no recovery short of TTY login).

use crate::{
    PendingOutputModeChange, delegate_output_management,
    protocols::output_management::{
        ModeReply, OutputManagementHandler, OutputManagementManagerState, PendingHeadConfig,
    },
    state::MargoState,
};

impl OutputManagementHandler for MargoState {
    fn output_management_state(&mut self) -> &mut OutputManagementManagerState {
        &mut self.protocols.output_management
    }

    fn apply_output_pending(
        &mut self,
        pending: std::collections::HashMap<String, PendingHeadConfig>,
        reply: &ModeReply,
    ) -> bool {
        if let Err(why) = self.check_output_pending(&pending) {
            tracing::warn!("output_management: rejecting configuration: {why}");
            return false;
        }
        let mut ok = true;
        let mut changed = false;

        // Pass 1 — enable/disable toggles, in output order. A head is
        // toggled when the client asks for the opposite of what it was
        // told (`reported_on`), not of what an earlier toggle in this
        // pass made it: switching a mirror source off brings the mirror
        // target back as the desktop, and the target's `disable_head`
        // (it reads as off) must not then switch it off again.
        //
        // Enables run before disables, so a swap (A off, B on) never
        // passes through "no output on" — `take_monitor_out_of_layout`
        // refuses that. Disable migrates clients off the doomed output
        // FIRST so the geometry pass below doesn't arrange against it.
        let reported_on: Vec<bool> = self.monitors.iter().map(|m| m.enabled).collect();
        let mut toggles: Vec<(usize, bool)> = pending
            .iter()
            .filter_map(|(name, p)| {
                let idx = self.monitors.iter().position(|m| m.name == *name)?;
                (reported_on[idx] != p.enabled()).then_some((idx, p.enabled()))
            })
            .collect();
        // Enables (`true`) first, then by output index.
        toggles.sort_by_key(|&(idx, on)| (!on, idx));
        for (mon_idx, on) in toggles {
            if on {
                self.enable_monitor(mon_idx);
            } else if !self.disable_monitor(mon_idx) {
                tracing::warn!(
                    output = %self.monitors[mon_idx].name,
                    "output_management: could not switch the output off"
                );
                ok = false;
            }
            changed = true;
        }

        // Apply scale, transform, position synchronously through
        // `Output::change_current_state` — same path as before,
        // updates smithay-side state plus broadcasts wl_output
        // events to clients.
        for (name, p) in &pending {
            let Some(mon_idx) = self.monitors.iter().position(|m| m.name == *name) else {
                continue;
            };
            if !p.enabled() {
                continue;
            }
            if !self.monitors[mon_idx].enabled {
                // Every `enable_head` head was switched on above; one that
                // is still off would have its changes dropped while the
                // client hears `succeeded`.
                tracing::warn!(output = %name, "output_management: head stayed off");
                ok = false;
                continue;
            }
            let mon = &self.monitors[mon_idx];
            let output = mon.output.clone();
            let mut local_change = false;

            if let Some(scale) = p.scale() {
                output.change_current_state(
                    None,
                    None,
                    Some(smithay::output::Scale::Fractional(scale)),
                    None,
                );
                local_change = true;
            }
            if let Some(t) = p.transform() {
                let smithay_t: smithay::utils::Transform = t.into();
                output.change_current_state(None, Some(smithay_t), None, None);
                local_change = true;
            }
            if let Some((x, y)) = p.position() {
                // Three sources of truth need to agree on the new
                // position: smithay's Output (broadcasts wl_output
                // geometry), smithay's Space (where the live render
                // iterates outputs from), and margo's own
                // `monitor_area.x/y` (used by `arrange_*`).
                output.change_current_state(
                    None,
                    None,
                    None,
                    Some(smithay::utils::Point::from((x, y))),
                );
                self.space
                    .map_output(&output, smithay::utils::Point::from((x, y)));
                self.monitors[mon_idx].monitor_area.x = x;
                self.monitors[mon_idx].monitor_area.y = y;
                local_change = true;
            }
            if let Some((w, h, refresh_mhz)) = p.mode() {
                self.output_pending
                    .mode_changes
                    .push(PendingOutputModeChange {
                        output_name: name.clone(),
                        width: w,
                        height: h,
                        refresh_mhz,
                        reply: reply.part(),
                    });
                tracing::info!(
                    "output_management: queued mode change {name}: {w}x{h}@{}.{:03}Hz",
                    refresh_mhz / 1000,
                    refresh_mhz % 1000,
                );
                local_change = true;
            }
            if local_change {
                self.refresh_output_work_area(&output);
                changed = true;
            }
        }
        if changed {
            self.arrange_all();
            self.request_repaint();
            if !self.output_pending.mode_changes.is_empty() {
                self.wake_repaint_backend();
            }
            // Re-publish topology so other wlr-output-management
            // clients (kanshi watchers, secondary wlr-randr) see
            // the new state.
            self.publish_output_topology();
        }
        // Success also when nothing changed: enabling an output that is
        // already on (`wlr-randr --output X --on`) is a valid no-op.
        ok
    }

    fn test_output_pending(
        &mut self,
        pending: &std::collections::HashMap<String, PendingHeadConfig>,
    ) -> bool {
        match self.check_output_pending(pending) {
            Ok(()) => true,
            Err(why) => {
                tracing::info!("output_management: test refused: {why}");
                false
            }
        }
    }

    fn supports_mode_changes(&self) -> bool {
        self.output_pending.mode_changes_supported
    }
}

impl MargoState {
    /// Why `pending` can't be applied — the checks `test` and `apply`
    /// share. Per-head values margo refuses outright (bad scale, custom
    /// mode, adaptive sync) are caught by the protocol layer first.
    fn check_output_pending(
        &self,
        pending: &std::collections::HashMap<String, PendingHeadConfig>,
    ) -> Result<(), String> {
        // Mode changes are queued for the backend; nothing drains them
        // on winit / headless.
        if !self.supports_mode_changes() && pending.values().any(|p| p.mode().is_some()) {
            return Err("mode changes are unsupported by this backend".into());
        }
        // Every head must name a live output; a stale name means the
        // client's view is out of date.
        if let Some(name) = pending
            .keys()
            .find(|name| !self.monitors.iter().any(|m| m.name == **name))
        {
            return Err(format!("unknown output {name}"));
        }
        // At least one output must be on afterwards — zero active outputs
        // strands the user with a dark screen and no recovery short of a
        // TTY login. wlr-randr / kanshi guard client-side, but the
        // protocol allows the request, so guard here too.
        let on_after = self
            .monitors
            .iter()
            .filter(|m| self.output_on_after(m, pending))
            .count();
        if on_after == 0 {
            return Err("it would leave no output on".into());
        }
        Ok(())
    }

    /// Whether `mon` is lit after `pending` is applied. An output the
    /// configuration leaves alone keeps its state. A mirror target reads
    /// as off (it is out of the layout) but is lit; it stays a target when
    /// the client repeats that `off`, and if its source goes off the mirror
    /// ends and the target is the desktop again — on either way.
    fn output_on_after(
        &self,
        mon: &crate::state::MargoMonitor,
        pending: &std::collections::HashMap<String, PendingHeadConfig>,
    ) -> bool {
        let target = self.mirror.is_target(&mon.name);
        match pending.get(&mon.name).map(PendingHeadConfig::enabled) {
            Some(true) => true,
            Some(false) => target && !mon.enabled,
            None => mon.enabled || target,
        }
    }
}

delegate_output_management!(MargoState);
