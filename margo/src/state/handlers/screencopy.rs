//! `wlr-screencopy-unstable-v1` handler — full-output / region screen
//! capture for grim, wf-recorder, OBS, Discord screen-share via xdp-wlr.
//!
//! Frame requests are deferred to the backend's render path: the
//! manager state holds the pending screencopy until the next frame
//! for the target output is rendered. The dmabuf-vs-SHM choice and
//! region clipping happens inside the manager itself.

use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::{
    delegate_screencopy,
    protocols::screencopy::{Screencopy, ScreencopyHandler, ScreencopyManagerState},
    state::MargoState,
};

impl ScreencopyHandler for MargoState {
    fn screencopy_state(&mut self) -> &mut ScreencopyManagerState {
        &mut self.screencopy_state
    }

    fn frame(&mut self, manager: &ZwlrScreencopyManagerV1, screencopy: Screencopy) {
        // Defer the actual buffer copy to the backend's render path —
        // the queue holds the screencopy until the next frame is
        // rendered for that output.
        self.screencopy_state.push(manager, screencopy);
        self.request_repaint();
    }
}
delegate_screencopy!(MargoState);
