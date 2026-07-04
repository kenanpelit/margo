//! `wlr_output_power_management_unstable_v1` server implementation.
//!
//! Lets external clients (idle daemons like `swayidle`, `wlr-randr`) power an
//! output's panel off/on (DPMS). `set_mode` maps straight onto
//! [`MargoState::request_dpms`] — the actual DRM off/on is the deferred-queue
//! + `DrmCompositor::clear()` path (see `backend::udev::frame`), and any input
//! still wakes a darkened panel. We emit the `mode` event on object creation
//! and whenever the panel power changes (via [`OutputPowerManagerState::
//! output_power_changed`], called from `request_dpms`).

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1, zwlr_output_power_v1,
};
use zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1;
use zwlr_output_power_v1::{Mode, ZwlrOutputPowerV1};

const VERSION: u32 = 1;

pub struct OutputPowerManagerState {
    /// Active power objects. One output can have several (multiple clients);
    /// all get the `mode` event when that output's power changes.
    controls: Vec<(Output, ZwlrOutputPowerV1)>,
}

pub struct OutputPowerManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait OutputPowerHandler {
    fn output_power_manager_state(&mut self) -> &mut OutputPowerManagerState;
    /// Apply a DPMS power change to `output` (true = on). Implemented via
    /// `request_dpms`, which is the recoverable deferred-queue path.
    fn set_output_power(&mut self, output: &Output, on: bool);
    /// Current power state of `output` (true = on), for the initial `mode`.
    fn output_power_is_on(&mut self, output: &Output) -> bool;
}

impl OutputPowerManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData>,
        D: Dispatch<ZwlrOutputPowerManagerV1, ()>,
        D: Dispatch<ZwlrOutputPowerV1, Output>,
        D: OutputPowerHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        display.create_global::<D, ZwlrOutputPowerManagerV1, _>(
            VERSION,
            OutputPowerManagerGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            controls: Vec::new(),
        }
    }

    /// Notify every power object bound to `output` of its new power state.
    /// Called from `request_dpms` so clients track DPMS changes (whether they
    /// originated from the protocol, a keybind, or `mctl`).
    pub fn output_power_changed(&self, output: &Output, on: bool) {
        let mode = if on { Mode::On } else { Mode::Off };
        for (o, ctrl) in &self.controls {
            if o == output {
                ctrl.mode(mode);
            }
        }
    }

    /// An output went away — fail its controls so clients drop them.
    pub fn output_removed(&mut self, output: &Output) {
        self.controls.retain(|(o, ctrl)| {
            if o == output {
                ctrl.failed();
                false
            } else {
                true
            }
        });
    }
}

impl<D> GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData, D>
    for OutputPowerManagerState
where
    D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData>,
    D: Dispatch<ZwlrOutputPowerManagerV1, ()>,
    D: Dispatch<ZwlrOutputPowerV1, Output>,
    D: OutputPowerHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrOutputPowerManagerV1>,
        _data: &OutputPowerManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, data: &OutputPowerManagerGlobalData) -> bool {
        (data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrOutputPowerManagerV1, (), D> for OutputPowerManagerState
where
    D: Dispatch<ZwlrOutputPowerManagerV1, ()>,
    D: Dispatch<ZwlrOutputPowerV1, Output>,
    D: OutputPowerHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrOutputPowerManagerV1,
        request: <ZwlrOutputPowerManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } => {
                match Output::from_resource(&output) {
                    Some(output) => {
                        let ctrl = data_init.init(id, output.clone());
                        // Initial state event, per the protocol.
                        let on = state.output_power_is_on(&output);
                        ctrl.mode(if on { Mode::On } else { Mode::Off });
                        state
                            .output_power_manager_state()
                            .controls
                            .push((output, ctrl));
                    }
                    None => {
                        // No such output — we still must init the new_id, then
                        // immediately fail it. The data Output is a throwaway
                        // (never registered as a global, never matched).
                        let dead = Output::new(
                            "dead".into(),
                            smithay::output::PhysicalProperties {
                                size: (0, 0).into(),
                                subpixel: smithay::output::Subpixel::Unknown,
                                make: String::new(),
                                model: String::new(),
                                serial_number: String::new(),
                            },
                        );
                        data_init.init(id, dead).failed();
                    }
                }
            }
            zwlr_output_power_manager_v1::Request::Destroy => (),
            _ => unreachable!("zwlr_output_power_manager_v1 request not in protocol XML"),
        }
    }
}

impl<D> Dispatch<ZwlrOutputPowerV1, Output, D> for OutputPowerManagerState
where
    D: Dispatch<ZwlrOutputPowerV1, Output>,
    D: OutputPowerHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrOutputPowerV1,
        request: <ZwlrOutputPowerV1 as Resource>::Request,
        output: &Output,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_v1::Request::SetMode { mode } => match mode {
                WEnum::Value(Mode::On) => state.set_output_power(output, true),
                WEnum::Value(Mode::Off) => state.set_output_power(output, false),
                _ => resource.post_error(
                    zwlr_output_power_v1::Error::InvalidMode,
                    "nonexistent power save mode",
                ),
            },
            zwlr_output_power_v1::Request::Destroy => (),
            _ => unreachable!("zwlr_output_power_v1 request not in protocol XML"),
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &ZwlrOutputPowerV1, _output: &Output) {
        state
            .output_power_manager_state()
            .controls
            .retain(|(_, ctrl)| ctrl != resource);
    }
}

#[macro_export]
macro_rules! delegate_output_power {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1: $crate::protocols::output_power::OutputPowerManagerGlobalData
        ] => $crate::protocols::output_power::OutputPowerManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1: ()
        ] => $crate::protocols::output_power::OutputPowerManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_v1::ZwlrOutputPowerV1: smithay::output::Output
        ] => $crate::protocols::output_power::OutputPowerManagerState);
    };
}
