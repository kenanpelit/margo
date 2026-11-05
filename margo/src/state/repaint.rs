//! Repaint scheduling and frame-callback delivery on `MargoState`.
//!
//! Moved out of `state.rs`: the dirty flag + wake ping the backends drain
//! (`request_repaint`, `take_repaint_request`, the vblank rate limiter),
//! per-output visibility helpers used to decide who gets a frame callback,
//! `send_frame_callbacks` with its sequence-based throttling, the hidden-
//! surface fallback, and the estimated-vblank timer for empty renders.
//! The per-output frame clock lives in `frame_clock_sched.rs`; FIFO /
//! commit-timing barrier release in `pacing.rs`.

use super::*;

impl MargoState {
    /// Record one rendered frame into the per-output rolling window that backs
    /// the `perf` topic's time-based metrics. Cheap and always-on: an
    /// allocation-free `get_mut` on the steady path (the entry exists after the
    /// first frame), a `push_back`, and a prune of samples older than the 60 s
    /// window (hard-capped so a high-refresh burst can't grow it unbounded).
    pub fn record_frame_sample(
        &mut self,
        output_name: &str,
        empty: bool,
        render_us: u32,
        damage_px: u32,
        scanout: bool,
    ) {
        use std::time::{Duration, Instant};
        const WINDOW: Duration = Duration::from_secs(60);
        const CAP: usize = 16_384;

        let now = Instant::now();
        let perf = match self.perf_counters.get_mut(output_name) {
            Some(perf) => perf,
            None => self
                .perf_counters
                .entry(output_name.to_string())
                .or_default(),
        };
        perf.samples.push_back(FrameSample {
            at: now,
            empty,
            render_us,
            damage_px,
            scanout,
        });
        while perf
            .samples
            .front()
            .is_some_and(|s| now.duration_since(s.at) > WINDOW)
        {
            perf.samples.pop_front();
        }
        while perf.samples.len() > CAP {
            perf.samples.pop_front();
        }
    }

    pub fn request_repaint(&mut self) {
        self.request_scene_repaint();
    }

    /// Wake the udev repaint source immediately to drain work that is not
    /// paced by scene damage: DPMS, DRM mode changes, image-copy frames, and
    /// PipeWire cast buffers. This deliberately does not dirty a frame clock.
    pub(crate) fn wake_repaint_backend(&self) {
        if let Some(ping) = &self.repaint_ping {
            ping.ping();
        }
    }

    /// Mark global scene damage without forcing an immediate event-loop wake.
    ///
    /// Unlike [`Self::request_repaint`], this is for self-sustaining animation
    /// ticks.  On the per-output clock path the already-armed present timers
    /// are the pacer; pinging as well would immediately re-enter
    /// post-dispatch, tick the animation again, and form a CPU busy-loop
    /// between vblanks.  A direct ping is only needed when timer insertion
    /// failed and no vblank/retry deadline can wake an enabled output.
    pub fn request_scene_repaint(&mut self) {
        self.repaint_requested = true;
        if !self.per_output_frame_clock_enabled() {
            let has_enabled_output = self.monitors.iter().any(|monitor| monitor.enabled);
            let every_enabled_output_backing_off = has_enabled_output
                && self
                    .monitors
                    .iter()
                    .filter(|monitor| monitor.enabled)
                    .all(|monitor| self.render_retry_pending.contains_key(&monitor.name));
            if self.pending_vblanks == 0
                && self.estimated_vblank_timers.is_empty()
                && !every_enabled_output_backing_off
                && let Some(ping) = &self.repaint_ping
            {
                ping.ping();
            }
            return;
        }

        self.mark_all_clocks_dirty();
        let missing_wake = self
            .monitors
            .iter()
            .filter(|monitor| monitor.enabled)
            .any(|monitor| {
                self.per_output_clocks
                    .get(&monitor.name)
                    .is_none_or(|clock| {
                        clock.dirty
                            && clock.timer_token.is_none()
                            && !clock.pending_vblank
                            && !self.render_retry_pending.contains_key(&monitor.name)
                    })
            });
        if missing_wake
            && self.repaint_hook.is_none()
            && let Some(ping) = &self.repaint_ping
        {
            ping.ping();
        }
    }

