//! DPMS (display power) + per-monitor enable/disable methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): blanking/waking outputs and
//! toggling a monitor on or off. Pure `MargoState` glue, no new types.

use super::*;

impl MargoState {
    /// Request a DPMS power change (real panel off/on via the udev
    /// backend's `DrmCompositor::clear()` / re-render). `on = Some(false)`
    /// powers the panel(s) OFF, `Some(true)` ON; `None` toggles globally
    /// (any-off → on-all, else off-all). `target = Some(name)` scopes to one
    /// output, `None` = all. Pushes to `output_pending.dpms` + kicks a repaint;
    /// no-ops on winit. Any subsequent input wakes the screen (see
    /// `wake_dpms_on_input`), so a black screen is always recoverable.
    pub fn request_dpms(&mut self, on: Option<bool>, target: Option<&str>) {
        let outputs: Vec<Output> = self
            .monitors
            .iter()
            .filter(|m| m.enabled)
            .filter(|m| target.is_none_or(|t| m.output.name() == t))
            .map(|m| m.output.clone())
            .collect();
        if outputs.is_empty() {
            return;
        }
        let want_on = on.unwrap_or(self.any_dpms_off);
        for o in &outputs {
            self.output_pending.dpms.push((o.clone(), want_on));
            // Tell wlr-output-power clients (swayidle etc.) about the change,
            // whatever triggered it (protocol, keybind, or mctl).
            self.protocols
                .output_power_manager
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

    /// Soft-disable a monitor: mark it inactive, migrate every client
    /// to the first remaining enabled monitor, and clear focus from it.
    /// Render and arrange paths skip disabled monitors so the panel
    /// stops getting dirty repaints; the underlying smithay `Output`
    /// stays alive so a later `enable_monitor` call can restore it
    /// without a full hotplug round-trip. Pertag state survives across
    /// the cycle.
    ///
    /// The panel is blanked through the DPMS path
    /// (`output_pending.dpms`, `DrmCompositor::clear()`), the output leaves
    /// the space, and the pointer is moved onto an enabled output.
    ///
    /// `true` when the monitor is off afterwards; `false` when refused
    /// (unknown index, already off, or no other monitor to move to).
    pub fn disable_monitor(&mut self, mon_idx: usize) -> bool {
        if mon_idx >= self.monitors.len() {
            return false;
        }
        // Switching off either end of a mirror ends it first (that puts a
        // mirror target back, so it is then switched off for real).
        let name = self.monitors[mon_idx].name.clone();
        self.mirror_output_gone(&name);
        self.take_monitor_out_of_layout(mon_idx, true)
    }

    /// Take a monitor out of the layout: clients migrate to another
    /// enabled monitor, the pointer and focus leave it, it leaves the
    /// space. `blank` also powers the panel down (a soft-disable); a
    /// mirror target keeps it lit to show the mirrored picture. `false`
    /// when refused (already out, or no other enabled monitor).
    pub(crate) fn take_monitor_out_of_layout(&mut self, mon_idx: usize, blank: bool) -> bool {
        if mon_idx >= self.monitors.len() {
            return false;
        }
        if !self.monitors[mon_idx].enabled {
            return false;
        }
        // Pick a migration target — first OTHER enabled monitor.
        let target = (0..self.monitors.len()).find(|&i| i != mon_idx && self.monitors[i].enabled);
        let Some(target) = target else {
            tracing::warn!(
                "disable_monitor: refusing to disable {} — no other enabled monitor",
                self.monitors[mon_idx].name
            );
            return false;
        };
        let target_tagset = self.monitors[target].current_tagset();
        let target_name = self.monitors[target].name.clone();
        let src_name = self.monitors[mon_idx].name.clone();
        // Only when keyboard focus is on this output does it move (to the
        // migration target); typing on a third output carries on.
        let focus_was_here = self.keyboard_focus_on_monitor(mon_idx);

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
        // Its clients live on `target` now: its own focus memory goes.
        // Other monitors' `selected` still name their own clients.
        self.monitors[mon_idx].selected = None;
        self.monitors[mon_idx].prev_selected = None;
        self.monitors[mon_idx].enabled = false;
        // Blank the panel the way DPMS off does: otherwise the CRTC keeps
        // scanning out the last frame, since the render loop skips a
        // disabled output. And take it out of the space so surfaces leave
        // it and nothing hit-tests against it.
        let output = self.monitors[mon_idx].output.clone();
        if blank {
            self.output_pending.dpms.push((output.clone(), false));
        }
        self.space.unmap_output(&output);
        self.wake_repaint_backend();
        // Move the pointer off the dark area onto an enabled output.
        self.clamp_pointer_to_outputs();
        self.refresh_pointer_monitor_tracking();
        self.drop_output_clock(&src_name);
        self.arrange_monitor(target);
        if focus_was_here {
            self.focus_most_recent_visible_or_clear(target);
        }
        self.publish_output_topology();
        self.mark_state_dirty();
        tracing::info!(
            from = %src_name,
            to = %target_name,
            "disabled output: migrated clients"
        );
        true
    }

    /// Whether keyboard focus is on monitor `mon_idx`: a window living
    /// there, a layer surface on its output, or nothing focused while the
    /// pointer is on it.
    fn keyboard_focus_on_monitor(&self, mon_idx: usize) -> bool {
        let focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        match focus {
            Some(FocusTarget::Window(w)) => self
                .client_idx(&w)
                .is_none_or(|i| self.clients[i].monitor == mon_idx),
            Some(FocusTarget::LayerSurface(l)) => {
                smithay::desktop::layer_map_for_output(&self.monitors[mon_idx].output)
                    .layer_for_surface(
                        l.wl_surface(),
                        smithay::desktop::WindowSurfaceType::TOPLEVEL,
                    )
                    .is_some()
            }
            Some(FocusTarget::SessionLock(_) | FocusTarget::Popup(_)) => false,
            None => self.pointer_monitor().is_none_or(|m| m == mon_idx),
        }
    }

    /// Re-enable a previously soft-disabled monitor. New windows can
    /// land on it again; arrange picks it up; render starts drawing
    /// it on the next frame.
    pub fn enable_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        // Switching a mirror target on ends the mirror, which puts it back.
        if self.mirror.is_target(&self.monitors[mon_idx].name) {
            let name = self.monitors[mon_idx].name.clone();
            self.mirror_output_gone(&name);
            return;
        }
        self.put_monitor_back_in_layout(mon_idx);
    }

    /// Undo [`Self::take_monitor_out_of_layout`].
    pub(crate) fn put_monitor_back_in_layout(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        if self.monitors[mon_idx].enabled {
            return;
        }
        self.monitors[mon_idx].enabled = true;
        let output = self.monitors[mon_idx].output.clone();
        let area = self.monitors[mon_idx].monitor_area;
        self.space.map_output(&output, (area.x, area.y));
        // Light the panel again (unless DPMS has every panel off: the
        // next wake turns this one on with the rest).
        if !self.any_dpms_off {
            self.output_pending.dpms.push((output, true));
            self.wake_repaint_backend();
        }
        self.arrange_monitor(mon_idx);
        self.publish_output_topology();
        self.mark_state_dirty();
        tracing::info!(output = %self.monitors[mon_idx].name, "re-enabled output");
    }
}
