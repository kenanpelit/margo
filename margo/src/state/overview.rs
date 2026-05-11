//! Overview (mango-ext-style grid + niri-style alt-Tab cycle) methods
//! on `MargoState`. Extracted from `state.rs` (roadmap Q1) so the
//! overview cycle/grid logic gets its own translation unit — touching
//! the cycle order or hover semantics no longer recompiles the rest
//! of the compositor.
//!
//! Public surface mirrors the original (`open_overview`,
//! `close_overview`, `toggle_overview`, `overview_focus_next/prev`,
//! `overview_activate`, `is_overview_open`, plus the
//! `overview_visible_clients_for_monitor` helper used by
//! `arrange_monitor`). Private helpers (`overview_transition_ms`,
//! `overview_visible_clients`, `overview_focus_step`) stay private to
//! this module via the inherent-impl scope rules.

use smithay::desktop::Window;

use super::{FocusTarget, MargoClient, MargoState};

impl MargoState {
    /// Geometric rect of the tag-thumbnail cell for `tag` (1..=9) on
    /// `mon_idx`. Returns `None` if the tag is out of range or the
    /// monitor doesn't exist. Same math as
    pub fn is_overview_open(&self) -> bool {
        self.monitors.iter().any(|mon| mon.is_overview)
    }

    /// Snappy overview transition duration (ms). Hard-coded for now —
    /// `animation_duration_move` defaults to 250 ms and the per-window
    /// move animation across N tiles is what made the previous
    /// overview feel laggy. 180 ms with the user's configured easing
    /// curve gives a smooth grid-zoom that still reads as animated.
    /// Fallback overview transition duration. `Config::overview_transition_ms`
    /// overrides this when non-zero; the default config value also
    /// happens to be 180 so behaviour is unchanged unless the user
    /// tunes it. See [`overview_transition_ms`] for the live read.
    const OVERVIEW_TRANSITION_MS: u32 = 180;

    /// Live overview-transition duration: config knob if set, else the
    /// hard-coded fallback. Used by `open_overview` / `close_overview`
    /// to seed `overview_transition_animation_ms`.
    fn overview_transition_ms(&self) -> u32 {
        let cfg = self.config.overview_transition_ms;
        if cfg > 0 { cfg } else { Self::OVERVIEW_TRANSITION_MS }
    }

    pub fn open_overview(&mut self) {
        // Collect the indices of monitors that actually flip into
        // overview on this call. Any monitor already in overview is
        // skipped — re-flipping would clobber `overview_backup_tagset`
        // with the all-tags overview tagset and the close path would
        // restore to `!0` (every tag) on every monitor. That was the
        // root cause of the "tüm pencereler aynı tag'da kalıyor"
        // regression in 8c58b20: the previous attempt mutated state
        // before deciding whether the monitor actually changed.
        let mut flipped: Vec<usize> = Vec::new();
        for (i, mon) in self.monitors.iter_mut().enumerate() {
            if !mon.is_overview {
                mon.overview_backup_tagset = mon.current_tagset().max(1);
                mon.is_overview = true;
                flipped.push(i);
            }
        }

        if flipped.is_empty() {
            return;
        }

        // NOTE: we deliberately don't reset `overview_cycle_pending`
        // here. `open_overview` is reachable from inside
        // `overview_focus_step` (alt+Tab while overview is closed → we
        // open + cycle in one call), and the input handler has ALREADY
        // set `overview_cycle_pending` + `overview_cycle_modifier_mask`
        // by the time we get here. Resetting them would clobber the
        // alt+Tab muscle memory — Alt release wouldn't auto-commit
        // because the flag the release branch reads is false.
        // `close_overview` and `overview_activate` handle the lifetime
        // of the flag on the way out.

        // Snappy 180 ms slide into the grid (vs the user's possibly
        // 250+ ms `animation_duration_move`). The per-client move
        // animation in arrange_monitor reads this override and falls
        // back to the configured value when None.
        self.overview_transition_animation_ms = Some(self.overview_transition_ms());
        self.arrange_monitors(&flipped);
        self.overview_transition_animation_ms = None;
        crate::protocols::dwl_ipc::broadcast_all(self);
    }

