//! Monitor dispatch actions: focus another monitor, move a client or
//! its tags to one, and step through the enabled monitors.

use crate::state::{FocusTarget, MargoState};

impl MargoState {
    pub fn focus_mon(&mut self, direction: i32) {
        if self.monitors.len() <= 1 {
            return;
        }
        let current = self.focused_monitor();
        let Some(next) = self.step_enabled_monitor(current, direction) else {
            return;
        };

        // Warp the cursor to the target monitor before changing
        // focus. Without this, sloppy-focus snaps focus right back
        // to whatever client the pointer is hovering on the source
        // output as soon as any motion event arrives — the
        // "Super+A bastım hiçbir şey olmuyor" symptom. The cursor
        // also has to actually leave the source monitor or the
        // user sees no feedback at all when the target side is
        // empty.
        let area = self.monitors[next].monitor_area;
        self.input_pointer.x = (area.x + area.width / 2) as f64;
        self.input_pointer.y = (area.y + area.height / 2) as f64;
        self.clamp_pointer_to_outputs();

        // Focus selection on the target monitor, in priority order:
        //   1. The monitor's stored `selected` if that client is
        //      still visible (Super+A → ... → Super+A geri dönüşte
        //      aynı pencere muscle memory).
        //   2. First visible client on the active tagset.
        //   3. None — clear focus so subsequent key events don't
        //      keep flowing into the *source* monitor's client.
        let tagset = self.monitors[next].current_tagset();
        let target_idx = self.monitors[next]
            .selected
            .filter(|&i| i < self.clients.len() && self.clients[i].is_visible_on(next, tagset))
            .or_else(|| {
                self.clients
                    .iter()
                    .position(|c| c.is_visible_on(next, tagset))
            });

        if let Some(idx) = target_idx {
            let window = self.clients[idx].window.clone();
            self.monitors[next].selected = Some(idx);
            self.focus_surface(Some(FocusTarget::Window(window)));
        } else {
            self.focus_surface(None);
        }

        self.mark_state_dirty();
        self.mark_state_dirty();
        // We warped the pointer ourselves (without going through
        // libinput's motion handler), so the
        // `refresh_pointer_monitor_tracking` call there never
        // fires. Run it manually so state snapshot's `active_output`
        // updates immediately — otherwise Super+Space right after
        // focusmon would still target the previous monitor.
        self.refresh_pointer_monitor_tracking();
        self.request_repaint();
    }

    /// Re-home client `idx` onto `target_mon` with `tags`: animate it off
    /// the source, drop it from the source's `selected` / `prev_selected`,
    /// and arrange both monitors. Shared by `tagmon` and an
    /// `xdg_toplevel.set_fullscreen(output)` aimed at another monitor.
    pub(crate) fn move_client_to_monitor(&mut self, idx: usize, target_mon: usize, tags: u32) {
        if idx >= self.clients.len() || target_mon >= self.monitors.len() {
            return;
        }
        let source_mon = self.clients[idx].monitor;
        if source_mon == target_mon {
            self.clients[idx].tags = tags;
            self.arrange_monitor(target_mon);
            return;
        }
        // Source side never gets a normal arrange pass for this client
        // again (its `.monitor` is about to point elsewhere), so without
        // this it just vanishes off the source screen mid-frame.
        self.animate_tag_departure(idx);
        self.clients[idx].monitor = target_mon;
        self.clients[idx].tags = tags;
        if let Some(source) = self.monitors.get_mut(source_mon) {
            if source.selected == Some(idx) {
                source.selected = None;
            }
            if source.prev_selected == Some(idx) {
                source.prev_selected = None;
            }
        }
        self.arrange_monitor(source_mon);
        self.arrange_monitor(target_mon);
    }

    /// The next enabled monitor after `from` in `direction` (wrapping),
    /// skipping soft-disabled outputs; `None` when there is no other one.
    pub(crate) fn step_enabled_monitor(&self, from: usize, direction: i32) -> Option<usize> {
        let len = self.monitors.len();
        if len == 0 {
            return None;
        }
        (1..len)
            .map(|step| {
                if direction > 0 {
                    (from + step) % len
                } else {
                    (from + len - step % len) % len
                }
            })
            .find(|&i| i != from && self.monitors[i].enabled)
    }

    pub fn tag_mon(&mut self, direction: i32) {
        if self.monitors.len() <= 1 {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current_mon = self.clients[idx].monitor;
        let Some(target_mon) = self.step_enabled_monitor(current_mon, direction) else {
            return;
        };
        let tagset = self.monitors[target_mon].current_tagset();
        self.move_client_to_monitor(idx, target_mon, tagset);

        // Follow the window across monitors. Without this the
        // cursor (and therefore sloppy-focus + the visible
        // selection) stays on the source output — the user moves a
        // window with `tagmon` or the matching 3-finger gesture and
        // the pointer is suddenly stranded on the empty side. Warp
        // first, then refocus through the standard path so border /
        // IPC watchers / scripting hooks all see a single coherent
        // "window is here now" event.
        let g = self.clients[idx].geom;
        if g.width > 0 && g.height > 0 {
            self.input_pointer.x = (g.x + g.width / 2) as f64;
            self.input_pointer.y = (g.y + g.height / 2) as f64;
        } else {
            // Degenerate slot — fall back to the target monitor's
            // centre so we at least leave the source output.
            let area = self.monitors[target_mon].monitor_area;
            self.input_pointer.x = (area.x + area.width / 2) as f64;
            self.input_pointer.y = (area.y + area.height / 2) as f64;
        }
        self.clamp_pointer_to_outputs();

        let window = self.clients[idx].window.clone();
        self.monitors[target_mon].selected = Some(idx);
        self.focus_surface(Some(FocusTarget::Window(window)));

        self.mark_state_dirty();
        self.mark_state_dirty();
        self.request_repaint();
    }
}