    /// Request a repaint for one physical output. On the per-output clock path
    /// this never dirties unrelated displays and repeated requests coalesce
    /// behind the output's existing timer/vblank. The legacy global clock has
    /// no scoped state, so it safely falls back to the paced
    /// [`Self::request_scene_repaint`] path.
    pub fn request_repaint_output(&mut self, output: &Output) {
        if !self.per_output_frame_clock_enabled() {
            self.request_scene_repaint();
            return;
        }

        self.repaint_requested = true;
        let should_ping = self.mark_output_clock_dirty(output);
        if should_ping {
            if let Some(ping) = &self.repaint_ping {
                ping.ping();
            }
        }
    }

    /// Whether Smithay's cached output set for `window` no longer matches its
    /// current bbox. `Space::map_element` and `Window::on_commit` update the
    /// location/tree immediately but output enter/leave membership only moves
    /// on `Space::refresh`.
    ///
    /// Keeping this check separate lets high-frequency commit and animation
    /// paths retain the cached fast path while still refreshing before the
    /// first render that crosses an output boundary.
    pub(crate) fn window_space_membership_is_stale(
        &self,
        window: &Window,
        cached: &[Output],
    ) -> bool {
        let Some(bbox) = self.space.element_bbox(window) else {
            return !cached.is_empty();
        };

        let mut actual_count = 0;
        for output in self.space.outputs() {
            let intersects = self
                .space
                .output_geometry(output)
                .is_some_and(|geometry| geometry.intersection(bbox).is_some());
            if intersects {
                actual_count += 1;
                if !cached.contains(output) {
                    return true;
                }
            }
        }
        actual_count != cached.len()
    }

    /// Whether the display render path currently includes this layer-shell
    /// role on `output`. Callback and commit scheduling use the same predicate
    /// as element construction so a hidden bar cannot drive empty KMS frames
    /// and a visible overview wallpaper cannot be accidentally frozen.
    pub(crate) fn layer_renders_on_output(&self, output: &Output, layer: WlrLayer) -> bool {
        if self.session_locked {
            return false;
        }
        let Some(mon_idx) = self
            .monitors
            .iter()
            .position(|monitor| monitor.enabled && monitor.output == *output)
        else {
            return false;
        };

        // The scroller renderer embeds only wallpaper-class layers in each
        // workspace cell. Top/Overlay (bar, notifications) are replaced by
        // the overview chrome.
        if self.scroller_overview.is_some() {
            return matches!(layer, WlrLayer::Background | WlrLayer::Bottom);
        }

        // Classic overview deliberately shows the complete desktop, including
        // layers, even if a thumbnail client owns an exclusive lease.
        self.monitors[mon_idx].is_overview || !self.monitor_has_exclusive_fullscreen(mon_idx)
    }

    /// Input follows layer visibility except in scroller overview, where
    /// Background/Bottom surfaces are transformed and repeated into cells.
    /// Their original LayerMap coordinates no longer match the picture, so
    /// overview navigation exclusively owns input.
    pub(crate) fn layer_accepts_input_on_output(&self, output: &Output, layer: WlrLayer) -> bool {
        self.scroller_overview.is_none() && self.layer_renders_on_output(output, layer)
    }

