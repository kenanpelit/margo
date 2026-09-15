//! `ext-idle-notify-v1` + `idle-inhibit` handlers.
//!
//! `IdleNotifierHandler` exposes the per-seat timer state to smithay;
//! `IdleInhibitHandler` lets clients (mpv, video players, anything
//! holding `zwp_idle_inhibit_manager_v1`) pause those timers while
//! they're playing. [`MargoState::recompute_idle_inhibit`] layers two
//! compositor-side heuristics on top of that protocol path (mango
//! ports): a client can be exempted from the "must be visible to
//! count" default, and a window rule or fullscreen state can inhibit
//! idle without the client ever calling the protocol at all.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        idle_inhibit::IdleInhibitHandler,
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
        seat::WaylandFocus,
    },
};

use crate::state::MargoState;

impl IdleNotifierHandler for MargoState {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}

impl IdleInhibitHandler for MargoState {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.insert(surface);
        self.recompute_idle_inhibit();
        tracing::debug!("idle_inhibit: count={}", self.idle_inhibitors.len());
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.remove(&surface);
        self.recompute_idle_inhibit();
        tracing::debug!("idle_uninhibit: count={}", self.idle_inhibitors.len());
    }
}

impl MargoState {
    /// Recompute whether idle timers should be inhibited right now, and
    /// push the result to the notifier. Three independent sources OR
    /// together:
    ///
    ///  * Protocol requests (`idle_inhibitors`) — filtered by visibility
    ///    unless `idleinhibit_ignore_visible` is set, so a background
    ///    player parked on a hidden tag doesn't keep the screen awake by
    ///    default. A request from a not-yet-mapped surface (no matching
    ///    `MargoClient` yet) always counts — there's no visibility to
    ///    check, and treating it as invisible would drop real inhibits
    ///    during the brief pre-map window.
    ///  * `idle_inhibit_when_focus` window rule — the focused client
    ///    inhibits unconditionally while focused, protocol request or
    ///    not.
    ///  * `idle_inhibit_when_fullscreen` — the focused client is
    ///    fullscreen, protocol request or not.
    ///
    /// Callers: the protocol handlers above (a request appeared or
    /// went away), `focus_surface` (focus-driven sources can flip),
    /// and `arrange_monitor` (covers a fullscreen toggle or a tag
    /// switch changing which inhibitors are visible, neither of which
    /// necessarily changes focus).
    pub(crate) fn recompute_idle_inhibit(&mut self) {
        let ignore_visible = self.config.idleinhibit_ignore_visible;
        let protocol_inhibited = self.idle_inhibitors.iter().any(|surface| {
            let Some(client) = self
                .clients
                .iter()
                .find(|c| c.window.wl_surface().as_deref() == Some(surface))
            else {
                return true;
            };
            ignore_visible
                || (client.monitor < self.monitors.len()
                    && client.is_visible_on(
                        client.monitor,
                        self.monitors[client.monitor].current_tagset(),
                    ))
        });

        let focused = self.focused_client_idx().map(|idx| &self.clients[idx]);
        let heuristic_inhibited = focused.is_some_and(|c| {
            c.idle_inhibit_when_focus
                || (self.config.idle_inhibit_when_fullscreen && c.is_fullscreen)
        });

        self.idle_notifier_state
            .set_is_inhibited(protocol_inhibited || heuristic_inhibited);
    }
}
