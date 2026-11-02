//! `wlr_gamma_control_unstable_v1` server implementation.
//!
//! Adapted from niri's `src/protocols/gamma_control.rs`. Lets clients like
//! `sunsetr`, `gammastep`, `wlsunset` push gamma ramps to the compositor's
//! outputs for blue-light filtering / night mode.
//!
//! The actual ramp application lives in the udev backend (see
//! `backend::udev::GammaProps`). Margo treats `set_gamma` as enqueueing a
//! request that the backend drains on the next frame.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use tracing::warn;
use wayland_protocols_wlr::gamma_control::v1::server::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};
use zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1;
use zwlr_gamma_control_v1::ZwlrGammaControlV1;

const VERSION: u32 = 1;

pub struct GammaControlManagerState {
    /// Active gamma controls — failed ones are removed.
    gamma_controls: HashMap<Output, ZwlrGammaControlV1>,
}

pub struct GammaControlManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait GammaControlHandler {
    fn gamma_control_manager_state(&mut self) -> &mut GammaControlManagerState;
    fn get_gamma_size(&mut self, output: &Output) -> Option<u32>;
    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> Option<()>;
}

pub struct GammaControlState {
    gamma_size: u32,
}

impl GammaControlManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlManagerGlobalData>,
        D: Dispatch<ZwlrGammaControlManagerV1, ()>,
        D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
        D: GammaControlHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = GammaControlManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ZwlrGammaControlManagerV1, _>(VERSION, global_data);

        Self {
            gamma_controls: HashMap::new(),
        }
    }

    /// Tell every active client that its control is gone (output unplugged etc).
    pub fn output_removed(&mut self, output: &Output) {
        if let Some(gamma_control) = self.gamma_controls.remove(output) {
            gamma_control.failed();
        }
    }
}

impl<D> GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlManagerGlobalData, D>
    for GammaControlManagerState
where
    D: GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlManagerGlobalData>,
    D: Dispatch<ZwlrGammaControlManagerV1, ()>,
    D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
    D: GammaControlHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrGammaControlManagerV1>,
        _manager_state: &GammaControlManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &GammaControlManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrGammaControlManagerV1, (), D> for GammaControlManagerState
where
    D: Dispatch<ZwlrGammaControlManagerV1, ()>,
    D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
    D: GammaControlHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrGammaControlManagerV1,
        request: <ZwlrGammaControlManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } => {
                if let Some(output) = Output::from_resource(&output) {
                    #[allow(clippy::map_entry)]
                    if !state
                        .gamma_control_manager_state()
                        .gamma_controls
                        .contains_key(&output)
                    {
                        if let Some(gamma_size) = state.get_gamma_size(&output) {
                            let zwlr_gamma_control =
                                data_init.init(id, GammaControlState { gamma_size });
                            zwlr_gamma_control.gamma_size(gamma_size);
                            state
                                .gamma_control_manager_state()
                                .gamma_controls
                                .insert(output, zwlr_gamma_control);
                            return;
                        }
                    }
                }
                data_init
                    .init(id, GammaControlState { gamma_size: 0 })
                    .failed();
            }
            zwlr_gamma_control_manager_v1::Request::Destroy => (),
            _ => unreachable!("zwlr_gamma_control_manager_v1 request not in protocol XML"),
        }
    }
}

impl<D> Dispatch<ZwlrGammaControlV1, GammaControlState, D> for GammaControlManagerState
where
    D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
    D: GammaControlHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrGammaControlV1,
        request: <ZwlrGammaControlV1 as Resource>::Request,
        data: &GammaControlState,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_gamma_control_v1::Request::SetGamma { fd } => {
                let gamma_controls = &mut state.gamma_control_manager_state().gamma_controls;
                let Some((output, _)) = gamma_controls.iter().find(|(_, x)| *x == resource) else {
                    return;
                };
                let output = output.clone();

                // Read 3*gamma_size u16 channels (R, G, B) from the fd.
                let ramp = match read_gamma_ramp(fd, data.gamma_size) {
                    Ok(ramp) => ramp,
                    Err(err) => {
                        warn!("gamma_control: rejecting ramp: {err}");
                        resource.failed();
                        gamma_controls.remove(&output);
                        let _ = state.set_gamma(&output, None);
                        return;
                    }
                };
                if state.set_gamma(&output, Some(ramp)).is_none() {
                    resource.failed();
                    let gamma_controls = &mut state.gamma_control_manager_state().gamma_controls;
                    gamma_controls.remove(&output);
                    let _ = state.set_gamma(&output, None);
                }
            }
            zwlr_gamma_control_v1::Request::Destroy => (),
            _ => unreachable!("zwlr_gamma_control_v1 request not in protocol XML"),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ZwlrGammaControlV1,
        _data: &GammaControlState,
    ) {
        let gamma_controls = &mut state.gamma_control_manager_state().gamma_controls;
        let Some((output, _)) = gamma_controls.iter().find(|(_, x)| *x == resource) else {
            return;
        };
        let output = output.clone();
        gamma_controls.remove(&output);
        let _ = state.set_gamma(&output, None);
    }
}