    /// Whether `client` is part of the live scene and may contribute pixels to
    /// `output` right now. Exclusive-fullscreen suppresses layers, not other
    /// desktop toplevels, matching Space rendering and hit-testing.
    pub(crate) fn client_renders_on_output(&self, client: &MargoClient, output: &Output) -> bool {
        if self.session_locked
            || client.is_initial_map_pending
            || client.is_minimized
            || client.is_killing
            || (client.is_in_scratchpad && !client.is_scratchpad_show)
        {
            return false;
        }
        let output_enabled = self
            .monitors
            .iter()
            .any(|monitor| monitor.enabled && monitor.output == *output);
        if !output_enabled {
            return false;
        }
        let Some(owner) = self
            .monitors
            .get(client.monitor)
            .filter(|monitor| monitor.enabled)
        else {
            return false;
        };
        if self.scroller_overview.is_some() {
            return owner.output == *output;
        }
        let tagset = if owner.is_overview {
            !0
        } else {
            owner.current_tagset()
        };
        client.is_visible_on(client.monitor, tagset)
    }

    /// Called by the udev backend after a successful `queue_frame`.
    /// Pushes the redraw scheduler into "frame in flight" mode so further
    /// repaint requests stay deferred until the page-flip completes.
    pub fn note_frame_queued(&mut self) {
        self.pending_vblanks += 1;
    }

    /// Called by the udev backend's `DrmEvent::VBlank` handler after
    /// `frame_submitted`. Decrements the in-flight counter and re-arms
    /// the redraw scheduler if the scene is dirty. Frame callbacks
    /// were already sent at `queue_frame` time (niri pattern), so
    /// VBlank itself does NOT bump sequence — `send_frame_callbacks`
    /// here is a safety-net call that dedups via the per-surface
    /// `last_sent_at` and only does work for surfaces that somehow
    /// missed the queue_frame round (rare: surfaces freshly attached
    /// after the queue, completely occluded surfaces hitting the
    /// 995 ms throttle, etc.). Also cancels any in-flight
    /// estimated-vblank timer for the output since the real vblank
    /// supersedes it.
    pub fn note_vblank(&mut self, output: &Output) {
        self.pending_vblanks = self.pending_vblanks.saturating_sub(1);

        let name = output.name();
        // Pulling the entry from the map cancels the timer at the
        // *logical* level — the timer source itself might still fire
        // (calloop has no atomic cancel), but its callback will find
        // the map empty and no-op. We intentionally do NOT call
        // `loop_handle.remove(token)` here: that would race against
        // the source's own `TimeoutAction::Drop` and produce the
        // "Received an event for non-existent source" warning that
        // was flooding the journal.
        self.estimated_vblank_timers.remove(&name);

        let now = self.clock.now();
        self.send_frame_callbacks(output, now);

        if self.pending_vblanks == 0 && self.repaint_requested {
            if let Some(ping) = &self.repaint_ping {
                ping.ping();
            }
        }
    }

    /// Install the wake handle the udev/winit backend created. Call once
    /// at startup; subsequent [`request_repaint`] calls will wake the
    /// loop via the supplied [`Ping`] sender.
    pub fn set_repaint_ping(&mut self, ping: Ping) {
        self.repaint_ping = Some(ping);
    }

    /// The cursor image changed: repaint the output(s) the pointer is on,
    /// the only ones that draw it, instead of every output.
    pub(crate) fn request_cursor_image_repaint(&mut self) {
        let pos = smithay::utils::Point::<f64, smithay::utils::Logical>::from((
            self.input_pointer.x,
            self.input_pointer.y,
        ));
        let outputs: Vec<Output> = self.space.output_under(pos).cloned().collect();
        for output in outputs {
            self.request_repaint_output(&output);
        }
    }

    /// Install the backend's repaint body (what the repaint-ping source
    /// runs), so a present timer can render the due outputs itself instead
    /// of pinging that source for one more event-loop wake-up.
    pub fn set_repaint_hook(&mut self, hook: Rc<dyn Fn(&mut MargoState)>) {
        self.repaint_hook = Some(hook);
    }

    /// Clone of the repaint-ping handle, for the per-output frame clock
    /// to wake the redraw scheduler from a present-timer callback
    /// without going through `request_repaint` (which would re-dirty
    /// every output's clock and re-arm every timer — we only want to
    /// nudge the loop so it renders whichever outputs are already due).
    pub(crate) fn repaint_ping_handle(&self) -> Option<Ping> {
        self.repaint_ping.clone()
    }

