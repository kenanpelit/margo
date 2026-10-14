//! Keyboard-focus + pointer-monitor methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): the cluster computing *who has
//! focus* and *which monitor the pointer is on* — focus resolution
//! (`compute_desired_focus`/`refresh_keyboard_focus`), the scroller insert /
//! index-shift helpers, and pointer→monitor tracking + clamping. Pure
//! `MargoState` glue, no new types.

use super::*;

impl MargoState {
    pub fn focus_surface(&mut self, target: Option<FocusTarget>) {
        let _span = tracy_client::span!("focus_surface");
        // Once ext-session-lock has been accepted, only its own surfaces may
        // acquire focus. This central guard covers new toplevels, popup grabs,
        // pointer focus changes, and layer focus races in one place. `None`
        // remains allowed so lock acquisition can clear stale desktop focus.
        if self.session_locked
            && target.as_ref().is_some_and(|target| match target {
                FocusTarget::SessionLock(surface) => !Self::lock_surface_ready(surface),
                _ => true,
            })
        {
            return;
        }
        // W3.4: push to per-monitor focus_history when a new client takes
        // focus. Store stable ids, prune closed/moved entries, drop the newly
        // focused id from every old position, and cap the MRU depth.
        const FOCUS_HISTORY_DEPTH: usize = 5;
        if let Some(FocusTarget::Window(w)) = &target {
            let new_idx = self.client_idx(w);
            if let Some(idx) = new_idx {
                // MRU recency key for the Super+Tab switcher — but NOT while the
                // switcher is cycling (preview-focus must not rewrite the MRU
                // order; only the committed pick does, in `mru_confirm`).
                if self.mru_switcher.is_none() {
                    self.focus_counter += 1;
                    self.clients[idx].last_focus_serial = self.focus_counter;
                }
                // Urgency is an attention request, not a standing state —
                // clear it the moment the window actually becomes the
                // user's focus (X11 WM_HINTS / GTK urgency-hint
                // convention). `is_urgent` is still otherwise dead until
                // `request_activation` (xdg_activation.rs) sets it.
                self.clients[idx].is_urgent = false;
                let mon = self.clients[idx].monitor;
                let client_id = self.clients[idx].id;
                let clients = &self.clients;
                for (monitor_idx, monitor) in self.monitors.iter_mut().enumerate() {
                    monitor.focus_history.retain(|id| {
                        clients
                            .iter()
                            .any(|client| client.id == *id && client.monitor == monitor_idx)
                            && *id != client_id
                    });
                }
                if mon < self.monitors.len() {
                    let hist = &mut self.monitors[mon].focus_history;
                    hist.push_front(client_id);
                    while hist.len() > FOCUS_HISTORY_DEPTH {
                        hist.pop_back();
                    }
                }
            }
        }
        // Capture the *previously* focused client BEFORE we rewrite
        // the keyboard focus — we need the old + new pair to drive
        // the border-colour cross-fade animation below.
        let prev_focus_idx = self.focused_client_idx();

        // Track focus history per-monitor so toplevel_destroyed can recall
        // the previously focused window (niri-style).
        if let Some(FocusTarget::Window(ref w)) = target {
            if let Some(new_idx) = self.client_idx(w) {
                let mon_idx = self.clients[new_idx].monitor;
                if mon_idx < self.monitors.len() {
                    let cur = self.monitors[mon_idx].selected;
                    if cur != Some(new_idx) {
                        self.monitors[mon_idx].prev_selected = cur;
                        self.monitors[mon_idx].selected = Some(new_idx);
                    }
                }
            }
        }

        let serial = SERIAL_COUNTER.next_serial();
        self.set_keyboard_focus(target, serial);

        // Focus highlight cross-fade. When focus moves between two
        // windows, animate both: the outgoing window's border colour
        // fades from `focuscolor` toward `bordercolor`, the incoming
        // one fades the other way. `tick_animations` drives the
        // sample; `border::refresh` reads the in-flight colour from
        // `opacity_animation.current_border_color` and renders that
        // instead of the static color_for() value.
        //
        // Per-client `opacity` (focused_opacity / unfocused_opacity)
        // is animated through the same struct so an unfocused window
        // also dims smoothly instead of snapping to its dimmer alpha
        // — same trick mango/dwl uses but with the right curve.
        let new_focus_idx = self.focused_client_idx();
        if prev_focus_idx != new_focus_idx
            && self.config.animations
            && self.config.animation_duration_focus > 0
        {
            let now = crate::utils::now_ms();
            let dur = self.config.animation_duration_focus;
            // Outgoing: drop focus highlight back to bordercolor +
            // dim opacity to unfocused.
            if let Some(idx) = prev_focus_idx {
                if idx < self.clients.len() {
                    let initial_color = self.clients[idx].opacity_animation.current_border_color;
                    let initial_color = if initial_color == [0.0, 0.0, 0.0, 0.0] {
                        crate::border::color_for(self, idx, true)
                    } else {
                        initial_color
                    };
                    let initial_opacity = self.clients[idx].focused_opacity;
                    self.clients[idx].opacity_animation = OpacityAnimation {
                        running: true,
                        initial_opacity,
                        target_opacity: self.clients[idx].unfocused_opacity,
                        current_opacity: initial_opacity,
                        time_started: now,
                        duration: dur,
                        initial_border_color: initial_color,
                        target_border_color: crate::border::color_for(self, idx, false),
                        current_border_color: initial_color,
                    };
                }
            }
            // Incoming: ramp up to focuscolor + brighten opacity.
            if let Some(idx) = new_focus_idx {
                if idx < self.clients.len() {
                    let initial_color = self.clients[idx].opacity_animation.current_border_color;
                    let initial_color = if initial_color == [0.0, 0.0, 0.0, 0.0] {
                        crate::border::color_for(self, idx, false)
                    } else {
                        initial_color
                    };
                    let initial_opacity = self.clients[idx].unfocused_opacity;
                    self.clients[idx].opacity_animation = OpacityAnimation {
                        running: true,
                        initial_opacity,
                        target_opacity: self.clients[idx].focused_opacity,
                        current_opacity: initial_opacity,
                        time_started: now,
                        duration: dur,
                        initial_border_color: initial_color,
                        target_border_color: crate::border::color_for(self, idx, true),
                        current_border_color: initial_color,
                    };
                }
            }
        }

        // Refresh border colors so the focused/unfocused distinction
        // updates without waiting for the next arrange.
        let mut repaint_outputs = Vec::with_capacity(2);
        for idx in [prev_focus_idx, new_focus_idx].into_iter().flatten() {
            let Some(client) = self.clients.get(idx) else {
                continue;
            };
            let Some(output) = self
                .monitors
                .get(client.monitor)
                .filter(|monitor| monitor.enabled)
                .map(|monitor| monitor.output.clone())
            else {
                continue;
            };
            if !repaint_outputs.contains(&output) {
                repaint_outputs.push(output);
            }
            for output in self.space.outputs_for_element(&client.window) {
                if !repaint_outputs.contains(&output) {
                    repaint_outputs.push(output);
                }
            }
        }
        crate::border::refresh(self);
        if repaint_outputs.is_empty() {
            self.request_repaint();
        } else {
            for output in repaint_outputs {
                self.request_repaint_output(&output);
            }
        }

        // Mark dirty so IPC watch subscribers see the new focus (
        // waybar-dwl, …). The struct gets its title / appid /
        // fullscreen / floating fields from `focused_client_idx`,
        // which we just changed; without this the bar would keep
        // showing the previously-focused window's title until the
        // next tag-switch / arrange caused some other broadcast to
        // fire. mango broadcasts on every focus change too — this
        // is straight parity.
        if prev_focus_idx != new_focus_idx {
            // Raise the newly-focused window in the scene z-order (dwl's
            // `focusclient` does the same). Invisible for non-overlapping
            // layouts; for `deck` it's what actually brings the focused
            // stack card to the front. `focus_stack` re-arranges anyway,
            // but `exchange_stack`, `focuswindow` over IPC and pointer
            // focus don't — so do it here, where every focus change lands.
            self.enforce_z_order();
            self.recompute_idle_inhibit();
            self.mark_state_dirty();
            // Phase 3 scripting: invoke any `on_focus_change`
            // handlers the user registered in init.rhai. Hooks
            // see the new focused state via `focused_appid()` /
            // `focused_title()`. Wrapped in a `prev != new` gate
            // because focus_surface is called speculatively from
            // `refresh_keyboard_focus` and we don't want to fire
            // hooks for no-op refreshes.
            crate::scripting::fire_focus_change(self);

            // `windowrule = confine_pointer:1` (mango `045c7003`): if the
            // window we just switched focus to has it and the cursor is
            // currently outside its bounds, snap to center immediately
            // instead of waiting for the next motion event — regaining
            // focus on an already-running game shouldn't leave the
            // cursor sitting on another monitor for one more frame.
            if let Some(idx) = new_focus_idx
                && self.clients[idx].confine_pointer
            {
                let (min_x, min_y, max_x, max_y) = self.clients[idx].confine_box();
                if !(min_x..=max_x).contains(&self.input_pointer.x)
                    || !(min_y..=max_y).contains(&self.input_pointer.y)
                {
                    self.input_pointer.x = (min_x + max_x) / 2.0;
                    self.input_pointer.y = (min_y + max_y) / 2.0;
                }
            }
        }
    }

