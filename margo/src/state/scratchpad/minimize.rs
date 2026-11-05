//! Window minimize / restore on `MargoState`.
//!
//! A minimized window is stored the way a tucked-away scratchpad is: it
//! stays in `clients` with `is_minimized` set, is unmapped from the scene
//! and fails `is_visible_on`, so layout, focus, overview and render all
//! skip it. Unlike a scratchpad it keeps its tiled / floating /
//! fullscreen state and comes back to the layout as it was — a
//! scratchpad returns as a centred float.
//!
//! Who minimizes: the app (`xdg_toplevel.set_minimized`, the CSD minimize
//! button; an X11 `_NET_WM_STATE_HIDDEN` / iconify request), a taskbar
//! (`zwlr_foreign_toplevel_handle_v1.set_minimized`) and the `minimize`
//! action. Who restores: `restore_last` / `restore_all` / `restore_window`,
//! a taskbar's `unset_minimized`, and every activation path
//! (`activate_window_idx`: dock click, xdg-activation, `focuswindowid`) —
//! focusing an unmapped surface would strand the keyboard.
//!
//! Port of the hyprland-minimize community plugin, natively: its
//! `special:minimized` workspace is this flag, its `origins.json` is the
//! window's own `tags` / `monitor` (kept while minimized), and its
//! `restore_to = current | origin` is `minimize_restore_in_place`.

use super::super::{FocusTarget, MargoState};
use crate::layout::Rect;

/// Move a floating rect from monitor area `from` to `to`, keeping its
/// offset inside the area and clamping it on screen (a window restored
/// onto a smaller or differently placed monitor must not land off it).
pub(crate) fn rehome_float_rect(rect: Rect, from: Rect, to: Rect) -> Rect {
    let width = rect.width.min(to.width.max(1));
    let height = rect.height.min(to.height.max(1));
    let x = to.x + (rect.x - from.x);
    let y = to.y + (rect.y - from.y);
    Rect {
        x: x.clamp(to.x, to.x + to.width - width),
        y: y.clamp(to.y, to.y + to.height - height),
        width,
        height,
    }
}