    pub fn take_repaint_request(&mut self) -> bool {
        let requested = self.repaint_requested;
        self.repaint_requested = false;
        requested
    }

    /// An output queued a frame. Frame callbacks are driven by
    /// `note_vblank` (real VBlank) and `on_estimated_vblank_timer` (empty
    /// render); this only notes that the post-render bookkeeping is owed,
    /// which [`Self::finish_repaint_batch`] runs once for the whole batch.
    pub fn post_repaint(&mut self, output: &Output, time: impl Into<std::time::Duration>) {
        let _ = time;
        let _ = output;
        self.post_repaint_pending = true;
    }

    /// End of a repaint batch (every output the backend rendered in one
    /// wake-up): refresh the smithay desktop space and popup map and mirror
    /// state to the foreign-toplevel / ext-workspace clients, once, if any
    /// output in the batch queued a frame. Returns whether it ran.
    ///
    /// These used to run after every output's frame. All four are global
    /// (`Space::refresh` recomputes every element against every output) and
    /// nothing between two outputs' renders in one batch moves an element,
    /// kills a surface or changes a tag, so the later runs were no-ops. Nor
    /// does rendering read what they update (space output membership,
    /// wl_surface.enter/leave, the popup map's dead entries): membership is
    /// read by frame callbacks, commit routing and the animation tick, all
    /// of which run outside the batch. So running them once after the last
    /// output leaves what every output draws, and everything after the
    /// batch observes, unchanged.
    pub fn finish_repaint_batch(&mut self) -> bool {
        if !std::mem::take(&mut self.post_repaint_pending) {
            return false;
        }
        self.space.refresh();
        self.popups.cleanup();
        // Mirror the toplevel set to wlr-foreign-toplevel-management clients
        // (diffing + idempotent).
        crate::protocols::wlr_foreign_toplevel::refresh(self);
        // Mirror tag state to ext-workspace clients (same diffing contract).
        crate::protocols::ext_workspace::refresh(self);
        true
    }

