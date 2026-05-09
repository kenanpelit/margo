//! `wlr-gamma-control-v1` handler — night-light / blue-light filter
//! pipeline (sunsetr / gammastep / wlsunset all bind here).
//!
//! `set_gamma` requests are queued onto `MargoState::pending_gamma`
//! and drained by the backend's render path. Coalescing collapses
//! duplicate ramps for the same output so a client that spams
//! `set_gamma` (sunsetr does, every minute) doesn't grow the queue
//! unbounded.

use smithay::output::Output;

use crate::{
    delegate_gamma_control,
    protocols::gamma_control::{GammaControlHandler, GammaControlManagerState},
    state::MargoState,
};

impl GammaControlHandler for MargoState {
    fn gamma_control_manager_state(&mut self) -> &mut GammaControlManagerState {
        &mut self.gamma_control_manager_state
    }

    fn get_gamma_size(&mut self, output: &Output) -> Option<u32> {
        self.monitors
            .iter()
            .find(|m| &m.output == output)
            .map(|m| m.gamma_size)
            .filter(|&s| s > 0)
    }

    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> Option<()> {
        // Coalesce: if a pending entry already exists for this
        // output, replace it. Avoids unbounded queue growth if a
        // client spams set_gamma faster than the backend drains.
        if let Some(existing) = self
            .pending_gamma
            .iter_mut()
            .find(|(o, _)| o == output)
        {
            existing.1 = ramp;
        } else {
            self.pending_gamma.push((output.clone(), ramp));
        }
        self.request_repaint();
        Some(())
    }
}
delegate_gamma_control!(MargoState);