impl MargoState {
    /// Indices of the user-minimized windows, oldest first.
    pub(crate) fn minimized_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.clients.len())
            .filter(|&i| self.clients[i].is_user_minimized())
            .collect();
        order.sort_by_key(|&i| self.clients[i].minimized_seq);
        order
    }

    /// Minimize client `idx`. A scratchpad is tucked away through its own
    /// path instead (it already has a hidden state and a toggle to come
    /// back with). No-op while locked, on a dying or not-yet-mapped
    /// window, or on one already minimized.
    pub(crate) fn minimize_client(&mut self, idx: usize) {
        if self.session_locked || idx >= self.clients.len() {
            return;
        }
        let c = &self.clients[idx];
        if c.is_killing || c.is_initial_map_pending || c.is_minimized {
            return;
        }
        if c.is_in_scratchpad {
            if c.is_scratchpad_show {
                self.hide_scratchpad_client(idx);
                self.mark_state_dirty();
            }
            return;
        }
        let was_focused = self.focused_client_idx() == Some(idx);
        let mon_idx = c.monitor;
        let seq = self
            .clients
            .iter()
            .map(|c| c.minimized_seq)
            .max()
            .unwrap_or(0)
            + 1;

        // Out of a tabbed group first: a group shows one member, and a
        // minimized active member would leave its slot empty.
        if let Some(gid) = self.clients[idx].group_id.take() {
            self.clients[idx].group_active = false;
            self.repair_group(gid);
        }
        let c = &mut self.clients[idx];
        c.is_minimized = true;
        c.minimized_seq = seq;
        let window = c.window.clone();
        if let Some(x11) = window.x11_surface() {
            let _ = x11.set_hidden(true);
        }
        self.space.unmap_elem(&window);
        if let Some(mon) = self.monitors.get_mut(mon_idx) {
            if mon.selected == Some(idx) {
                mon.selected = None;
            }
            if mon.prev_selected == Some(idx) {
                mon.prev_selected = None;
            }
        }
        if mon_idx < self.monitors.len() {
            self.arrange_monitor(mon_idx);
            // Only hand focus on when the minimized window had it: a dock
            // minimizing a background window must not move focus.
            if was_focused {
                self.focus_most_recent_visible_or_clear(mon_idx);
            }
        }
        self.mark_state_dirty();
        self.request_repaint();
    }

    /// Bring a minimized window back and focus it: onto the focused
    /// monitor's current tag, or (`minimize_restore_in_place`) to its own
    /// tag, switching the view there. No-op unless `idx` is minimized.
    pub(crate) fn restore_minimized(&mut self, idx: usize) {
        if self.session_locked
            || idx >= self.clients.len()
            || !self.clients[idx].is_user_minimized()
        {
            return;
        }
        self.clear_minimized(idx);
        let source_mon = self.clients[idx].monitor;
        if self.config.minimize_restore_in_place && source_mon < self.monitors.len() {
            let tags = self.clients[idx].tags;
            if tags & self.monitors[source_mon].current_tagset() == 0 {
                let lowest = tags & tags.wrapping_neg();
                self.view_tag_on(source_mon, if lowest != 0 { lowest } else { tags });
            }
            self.arrange_monitor(source_mon);
        } else {
            let target_mon = self.focused_monitor();
            if target_mon >= self.monitors.len() {
                return;
            }
            let tagset = self.monitors[target_mon].current_tagset();
            if tagset != 0 {
                self.clients[idx].tags = tagset;
            }
            if source_mon != target_mon && source_mon < self.monitors.len() {
                let (from, to) = (
                    self.monitors[source_mon].work_area,
                    self.monitors[target_mon].work_area,
                );
                let c = &mut self.clients[idx];
                c.float_geom = rehome_float_rect(c.float_geom, from, to);
                c.monitor = target_mon;
                self.arrange_monitor(source_mon);
            } else {
                self.clients[idx].monitor = target_mon;
            }
            self.arrange_monitor(target_mon);
        }
        let window = self.clients[idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window.clone())));
        self.space.raise_element(&window, true);
        self.enforce_z_order();
        self.mark_state_dirty();
        self.request_repaint();
    }

    /// Drop the minimized state (flag, order, X11 hidden hint) without
    /// placing or focusing the window — for callers that place it
    /// themselves (`summon`).
    pub(crate) fn clear_minimized(&mut self, idx: usize) {
        let Some(c) = self.clients.get_mut(idx) else {
            return;
        };
        if !c.is_minimized {
            return;
        }
        c.is_minimized = false;
        c.minimized_seq = 0;
        if let Some(x11) = c.window.x11_surface() {
            let _ = x11.set_hidden(false);
        }
    }

    /// Action `minimize`: the focused window.
    pub fn minimize_focused(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            self.minimize_client(idx);
        }
    }

    /// Action `minimize <ID>`: the window with stable id `id`.
    pub fn minimize_window_id(&mut self, id: u64) {
        if let Some(idx) = self.clients.iter().position(|c| c.id == id) {
            self.minimize_client(idx);
        }
    }

    /// Action `restore_window <ID>`.
    pub fn restore_minimized_id(&mut self, id: u64) {
        if let Some(idx) = self.clients.iter().position(|c| c.id == id) {
            self.restore_minimized(idx);
        }
    }

    /// Action `restore_last`: the most recently minimized window.
    pub fn restore_last_minimized(&mut self) {
        if let Some(&idx) = self.minimized_order().last() {
            self.restore_minimized(idx);
        }
    }

    /// Action `restore_all`: every minimized window, oldest first, so the
    /// most recently minimized one ends up focused on top. Restoring
    /// never adds or removes clients, so the indices stay valid.
    pub fn restore_all_minimized(&mut self) {
        for idx in self.minimized_order() {
            self.restore_minimized(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn float_rect_keeps_its_offset_on_the_new_monitor() {
        let from = r(0, 30, 1920, 1050);
        let to = r(1920, 0, 2560, 1440);
        assert_eq!(
            rehome_float_rect(r(100, 130, 800, 600), from, to),
            r(2020, 100, 800, 600)
        );
    }

    #[test]
    fn float_rect_is_clamped_onto_a_smaller_monitor() {
        let from = r(0, 0, 2560, 1440);
        let to = r(2560, 0, 1280, 800);
        // Bottom-right corner window on a big screen: pulled back on.
        assert_eq!(
            rehome_float_rect(r(1800, 900, 700, 500), from, to),
            r(2560 + 1280 - 700, 800 - 500, 700, 500)
        );
        // Larger than the target: shrunk to it, pinned to its origin.
        assert_eq!(
            rehome_float_rect(r(0, 0, 2000, 1000), from, to),
            r(2560, 0, 1280, 800)
        );
    }
}
