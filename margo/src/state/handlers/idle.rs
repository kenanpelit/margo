//! `ext-idle-notify-v1` + `idle-inhibit` handlers.
//!
//! `IdleNotifierHandler` exposes the per-seat timer state to smithay;
//! `IdleInhibitHandler` lets clients (mpv, video players, anything
//! holding `zwp_idle_inhibit_manager_v1`) pause those timers while
//! they're playing.

use smithay::{
    delegate_idle_inhibit, delegate_idle_notify,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        idle_inhibit::IdleInhibitHandler,
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
    },
};

use crate::state::MargoState;

impl IdleNotifierHandler for MargoState {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}
delegate_idle_notify!(MargoState);

impl IdleInhibitHandler for MargoState {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.insert(surface);
        // Pause idle timers as long as anything is inhibiting.
        let inhibited = !self.idle_inhibitors.is_empty();
        self.idle_notifier_state.set_is_inhibited(inhibited);
        tracing::debug!(
            "idle_inhibit: active={} count={}",
            inhibited,
            self.idle_inhibitors.len()
        );
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.remove(&surface);
        let inhibited = !self.idle_inhibitors.is_empty();
        self.idle_notifier_state.set_is_inhibited(inhibited);
        tracing::debug!(
            "idle_uninhibit: active={} count={}",
            inhibited,
            self.idle_inhibitors.len()
        );
    }
}
delegate_idle_inhibit!(MargoState);