    pub fn close_overview(&mut self, activate_window: Option<Window>) {
        let was_open = self.is_overview_open();
        if !was_open {
            return;
        }

        // Drop any pending alt+Tab commit — overview is closing now,
        // a stray modifier-release after this point shouldn't trigger
        // a second `overview_activate` (which would reopen overview).
        self.overview_cycle_pending = false;
        self.overview_cycle_modifier_mask = margo_config::Modifiers::empty();

        let previous_focus = self.focused_client_idx();
        // Fallback chain for "which client should be focused after
        // close":
        //   1. The explicit `activate_window` arg (mouse click on a
        //      thumbnail, `overview_activate` action).
        //   2. The currently-hovered thumbnail — covers keyboard
        //      navigation followed by `Esc` / `alt+Tab` /
        //      `toggleoverview`. Without this, `alt+ctrl+Tab` would
        //      shift the visible highlight but `previous_focus`
        //      would yank focus back to whatever was active before
        //      overview opened, defeating the entire navigation.
        //   3. `previous_focus` below — pre-overview focused client,
        //      used when no thumbnail was ever hovered.
        let activate_idx = activate_window
            .as_ref()
            .and_then(|window| self.clients.iter().position(|client| &client.window == window))
            .or_else(|| self.clients.iter().position(|c| c.is_overview_hovered));

        // Same targeting as open_overview: only arrange the monitors
        // that actually leave overview state. Track them up-front so
        // the tagset restore + arrange operate on the same set even
        // if some side effect somewhere later mutates `is_overview`.
        let mut flipped: Vec<usize> = Vec::new();
        for mon_idx in 0..self.monitors.len() {
            if !self.monitors[mon_idx].is_overview {
                continue;
            }

            let seltags = self.monitors[mon_idx].seltags;
            let backup = self.monitors[mon_idx].overview_backup_tagset.max(1);
            let target_tagset = activate_idx
                .filter(|&idx| self.clients[idx].monitor == mon_idx)
                .map(|idx| {
                    let tags = self.clients[idx].tags;
                    let backup_intersection = tags & backup;
                    if backup_intersection != 0 {
                        backup_intersection
                    } else {
                        tags & tags.wrapping_neg()
                    }
                })
                .filter(|tagset| *tagset != 0)
                .unwrap_or(backup);

            self.monitors[mon_idx].is_overview = false;
            self.monitors[mon_idx].tagset[seltags] = target_tagset;
            self.update_pertag_for_tagset(mon_idx, target_tagset);
            flipped.push(mon_idx);
        }

        // Clear hover state on every client — overview is gone, the
        // border layer should drop back to its non-overview palette
        // immediately. Doing this before arrange means the very next
        // border::refresh sees a coherent post-overview world.
        for client in self.clients.iter_mut() {
            client.is_overview_hovered = false;
        }

        self.overview_transition_animation_ms = Some(self.overview_transition_ms());
        self.arrange_monitors(&flipped);
        self.overview_transition_animation_ms = None;

        let focus_idx = activate_idx.or(previous_focus).filter(|&idx| {
            self.monitors
                .get(self.clients[idx].monitor)
                .is_some_and(|mon| {
                    self.clients[idx].is_visible_on(
                        self.clients[idx].monitor,
                        mon.current_tagset(),
                    )
                })
        });

        if let Some(idx) = focus_idx {
            let mon_idx = self.clients[idx].monitor;
            if mon_idx < self.monitors.len() {
                self.monitors[mon_idx].selected = Some(idx);
            }
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
        } else {
            let mon_idx = self.focused_monitor();
            self.focus_first_visible_or_clear(mon_idx);
        }

        crate::protocols::dwl_ipc::broadcast_all(self);
    }

    pub fn toggle_overview(&mut self) {
        if self.is_overview_open() {
            self.close_overview(None);
        } else {
            self.open_overview();
        }
    }

    /// All clients shown as overview thumbnails, in the order
    /// `alt+Tab` should walk them. Driven by
    /// `Config::overview_cycle_order`:
    ///
    /// * `Mru` — `focus_history` first (most-recent first), then
    ///   any remaining visible clients in clients-vec order.
    ///   Matches i3/sway/Hypr/niri/GNOME muscle memory.
    /// * `Tag` — tag 1 → 9 in order, clients-vec order inside each
    ///   tag. Spatial-memory: tag 1's windows always first, tag 9's
    ///   always last, independent of focus history.
    /// * `Mixed` — current tag's clients in MRU order, then the
    ///   remaining tags in strict tag order. "MRU where you live,
    ///   tag elsewhere."
    fn overview_visible_clients(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for mon_idx in 0..self.monitors.len() {
            if !self.monitors[mon_idx].is_overview {
                continue;
            }
            out.extend(self.overview_visible_clients_for_monitor(mon_idx));
        }
        out
    }

