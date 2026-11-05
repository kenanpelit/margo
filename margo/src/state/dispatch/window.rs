//! Per-window dispatch actions: sticky, floating, fullscreen (and the
//! exclusive-fullscreen query), move/resize of floating windows and zoom.

use smithay::desktop::WindowSurface;

use super::{MAX_FLOAT_COORD, MIN_FLOAT_SIZE, osd_toast};
use crate::state::{FullscreenMode, MargoState};

impl MargoState {
    /// Refresh `clients[idx].float_geom` from its current (tiled) `geom`
    /// before it's used as the base for a move / resize / float-toggle,
    /// when the stored value can't be trusted: the window wasn't already
    /// floating — whatever `float_geom` holds could be left over from an
    /// unrelated, possibly long-past floating episode — or it *is*
    /// floating but the remembered size has collapsed below
    /// [`MIN_FLOAT_SIZE`].
    ///
    /// That second case is the exact shape of a real bug: `movewin` /
    /// `resizewin` always floated the focused window and only seeded
    /// `float_geom` from `geom` when `float_geom.width == 0` — so a
    /// window once shrunk to a sliver by `resizewin`, then tiled again
    /// (looking perfectly normal), silently snapped back to that
    /// invisible sliver the next time *any* of `movewin` / `resizewin` /
    /// `togglefloating` floated it, because the stale-but-nonzero
    /// `float_geom` was never invalidated. A window currently floating
    /// with a sane, user-placed `float_geom` is left alone — only a
    /// tiled window (whose `float_geom` is irrelevant history) or a
    /// genuinely broken one gets reseeded.
    fn reseed_float_geom_if_stale(&mut self, idx: usize) {
        let c = &self.clients[idx];
        let stale = !c.is_floating
            || c.float_geom.width < MIN_FLOAT_SIZE
            || c.float_geom.height < MIN_FLOAT_SIZE;
        if stale {
            self.clients[idx].float_geom = self.clients[idx].geom;
        }
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
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
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
        self.mark_state_dirty();
        self.request_repaint();
        crate::scripting::fire_focus_change(self);

        let title = if was_sticky {
            "Sticky off"
        } else {
            "Sticky on"
        };
        let body = if appid.is_empty() {
            String::from("Focused window")
        } else {
            appid
        };
        osd_toast(title, "view-pin-symbolic", &body);
    }

    pub fn toggle_floating(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            let will_float = !self.clients[idx].is_floating;
            if will_float {
                // Transitioning tiled -> floating: the tiled slot is the
                // only geometry guaranteed sane right now.
                self.reseed_float_geom_if_stale(idx);
            }
            self.clients[idx].is_floating = will_float;
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
            // the snapshot reports `floating` per client; the old per-output focused
            // client; the bar status indicator (noctalia "tile/float"
            // glyph) needs an explicit broadcast or it stays stale.
            self.mark_state_dirty();
        }
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
    /// 4. `mark_state_dirty` pushes a watch frame (which carries
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
    ///    new mode to size the geometry, then IPC watch subscribers
    ///    (noctalia / waybar-dwl) see the updated state snapshot.
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

        if let WindowSurface::Wayland(toplevel) = self.clients[idx].window.underlying_surface() {
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
        self.mark_state_dirty();
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
                && !c.is_initial_map_pending
                && !c.is_minimized
                && !c.is_killing
                && (!c.is_in_scratchpad || c.is_scratchpad_show)
                && c.fullscreen_mode == FullscreenMode::Exclusive
                && c.is_visible_on(mon_idx, active_tagset)
        })
    }

    pub fn move_focused(&mut self, dx: i32, dy: i32) {
        if let Some(idx) = self.focused_client_idx() {
            self.reseed_float_geom_if_stale(idx);
            self.clients[idx].is_floating = true;
            let g = &mut self.clients[idx].float_geom;
            g.x =
                g.x.saturating_add(dx)
                    .clamp(-MAX_FLOAT_COORD, MAX_FLOAT_COORD);
            g.y =
                g.y.saturating_add(dy)
                    .clamp(-MAX_FLOAT_COORD, MAX_FLOAT_COORD);
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn resize_focused(&mut self, dw: i32, dh: i32) {
        if let Some(idx) = self.focused_client_idx() {
            self.reseed_float_geom_if_stale(idx);
            self.clients[idx].is_floating = true;
            let g = &mut self.clients[idx].float_geom;
            g.width = g
                .width
                .saturating_add(dw)
                .clamp(MIN_FLOAT_SIZE, MAX_FLOAT_COORD);
            g.height = g
                .height
                .saturating_add(dh)
                .clamp(MIN_FLOAT_SIZE, MAX_FLOAT_COORD);
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

        self.swap_clients(a, b);
        self.arrange_monitor(mon_idx);
    }
}