    /// Send `wl_surface.frame` done callbacks to every surface visible
    /// on `output`, with niri-style sequence-based dedup: each surface
    /// gets at most ONE callback per output refresh cycle, no matter
    /// how many commits the client has fired since the last cycle.
    /// This is what stops gtk4-layer-shell clients (mshell-frame,
    /// noctalia) from getting back-to-back `frame_done` events and
    /// entering a busy commit loop — the same loop that made
    /// mshell-on-margo flicker while mshell-on-Hyprland and
    /// mshell-on-niri are smooth.
    pub fn send_frame_callbacks(&mut self, output: &Output, time: impl Into<std::time::Duration>) {
        let time = time.into();
        // Every real or estimated present ends here: the next post-dispatch
        // hook may advance running animations for the next frame.
        self.animation_presented_since_tick = true;
        let presented_at = std::time::Instant::now();
        if let Some(at) = self.last_present_at.get_mut(output.name().as_str()) {
            *at = presented_at;
        } else {
            self.last_present_at.insert(output.name(), presented_at);
        }
        // Current sequence for this output. Bumped by `note_vblank`
        // and `on_estimated_vblank_timer`; surfaces stamped with this
        // value already received a frame_done this cycle.
        let sequence = *self
            .frame_callback_sequence
            .entry(output.name())
            .or_insert(0);

        // Backup throttle handed to smithay's send_frame. niri uses
        // 995 ms — long enough that it never interferes with our
        // per-cycle dedup. The real cadence comes from how often *we*
        // call this function (once per real or estimated vblank).
        let throttle = Some(std::time::Duration::from_millis(995));

        // ONLY sequence-based dedup; we deliberately do NOT add
        // niri's `surface_primary_scanout_output` filter here. Even
        // though `update_primary_scanout_outputs` now runs in the
        // udev render path, subsurfaces without per-frame buffers
        // (gtk4-layer-shell widget containers, IME composition
        // surfaces, etc.) end up with `primary_scanout = None`
        // because smithay's `element_was_presented` reports false
        // for any surface that produced no render element this
        // frame. Applying the filter then silently drops frame_done
        // callbacks for those surfaces, and the clients that own
        // them (Helium toolbar surface, mshell GTK frame clock)
        // enter a "frame_done never arrived → render harder"
        // recovery loop that burns RAM and locks the system in
        // ~10 s. Output-level filtering is still done upstream by
        // `space.outputs_for_element` / `layer_map_for_output`
        // iteration in the caller, so dropping the primary_scanout
        // gate doesn't cause cross-output spam.
        let should_send = |_surface: &WlSurface,
                           states: &smithay::wayland::compositor::SurfaceData|
         -> Option<Output> {
            let frame_state = states
                .data_map
                .get_or_insert(SurfaceFrameThrottlingState::default);
            let mut last_sent_at = frame_state.last_sent_at.borrow_mut();
            if let Some((last_output, last_sequence)) = &*last_sent_at {
                if last_output == output && *last_sequence == sequence {
                    return None;
                }
            }
            *last_sent_at = Some((output.clone(), sequence));
            Some(output.clone())
        };

        if !self.session_locked {
            // `elements_for_output` filters on the same per-element output
            // map `outputs_for_element` reads, without cloning that map
            // into a fresh Vec for every window on every vblank.
            for window in self.space.elements_for_output(output) {
                window.send_frame(output, time, throttle, should_send);
            }

            // Warm-up nudge: for `warmup_hidden_ms` after a window first maps, keep
            // delivering frame callbacks to it even while it sits on a hidden tag
            // (it isn't in `space.elements()` above). Frame-throttled clients —
            // Electron / CEF (Spotify, Webcord, Ferdium, …) — stall their renderer
            // when no frames arrive, so apps launched at login onto a background
            // tag never finish initialising until the tag is first visited. This
            // lets them warm up regardless. The sequence-based `should_send` dedup
            // keeps current-tag windows already served above from a double send.
            let warmup = self.config.warmup_hidden_ms;
            if warmup > 0
                && let Some(mon_idx) = self.monitors.iter().position(|m| &m.output == output)
            {
                let now = std::time::Instant::now();
                let window = std::time::Duration::from_millis(warmup as u64);
                let tagset = self.monitors[mon_idx].current_tagset();
                for c in &mut self.clients {
                    let Some(mapped_at) = c.mapped_at else {
                        continue;
                    };

                    if now.saturating_duration_since(mapped_at) >= window {
                        c.mapped_at = None;
                        continue;
                    }

                    if c.monitor == mon_idx
                        && !c.is_initial_map_pending
                        && !c.is_visible_on(mon_idx, tagset)
                        && !c.is_minimized
                        && !c.is_killing
                        && !c.is_in_scratchpad
                    {
                        c.window.send_frame(output, time, throttle, should_send);
                    }
                }
            }

            // While the scroller overview is open it renders EVERY tag's
            // windows live — including ones unmapped from `space` (off-screen
            // tags). Those never appear in `space.elements()`, so without
            // this they get no frame callbacks and frame-throttled clients
            // (GTK, Electron, …) won't repaint to the slot size
            // `prearrange_overview_tags` configured for them — leaving them
            // stale-sized in the overview until their tag is visited once.
            // Nudge every overview-shown window on this output's monitor; the
            // sequence-based `should_send` dedup keeps the current-tag windows
            // already served above from being sent twice.
            if self.is_scroller_overview_open()
                && let Some(mon_idx) = self.monitors.iter().position(|m| &m.output == output)
            {
                for c in &self.clients {
                    if c.monitor == mon_idx
                        && !c.is_initial_map_pending
                        && !c.is_minimized
                        && !c.is_killing
                        && !c.is_in_scratchpad
                    {
                        c.window.send_frame(output, time, throttle, should_send);
                    }
                }
            }

            {
                let map = layer_map_for_output(output);
                for layer in map
                    .layers()
                    .filter(|layer| self.layer_renders_on_output(output, layer.layer()))
                {
                    layer.send_frame(output, time, throttle, should_send);
                }
            }
        }

        // A locker (mlock) that dies without calling unlock_and_destroy --
        // killed, crashed, SIGKILLed -- leaves session_locked deliberately
        // true (see `SessionLockHandler::lock`'s doc comment: staying
        // locked is the correct security behavior, since auto-unlocking on
        // a dead locker would let anyone bypass the lock by killing it).
        // But `ClientData::disconnected` never runs cleanup, so the dead
        // client's LockSurface would otherwise sit in `lock_surfaces`
        // forever, and this function (along with the udev render path)
        // touches it every frame. `LockSurface::alive()` reflects whether
        // the underlying wl_surface resource still exists server-side --
        // false once the owning client is gone -- so prune here, the one
        // place documented to run exactly once per present cycle, before
        // anything below reads the list.
        self.lock_surfaces.retain(|(_, ls)| ls.alive());

        for (lock_output, lock_surface) in &self.lock_surfaces {
            if lock_output == output {
                send_frames_surface_tree(
                    lock_surface.wl_surface(),
                    output,
                    time,
                    throttle,
                    should_send,
                );
            }
        }

        if let CursorImageStatus::Surface(surface) = &self.cursor_status
            && self
                .space
                .output_under((self.input_pointer.x, self.input_pointer.y))
                .next()
                == Some(output)
        {
            send_frames_surface_tree(surface, output, time, throttle, should_send);
        }

        // Presentation pacing: this function runs exactly once per present
        // cycle (real vblank, empty present, estimated vblank), which makes
        // it the FIFO/commit-timing barrier release point for this output.
        self.release_pacing_barriers(output);
    }