    /// Per-monitor variant of `overview_visible_clients`. Returns the
    /// thumbnail order both `overview_focus_step` (cycle) and
    /// `arrange_monitor` (visual grid) use, so left-to-right in the
    /// overview reflects MRU / tag / mixed depending on
    /// `Config::overview_cycle_order`. Decoupled from the multi-monitor
    /// path so the arrange-time call site can request just this
    /// monitor's slice.
    pub(crate) fn overview_visible_clients_for_monitor(
        &self,
        mon_idx: usize,
    ) -> Vec<usize> {
        use margo_config::OverviewCycleOrder;
        let mut out = Vec::new();
        let mut seen: std::collections::HashSet<usize> =
            std::collections::HashSet::new();

        let visible_here = |i: usize, c: &MargoClient| -> bool {
            c.monitor == mon_idx
                && !c.is_initial_map_pending
                && !c.is_minimized
                && !c.is_killing
                && !c.is_in_scratchpad
                && i < self.clients.len()
        };

        let push_mru = |out: &mut Vec<usize>,
                        seen: &mut std::collections::HashSet<usize>,
                        tag_filter: u32| {
            for &i in &self.monitors[mon_idx].focus_history {
                if i >= self.clients.len() {
                    continue;
                }
                let c = &self.clients[i];
                if tag_filter != 0 && (c.tags & tag_filter) == 0 {
                    continue;
                }
                if visible_here(i, c) && seen.insert(i) {
                    out.push(i);
                }
            }
        };
        let push_tag_order = |out: &mut Vec<usize>,
                              seen: &mut std::collections::HashSet<usize>,
                              skip_tags: u32| {
            for tag_idx in 0..crate::layout::MAX_TAGS as u32 {
                let tag_bit = 1u32 << tag_idx;
                if (skip_tags & tag_bit) != 0 {
                    continue;
                }
                for (i, c) in self.clients.iter().enumerate() {
                    if (c.tags & tag_bit) == 0 {
                        continue;
                    }
                    if visible_here(i, c) && seen.insert(i) {
                        out.push(i);
                    }
                }
            }
        };

        match self.config.overview_cycle_order {
            OverviewCycleOrder::Mru => {
                push_mru(&mut out, &mut seen, 0);
                // Trailing tail: anything `focus_history` never
                // touched (newly-mapped, never-focused) goes at the
                // end in clients-vec order. Without this a brand-new
                // window would be unreachable via alt+Tab until it
                // gained focus once.
                for (i, c) in self.clients.iter().enumerate() {
                    if visible_here(i, c) && seen.insert(i) {
                        out.push(i);
                    }
                }
            }
            OverviewCycleOrder::Tag => {
                push_tag_order(&mut out, &mut seen, 0);
            }
            OverviewCycleOrder::Mixed => {
                // Current tag(set) in MRU order: covers the common
                // case where the user is rapidly alternating between
                // two windows on the active tag. Remaining tags fall
                // back to strict tag order for predictability.
                let cur_tagset = self.monitors[mon_idx].current_tagset();
                push_mru(&mut out, &mut seen, cur_tagset);
                push_tag_order(&mut out, &mut seen, cur_tagset);
            }
        }
        out
    }

    pub fn overview_focus_next(&mut self) {
        self.overview_focus_step(1);
    }

    pub fn overview_focus_prev(&mut self) {
        self.overview_focus_step(-1);
    }