    pub(crate) fn lock_surface_ready(
        surface: &smithay::wayland::session_lock::LockSurface,
    ) -> bool {
        smithay::backend::renderer::utils::with_renderer_surface_state(
            surface.wl_surface(),
            |state| state.buffer().is_some(),
        )
        .unwrap_or(false)
    }

    pub fn focused_client_idx(&self) -> Option<usize> {
        let keyboard = self.seat.get_keyboard()?;
        let focus = keyboard.current_focus()?;
        if let FocusTarget::Window(focused) = focus {
            self.client_idx(&focused)
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

        // A `mosaic_overflow_stack` peek that just became the desired
        // focus (click, alt-tab, keybind — anything that routes through
        // `compute_desired_focus`) rejoins normal packing: clear the flag
        // and let the next `arrange_monitor` fold it back into
        // `mosaic_arrange`, same as `Client::is_mosaic_stacked`'s doc
        // comment promises.
        if let Some(FocusTarget::Window(w)) = &desired
            && let Some(idx) = self.client_idx(w)
            && self.clients[idx].is_mosaic_stacked
        {
            self.clients[idx].is_mosaic_stacked = false;
            let mon = self.clients[idx].monitor;
            if mon < self.monitors.len() {
                self.arrange_monitor(mon);
            }
        }

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
                if let Some((_, s)) = self
                    .lock_surfaces
                    .iter()
                    .find(|(o, surface)| o == &out && Self::lock_surface_ready(surface))
                {
                    return Some(FocusTarget::SessionLock(s.clone()));
                }
            }
            return self
                .lock_surfaces
                .iter()
                .find(|(_, surface)| Self::lock_surface_ready(surface))
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
                        .filter(|mapped| self.layer_accepts_input_on_output(output, mapped.layer()))
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

    /// Index of `window` in `clients`.
    pub(crate) fn client_idx(&self, window: &Window) -> Option<usize> {
        self.clients.iter().position(|c| c.window == *window)
    }

    /// Swap two entries of `clients`, keeping every monitor's
    /// `selected` / `prev_selected` on the same *windows*.
    pub(crate) fn swap_clients(&mut self, a: usize, b: usize) {
        self.clients.swap(a, b);
        self.remap_monitor_indices(|i| {
            if i == a {
                b
            } else if i == b {
                a
            } else {
                i
            }
        });
    }

    /// Move the client at `from` so it ends up at index `to` (remove +
    /// insert), keeping monitor index slots on the same windows.
    pub(crate) fn move_client_index(&mut self, from: usize, to: usize) {
        let client = self.clients.remove(from);
        self.clients.insert(to, client);
        self.remap_monitor_indices(|i| index_after_move(i, from, to));
    }

    fn remap_monitor_indices(&mut self, map: impl Fn(usize) -> usize) {
        for mon in &mut self.monitors {
            for slot in [&mut mon.selected, &mut mon.prev_selected] {
                if let Some(s) = slot.as_mut() {
                    *s = map(*s);
                }
            }
        }
    }

    /// Remove a closing window's stable identity from every monitor MRU.
    pub(crate) fn remove_focus_history_id(&mut self, client_id: u64) {
        for monitor in &mut self.monitors {
            monitor.focus_history.retain(|id| *id != client_id);
        }
    }

    /// Pointer moved / clicked / scrolled: re-show a hidden cursor and
    /// (re)start the `cursor_hide_timeout` countdown.
    pub(crate) fn note_pointer_activity(&mut self) {
        self.pointer_activity_at = std::time::Instant::now();
        if self.cursor_hidden {
            self.cursor_hidden = false;
            self.repaint_pointer_output();
        }
        self.arm_cursor_hide_timer();
    }

    /// Ensure the hide countdown is running when `cursor_hide_timeout` is
    /// set. One timer: it reschedules itself for the remainder whenever
    /// activity happened since it was armed, so motion at 1000 Hz never
    /// touches the event loop's source list.
    pub(crate) fn arm_cursor_hide_timer(&mut self) {
        let timeout = self.config.cursor_hide_timeout;
        if timeout == 0 {
            if let Some(token) = self.cursor_hide_timer.take() {
                self.loop_handle.remove(token);
            }
            if self.cursor_hidden {
                self.cursor_hidden = false;
                self.repaint_pointer_output();
            }
            return;
        }
        if self.cursor_hide_timer.is_some() {
            return;
        }
        let timeout = std::time::Duration::from_secs(u64::from(timeout));
        let timer = calloop::timer::Timer::from_duration(timeout);
        match self
            .loop_handle
            .insert_source(timer, |_, _, state: &mut MargoState| {
                let timeout =
                    std::time::Duration::from_secs(u64::from(state.config.cursor_hide_timeout));
                let idle = state.pointer_activity_at.elapsed();
                if state.config.cursor_hide_timeout > 0 && idle < timeout {
                    return calloop::timer::TimeoutAction::ToDuration(timeout - idle);
                }
                // Clear first: the source is dropped on return.
                state.cursor_hide_timer = None;
                if state.config.cursor_hide_timeout > 0 && !state.cursor_hidden {
                    state.cursor_hidden = true;
                    state.repaint_pointer_output();
                }
                calloop::timer::TimeoutAction::Drop
            }) {
            Ok(token) => self.cursor_hide_timer = Some(token),
            Err(e) => tracing::warn!("cursor hide timer: {e}"),
        }
    }

    fn repaint_pointer_output(&mut self) {
        let pos = smithay::utils::Point::<f64, smithay::utils::Logical>::from((
            self.input_pointer.x,
            self.input_pointer.y,
        ));
        let output = self.space.output_under(pos).next().cloned();
        if let Some(output) = output {
            self.request_repaint_output(&output);
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

/// Where the element at index `i` lands after moving the element at
/// `from` to `to` with `Vec::remove` + `Vec::insert`.
fn index_after_move(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && (from + 1..=to).contains(&i) {
        i - 1
    } else if to < from && (to..from).contains(&i) {
        i + 1
    } else {
        i
    }
}

#[cfg(test)]
mod index_tests {
    use super::index_after_move;

    #[test]
    fn index_after_move_matches_vec_remove_insert() {
        for len in 1..6usize {
            for from in 0..len {
                for to in 0..len {
                    let mut v: Vec<usize> = (0..len).collect();
                    let x = v.remove(from);
                    v.insert(to, x);
                    for (new_pos, &orig) in v.iter().enumerate() {
                        assert_eq!(index_after_move(orig, from, to), new_pos);
                    }
                }
            }
        }
    }
}
