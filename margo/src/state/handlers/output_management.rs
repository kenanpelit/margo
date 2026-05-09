//! `wlr-output-management-v1` handler — runtime topology / scale /
//! transform / mode / disable changes via `wlr-randr` and `kanshi`.
//!
//! Mode changes are deferred onto `pending_output_mode_changes`
//! because the actual DRM re-modeset happens in the udev backend
//! (which holds the DrmCompositor); doing it here would require
//! plumbing a backend handle onto MargoState. Disable refuses to
//! kill the last active output (would strand the user with a dark
//! screen and no recovery short of TTY login).

use crate::{
    delegate_output_management,
    protocols::output_management::{
        OutputManagementHandler, OutputManagementManagerState, PendingHeadConfig,
    },
    state::MargoState,
    PendingOutputModeChange,
};

impl OutputManagementHandler for MargoState {
    fn output_management_state(&mut self) -> &mut OutputManagementManagerState {
        &mut self.output_management_state
    }

    fn apply_output_pending(
        &mut self,
        pending: std::collections::HashMap<String, PendingHeadConfig>,
    ) -> bool {
        // Disable / re-enable handled BEFORE the geometry-update
        // loop — toggling `monitor.enabled` is cheap, but if a
        // pending head says "disable + change mode" we'd want the
        // mode change to apply against the *enabled* monitor; same
        // for "enable + change scale".
        //
        // Refuse to disable the LAST enabled monitor — leaving zero
        // active outputs strands the user with a dark screen.
        // wlr-randr / kanshi typically guard client-side, but the
        // protocol allows the request, so guard server-side too.
        let any_disable = pending.values().any(|p| !p.enabled());
        if any_disable {
            let currently_enabled =
                self.monitors.iter().filter(|m| m.enabled).count();
            let pending_disabling = pending
                .iter()
                .filter(|(name, p)| {
                    !p.enabled()
                        && self
                            .monitors
                            .iter()
                            .any(|m| m.name == **name && m.enabled)
                })
                .count();
            if pending_disabling >= currently_enabled {
                tracing::warn!(
                    "output_management: rejecting disable — would leave 0 active outputs"
                );
                return false;
            }
        }

        let mut changed = false;
        // Pass 1 — enable/disable toggles. Disable migrates clients
        // off the doomed output FIRST so the geometry pass below
        // doesn't try to arrange against it.
        for (name, p) in &pending {
            let Some(mon_idx) = self.monitors.iter().position(|m| m.name == *name) else {
                continue;
            };
            let was_enabled = self.monitors[mon_idx].enabled;
            let want_enabled = p.enabled();
            if was_enabled == want_enabled {
                continue;
            }
            if !want_enabled {
                self.disable_monitor(mon_idx);
            } else {
                self.enable_monitor(mon_idx);
            }
            changed = true;
        }

        // Apply scale, transform, position synchronously through
        // `Output::change_current_state` — same path as before,
        // updates smithay-side state plus broadcasts wl_output
        // events to clients.
        for (name, p) in &pending {
            let Some(mon_idx) = self.monitors.iter().position(|m| m.name == *name) else {
                tracing::warn!(
                    "output_management: ignoring pending head for unknown output {name}"
                );
                continue;
            };
            if !self.monitors[mon_idx].enabled {
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
                self.pending_output_mode_changes
                    .push(PendingOutputModeChange {
                        output_name: name.clone(),
                        width: w,
                        height: h,
                        refresh_mhz,
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
            // Re-publish topology so other wlr-output-management
            // clients (kanshi watchers, secondary wlr-randr) see
            // the new state.
            self.publish_output_topology();
        }
        changed
    }
}
delegate_output_management!(MargoState);
