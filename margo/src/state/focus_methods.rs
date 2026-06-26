//! Keyboard-focus + pointer-monitor methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): the cluster computing *who has
//! focus* and *which monitor the pointer is on* — focus resolution
//! (`compute_desired_focus`/`refresh_keyboard_focus`), the scroller insert /
//! index-shift helpers, and pointer→monitor tracking + clamping. Pure
//! `MargoState` glue, no new types.

use super::*;

impl MargoState {
    pub fn focused_client_idx(&self) -> Option<usize> {
        let keyboard = self.seat.get_keyboard()?;
        let focus = keyboard.current_focus()?;
        if let FocusTarget::Window(focused) = focus {
            self.clients.iter().position(|c| c.window == focused)
        } else {
            None
        }
    }

    pub fn focused_monitor(&self) -> usize {
        self.focused_client_idx()
            .map(|i| self.clients[i].monitor)
            .or_else(|| self.pointer_monitor())
            .unwrap_or(0)
    }

    /// Centralised "what should keyboard focus be right now?" — the niri
    /// pattern. We can't rely on transitional events (layer_destroyed
    /// alone, set_focus from new_surface) because real clients change
    /// focus state in ways those events don't fire for:
    ///
    ///   * **noctalia's launcher / settings panels** don't create or
    ///     destroy a layer surface when they open/close. They keep one
    ///     `MainScreen` `WlrLayershell` per output and just toggle its
    ///     `keyboardFocus` between `Exclusive` and `None`. The transition
    ///     surfaces only as a `wl_surface.commit` with a different
    ///     cached `keyboard_interactivity` — no destroy callback, no
    ///     unmap. Without recomputing focus on every layer commit we
    ///     never notice the panel closed and the key events keep going
    ///     into the void.
    ///   * **session lock with multiple outputs**. Quickshell creates one
    ///     `WlSessionLockSurface` per screen; only the surface on the
    ///     output the user is looking at should hold focus, and that has
    ///     to track cursor motion across outputs.
    ///
    /// This method picks a target by priority and pushes it through the
    /// existing `focus_surface` plumbing only if it differs from the
    /// current focus, so it's cheap to call after every relevant event.
    pub fn refresh_keyboard_focus(&mut self) {
        let desired = self.compute_desired_focus();

        let current = self.seat.get_keyboard().and_then(|kb| kb.current_focus());
        if current.as_ref() == desired.as_ref() {
            tracing::debug!(
                "refresh_keyboard_focus: noop (locked={}, current={:?})",
                self.session_locked,
                current.as_ref().map(focus_target_label),
            );
            return;
        }
        tracing::info!(
            "refresh_keyboard_focus: locked={} current={:?} -> desired={:?}",
            self.session_locked,
            current.as_ref().map(focus_target_label),
            desired.as_ref().map(focus_target_label),
        );
        self.focus_surface(desired);
    }

    fn compute_desired_focus(&self) -> Option<FocusTarget> {
        if self.session_locked {
            // Lock surface on the output under the cursor wins, with
            // graceful fallbacks: focused-monitor's surface, then any
            // surface (so we never end up locked with no focus at all).
            let pointer_output = self
                .monitor_at_point(self.input_pointer.x, self.input_pointer.y)
                .and_then(|i| self.monitors.get(i).map(|m| m.output.clone()));

            if let Some(out) = pointer_output {
                if let Some((_, s)) = self.lock_surfaces.iter().find(|(o, _)| o == &out) {
                    return Some(FocusTarget::SessionLock(s.clone()));
                }
            }
            return self
                .lock_surfaces
                .first()
                .map(|(_, s)| FocusTarget::SessionLock(s.clone()));
        }

        // Highest-priority Exclusive keyboard layer. Per wlr-layer-shell,
        // Overlay outranks Top for input as well as paint — so an Exclusive
        // Overlay surface (e.g. the screenshot region selector) must beat a
        // Top-layer Exclusive menu (dashboard / quick-settings) left open
        // underneath it; otherwise the selector never receives Enter/Esc.
        // Scan Overlay first, then Top.
        use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer};
        for want in [WlrLayer::Overlay, WlrLayer::Top] {
            for layer in self.layer_shell_state.layer_surfaces().rev() {
                let exclusive = layer.with_cached_state(|data| {
                    data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                        && data.layer == want
                });
                if !exclusive {
                    continue;
                }
                let mapped = self.space.outputs().find_map(|output| {
                    let map = layer_map_for_output(output);
                    map.layers()
                        .find(|m| m.layer_surface() == &layer)
                        .map(|m| m.layer_surface().clone())
                });
                if let Some(s) = mapped {
                    return Some(FocusTarget::LayerSurface(s));
                }
            }
        }