    /// Callback-only safety net for surfaces that are not currently mapped to
    /// an output (most importantly Chromium/Electron windows on a hidden tag).
    /// Such clients otherwise receive no `wl_surface.frame` after the initial
    /// warm-up window and can put their renderer/decoder into a deep suspended
    /// state, making a tag return take seconds to recover.
    ///
    /// This does not dirty KMS or render a frame. Normally it mirrors niri's
    /// one-second fallback. During a freshly mapped hidden window's configured
    /// warm-up period the permanent timer temporarily runs at 30 Hz with a
    /// matching throttle, preserving Electron/CEF startup without manufacturing
    /// empty physical frames.
    pub fn send_frame_callbacks_fallback(&mut self) {
        let time = self.clock.now();
        let now = std::time::Instant::now();
        let warmup_window = std::time::Duration::from_millis(self.config.warmup_hidden_ms as u64);

        for client in &self.clients {
            let rendered_now = if self.scroller_overview.is_some() {
                self.monitors
                    .get(client.monitor)
                    .is_some_and(|monitor| self.client_renders_on_output(client, &monitor.output))
            } else {
                self.space
                    .outputs_for_element(&client.window)
                    .iter()
                    .any(|output| self.client_renders_on_output(client, output))
            };
            if !client.is_initial_map_pending && !client.is_killing && !rendered_now {
                let warming = self.config.warmup_hidden_ms > 0
                    && !client.is_minimized
                    && (!client.is_in_scratchpad || client.is_scratchpad_show)
                    && client.mapped_at.is_some_and(|mapped_at| {
                        now.saturating_duration_since(mapped_at) < warmup_window
                    });
                let throttle = Some(if warming {
                    std::time::Duration::from_millis(32)
                } else {
                    std::time::Duration::from_millis(995)
                });
                client.window.send_frame(
                    &self.frame_callback_fallback_output,
                    time,
                    throttle,
                    |_, _| None,
                );
            }
        }

        // Hidden surfaces never reach a per-output present pass; release
        // their pacing barriers here so a FIFO client on a hidden tag keeps
        // draining its commit queue at the fallback cadence.
        self.release_hidden_pacing_barriers();

        self.display_handle.flush_clients().ok();
    }

