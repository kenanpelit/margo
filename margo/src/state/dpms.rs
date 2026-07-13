//! DPMS (display power) + per-monitor enable/disable methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): blanking/waking outputs and
//! toggling a monitor on or off. Pure `MargoState` glue, no new types.

use super::*;

impl MargoState {
    /// Soft-disable a monitor: mark it inactive, migrate every client
    /// to the first remaining enabled monitor, and clear focus from it.
    /// Render and arrange paths skip disabled monitors so the panel
    /// stops getting dirty repaints; the underlying smithay `Output`
    /// stays alive so a later `enable_monitor` call can restore it
    /// without a full hotplug round-trip. Pertag state survives across
    /// the cycle.
    ///
    /// Note: the DRM connector is NOT powered off here — that needs
    /// the udev backend's DrmCompositor handle, plumbed separately.
    /// What this fixes: the wlr-output-management protocol-level
    /// "disable" request now succeeds, kanshi profiles that toggle
    /// outputs flip cleanly, and the bar / state file see the right
    /// active-output set. Power-off of the panel is a follow-up.
    /// Request a DPMS power change (real panel off/on via the udev
    /// backend's `DrmCompositor::clear()` / re-render). `on = Some(false)`
    /// powers the panel(s) OFF, `Some(true)` ON; `None` toggles globally
    /// (any-off → on-all, else off-all). `target = Some(name)` scopes to one
    /// output, `None` = all. Pushes to `pending_dpms` + kicks a repaint;
    /// no-ops on winit. Any subsequent input wakes the screen (see
    /// `wake_dpms_on_input`), so a black screen is always recoverable.
    pub fn request_dpms(&mut self, on: Option<bool>, target: Option<&str>) {
        let outputs: Vec<Output> = self
            .monitors
            .iter()
            .filter(|m| target.is_none_or(|t| m.output.name() == t))
            .map(|m| m.output.clone())
            .collect();
        if outputs.is_empty() {
            return;
        }
        let want_on = on.unwrap_or(self.any_dpms_off);
        for o in &outputs {
            self.pending_dpms.push((o.clone(), want_on));
            // Tell wlr-output-power clients (swayidle etc.) about the change,
            // whatever triggered it (protocol, keybind, or mctl).
            self.output_power_manager_state
                .output_power_changed(o, want_on);
        }
        self.any_dpms_off = !want_on;
        self.dpms_off_at = if want_on {
            None
        } else {
            Some(std::time::Instant::now())
        };
        self.wake_repaint_backend();
    }

    /// Safety net: called from `handle_input` on any real input event. If a
    /// panel is DPMS-off, wake all outputs — so the user can never get stuck
    /// on a black screen. Ignores input within a short grace after the off so
    /// the triggering keystroke / click (its release, the Enter that ran
    /// `mctl`, settling pointer motion) doesn't bounce the panel straight
    /// back on.
    /// Returns `true` if this call actually woke a darkened panel — the
    /// caller then swallows the triggering event so the keystroke / click
    /// that wakes the screen doesn't also reach the focused surface (e.g. a
    /// stray newline in the terminal you ran `mctl dispatch dpms off` from).
    pub fn wake_dpms_on_input(&mut self) -> bool {
        const GRACE: std::time::Duration = std::time::Duration::from_millis(1200);
        if self.any_dpms_off && self.dpms_off_at.is_none_or(|t| t.elapsed() >= GRACE) {
            self.request_dpms(Some(true), None);
            return true;
        }
        false
    }

    pub fn disable_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        if !self.monitors[mon_idx].enabled {
            return;
        }
        // Pick a migration target — first OTHER enabled monitor.
        let target = (0..self.monitors.len()).find(|&i| i != mon_idx && self.monitors[i].enabled);
        let Some(target) = target else {
            tracing::warn!(
                "disable_monitor: refusing to disable {} — no other enabled monitor",
                self.monitors[mon_idx].name
            );
            return;
        };
        let target_tagset = self.monitors[target].current_tagset();
        let target_name = self.monitors[target].name.clone();
        let src_name = self.monitors[mon_idx].name.clone();

        // Migrate every client living on the doomed monitor.
        for c in self.clients.iter_mut() {
            if c.monitor == mon_idx {
                c.monitor = target;
                // Pull onto an active tag of the new home so the
                // client doesn't vanish into a hidden tagset.
                if c.tags & target_tagset == 0 {
                    c.tags = target_tagset;
                }
            }
        }
        // Clear focus history that points at the disabled monitor.
        if self.focused_monitor() == mon_idx {
            for mon in &mut self.monitors {
                mon.selected = None;
            }
        }
        self.monitors[mon_idx].enabled = false;
        self.drop_output_clock(&src_name);
        self.arrange_monitor(target);
        self.focus_first_visible_or_clear(target);
        self.publish_output_topology();
        self.mark_state_dirty();
        tracing::info!(
            from = %src_name,
            to = %target_name,
            "disabled output: migrated clients"
        );
    }

    /// Re-enable a previously soft-disabled monitor. New windows can
    /// land on it again; arrange picks it up; render starts drawing
    /// it on the next frame.
    pub fn enable_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        if self.monitors[mon_idx].enabled {
            return;
        }
        self.monitors[mon_idx].enabled = true;
        self.arrange_monitor(mon_idx);
        self.publish_output_topology();
        self.mark_state_dirty();
        tracing::info!(output = %self.monitors[mon_idx].name, "re-enabled output");
    }
}
