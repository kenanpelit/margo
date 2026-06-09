//! `wlr-output-power-management-v1` handler — external DPMS control.
//!
//! `set_mode(on|off)` from an idle daemon (`swayidle`) or `wlr-randr` maps
//! onto `request_dpms`, the same recoverable deferred-queue path the keybind
//! and `mctl dispatch dpms` use. Any subsequent input still wakes a darkened
//! panel (see `wake_dpms_on_input`).

use smithay::output::Output;

use crate::{
    delegate_output_power,
    protocols::output_power::{OutputPowerHandler, OutputPowerManagerState},
    state::MargoState,
};

impl OutputPowerHandler for MargoState {
    fn output_power_manager_state(&mut self) -> &mut OutputPowerManagerState {
        &mut self.output_power_manager_state
    }

    fn set_output_power(&mut self, output: &Output, on: bool) {
        let name = output.name();
        self.request_dpms(Some(on), Some(&name));
    }

    fn output_power_is_on(&mut self, _output: &Output) -> bool {
        // DPMS is driven globally (request_dpms with no target hits every
        // output), so the global flag is an accurate initial state.
        !self.any_dpms_off
    }
}
delegate_output_power!(MargoState);