    /// Next delay for the permanent callback-only timer. It stays cheap at
    /// idle and accelerates only while an eligible hidden client is inside its
    /// finite startup warm-up window.
    pub fn frame_callback_fallback_interval(&self) -> std::time::Duration {
        let warmup = self.config.warmup_hidden_ms;
        if warmup > 0 {
            let now = std::time::Instant::now();
            let window = std::time::Duration::from_millis(warmup as u64);
            let has_warming_hidden = self.clients.iter().any(|client| {
                if client.is_initial_map_pending
                    || client.is_minimized
                    || client.is_killing
                    || (client.is_in_scratchpad && !client.is_scratchpad_show)
                {
                    return false;
                }
                let rendered_now = self
                    .monitors
                    .get(client.monitor)
                    .is_some_and(|monitor| self.client_renders_on_output(client, &monitor.output));
                !rendered_now
                    && client
                        .mapped_at
                        .is_some_and(|mapped_at| now.saturating_duration_since(mapped_at) < window)
            });
            if has_warming_hidden {
                return std::time::Duration::from_millis(32);
            }
        }
        std::time::Duration::from_secs(1)
    }

    /// Schedule an estimated-vblank Timer for `output`. Called from the
    /// udev render path when `render_frame` reports `is_empty == true`
    /// (no damage, no DRM page-flip, no real VBlank coming back). Without
    /// this, gtk4-layer-shell clients would stall waiting for a
    /// `wl_surface.frame` callback that the empty-render path swallowed.
    ///
    /// At most ONE timer in flight per output. If a timer is already
    /// queued we return early — the existing timer will fire at the next
    /// estimated vblank.
    pub fn queue_estimated_vblank_timer(
        &mut self,
        output: &Output,
        refresh_interval: std::time::Duration,
    ) {
        use smithay::reexports::calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        };

        let name = output.name();
        if self.estimated_vblank_timers.contains_key(&name) {
            return;
        }

        let timer = Timer::from_duration(refresh_interval);
        let cb_name = name.clone();
        let cb_output = output.clone();
        let token: Result<RegistrationToken, _> =
            self.loop_handle.insert_source(timer, move |_, _, state| {
                // Only do work if our entry is still here. If
                // `note_vblank` (real VBlank) raced us and removed
                // the entry first, our callback fires harmlessly —
                // returning `Drop` is still correct because the
                // source is one-shot.
                if state.estimated_vblank_timers.remove(&cb_name).is_some() {
                    state.on_estimated_vblank_timer(&cb_output);
                }
                TimeoutAction::Drop
            });

        match token {
            Ok(t) => {
                self.estimated_vblank_timers.insert(name, t);
            }
            Err(e) => {
                tracing::warn!("queue_estimated_vblank_timer insert_source failed: {e}");
            }
        }
    }

    /// Fired by the estimated-vblank Timer. Bumps the per-output
    /// frame_callback_sequence and re-sends frame callbacks so clients
    /// stay paced at refresh rate even when the scene is idle.
    pub fn on_estimated_vblank_timer(&mut self, output: &Output) {
        let entry = self
            .frame_callback_sequence
            .entry(output.name())
            .or_insert(0);
        *entry = entry.wrapping_add(1);
        let now = self.clock.now();
        self.send_frame_callbacks(output, now);
        self.display_handle.flush_clients().ok();

        // Scene damage may have accumulated while this estimated-vblank
        // timer was pacing an empty legacy-clock frame.  `request_scene_repaint`
        // deliberately did not add a second wake; hand ownership back to the
        // repaint source now that the pacing deadline has arrived.
        if !self.per_output_frame_clock_enabled()
            && self.repaint_requested
            && self.pending_vblanks == 0
            && let Some(ping) = &self.repaint_ping
        {
            ping.ping();
        }
    }
}