/// Read one `set_gamma` ramp — `3 * gamma_size` native-endian u16s (R, G, B)
/// — from the client's fd. Any short or oversized ramp is an error.
///
/// This runs on the compositor thread and the global is open to clients, so
/// it must never block: a client could pass the read end of a pipe it never
/// writes and freeze every output and all input. The fd is switched to
/// `O_NONBLOCK` first (wlroots does the same); data that isn't there yet is
/// a short ramp, and the control fails.
pub fn read_gamma_ramp(fd: OwnedFd, gamma_size: u32) -> std::io::Result<Vec<u16>> {
    use smithay::reexports::rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
    let flags = fcntl_getfl(&fd)?;
    fcntl_setfl(&fd, flags | OFlags::NONBLOCK)?;

    // Start with a u16 slice so it's aligned correctly.
    let mut gamma = vec![0u16; gamma_size as usize * 3];
    let mut file = File::from(fd);
    file.read_exact(bytemuck::cast_slice_mut(&mut gamma))?;
    // Verify that there's no more data. A pipe whose writer is still open
    // reports WouldBlock here: nothing more has been sent, so accept it.
    match file.read(&mut [0]) {
        Ok(0) => Ok(gamma),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(gamma),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "ramp is larger than 3 * gamma_size",
        )),
        Err(err) => Err(err),
    }
}

#[macro_export]
macro_rules! delegate_gamma_control {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1: $crate::protocols::gamma_control::GammaControlManagerGlobalData
        ] => $crate::protocols::gamma_control::GammaControlManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1: ()
        ] => $crate::protocols::gamma_control::GammaControlManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_v1::ZwlrGammaControlV1: $crate::protocols::gamma_control::GammaControlState
        ] => $crate::protocols::gamma_control::GammaControlManagerState);
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::rustix;
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Run `read_gamma_ramp` on another thread so a blocking read fails the
    /// test instead of hanging it.
    fn read_with_timeout(fd: OwnedFd, gamma_size: u32) -> std::io::Result<Vec<u16>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(read_gamma_ramp(fd, gamma_size));
        });
        rx.recv_timeout(Duration::from_secs(2))
            .expect("read_gamma_ramp blocked on the client's fd")
    }

    fn ramp_bytes(gamma_size: u32) -> Vec<u8> {
        let ramp: Vec<u16> = (0..gamma_size * 3).map(|i| i as u16).collect();
        bytemuck::cast_slice(&ramp).to_vec()
    }

    fn memfd_with(bytes: &[u8]) -> OwnedFd {
        let fd = rustix::fs::memfd_create("gamma-test", rustix::fs::MemfdFlags::CLOEXEC)
            .expect("memfd_create");
        let mut file = File::from(fd);
        file.write_all(bytes).expect("write memfd");
        rustix::fs::seek(&file, rustix::fs::SeekFrom::Start(0)).expect("seek");
        file.into()
    }

    #[test]
    fn a_pipe_that_is_never_written_does_not_block() {
        // A client can hand margo the read end of a pipe and never write
        // to it. The read runs on the compositor thread, so blocking here
        // froze every output and all input.
        let (read, _write) = rustix::pipe::pipe().expect("pipe");
        assert!(read_with_timeout(read, 4).is_err());
    }

    #[test]
    fn a_short_ramp_in_a_pipe_is_rejected_without_blocking() {
        let (read, write) = rustix::pipe::pipe().expect("pipe");
        let mut w = File::from(write);
        w.write_all(&ramp_bytes(4)[..10]).expect("write");
        // Writer stays open: a blocking read would wait for the rest.
        assert!(read_with_timeout(read, 4).is_err());
        drop(w);
    }

    #[test]
    fn a_full_ramp_in_a_pipe_whose_writer_is_still_open_is_accepted() {
        let (read, write) = rustix::pipe::pipe().expect("pipe");
        let mut w = File::from(write);
        w.write_all(&ramp_bytes(4)).expect("write");
        let ramp = read_with_timeout(read, 4).expect("complete ramp");
        assert_eq!(ramp, (0..12).collect::<Vec<u16>>());
        drop(w);
    }

    #[test]
    fn a_complete_ramp_in_a_memfd_is_read() {
        let ramp = read_with_timeout(memfd_with(&ramp_bytes(256)), 256).expect("ramp");
        assert_eq!(ramp.len(), 768);
        assert_eq!(ramp[767], 767);
    }

    #[test]
    fn short_and_oversized_ramps_are_rejected() {
        let bytes = ramp_bytes(256);
        assert!(read_with_timeout(memfd_with(&bytes[..bytes.len() - 2]), 256).is_err());
        let mut long = bytes.clone();
        long.extend_from_slice(&[0, 0]);
        assert!(read_with_timeout(memfd_with(&long), 256).is_err());
    }
}