    /// Cycle the overview thumbnail one step in `dir` (+1 = next,
    /// −1 = prev). niri-style keyboard-first MRU navigator:
    ///
    /// * **Overview closed?** Open it first. The first cycle press
    ///   then lands on the natural starting thumbnail (first for +1,
    ///   last for −1) — single-keystroke "open + select first".
    /// * **Cycle wrap-around** matches alt+Tab on every other DE.
    /// * **Focus follows the cycle** — every step calls
    ///   `focus_surface(Some(FocusTarget::Window(...)))` so the
    ///   border immediately repaints with `focuscolor`, smithay's
    ///   keyboard focus is on the new thumbnail's window, and
    ///   activating it later (Enter / `overview_activate`) is just
    ///   `close_overview(focus)`. Overview stays open between
    ///   cycles — user keeps tapping Tab to walk the MRU.
    /// * **Pointer warp** to thumbnail centre keeps the next mouse
    ///   motion from yanking hover off the keyboard-selected
    ///   thumbnail.
    fn overview_focus_step(&mut self, dir: i32) {
        // First press while closed = open + select natural start.
        if !self.is_overview_open() {
            self.open_overview();
        }
        let list = self.overview_visible_clients();
        if list.is_empty() {
            return;
        }
        // Anchor the cycle. Priority:
        //   1. An already-hovered thumbnail (keyboard cycle in progress).
        //   2. The currently-focused client's position in the list.
        //      With MRU order that's index 0 (the focused window is
        //      the freshest entry in `focus_history`), so the first
        //      `dir = +1` press lands on index 1 = the previously-used
        //      window. This is the standard alt+Tab behaviour every
        //      other DE ships: one tap moves you to the *other* MRU
        //      window, not back to yourself. With `tag` / `mixed`
        //      modes the focused window's index can be anywhere in
        //      the list, but the same "step away from focused" rule
        //      gives the user a meaningful first move.
        //   3. Fall through to position 0 / n-1 only if no client is
        //      focused (empty workspace, lock screen edge cases).
        let cur = list
            .iter()
            .position(|&i| self.clients[i].is_overview_hovered)
            .or_else(|| {
                self.focused_client_idx()
                    .and_then(|f| list.iter().position(|&i| i == f))
            });
        let n = list.len() as i32;
        let next_pos = match cur {
            Some(p) => (((p as i32 + dir).rem_euclid(n)) + n).rem_euclid(n),
            None => {
                if dir > 0 {
                    0
                } else {
                    n - 1
                }
            }
        } as usize;
        let new_idx = list[next_pos];

        for &i in &list {
            self.clients[i].is_overview_hovered = false;
        }
        self.clients[new_idx].is_overview_hovered = true;

        // Note: deliberately NO `arrange_monitor` here. Mango-ext
        // overview is a Grid layout — every cell stays put across
        // a cycle, only the *selected* state changes. Skipping the
        // arrange means the only state that flips this tick is
        // `is_overview_hovered`, which `border::refresh` reads on
        // the very next frame. Result: the focuscolor border lights
        // up the new selection in a single render, no animation
        // gate, no per-client move recompute, no opacity
        // crossfade — what the user calls "instant."

        // Pointer warp to thumbnail centre so a subsequent mouse
        // motion doesn't yank `is_overview_hovered` off our
        // keyboard pick. Geometry is steady (no in-flight arrange)
        // so the centre we compute is the cell the user is about
        // to click.
        let g = self.clients[new_idx].geom;
        if g.width > 0 && g.height > 0 {
            self.input_pointer.x = (g.x + g.width / 2) as f64;
            self.input_pointer.y = (g.y + g.height / 2) as f64;
            self.clamp_pointer_to_outputs();
        }

        // Don't call `focus_surface` here. While overview is open,
        // `border::refresh` already paints `is_overview_hovered`
        // with `focuscolor` (margo/src/border.rs:64), so the border
        // colour tracks the selection without going through the
        // smithay focus path. Calling `focus_surface` on every Tab
        // press also kicks off an opacity-crossfade animation per
        // step (state.rs:200-208) and shuffles dwl-ipc focus_history
        // — both visible side-effects that made the cycle feel
        // sluggish ("border yerine sadece imleç dolaşıyor"). The
        // user commits the cycle's choice via `overview_activate`
        // (Enter), which closes the overview onto the hovered
        // thumbnail and runs the focus path once.
        crate::border::refresh(self);
        self.request_repaint();
        tracing::debug!(
            target: "overview",
            dir = dir,
            new_idx = new_idx,
            list_len = list.len(),
            "cycle",
        );
    }

    /// Close overview activating whichever thumbnail keyboard
    /// navigation last highlighted (or the cursor-hovered one).
    /// No-op outside overview. With no hover set, falls through to
    /// `close_overview(None)` which restores the pre-overview tag
    /// without changing focus.
    pub fn overview_activate(&mut self) {
        if !self.is_overview_open() {
            return;
        }
        let window = self
            .clients
            .iter()
            .find(|c| c.is_overview_hovered)
            .map(|c| c.window.clone());
        self.close_overview(window);
    }
}
