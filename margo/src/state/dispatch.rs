//! Dispatch-action methods on `MargoState`. Extracted from `state.rs`
//! (roadmap Q1). These are the entry points every keybind and IPC
//! command lands in: focus stack navigation, tag mask flips, layout
//! switches, sticky/floating/fullscreen toggles, gap and master
//! tuning, monitor warps. Roughly 1250 LOC of "user typed Super+J,
//! make it happen" plumbing — pure compositor commands, none of which
//! belong in the central state.rs translation unit.
//!
//! Everything in here is an inherent method on `MargoState`; the lift
//! is signature-preserving so call sites (binds, mctl, scripting) are
//! unchanged. Tag-switching adaptive layouts, canvas pan/reset,
//! per-monitor focus warp, exclusive-fullscreen suppression — all live
//! here together because they share the same "mutate state then
//! arrange + broadcast" idiom, and grouping them keeps the touch
//! surface for adding a new dispatch action confined to one file.

use smithay::{desktop::WindowSurface, wayland::seat::WaylandFocus};

use super::{
    read_toplevel_identity, ClosingClient, FocusTarget, FullscreenMode, MargoState,
    WindowRuleReason,
};
use crate::layout::LayoutId;

impl MargoState {
    // ── Actions ───────────────────────────────────────────────────────────────