        // Otherwise: monitor's last-selected client (focus history),
        // falling back to the topmost visible client on the same monitor.
        let mon_idx = self
            .pointer_monitor()
            .or_else(|| self.focused_client_idx().map(|i| self.clients[i].monitor))?;
        if mon_idx >= self.monitors.len() {
            return None;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        if let Some(idx) = self.monitors[mon_idx].selected.filter(|&i| {
            i < self.clients.len()
                && self.clients[i].monitor == mon_idx
                && self.clients[i].is_visible_on(mon_idx, tagset)
        }) {
            return Some(FocusTarget::Window(self.clients[idx].window.clone()));
        }
        let idx = self
            .clients
            .iter()
            .position(|c| c.monitor == mon_idx && c.is_visible_on(mon_idx, tagset))?;
        Some(FocusTarget::Window(self.clients[idx].window.clone()))
    }

    /// For scroller layout, return the client-vector index where a newly
    /// created window should land — right after the currently focused client
    /// on the same monitor. Returns `None` if the target monitor isn't using
    /// scroller (any layout) or if there's no focused client there.
    pub(crate) fn scroller_insert_position(&self, target_mon: usize) -> Option<usize> {
        let mon = self.monitors.get(target_mon)?;
        if mon.current_layout() != crate::layout::LayoutId::Scroller {
            return None;
        }
        let focused_idx = self.focused_client_idx()?;
        if self.clients[focused_idx].monitor != target_mon {
            return None;
        }
        Some(focused_idx + 1)
    }

    /// Inserting a client mid-vec invalidates any monitor.selected /
    /// prev_selected indices that pointed at positions ≥ insert position.
    /// Bump them up by one so they keep referring to the same client.
    pub(crate) fn shift_indices_at_or_after(&mut self, insert_pos: usize) {
        for mon in self.monitors.iter_mut() {
            if let Some(s) = mon.selected.as_mut() {
                if *s >= insert_pos {
                    *s += 1;
                }
            }
            if let Some(s) = mon.prev_selected.as_mut() {
                if *s >= insert_pos {
                    *s += 1;
                }
            }
        }
    }

    /// Inverse of `shift_indices_at_or_after`: a client at `removed_pos` was
    /// just dropped. Shift any monitor index pointing at a later position
    /// down by one, and clear those that pointed exactly at the removed slot.
    pub(crate) fn shift_indices_after_remove(&mut self, removed_pos: usize) {
        for mon in self.monitors.iter_mut() {
            for slot in [&mut mon.selected, &mut mon.prev_selected] {
                if let Some(s) = slot.as_mut() {
                    if *s == removed_pos {
                        *slot = None;
                    } else if *s > removed_pos {
                        *s -= 1;
                    }
                }
            }
        }
    }

    pub(crate) fn pointer_monitor(&self) -> Option<usize> {
        self.monitor_at_point(self.input_pointer.x, self.input_pointer.y)
    }

    /// Detect monitor crossings on pointer motion and refresh
    /// state snapshot when the cursor enters a new output. Cheap inside
    /// the common case (same monitor → integer compare, no I/O);
    /// only crossings — rare relative to motion events — pay the
    /// serialization cost. Keeps `state.active_output` in sync so
    /// `Super+Space` on an empty monitor opens the launcher there,
    /// not on whichever monitor most-recently had a focused client.
    pub fn refresh_pointer_monitor_tracking(&mut self) {
        let current = self.pointer_monitor();
        if self.input_pointer.last_monitor != current {
            self.input_pointer.last_monitor = current;
            // Crossing *into* a monitor makes the pointer the freshest
            // "where is the user" signal, so menu opens follow the cursor
            // (e.g. mouse onto an empty output → launcher opens there).
            // Leaving every output (`None`) doesn't flip the source — we
            // keep the last meaningful one.
            if current.is_some() {
                self.active_output_source = crate::state::ActiveOutputSource::Pointer;
            }
            self.mark_state_dirty();
        }
    }

    fn monitor_at_point(&self, x: f64, y: f64) -> Option<usize> {
        self.monitors.iter().position(|mon| {
            let area = mon.monitor_area;
            x >= area.x as f64
                && y >= area.y as f64
                && x < (area.x + area.width) as f64
                && y < (area.y + area.height) as f64
        })
    }

    pub fn clamp_pointer_to_outputs(&mut self) {
        if self.monitors.is_empty() {
            return;
        }

        let mut min_x = self.monitors[0].monitor_area.x;
        let mut min_y = self.monitors[0].monitor_area.y;
        let mut max_x = self.monitors[0].monitor_area.x + self.monitors[0].monitor_area.width;
        let mut max_y = self.monitors[0].monitor_area.y + self.monitors[0].monitor_area.height;

        for mon in &self.monitors[1..] {
            let area = mon.monitor_area;
            min_x = min_x.min(area.x);
            min_y = min_y.min(area.y);
            max_x = max_x.max(area.x + area.width);
            max_y = max_y.max(area.y + area.height);
        }

        self.input_pointer.x = self.input_pointer.x.clamp(min_x as f64, (max_x - 1) as f64);
        self.input_pointer.y = self.input_pointer.y.clamp(min_y as f64, (max_y - 1) as f64);
    }
}
