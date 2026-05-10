//! Server half of the integration test fixture (W1.6).
//!
//! Wraps a real `MargoState` driven by a real `calloop::EventLoop`,
//! the same way `main.rs` does — minus the udev / winit backend.
//! Tests poke the Wayland protocol surface from the [`super::client`]
//! side, the [`super::fixture::Fixture`] threads dispatch between
//! them, and the protocol-handler impls in `state/handlers/` execute
//! against the same code path real margo runs.

use std::time::Duration;

use calloop::EventLoop;
use margo_config::Config;
use smithay::reexports::wayland_server::Display;

use crate::state::MargoState;

/// Owns the margo-side compositor: event loop, display, state.
///
/// `dispatch` runs one event-loop turn at zero timeout, then forces
/// any pending client events out via `flush_clients`. That mirrors
/// the per-iteration flush in `main.rs`'s display source.
pub struct Server {
    pub event_loop: EventLoop<'static, MargoState>,
    pub display: Display<MargoState>,
    pub state: MargoState,
}

impl Server {
    pub fn new(config: Config) -> Self {
        let event_loop: EventLoop<'static, MargoState> =
            EventLoop::try_new().expect("test EventLoop::try_new");
        let mut display: Display<MargoState> =
            Display::new().expect("test Display::new");
        let loop_handle = event_loop.handle();
        let loop_signal = event_loop.get_signal();

        let state = MargoState::new(
            config,
            &mut display,
            loop_handle,
            loop_signal,
            None,
        );

        Self {
            event_loop,
            display,
            state,
        }
    }

    /// One zero-timeout event-loop dispatch + dispatch any queued
    /// client requests on the display fd + flush responses.
    pub fn dispatch(&mut self) {
        self.event_loop
            .dispatch(Duration::ZERO, &mut self.state)
            .expect("server dispatch");
        self.display
            .dispatch_clients(&mut self.state)
            .expect("dispatch_clients");
        self.state
            .display_handle
            .flush_clients()
            .expect("flush_clients");
    }
}