    pub fn kill_focused(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            if let WindowSurface::Wayland(toplevel) = self.clients[idx].window.underlying_surface() {
                toplevel.send_close();
            }
        }
    }

    pub fn focus_stack(&mut self, direction: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();

        let visible: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset))
            .map(|(i, _)| i)
            .collect();

        if visible.is_empty() {
            return;
        }

        let len = visible.len();
        let current_pos = self
            .focused_client_idx()
            .and_then(|ci| visible.iter().position(|&vi| vi == ci))
            .unwrap_or(0);

        let new_pos = if direction > 0 {
            (current_pos + 1) % len
        } else {
            (current_pos + len - 1) % len
        };

        let new_idx = visible[new_pos];
        self.monitors[mon_idx].prev_selected = self.monitors[mon_idx].selected;
        self.monitors[mon_idx].selected = Some(new_idx);
        let window = self.clients[new_idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window)));
        self.arrange_monitor(mon_idx);
    }

    pub fn exchange_stack(&mut self, direction: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();

        let visible: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset))
            .map(|(i, _)| i)
            .collect();

        if visible.len() < 2 {
            return;
        }

        let Some(current_idx) = self.focused_client_idx() else {
            return;
        };
        let Some(current_pos) = visible.iter().position(|&idx| idx == current_idx) else {
            return;
        };

        let len = visible.len();
        let target_pos = if direction > 0 {
            (current_pos + 1) % len
        } else {
            (current_pos + len - 1) % len
        };
        let target_idx = visible[target_pos];
        let window = self.clients[current_idx].window.clone();
        self.clients.swap(current_idx, target_idx);
        self.arrange_monitor(mon_idx);
        self.focus_surface(Some(FocusTarget::Window(window)));
    }

    pub fn view_tag(&mut self, tagmask: u32) {
        if tagmask == 0 {
            return;
        }
        // If a tagrule pins this tag to a specific monitor, jump focus
        // there first so multi-monitor users get niri-style "tag 7 is on
        // eDP-1, super+7 from anywhere takes me to it" behaviour. We
        // skip the redirect when the user is already on the home
        // monitor or the tagmask is the all-tags special value.
        let current_mon = self.focused_monitor();
        let mon_idx = if tagmask != u32::MAX {
            if let Some(home) = self.tag_home_monitor(tagmask) {
                if home != current_mon && home < self.monitors.len() {
                    self.warp_focus_to_monitor(home);
                }
                home
            } else {
                current_mon
            }
        } else {
            current_mon
        };
        if mon_idx >= self.monitors.len() {
            return;
        }
        let seltags = self.monitors[mon_idx].seltags;
        let current = self.monitors[mon_idx].tagset[seltags];

        // dwm/mango pattern: tagset has two slots. The "active" slot is
        // tagset[seltags]; the other slot remembers the previously viewed
        // tagmask. If the user re-presses the binding for the *current*
        // tag, swap the two slots so we land on the previous tag — like
        // alt-tab for workspaces.
        let new_tagmask = if current == tagmask {
            let other = self.monitors[mon_idx].tagset[seltags ^ 1];
            if other == 0 || other == current {
                // No meaningful previous tag → no-op (don't toggle into
                // an empty/identical state).
                return;
            }
            self.monitors[mon_idx].seltags = seltags ^ 1;
            other
        } else {
            // First press of a different tag: stash current as "previous"
            // in the other slot, then write new mask into active slot.
            self.monitors[mon_idx].tagset[seltags ^ 1] = current;
            self.monitors[mon_idx].tagset[seltags] = tagmask;
            tagmask
        };

        // ── Tag transition animation ──────────────────────────────
        //
        // Before flipping the tagset we:
        //
        //   * Capture every client that's about to become invisible
        //     into a `ClosingClient` with `kind = Slide(direction)`
        //     and `is_close = true`. The renderer will draw them
        //     sliding off-screen for `animation_duration_tag` ms;
        //     when settled they pop off the list. (Outgoing windows
        //     stay rendered through the transition so the user sees
        //     them leaving, instead of winking out instantly.)
        //
        //   * Stage every client that's about to become visible at
        //     an off-screen geom so `arrange_monitor` (called below)
        //     starts a Move animation from off-screen → target slot.
        //     That gives the inbound slide for free; we don't need
        //     a second render path.
        //
        // Direction: derived from the bit-position delta of the tag
        // mask. Going to a higher tag → enter from the right / bottom;
        // going to a lower tag → enter from the left / top. Niri does
        // the same; mango's vertical mode swaps the axis.
        let do_anim = self.config.animations
            && self.config.animation_duration_tag > 0
            && current != new_tagmask;
        let direction = self.config.tag_animation_direction;
        let mon_geom = self.monitors[mon_idx].monitor_area;
        let new_idx = current.trailing_zeros() as i32;
        let old_idx_target = new_tagmask.trailing_zeros() as i32;
        let going_forward = old_idx_target > new_idx;
        // Offscreen *staging* origin for the inbound slide. We only set
        // x/y here; the size is taken from the client's previous c.geom
        // below so the animation is a pure translate (no size change,
        // no resize-snapshot capture, no scaling artefacts). The
        // previous version of this code stored a 1×1 rect here, which
        // forced arrange_monitor to start a `1×1 → target.size` move
        // animation; arrange flagged `slot_size_changed` and the
        // renderer ran the resize-snapshot crossfade scaled from a
        // tiny rect up to the slot. That's the "border kadar hızlı
        // hareket etmiyor, sonra yerine oturuyor" symptom — the
        // border tracked the interpolated *slot*, but the snapshot
        // visually expanded from a point because the start size was
        // degenerate.
        let (off_in_xy, off_out_xy): ((i32, i32), (i32, i32)) = match (direction, going_forward) {
            (margo_config::TagAnimDirection::Horizontal, true) => (
                (mon_geom.x + mon_geom.width + 50, mon_geom.y),
                (mon_geom.x - mon_geom.width - 50, mon_geom.y),
            ),
            (margo_config::TagAnimDirection::Horizontal, false) => (
                (mon_geom.x - mon_geom.width - 50, mon_geom.y),
                (mon_geom.x + mon_geom.width + 50, mon_geom.y),
            ),
            (margo_config::TagAnimDirection::Vertical, true) => (
                (mon_geom.x, mon_geom.y + mon_geom.height + 50),
                (mon_geom.x, mon_geom.y - mon_geom.height - 50),
            ),
            (margo_config::TagAnimDirection::Vertical, false) => (
                (mon_geom.x, mon_geom.y - mon_geom.height - 50),
                (mon_geom.x, mon_geom.y + mon_geom.height + 50),
            ),
        };
        let _ = off_out_xy;
        let slide_dir = match (direction, going_forward) {
            (margo_config::TagAnimDirection::Horizontal, true) => {
                crate::render::open_close::SlideDirection::Left
            }
            (margo_config::TagAnimDirection::Horizontal, false) => {
                crate::render::open_close::SlideDirection::Right
            }
            (margo_config::TagAnimDirection::Vertical, true) => {
                crate::render::open_close::SlideDirection::Up
            }
            (margo_config::TagAnimDirection::Vertical, false) => {
                crate::render::open_close::SlideDirection::Down
            }
        };
        let now = crate::utils::now_ms();

        // Snapshot outgoing clients into the close-animation pipeline.
        // We DON'T touch the live `clients` vec — those entries stay
        // around but become invisible per the new tagset; the render
        // path skips them naturally. The snapshot we push here uses
        // the same OpenCloseRenderElement as the toplevel close path,
        // just with a slide kind instead of zoom.
        if do_anim {
            for c in self.clients.iter() {
                if c.monitor != mon_idx {
                    continue;
                }
                let was_vis = c.is_visible_on(mon_idx, current);
                let is_vis = c.is_visible_on(mon_idx, new_tagmask);
                if was_vis && !is_vis {
                    let surface = c.window.wl_surface().map(|s| (*s).clone());
                    self.closing_clients.push(ClosingClient {
                        id: smithay::backend::renderer::element::Id::new(),
                        texture: None,
                        capture_pending: surface.is_some(),
                        geom: c.geom,
                        monitor: mon_idx,
                        // Outgoing snapshot needs to render on *this*
                        // tagset until the slide completes — pin its
                        // visibility tag bitmap to all-bits-set so
                        // `push_closing_clients` always draws it.
                        // The list-removal in `tick_animations` is
                        // what bounds its lifetime.
                        tags: !0u32,
                        time_started: now,
                        duration: self.config.animation_duration_tag,
                        progress: 0.0,
                        kind: crate::render::open_close::OpenCloseKind::Slide(slide_dir),
                        extreme_scale: 1.0, // pure slide, no scale
                        border_radius: self.config.border_radius as f32,
                        source_surface: surface,
                    });
                }
            }
        }

        // Stage incoming clients at an off-screen *but full-size*
        // staging rect so arrange_monitor's Move animation slides
        // them in as a pure translate. We deliberately preserve the
        // client's previous c.geom dimensions: the layout for a
        // returning tag almost always recomputes the same slot size,
        // so initial.size == target.size, `slot_size_changed` is
        // false, and the renderer skips the resize-snapshot path
        // entirely. Border tracks the interpolated `c.geom` and the
        // surface buffer (which is already at the target size,
        // committed during the *previous* visit to this tag) follows
        // it via map_element on each tick. Result: border and surface
        // travel as a unit, with no settle / pop / scale-in.
        if do_anim {
            for c in self.clients.iter_mut() {
                if c.monitor != mon_idx {
                    continue;
                }
                let was_vis = c.is_visible_on(mon_idx, current);
                let is_vis = c.is_visible_on(mon_idx, new_tagmask);
                if !was_vis && is_vis {
                    // First-show case: the client was never properly
                    // arranged on this monitor (typically because it
                    // mapped while a different tag was active — e.g.
                    // a startup script launching apps onto their home
                    // tags before the user has visited those tags).
                    // c.geom is still default `(0, 0, 0, 0)` and no
                    // configure has been sent for the actual slot
                    // size yet. Skip the tag-in animation entirely:
                    // staging an offscreen rect with a fabricated
                    // size would force arrange to run a size-changing
                    // move animation (`mon/2 → slot`), the renderer
                    // would try to capture a resize-snapshot from a
                    // surface tree without a usable buffer, that
                    // capture would fail or render at the wrong size,
                    // and the user would see the live surface stuck
                    // at its default size while the border tracked
                    // the slot — exactly the "first-launch via
                    // semsumo doesn't fit, pkill+relaunch fixes it"
                    // symptom on Spotify and Helium. By falling
                    // through, `arrange_monitor` runs its
                    // direct-snap branch (because `old.width == 0`
                    // makes `should_animate` false), pushes the
                    // window to its slot in one go, and sends the
                    // first valid configure. The next tag-switch
                    // visit will animate normally with a populated
                    // c.geom.
                    if c.geom.width <= 0 || c.geom.height <= 0 {
                        continue;
                    }
                    c.geom = crate::layout::Rect {
                        x: off_in_xy.0,
                        y: off_in_xy.1,
                        width: c.geom.width,
                        height: c.geom.height,
                    };
                    // Force arrange to start a fresh animation (the
                    // already_animating_to_target guard would skip if
                    // a previous animation's target happens to match).
                    c.animation.running = false;
                }
            }
        }

        self.update_pertag_for_tagset(mon_idx, new_tagmask);
        self.arrange_monitor(mon_idx);
        self.focus_first_visible_or_clear(mon_idx);
        if do_anim {
            self.request_repaint();
        }
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        // Phase 3 scripting: fire `on_tag_switch` handlers. Runs
        // after focus + broadcast so a handler reading
        // `current_tag()` / `focused_appid()` sees the post-switch
        // state, not the pre-switch.
        crate::scripting::fire_tag_switch(self);
    }

    pub fn toggle_view_tag(&mut self, tagmask: u32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let seltags = self.monitors[mon_idx].seltags;
        let current = self.monitors[mon_idx].tagset[seltags];
        let new = current ^ tagmask;
        if new != 0 {
            self.monitors[mon_idx].tagset[seltags] = new;
            self.update_pertag_for_tagset(mon_idx, new);
            self.arrange_monitor(mon_idx);
            self.focus_first_visible_or_clear(mon_idx);
            crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        }
    }

    pub fn view_relative(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() || delta == 0 {
            return;
        }
        let current = self.monitors[mon_idx].current_tagset();
        let current_tag = if current.count_ones() == 1 {
            current.trailing_zeros() as i32
        } else {
            0
        };
        let max = crate::MAX_TAGS as i32;
        let next = (current_tag + delta).rem_euclid(max);
        self.view_tag(1u32 << next);
    }

    pub fn tag_focused(&mut self, tagmask: u32) {
        if tagmask == 0 {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };

        let mon_idx = self.clients[idx].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }
        self.clients[idx].old_tags = self.clients[idx].tags;
        self.clients[idx].is_tag_switching = true;
        self.clients[idx].animation.running = false;
        self.clients[idx].tags = tagmask;
        self.arrange_monitor(mon_idx);

        if !self.clients[idx].is_visible_on(mon_idx, self.monitors[mon_idx].current_tagset()) {
            self.focus_first_visible_or_clear(mon_idx);
        }

        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
    }

    pub fn tag_relative(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current = self.clients[idx].tags;
        let current_tag = if current.count_ones() == 1 {
            current.trailing_zeros() as i32
        } else {
            self.monitors
                .get(self.clients[idx].monitor)
                .map(|mon| mon.current_tagset().trailing_zeros() as i32)
                .unwrap_or(0)
        };
        let max = crate::MAX_TAGS as i32;
        let next = (current_tag + delta).rem_euclid(max);
        self.tag_focused(1u32 << next);
    }

    pub fn toggle_client_tag(&mut self, tagmask: u32) {
        let Some(idx) = self.focused_client_idx() else {
            return;
        };

        let mon_idx = self.clients[idx].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }
        let new = self.clients[idx].tags ^ tagmask;
        if new != 0 {
            self.clients[idx].old_tags = self.clients[idx].tags;
            self.clients[idx].is_tag_switching = true;
            self.clients[idx].animation.running = false;
            self.clients[idx].tags = new;
            self.arrange_monitor(mon_idx);

            if !self.clients[idx].is_visible_on(mon_idx, self.monitors[mon_idx].current_tagset()) {
                self.focus_first_visible_or_clear(mon_idx);
            }

            crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        }
    }

    pub fn set_layout(&mut self, name: &str) {
        if let Some(layout) = LayoutId::from_name(name) {
            let mon_idx = self.focused_monitor();
            if mon_idx >= self.monitors.len() {
                return;
            }
            let curtag = self.monitors[mon_idx].pertag.curtag;
            self.monitors[mon_idx].pertag.ltidxs[curtag] = layout;
            // User explicitly picked a layout — adaptive auto-layout
            // must back off on this tag so its choice survives every
            // subsequent arrange pass. Reset by `view_tag` switching
            // to a tag that's never been touched by `setlayout` and
            // letting auto-layout pick again.
            self.monitors[mon_idx].pertag.user_picked_layout[curtag] = true;
            self.arrange_monitor(mon_idx);
        }
    }

    /// Adaptive layout heuristic: pick the most ergonomic layout for
    /// the current tag based on visible-client count + monitor aspect
    /// ratio. Called from `arrange_monitor` when `Config::auto_layout`
    /// is on. Skipped on tags where the user has explicitly called
    /// `setlayout` (sticky `pertag.user_picked_layout` flag) so a
    /// deliberate user choice is never overridden.
    pub(crate) fn maybe_apply_adaptive_layout(&mut self, mon_idx: usize) {
        let curtag = self.monitors[mon_idx].pertag.curtag;
        if self
            .monitors[mon_idx]
            .pertag
            .user_picked_layout
            .get(curtag)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        let mon_area = self.monitors[mon_idx].monitor_area;

        // Count tile-eligible visible clients (skip floating /
        // fullscreen / scratchpad / minimised — they don't take up a
        // tile slot).
        let count = self
            .clients
            .iter()
            .filter(|c| c.is_visible_on(mon_idx, tagset) && c.is_tiled())
            .count();
        if count == 0 {
            // Empty tag — keep whatever's set so the user sees the
            // *next* arrival land in a sensible layout for one.
            return;
        }

        let aspect = if mon_area.height > 0 {
            mon_area.width as f32 / mon_area.height as f32
        } else {
            16.0 / 9.0
        };
        let very_wide = aspect >= 2.4; // ultrawide / 32:9
        let wide = aspect >= 1.5; // 16:9 / 16:10
        let portrait = aspect <= 0.9; // rotated panels

        // Heuristic. Tuned for the user's two-monitor setup
        // (DP-3 2560x1440 → wide; eDP-1 1920x1200 → wide-ish):
        //
        //   1 client  → monocle    (no point splitting space for one)
        //   2 clients → tile        (master/stack ratio classic)
        //   3-5 wide  → scroller    (niri-style horizontal tracks)
        //   3-5 portrait → vertical_scroller
        //   6+  wide  → grid
        //   6+  ultrawide → vertical_scroller (long horizontal track)
        //   6+  portrait → vertical_grid
        //
        // The thresholds and choices are deliberately conservative —
        // adaptive should "feel right" 90% of the time, never wrong.
        // A user who wants a different mapping toggles it off and
        // bumps `setlayout` directly per tag.
        let chosen = match (count, very_wide, wide, portrait) {
            (1, _, _, _) => crate::layout::LayoutId::Monocle,
            (2, _, _, _) => crate::layout::LayoutId::Tile,
            (3..=5, _, _, true) => crate::layout::LayoutId::VerticalScroller,
            (3..=5, _, true, _) => crate::layout::LayoutId::Scroller,
            (3..=5, _, _, _) => crate::layout::LayoutId::Tile,
            (_, _, _, true) => crate::layout::LayoutId::VerticalGrid,
            (_, true, _, _) => crate::layout::LayoutId::VerticalScroller,
            (_, _, true, _) => crate::layout::LayoutId::Grid,
            _ => crate::layout::LayoutId::Tile,
        };

        if self.monitors[mon_idx].pertag.ltidxs[curtag] != chosen {
            tracing::info!(
                "auto_layout: tag={} clients={} aspect={:.2} → {:?}",
                curtag,
                count,
                aspect,
                chosen,
            );
            self.monitors[mon_idx].pertag.ltidxs[curtag] = chosen;
        }
    }

    /// Spatial-canvas pan: shift the *viewport* on the active tag by
    /// (dx, dy) logical pixels. Stored per-tag so each tag remembers
    /// where the user had been "looking" in the canvas. The
    /// `Canvas` layout reads the offset on every arrange and
    /// translates each client's `canvas_geom` by it — clients stay
    /// anchored on the canvas, the viewport moves.
    pub fn canvas_pan(&mut self, dx: i32, dy: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_x.get_mut(curtag) {
            *slot += dx as f64;
        }
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_y.get_mut(curtag) {
            *slot += dy as f64;
        }
        self.arrange_monitor(mon_idx);
    }

    /// Reset the active tag's canvas viewport to the origin (0, 0).
    pub fn canvas_reset(&mut self) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_x.get_mut(curtag) {
            *slot = 0.0;
        }
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_y.get_mut(curtag) {
            *slot = 0.0;
        }
        self.arrange_monitor(mon_idx);
    }

    pub fn switch_layout(&mut self) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let current = self.monitors[mon_idx].current_layout().name();
        let layouts: Vec<String> = if self.config.circle_layouts.is_empty() {
            vec!["tile", "scroller", "grid", "monocle", "deck"]
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            self.config.circle_layouts.clone()
        };
        if layouts.is_empty() {
            return;
        }
        let current_pos = layouts.iter().position(|name| name == current).unwrap_or(0);
        let next = layouts[(current_pos + 1) % layouts.len()].clone();
        self.set_layout(&next);
        self.notify_layout(&next);
    }

    /// Toggle the focused client's "sticky" / global state — visible
    /// on every tag of its current monitor instead of only the tag
    /// it was tagged with. Equivalent to niri-float-sticky's
    /// per-window sticky toggle, but built into the compositor so
    /// no external daemon is needed.
    ///
    /// Implementation: when sticking, save the current tag mask onto
    /// `old_tags` and overwrite `tags = u32::MAX`. Every
    /// `is_visible_on(mon, tagset)` check walks `(tags & tagset)
    /// != 0`, and `u32::MAX & anything` is `anything` (non-zero
    /// for any active tagset), so the window shows up wherever the
    /// monitor goes.
    ///
    /// When unsticking, restore from `old_tags`. If `old_tags` is
    /// 0 (rule never saved one — a freshly-created sticky-by-rule
    /// client) fall back to whichever tag is currently visible on
    /// the monitor so the window doesn't vanish.
    ///
    /// Cross-monitor sticky (window visible on multiple monitors at
    /// once) is a separate, much-bigger change — would need scene-
    /// graph mapping per output. Skipped for now; this covers the
    /// niri-float-sticky single-monitor "appears on every tag of
    /// this output" case which is the 95% use.
    pub fn toggle_sticky(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        // Don't sticky scratchpads — they have their own
        // visibility model (`is_scratchpad_show` flag); flipping
        // tags out from under the scratchpad path would confuse it.
        if self.clients[idx].is_in_scratchpad {
            tracing::info!("toggle_sticky: skipped (client is in scratchpad)");
            return;
        }
        let was_sticky = self.clients[idx].is_global;
        let mon_idx = self.clients[idx].monitor;
        let appid = self.clients[idx].app_id.clone();

        if was_sticky {
            // Restore previous tag mask. Fall back to the monitor's
            // currently-visible tag if old_tags wasn't populated
            // (rule-driven sticky-from-spawn never went through
            // toggle).
            let restored = if self.clients[idx].old_tags != 0 {
                self.clients[idx].old_tags
            } else {
                self.monitors
                    .get(mon_idx)
                    .map(|m| m.current_tagset())
                    .filter(|m| *m != 0)
                    .unwrap_or(1)
            };
            self.clients[idx].tags = restored;
            self.clients[idx].is_global = false;
        } else {
            self.clients[idx].old_tags = self.clients[idx].tags;
            self.clients[idx].tags = u32::MAX;
            self.clients[idx].is_global = true;
        }

        self.arrange_monitor(mon_idx);
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        self.request_repaint();
        crate::scripting::fire_focus_change(self);

        // OSD-style notification — short timeout so it doesn't
        // pile up if the user toggles a few windows in a row.
        let title = if was_sticky { "Sticky off" } else { "Sticky on" };
        let body = if appid.is_empty() {
            String::from("Focused window")
        } else {
            appid
        };
        let _ = crate::utils::spawn([
            "notify-send", "-a", "margo",
            "-i", "view-pin-symbolic",
            "-t", "1200",
            title, &body,
        ]);
    }

    /// Fire an OSD-style notification telling the user the active
    /// layout just changed. Called from `switch_layout` (cycle) and
    /// from the `setlayout` dispatch handler (explicit pick) — not
    /// from `set_layout` itself, because that's also called
    /// internally for window-rule application and we don't want to
    /// notify on every rule-driven re-arrangement.
    pub fn notify_layout(&self, name: &str) {
        // W3.5: enrich the toast with position-in-cycle context so
        // users navigating the 14-layout catalogue see where they
        // are. Format: `<name> (<pos>/<total>) → <next>`. Falls
        // back to bare name when not in `circle_layout`.
        let cycle: Vec<String> = if self.config.circle_layouts.is_empty() {
            vec![]
        } else {
            self.config.circle_layouts.clone()
        };
        let body = if let Some(pos) = cycle.iter().position(|n| n == name) {
            let total = cycle.len();
            let next = &cycle[(pos + 1) % total];
            format!("{name}  ({}/{total}) → next: {next}", pos + 1)
        } else {
            name.to_string()
        };
        let _ = crate::utils::spawn([
            "notify-send", "-a", "margo",
            "-i", "view-grid-symbolic",
            "-t", "1200",
            "Margo Layout", &body,
        ]);
    }

    /// Toast for layout-adjacent actions (proportion preset,
    /// gap toggle). Same look-and-feel as `notify_layout` so the
    /// user can rely on the in-corner toast giving consistent
    /// state feedback for layout-cycle keybinds.
    pub fn notify_layout_state(&self, action: &str, value: &str) {
        let body = format!("{action}: {value}");
        let _ = crate::utils::spawn([
            "notify-send", "-a", "margo",
            "-i", "view-grid-symbolic",
            "-t", "1000",
            "Margo Layout", &body,
        ]);
    }

    pub fn toggle_floating(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            self.clients[idx].is_floating = !self.clients[idx].is_floating;
            if self.clients[idx].is_floating && self.clients[idx].float_geom.width == 0 {
                self.clients[idx].float_geom = self.clients[idx].geom;
            }
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
            // dwl-ipc-v2 reports `floating` per output's focused
            // client; the bar status indicator (noctalia "tile/float"
            // glyph) needs an explicit broadcast or it stays stale.
            crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        }
    }

    pub fn set_focused_proportion(&mut self, proportion: f32) {
        if let Some(idx) = self.focused_client_idx() {
            self.clients[idx].scroller_proportion = proportion.clamp(0.1, 1.0);
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn switch_focused_proportion_preset(&mut self) {
        if self.config.scroller_proportion_presets.is_empty() {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current = self.clients[idx].scroller_proportion;
        let presets = &self.config.scroller_proportion_presets;
        let current_pos = presets
            .iter()
            .position(|value| (*value - current).abs() < 0.01)
            .unwrap_or(0);
        let next_proportion = presets[(current_pos + 1) % presets.len()];
        self.clients[idx].scroller_proportion = next_proportion;
        let mon_idx = self.clients[idx].monitor;
        self.arrange_monitor(mon_idx);
        // W3.5: toast feedback for the cycling action.
        self.notify_layout_state(
            "scroller proportion",
            &format!("{:.2}", next_proportion),
        );
    }

    /// Toggle the focused client between [`FullscreenMode::Exclusive`] and
    /// [`FullscreenMode::Off`]. Bound to `togglefullscreen_exclusive`. The
    /// difference vs `togglefullscreen` is that the render path will
    /// suppress every layer-shell surface on this output while
    /// `Exclusive` is active — the bar disappears, the focused window
    /// covers the panel pixels too.
    pub fn toggle_fullscreen_exclusive(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            let target = if self.clients[idx].fullscreen_mode == FullscreenMode::Exclusive {
                FullscreenMode::Off
            } else {
                FullscreenMode::Exclusive
            };
            self.set_client_fullscreen_mode(idx, target);
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            let target = !self.clients[idx].is_fullscreen;
            self.set_client_fullscreen(idx, target);
        }
    }

    /// Set a client's fullscreen state and inform the client via
    /// the xdg_toplevel protocol so it actually re-renders for
    /// fullscreen (drops decorations, fills the new geom).
    ///
    /// Three things happen in lockstep here:
    ///
    /// 1. `client.is_fullscreen` flips — drives margo's layout
    ///    pass (arrange_monitor gives a fullscreen client the
    ///    full monitor rect).
    /// 2. `xdg_toplevel.with_pending_state` adds / removes the
    ///    `Fullscreen` state and pins the size to the monitor
    ///    rect. Without this, browsers + native fullscreen apps
    ///    keep rendering the windowed UI even when their geom
    ///    has changed — they trust the protocol state, not the
    ///    geom. This was the bug behind "F11 / video player
    ///    fullscreen does nothing" until W4.5.
    /// 3. `arrange_monitor` runs the layout pass which queues
    ///    the actual configure send + rerenders the scene.
    /// 4. `broadcast_monitor` updates dwl-ipc bars (which carry
    ///    the focused-client `fullscreen` flag for the icon
    ///    indicator).
    ///
    /// X11 clients (XWayland) follow a different protocol path
    /// (NetWMState); we just flip the flag for them and let
    /// arrange handle geometry — that path was already correct
    /// before this fix because XWayland clients trust geom
    /// without a state-change packet.
    pub fn set_client_fullscreen(&mut self, idx: usize, fullscreen: bool) {
        // Backward-compat shim: `bool` API maps to `WorkArea` mode.
        // XDG `set_fullscreen()` requests + the existing keybind path
        // both still go through here. Real two-way distinction lives
        // in [`set_client_fullscreen_mode`].
        let mode = if fullscreen {
            FullscreenMode::WorkArea
        } else {
            FullscreenMode::Off
        };
        self.set_client_fullscreen_mode(idx, mode);
    }

    /// Apply a [`FullscreenMode`] to a client. The single source of truth
    /// for fullscreen — `set_client_fullscreen` is a shim, the dispatch
    /// actions `togglefullscreen` / `togglefullscreen_exclusive` route
    /// through here.
    ///
    /// Three rotating concerns:
    ///
    /// 1. `MargoClient` state — `fullscreen_mode` + the
    ///    backward-compat `is_fullscreen` bool stay in lock-step.
    /// 2. xdg_toplevel pending state — Wayland clients get the
    ///    `Fullscreen` state bit + a size hint matching the mode
    ///    (`work_area` for WorkArea, `monitor_area` for Exclusive).
    ///    X11 surfaces are skipped; NetWMState round-trip isn't
    ///    wired today (known limitation, see `state/handlers/x11.rs`).
    /// 3. Layout pass + IPC broadcast — `arrange_monitor` reads the
    ///    new mode to size the geometry, then dwl-ipc clients
    ///    (noctalia / waybar-dwl) see the updated state.json.
    pub fn set_client_fullscreen_mode(&mut self, idx: usize, mode: FullscreenMode) {
        if idx >= self.clients.len() {
            return;
        }
        let mon_idx = self.clients[idx].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }
        self.clients[idx].fullscreen_mode = mode;
        self.clients[idx].is_fullscreen = mode != FullscreenMode::Off;

        if let WindowSurface::Wayland(toplevel) =
            self.clients[idx].window.underlying_surface()
        {
            use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
            // The size hint matches the mode: WorkArea respects the
            // bar's exclusion zone, Exclusive covers the entire
            // panel. Clients honour this for their initial buffer
            // allocation; the actual rect lands via `arrange_monitor`.
            let target_size = match mode {
                FullscreenMode::Off => None,
                FullscreenMode::WorkArea => {
                    let wa = self.monitors[mon_idx].work_area;
                    Some(smithay::utils::Size::from((wa.width, wa.height)))
                }
                FullscreenMode::Exclusive => {
                    let ma = self.monitors[mon_idx].monitor_area;
                    Some(smithay::utils::Size::from((ma.width, ma.height)))
                }
            };
            toplevel.with_pending_state(|state| {
                if mode == FullscreenMode::Off {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                    state.size = None;
                } else {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = target_size;
                }
            });
            toplevel.send_pending_configure();
        }

        tracing::info!(
            target: "fullscreen",
            client = idx,
            mode = ?mode,
            "applied",
        );

        self.arrange_monitor(mon_idx);
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
    }

    /// Does any client on `mon_idx` currently hold an exclusive
    /// fullscreen lease? Used by the render path to decide whether
    /// to suppress layer-shell surfaces (bar / notification overlay)
    /// for that output.
    pub fn monitor_has_exclusive_fullscreen(&self, mon_idx: usize) -> bool {
        if mon_idx >= self.monitors.len() {
            return false;
        }
        let active_tagset = self.monitors[mon_idx].current_tagset();
        self.clients.iter().any(|c| {
            c.monitor == mon_idx
                && c.fullscreen_mode == FullscreenMode::Exclusive
                && c.is_visible_on(mon_idx, active_tagset)
        })
    }


    pub fn inc_nmaster(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        let current = self.monitors[mon_idx].pertag.nmasters[curtag] as i32;
        self.monitors[mon_idx].pertag.nmasters[curtag] = (current + delta).max(0) as u32;
        self.arrange_monitor(mon_idx);
    }

    pub fn set_mfact(&mut self, delta: f32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        let current = self.monitors[mon_idx].pertag.mfacts[curtag];
        self.monitors[mon_idx].pertag.mfacts[curtag] = (current + delta).clamp(0.05, 0.95);
        self.arrange_monitor(mon_idx);
    }

    pub fn toggle_gaps(&mut self) {
        self.enable_gaps = !self.enable_gaps;
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.notify_layout_state(
            "gaps",
            if self.enable_gaps { "on" } else { "off" },
        );
    }

    pub fn inc_gaps(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if let Some(mon) = self.monitors.get_mut(mon_idx) {
            mon.gappih = (mon.gappih + delta).max(0);
            mon.gappiv = (mon.gappiv + delta).max(0);
            mon.gappoh = (mon.gappoh + delta).max(0);
            mon.gappov = (mon.gappov + delta).max(0);
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn move_focused(&mut self, dx: i32, dy: i32) {
        if let Some(idx) = self.focused_client_idx() {
            if self.clients[idx].float_geom.width == 0 {
                self.clients[idx].float_geom = self.clients[idx].geom;
            }
            self.clients[idx].is_floating = true;
            self.clients[idx].float_geom.x += dx;
            self.clients[idx].float_geom.y += dy;
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn resize_focused(&mut self, dw: i32, dh: i32) {
        if let Some(idx) = self.focused_client_idx() {
            if self.clients[idx].float_geom.width == 0 {
                self.clients[idx].float_geom = self.clients[idx].geom;
            }
            self.clients[idx].is_floating = true;
            self.clients[idx].float_geom.width = (self.clients[idx].float_geom.width + dw).max(50);
            self.clients[idx].float_geom.height = (self.clients[idx].float_geom.height + dh).max(50);
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn zoom(&mut self) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        let Some(focused_idx) = self.focused_client_idx() else {
            return;
        };

        let tiled: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset) && c.is_tiled())
            .map(|(i, _)| i)
            .collect();

        if tiled.len() < 2 {
            return;
        }

        let focused_pos = tiled.iter().position(|&i| i == focused_idx);
        let (a, b) = if focused_pos == Some(0) {
            (tiled[0], tiled[1])
        } else if let Some(pos) = focused_pos {
            (tiled[0], tiled[pos])
        } else {
            return;
        };

        self.clients.swap(a, b);
        self.arrange_monitor(mon_idx);
    }

    pub fn focus_mon(&mut self, direction: i32) {
        if self.monitors.len() <= 1 {
            return;
        }
        let current = self.focused_monitor();
        let len = self.monitors.len();
        let next = if direction > 0 {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        if next == current {
            return;
        }

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

        crate::protocols::dwl_ipc::broadcast_monitor(self, current);
        crate::protocols::dwl_ipc::broadcast_monitor(self, next);
        self.request_repaint();
    }

    pub fn tag_mon(&mut self, direction: i32) {
        if self.monitors.len() <= 1 {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current_mon = self.clients[idx].monitor;
        let len = self.monitors.len();
        let target_mon = if direction > 0 {
            (current_mon + 1) % len
        } else {
            (current_mon + len - 1) % len
        };
        let tagset = self.monitors[target_mon].current_tagset();
        self.clients[idx].monitor = target_mon;
        self.clients[idx].tags = tagset;
        self.arrange_monitor(current_mon);
        self.arrange_monitor(target_mon);

        // Follow the window across monitors. Without this the
        // cursor (and therefore sloppy-focus + the visible
        // selection) stays on the source output — the user moves a
        // window with `tagmon` or the matching 3-finger gesture and
        // the pointer is suddenly stranded on the empty side. Warp
        // first, then refocus through the standard path so border /
        // dwl-ipc / scripting hooks all see a single coherent
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

        crate::protocols::dwl_ipc::broadcast_monitor(self, current_mon);
        crate::protocols::dwl_ipc::broadcast_monitor(self, target_mon);
        self.request_repaint();
    }
}

// ── Deferred initial map (out-of-trait helper) ───────────────────────────────

impl MargoState {
    /// Finalize the deferred initial map of a client created in
    /// `new_toplevel` but held back from `space.map_element` until its
    /// app_id had a chance to arrive. Called from the commit handler
    /// the first time a buffer is attached to the toplevel's surface.
    /// At this point Qt clients have invariably set `app_id`, so
    /// window rules can be applied with their full intended effect
    /// (`isfloating`, custom geom, tag pinning, …) BEFORE the window
    /// is ever placed in the smithay space — no rule-jump flicker.
    pub(crate) fn finalize_initial_map(&mut self, idx: usize) {
        // Sync the latest app_id / title from the surface before
        // running window rules — by this point Qt has had its chance.
        if idx >= self.clients.len() {
            return;
        }
        if let WindowSurface::Wayland(toplevel) = self.clients[idx].window.underlying_surface() {
            let (app_id, title) = read_toplevel_identity(toplevel);
            self.clients[idx].app_id = app_id;
            self.clients[idx].title = title;
        }

        // Now run rules with the live app_id/title.
        let _changed = self.reapply_rules(idx, WindowRuleReason::InitialMap);

        // Tag-home redirect: if rules picked tag N but didn't pin a
        // monitor, route to the tag's home output.
        let no_explicit_monitor = !self
            .matching_window_rules(
                &self.clients[idx].app_id,
                &self.clients[idx].title,
            )
            .iter()
            .any(|r| r.monitor.is_some());
        if no_explicit_monitor {
            if let Some(home) = self.tag_home_monitor(self.clients[idx].tags) {
                self.clients[idx].monitor = home;
            }
        }

        let target_mon = self.clients[idx].monitor;
        let focus_new =
            !self.clients[idx].no_focus && !self.clients[idx].open_silent;
        let window = self.clients[idx].window.clone();

        let map_loc = self
            .monitors
            .get(target_mon)
            .map(|m| (m.monitor_area.x, m.monitor_area.y))
            .unwrap_or((0, 0));
        self.space.map_element(window.clone(), map_loc, true);

        if focus_new {
            if target_mon < self.monitors.len() {
                self.monitors[target_mon].prev_selected =
                    self.monitors[target_mon].selected;
                self.monitors[target_mon].selected = Some(idx);
            }
            self.focus_surface(Some(FocusTarget::Window(window)));
        }

        // Mark the client mapped BEFORE arrange so the layout pass
        // sees it as a real participant.
        self.clients[idx].is_initial_map_pending = false;

        if !self.monitors.is_empty() {
            self.arrange_monitor(target_mon);
        }

        // Named scratchpad bootstrap. If the windowrule flagged this
        // client as a named scratchpad (mango's `isnamedscratchpad:1`
        // pattern), promote it to a *visible* scratchpad on first
        // map: `is_in_scratchpad = true`, `is_scratchpad_show = true`,
        // float_geom from the rule, focus retained. The user-side
        // mental model is "press the bind → my scratchpad appears
        // here", so the very first press of the toggle key (which
        // spawned the app in the first place because nothing was
        // running) MUST land a visible window. Subsequent presses
        // toggle hide / show via the regular `switch_scratchpad_state`
        // path.
        //
        // Earlier this branch tucked the freshly-spawned client away
        // (unmap + is_minimized) on the theory that the spawn-cmd
        // and the visibility toggle were two separate steps. They
        // aren't on the user side — pressing the bind once should
        // result in a visible scratchpad; pressing again should
        // hide it. Only the second-and-later cycles go through
        // toggle_named_scratchpad's switch_scratchpad_state branch.
        if self.clients[idx].is_named_scratchpad
            && !self.clients[idx].is_in_scratchpad
        {
            self.clients[idx].is_in_scratchpad = true;
            self.clients[idx].is_scratchpad_show = true;
            self.clients[idx].is_floating = true;
            // Don't unmap — leave the window where finalize_initial_map's
            // own map_element / arrange_monitor placed it. The
            // windowrule's float_geom (offsetx/offsety/width/height)
            // already drove that placement, so the visible result
            // matches the show_scratchpad_client positioning we'd
            // otherwise apply on a subsequent toggle.
            tracing::info!(
                "named_scratchpad bootstrap: app_id={} visible from first map",
                self.clients[idx].app_id,
            );
        }

        // Note: clients that mapped onto a non-active tag intentionally
        // get NO bootstrap configure here. An earlier version of this
        // code seeded `c.geom` with the monitor's work area and sent a
        // matching configure so the client could commit at "some size"
        // during launch. That actively hurt: XWayland clients (Spotify
        // is the canonical case) commit at the bootstrap size, cache
        // it as their natural extent, and then resist the smaller
        // configure the eventual tag-switch arrange tries to send —
        // the surface stays stuck at the larger bootstrap size and the
        // `clipped_surface` shader ends up cropping the right / bottom
        // of the visible content. Leaving `c.geom` at the default zero
        // rect lets `view_tag`'s "skip tag-in staging when c.geom is
        // degenerate" branch fall through to `arrange_monitor`'s
        // direct-snap path, which sends a *first* configure at the
        // real slot size. Native Wayland clients always honour that
        // first configure; XWayland clients honour it far more
        // reliably than a subsequent shrink.


        // Kick off the open animation if globally enabled, this client
        // didn't opt out (window-rule `no_animation` / `open_silent`),
        // and the user configured a non-zero open duration. The
        // renderer captures the surface into a `GlesTexture` on the
        // very next frame (driven by `opening_capture_pending`) and
        // from then on the live `wl_surface` is hidden — we only draw
        // the snapshot through `OpenCloseRenderElement` until the
        // curve settles. This eliminates the "instant pop at the new
        // geom for one frame, then the animation kicks in" flash that
        // pure wrap-the-live-surface approaches produce.
        if self.config.animations
            && self.config.animation_duration_open > 0
            && !self.clients[idx].no_animation
            && !self.clients[idx].open_silent
        {
            // Per-client override (set by window-rule
            // `animation_type_open=…`) wins over the global config.
            let kind_str = self.clients[idx]
                .animation_type_open
                .clone()
                .unwrap_or_else(|| self.config.animation_type_open.clone());
            let kind = crate::render::open_close::OpenCloseKind::parse(&kind_str);
            let now = crate::utils::now_ms();
            self.clients[idx].opening_animation =
                Some(crate::animation::OpenCloseClientAnim {
                    kind,
                    time_started: now,
                    duration: self.config.animation_duration_open,
                    progress: 0.0,
                    extreme_scale: self.config.zoom_initial_ratio.clamp(0.05, 1.0),
                });
            self.clients[idx].opening_capture_pending = true;
            self.request_repaint();
        }

        tracing::info!(
            "finalize_initial_map: app_id={} idx={idx} monitor={target_mon} \
             floating={} tags={:#x} open_anim={}",
            self.clients[idx].app_id,
            self.clients[idx].is_floating,
            self.clients[idx].tags,
            self.clients[idx].opening_animation.is_some(),
        );

        // Phase 3 scripting: invoke `on_window_open` handlers now
        // that app_id / title / window-rules have all settled. A
        // handler that calls `focused_appid()` sees the just-mapped
        // window's identity, and dispatches like `tagview` /
        // `togglefloating` apply to it because focus has already
        // been pushed to it earlier in this function.
        crate::scripting::fire_window_open(self);

        // Notify xdp-gnome's window picker so a live screencast
        // share dialog refreshes its list while open.
        self.emit_windows_changed();
    }
}
