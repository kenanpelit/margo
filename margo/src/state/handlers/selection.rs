//! Selection-family handlers — clipboard / primary selection / X11
//! mirror.
//!
//! Bundles the four protocols that together implement copy-paste:
//! `wl_data_device_manager` (drag / drop / clipboard from native
//! clients), `wp_primary_selection` (middle-click paste),
//! `wlr_data_control` (clipboard managers like CopyQ / cliphist /
//! clipse), and the bridging `SelectionHandler` that mirrors
//! Wayland selections into XWayland and back.

use std::os::unix::io::OwnedFd;

use smithay::{
    delegate_data_control, delegate_data_device, delegate_ext_data_control,
    delegate_primary_selection,
    input::{dnd::DndGrabHandler, Seat},
    wayland::selection::{
        data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler},
        ext_data_control::{
            DataControlHandler as ExtDataControlHandler,
            DataControlState as ExtDataControlState,
        },
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
        wlr_data_control::{DataControlHandler, DataControlState},
        SelectionHandler, SelectionSource, SelectionTarget,
    },
};

use crate::state::MargoState;

impl SelectionHandler for MargoState {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.new_selection(ty, source.map(|source| source.mime_types())) {
                tracing::warn!(?err, ?ty, "failed to mirror Wayland selection to XWayland");
            }
        }
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.send_selection(ty, mime_type, fd) {
                tracing::warn!(?err, ?ty, "failed to send Wayland selection to XWayland");
            }
        }
    }
}

impl DataDeviceHandler for MargoState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}
impl WaylandDndGrabHandler for MargoState {}
impl DndGrabHandler for MargoState {}
delegate_data_device!(MargoState);

impl PrimarySelectionHandler for MargoState {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}
delegate_primary_selection!(MargoState);

impl DataControlHandler for MargoState {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}
delegate_data_control!(MargoState);

impl ExtDataControlHandler for MargoState {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.ext_data_control_state
    }
}
delegate_ext_data_control!(MargoState);
