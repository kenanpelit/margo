//! Tag dispatch actions: view/toggle/move tags, relative and carousel
//! steps, `view_insert`, the edge-scroller pointer focus and the tag
//! departure animation, plus the pure carousel / edge-scroller decision
//! helpers they share (unit-tested without a compositor).

use smithay::wayland::seat::WaylandFocus;

use crate::layout::LayoutId;
use crate::state::{ClosingClient, MargoState};

impl MargoState {
    pub fn view_tag(&mut self, tagmask: u32) {
        let tagmask = tagmask & crate::TAGMASK;
        if tagmask == 0 {
            return;
        }
        // If a tagrule pins this tag to a specific monitor, jump focus
        // there first so multi-monitor users get niri-style "tag 7 is on
        // eDP-1, super+7 from anywhere takes me to it" behaviour. We
        // skip the redirect when the user is already on the home
        // monitor or the tagmask is the all-tags special value.
        let current_mon = self.focused_monitor();
        let mon_idx = if tagmask != crate::TAGMASK {
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
        self.view_tag_on(mon_idx, tagmask);
    }

    /// Show `tagmask` on monitor `mon_idx` — [`Self::view_tag`] once the
    /// monitor is picked. Activating a window on a hidden tag uses it
    /// directly: the tag must change on the window's own monitor, not on
    /// the focused one.
    pub fn view_tag_on(&mut self, mon_idx: usize, tagmask: u32) {
        let tagmask = tagmask & crate::TAGMASK;
        if tagmask == 0 || mon_idx >= self.monitors.len() {
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
        // A pending carousel direction (set by a wrap-around relative
        // move) overrides the tag-index comparison; consume it here so
        // the next, non-wrapping transition behaves normally.
        let going_forward = transition_forward(
            std::mem::replace(&mut self.tag_carousel_dir, 0),
            old_idx_target,
            new_idx,
        );
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
            let radii: Vec<f32> = (0..self.clients.len())
                .map(|i| self.client_radius(i))
                .collect();
            let focused_idx = self.focused_client_idx();
            for (ci, c) in self.clients.iter().enumerate() {
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
                        border_radius: radii[ci],
                        opacity: crate::animation::client_opacity(c, focused_idx == Some(ci)),
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
        self.focus_most_recent_visible_or_clear(mon_idx);
        if do_anim {
            let output = self.monitors[mon_idx].output.clone();
            self.request_repaint_output(&output);
        }
        self.mark_state_dirty();
        // Phase 3 scripting: fire `on_tag_switch` handlers. Runs
        // after focus + broadcast so a handler reading
        // `current_tag()` / `focused_appid()` sees the post-switch
        // state, not the pre-switch.
        crate::scripting::fire_tag_switch(self);
    }

    pub fn toggle_view_tag(&mut self, tagmask: u32) {
        let tagmask = tagmask & crate::TAGMASK;
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
            self.focus_most_recent_visible_or_clear(mon_idx);
            self.mark_state_dirty();
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
        // With tag_gather the occupied tags are always packed at the front,
        // so wrap at "last occupied + 1" rather than into empty tags.
        let wrap_len = if self.config.tag_gather {
            let tag_bits = (1u32 << max) - 1;
            let highest = self
                .clients
                .iter()
                .filter(|c| c.monitor == mon_idx)
                .map(|c| (u32::BITS - (c.tags & tag_bits).leading_zeros()) as i32)
                .max()
                .unwrap_or(0);
            gather_wrap_len(highest, max)
        } else {
            max
        };
        // Carousel: when the move wraps past the first / last tag, force
        // the slide to continue in the travel direction (so 9→1 going
        // right slides rightward) instead of the long reverse the
        // tag-index delta would otherwise pick. Consumed in `view_tag`.
        let (next, carousel_dir) =
            carousel_step_within(current_tag, delta, max, wrap_len, self.config.tag_carousel);
        // A one-tag wrap region wraps onto itself: nothing to switch to.
        if current.count_ones() == 1 && (1u32 << next) == current {
            return;
        }
        if carousel_dir != 0 {
            self.tag_carousel_dir = carousel_dir;
        }
        self.view_tag(1u32 << next);
    }

    /// mango 0.16.1's `view_insert`: view the adjacent tag (`next` =
    /// false → left, true → right) if it's empty; otherwise make room by
    /// inserting a fresh empty tag before/after the current one — shifting
    /// every later tag's clients + pertag state up by one — and switch to
    /// it. No-op when the current view isn't a single tag, or there's no
    /// room to shift into (the last tag is already occupied).
    pub fn view_insert(&mut self, next: bool) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() || self.monitors[mon_idx].is_overview {
            return;
        }
        let seltags = self.monitors[mon_idx].seltags;
        let curmask = self.monitors[mon_idx].tagset[seltags];
        if curmask == 0 || !curmask.is_power_of_two() {
            return;
        }
        let cur = curmask.trailing_zeros() as usize + 1;
        let max = crate::MAX_TAGS;

        let occupied = |n: usize, state: &Self| -> bool {
            state
                .clients
                .iter()
                .any(|c| c.is_visible_on(mon_idx, 1u32 << (n - 1)))
        };

        let target = if next {
            if cur >= max {
                return;
            }
            cur + 1
        } else if cur == 1 {
            cur
        } else if !occupied(cur - 1, self) {
            cur - 1
        } else {
            cur
        };

        if !occupied(target, self) {
            self.view_tag(1u32 << (target - 1));
            return;
        }

        if target >= max || occupied(max, self) {
            return;
        }

        let mut map = [0usize; crate::MAX_TAGS + 1];
        for (i, m) in map.iter_mut().enumerate().take(target).skip(1) {
            *m = i;
        }
        for (i, m) in map.iter_mut().enumerate().take(max).skip(target) {
            *m = i + 1;
        }
        map[max] = max;
        self.remap_monitor_tags(mon_idx, &map);

        // Suppress tag_gather across the view switch below: the newly
        // freed `target` slot is empty by construction, and without this
        // guard a live tag_gather would immediately compact it away
        // again before the user ever sees it (view_tag's own arrange
        // pass already treats the *new* current view as occupied, but
        // this belt-and-suspenders matches mango's own view_insert).
        let saved_gather = self.config.tag_gather;
        self.config.tag_gather = false;
        self.view_tag(1u32 << (target - 1));
        self.config.tag_gather = saved_gather;
    }

    /// Edge-scroller pointer focus: when the pointer rests at the
    /// leading / trailing edge of a scroller layout and is moving
    /// slowly (per-event magnitude `speed` below
    /// `edge_scroller_focus_allow_speed`), shift focus to the adjacent
    /// column. Disabled when the speed knob is `0.0`. Debounced via
    /// `edge_scroller_armed` so it fires once per edge dwell.
    pub fn maybe_edge_scroller_focus(&mut self, speed: f64) {
        let allow = self.config.edge_scroller_focus_allow_speed;
        if !self.config.edge_scroller_pointer_focus || allow <= 0.0 {
            self.edge_scroller_armed = true;
            return;
        }
        let Some(mon_idx) = self.input_pointer.last_monitor else {
            self.edge_scroller_armed = true;
            return;
        };
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        let layout = mon.pertag.ltidxs[mon.pertag.curtag];
        let area = mon.monitor_area;
        let outcome = edge_scroller_decision(
            layout,
            self.input_pointer.x,
            self.input_pointer.y,
            area.x,
            area.y,
            area.width,
            area.height,
            speed,
            allow,
            self.edge_scroller_armed,
        );
        self.edge_scroller_armed = outcome.armed;
        if let Some(dir) = outcome.focus {
            self.focus_stack(dir);
        }
    }

    /// Snapshot `idx` into a fading-out [`ClosingClient`] before its own
    /// `tags` change out from under it, so it fades cleanly instead of
    /// winking out mid-frame. Called from every dispatch that reassigns
    /// a window's tags/monitor (`tag`, `toggletag`, `setclienttags`,
    /// `tagmon`) — `tagview`/`movetagview` call it too (via
    /// `tag_focused`) but it self-filters out there: the snapshot is
    /// tagged with the client's *current* (pre-mutation) tags, and
    /// `push_closing_clients` only draws a snapshot while its tags still
    /// overlap the monitor's *actively viewed* tagset. Plain `tag`
    /// leaves the viewer's tagset untouched, so the overlap holds and
    /// the fade plays; `tagview` changes the viewer's tagset in the same
    /// breath, so the overlap breaks immediately and this is a no-op —
    /// correct, since that path gets a real slide-in from `view_tag`
    /// instead (see the `tagview` dispatch arm).
    ///
    /// No-op if animations are off, `animation_duration_tag` is `0`, or
    /// the window wasn't visible to begin with (moving a background/
    /// scratchpad window around shouldn't flash anything on screen).
    pub(crate) fn animate_tag_departure(&mut self, idx: usize) {
        if !self.config.animations || self.config.animation_duration_tag == 0 {
            return;
        }
        let mon_idx = self.clients[idx].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }
        let border_radius = self.client_radius(idx);
        let opacity = crate::animation::client_opacity(
            &self.clients[idx],
            self.focused_client_idx() == Some(idx),
        );
        let c = &self.clients[idx];
        if !c.is_visible_on(mon_idx, self.monitors[mon_idx].current_tagset()) {
            return;
        }
        let surface = c.window.wl_surface().map(|s| (*s).clone());
        self.closing_clients.push(ClosingClient {
            id: smithay::backend::renderer::element::Id::new(),
            texture: None,
            capture_pending: surface.is_some(),
            geom: c.geom,
            monitor: mon_idx,
            tags: c.tags,
            time_started: crate::utils::now_ms(),
            duration: self.config.animation_duration_tag,
            progress: 0.0,
            kind: crate::render::open_close::OpenCloseKind::Fade,
            extreme_scale: 1.0,
            border_radius,
            opacity,
            source_surface: surface,
        });
    }

    pub fn tag_focused(&mut self, tagmask: u32) {
        let tagmask = tagmask & crate::TAGMASK;
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
        self.animate_tag_departure(idx);
        self.clients[idx].old_tags = self.clients[idx].tags;
        self.clients[idx].is_tag_switching = true;
        self.clients[idx].animation.running = false;
        self.clients[idx].tags = tagmask;
        self.arrange_monitor(mon_idx);

        if !self.clients[idx].is_visible_on(mon_idx, self.monitors[mon_idx].current_tagset()) {
            self.focus_most_recent_visible_or_clear(mon_idx);
        }

        self.mark_state_dirty();
    }

    /// `tagview`/`movetagview`: move the focused window to `tagmask`
    /// **and** follow it there. Reuses `tag_focused` for the move, then
    /// clears the `is_tag_switching` guard it set before handing off to
    /// `view_tag`: that guard exists to stop a window snapping into an
    /// unrelated, unstaged position on plain `tag` (nobody's watching
    /// the destination), but here `view_tag` is about to stage every
    /// newly-visible client — this one included, now that its tags
    /// point at `tagmask` — at a coherent off-screen start before
    /// arranging. Leaving the guard set would make this one client
    /// snap instead of riding the same slide-in as its neighbours.
    pub fn tag_view(&mut self, tagmask: u32) {
        let tagmask = tagmask & crate::TAGMASK;
        if tagmask == 0 {
            return;
        }
        let idx = self.focused_client_idx();
        self.tag_focused(tagmask);
        if let Some(idx) = idx {
            self.clients[idx].is_tag_switching = false;
        }
        self.view_tag(tagmask);
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
        let tagmask = tagmask & crate::TAGMASK;
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
        let new = self.clients[idx].tags ^ tagmask;
        if new != 0 {
            self.animate_tag_departure(idx);
            self.clients[idx].old_tags = self.clients[idx].tags;
            self.clients[idx].is_tag_switching = true;
            self.clients[idx].animation.running = false;
            self.clients[idx].tags = new;
            self.arrange_monitor(mon_idx);

            if !self.clients[idx].is_visible_on(mon_idx, self.monitors[mon_idx].current_tagset()) {
                self.focus_most_recent_visible_or_clear(mon_idx);
            }

            self.mark_state_dirty();
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Pure decision helpers (no `&mut self`), split out of the methods
// above so the tag-carousel and edge-scroller logic is unit-testable
// without standing up a full compositor `State`.
// ─────────────────────────────────────────────────────────────

/// One relative tag step. Returns the target tag *index* (0-based,
/// already wrapped into `0..max`) and the carousel slide direction to
/// stash (`+1`/`-1` only when the move wrapped past an end *and*
/// carousel is enabled, else `0`).
pub fn carousel_step(current_tag: i32, delta: i32, max: i32, carousel: bool) -> (i32, i8) {
    let raw = current_tag + delta;
    let next = raw.rem_euclid(max);
    let dir = if carousel && (raw < 0 || raw >= max) {
        delta.signum() as i8
    } else {
        0
    };
    (next, dir)
}

/// How many tags a relative view wraps over. With `tag_gather` on, tags
/// past the last occupied one are dead space, so the wrap point is
/// "highest occupied tag + 1" (capped at `max`) instead of an empty last
/// tag (mango 0.17.3 `94fd551e`). `highest_occupied` is the 1-based tag
/// number (0 when the monitor has no clients).
pub fn gather_wrap_len(highest_occupied: i32, max: i32) -> i32 {
    (highest_occupied + 1).clamp(1, max)
}

/// [`carousel_step`] over the first `wrap_len` tags only. Standing on a
/// tag past that region (an empty tag the user switched to directly):
/// forward jumps back to the first tag, backward steps down normally.
pub fn carousel_step_within(
    current_tag: i32,
    delta: i32,
    max: i32,
    wrap_len: i32,
    carousel: bool,
) -> (i32, i8) {
    let wrap_len = wrap_len.clamp(1, max);
    if current_tag < wrap_len {
        return carousel_step(current_tag, delta, wrap_len, carousel);
    }
    if delta > 0 {
        return (0, if carousel { 1 } else { 0 });
    }
    carousel_step(current_tag, delta, max, carousel)
}

/// Resolve the tag-transition slide direction: a pending carousel
/// direction (already consumed by the caller) overrides the natural
/// tag-index comparison.
pub fn transition_forward(carousel_dir: i8, old_idx_target: i32, new_idx: i32) -> bool {
    match carousel_dir {
        d if d > 0 => true,
        d if d < 0 => false,
        _ => old_idx_target > new_idx,
    }
}

/// Outcome of the edge-scroller pointer-focus decision.
#[derive(Debug, PartialEq, Eq)]
pub struct EdgeOutcome {
    /// Direction to shift focus (`-1`/`+1`), or `None` for no change.
    pub focus: Option<i32>,
    /// New debounce state (`armed`) to store back.
    pub armed: bool,
}

pub fn edge_scroller_decision(
    layout: LayoutId,
    px: f64,
    _py: f64,
    ax: i32,
    _ay: i32,
    aw: i32,
    _ah: i32,
    speed: f64,
    allow: f32,
    armed: bool,
) -> EdgeOutcome {
    const MARGIN: f64 = 8.0;
    let (dir, at_edge) = match layout {
        LayoutId::Scroller => {
            if px <= ax as f64 + MARGIN {
                (-1, true)
            } else if px >= (ax + aw) as f64 - MARGIN {
                (1, true)
            } else {
                (0, false)
            }
        }
        _ => (0, false),
    };
    if !at_edge {
        // Re-arm once the pointer leaves the edge zone.
        return EdgeOutcome {
            focus: None,
            armed: true,
        };
    }
    if armed && speed < allow as f64 {
        EdgeOutcome {
            focus: Some(dir),
            armed: false,
        }
    } else {
        EdgeOutcome { focus: None, armed }
    }
}

#[cfg(test)]
mod decision_tests {
    use super::{
        EdgeOutcome, carousel_step, carousel_step_within, edge_scroller_decision, gather_wrap_len,
        transition_forward,
    };
    use crate::layout::LayoutId;

    // ── tag carousel ───────────────────────────────────────

    #[test]
    fn gather_wrap_len_is_last_occupied_plus_one_capped() {
        assert_eq!(gather_wrap_len(0, 9), 1);
        assert_eq!(gather_wrap_len(3, 9), 4);
        assert_eq!(gather_wrap_len(9, 9), 9);
    }

    #[test]
    fn carousel_within_gathered_region_wraps_at_last_plus_one() {
        // Clients on tags 1-3 → region is 4 tags; tag 4 (idx 3) + 1 wraps to tag 1.
        assert_eq!(carousel_step_within(3, 1, 9, 4, true), (0, 1));
        // Tag 1 - 1 wraps to tag 4 (idx 3), not the empty tag 9.
        assert_eq!(carousel_step_within(0, -1, 9, 4, true), (3, -1));
        // Inside the region a normal step is untouched.
        assert_eq!(carousel_step_within(1, 1, 9, 4, true), (2, 0));
    }

    #[test]
    fn carousel_within_full_region_matches_plain_step() {
        assert_eq!(
            carousel_step_within(8, 1, 9, 9, true),
            carousel_step(8, 1, 9, true)
        );
        assert_eq!(
            carousel_step_within(0, -1, 9, 9, false),
            carousel_step(0, -1, 9, false)
        );
    }

    #[test]
    fn carousel_from_past_the_gathered_region() {
        // On empty tag 6 (idx 5) with the region at 4: forward → tag 1.
        assert_eq!(carousel_step_within(5, 1, 9, 4, true), (0, 1));
        assert_eq!(carousel_step_within(5, 1, 9, 4, false), (0, 0));
        // Backward just steps down to tag 5.
        assert_eq!(carousel_step_within(5, -1, 9, 4, true), (4, 0));
    }

    #[test]
    fn carousel_no_wrap_keeps_dir_zero() {
        // tag 1 (idx 0) → tag 2 (idx 1), no wrap.
        assert_eq!(carousel_step(0, 1, 9, true), (1, 0));
        // middle move.
        assert_eq!(carousel_step(4, 2, 9, true), (6, 0));
    }

    #[test]
    fn carousel_wrap_forward_sets_positive_dir() {
        // tag 9 (idx 8) + 1 → wraps to tag 1 (idx 0), slide forward.
        assert_eq!(carousel_step(8, 1, 9, true), (0, 1));
    }

    #[test]
    fn carousel_wrap_backward_sets_negative_dir() {
        // tag 1 (idx 0) - 1 → wraps to tag 9 (idx 8), slide backward.
        assert_eq!(carousel_step(0, -1, 9, true), (8, -1));
    }

    #[test]
    fn carousel_disabled_never_sets_dir() {
        // Same wrap, but carousel off → dir 0 (natural transition wins).
        assert_eq!(carousel_step(8, 1, 9, false), (0, 0));
        assert_eq!(carousel_step(0, -1, 9, false), (8, 0));
    }

    #[test]
    fn transition_forward_carousel_dir_overrides_indices() {
        // Positive carousel dir → forward regardless of index order.
        assert!(transition_forward(1, 0, 8));
        // Negative carousel dir → backward regardless of index order.
        assert!(!transition_forward(-1, 8, 0));
    }

    #[test]
    fn transition_forward_falls_back_to_index_compare() {
        // No carousel dir: forward iff old target index > new index.
        assert!(transition_forward(0, 8, 0));
        assert!(!transition_forward(0, 0, 8));
        assert!(!transition_forward(0, 3, 3));
    }

    // ── edge scroller ──────────────────────────────────────

    // 1000-wide area at origin; MARGIN is 8px.
    const AX: i32 = 0;

    const AY: i32 = 0;

    const AW: i32 = 1000;

    const AH: i32 = 1000;

    #[test]
    fn left_edge_slow_and_armed_focuses_previous() {
        let o = edge_scroller_decision(
            LayoutId::Scroller,
            5.0,
            500.0,
            AX,
            AY,
            AW,
            AH,
            10.0,
            40.0,
            true,
        );
        assert_eq!(
            o,
            EdgeOutcome {
                focus: Some(-1),
                armed: false
            }
        );
    }

    #[test]
    fn right_edge_slow_and_armed_focuses_next() {
        let o = edge_scroller_decision(
            LayoutId::Scroller,
            995.0,
            500.0,
            AX,
            AY,
            AW,
            AH,
            10.0,
            40.0,
            true,
        );
        assert_eq!(o.focus, Some(1));
        assert!(!o.armed);
    }

    #[test]
    fn middle_of_area_does_not_fire_and_rearms() {
        let o = edge_scroller_decision(
            LayoutId::Scroller,
            500.0,
            500.0,
            AX,
            AY,
            AW,
            AH,
            10.0,
            40.0,
            false,
        );
        assert_eq!(
            o,
            EdgeOutcome {
                focus: None,
                armed: true
            }
        );
    }

    #[test]
    fn fast_motion_at_edge_does_not_fire() {
        // Speed above the allow threshold → no focus, stays armed.
        let o = edge_scroller_decision(
            LayoutId::Scroller,
            5.0,
            500.0,
            AX,
            AY,
            AW,
            AH,
            99.0,
            40.0,
            true,
        );
        assert_eq!(o.focus, None);
        assert!(o.armed);
    }

    #[test]
    fn already_disarmed_does_not_refire_at_edge() {
        // Dwelling at the edge: once it fired (armed=false) it won't
        // fire again until the pointer leaves the edge zone.
        let o = edge_scroller_decision(
            LayoutId::Scroller,
            5.0,
            500.0,
            AX,
            AY,
            AW,
            AH,
            10.0,
            40.0,
            false,
        );
        assert_eq!(o.focus, None);
        assert!(!o.armed);
    }

    #[test]
    fn non_scroller_layout_never_fires() {
        let o = edge_scroller_decision(LayoutId::Tile, 5.0, 5.0, AX, AY, AW, AH, 1.0, 40.0, true);
        assert_eq!(
            o,
            EdgeOutcome {
                focus: None,
                armed: true
            }
        );
    }
}
