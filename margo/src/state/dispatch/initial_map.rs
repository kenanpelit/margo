//! The deferred initial map of a new toplevel: window rules, placement
//! and focus policy once its first commit arrives (or the lock ends).

use smithay::{desktop::WindowSurface, wayland::seat::WaylandFocus};

use crate::state::{
    FocusTarget, MargoState, WindowRuleReason, read_toplevel_identity, read_toplevel_size_hints,
};

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
        self.finalize_initial_map_with_policy(idx, true, true);
    }

    fn finalize_initial_map_with_policy(
        &mut self,
        idx: usize,
        allow_focus: bool,
        allow_animation: bool,
    ) {
        // Sync the latest app_id / title from the surface before
        // running window rules — by this point Qt has had its chance.
        if idx >= self.clients.len() {
            return;
        }
        if let WindowSurface::Wayland(toplevel) = self.clients[idx].window.underlying_surface() {
            let (app_id, title) = read_toplevel_identity(toplevel);
            // Baseline size hints from the app's own xdg_toplevel request
            // (e.g. a chat client's "don't go narrower than 300px"). A
            // window_rules.conf rule with a nonzero value overrides this
            // right below, in `reapply_rules` -> `apply_matched_window_rules`.
            let (min_w, min_h, max_w, max_h) = read_toplevel_size_hints(toplevel);
            self.clients[idx].app_id = app_id;
            self.clients[idx].title = title;
            self.clients[idx].min_width = min_w;
            self.clients[idx].min_height = min_h;
            self.clients[idx].max_width = max_w;
            self.clients[idx].max_height = max_h;
        }

        // Now run rules with the live app_id/title.
        let _changed = self.reapply_rules(idx, WindowRuleReason::InitialMap);

        // Tag-home redirect: if rules picked tag N but didn't pin a
        // monitor, route to the tag's home output.
        let no_explicit_monitor = !self
            .matching_window_rules(&self.clients[idx].app_id, &self.clients[idx].title)
            .iter()
            .any(|(_, r)| r.monitor.is_some());
        if no_explicit_monitor {
            if let Some(home) = self.tag_home_monitor(self.clients[idx].tags) {
                self.clients[idx].monitor = home;
            }
        }
        // A window a session restore relaunched goes to its saved place,
        // over the window rules (the workspace-restore plugin's exec rule).
        let app_id = self.clients[idx].app_id.clone();
        if let Some((mon, tags)) = self.session_take_placement(&app_id) {
            self.clients[idx].monitor = mon;
            self.clients[idx].tags = tags;
        }

        let target_mon = self.clients[idx].monitor;
        let focus_new =
            allow_focus && !self.clients[idx].no_focus && !self.clients[idx].open_silent;
        let window = self.clients[idx].window.clone();

        let map_loc = self
            .monitors
            .get(target_mon)
            .map(|m| (m.monitor_area.x, m.monitor_area.y))
            .unwrap_or((0, 0));
        self.space.map_element(window.clone(), map_loc, true);

        if focus_new {
            if target_mon < self.monitors.len() {
                self.monitors[target_mon].prev_selected = self.monitors[target_mon].selected;
                self.monitors[target_mon].selected = Some(idx);
            }
            self.focus_surface(Some(FocusTarget::Window(window)));
        }

        // Mark the client mapped BEFORE arrange so the layout pass
        // sees it as a real participant.
        self.clients[idx].is_initial_map_pending = false;
        // Stamp the map time so `send_frame_callbacks` can keep nudging this
        // window for `warmup_hidden_ms` even on a hidden tag (lets Electron /
        // CEF apps launched at login finish initialising without visiting
        // their tag).
        self.clients[idx].mapped_at = Some(std::time::Instant::now());

        // Auto-group by `group:1` windowrule (no-op unless a rule
        // matched). Runs before the arrange below so the collapse to a
        // single slot lands in the same map pass.
        self.maybe_auto_group(idx);
        self.session_window_mapped(idx);

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
        if self.clients[idx].is_named_scratchpad && !self.clients[idx].is_in_scratchpad {
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
        if allow_animation
            && self.config.animations
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
            self.clients[idx].opening_animation = Some(crate::animation::OpenCloseClientAnim {
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

    /// Complete only the toplevels that actually attached a buffer while the
    /// desktop was locked. Stable ids survive smart-insert/plugin mutations of
    /// the client Vec; each id is resolved immediately before use. These maps
    /// are intentionally silent: unlocking restores the previous selection
    /// instead of letting the last background launch steal focus or animate.
    pub(crate) fn finish_lock_deferred_maps(&mut self) {
        use smithay::backend::renderer::utils::with_renderer_surface_state;

        let ids: Vec<u64> = self
            .clients
            .iter()
            .filter(|client| {
                client.is_initial_map_pending
                    && client.window.wl_surface().is_some_and(|surface| {
                        with_renderer_surface_state(&surface, |state| state.buffer().is_some())
                            .unwrap_or(false)
                    })
            })
            .map(|client| client.id)
            .collect();
        for id in ids {
            if let Some(idx) = self
                .clients
                .iter()
                .position(|client| client.id == id && client.is_initial_map_pending)
            {
                self.finalize_initial_map_with_policy(idx, false, false);
            }
        }
    }
}
