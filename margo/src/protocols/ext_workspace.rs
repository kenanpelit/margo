//! ext-workspace-v1 server state.
//!
//! Smithay does not yet implement ext-workspace-v1.  Workspace/tag state is
//! already exposed to status-bar clients (Waybar, etc.) via the
//! `dwl-ipc-unstable-v2` protocol implemented in `dwl_ipc.rs`.  This module
//! will be filled in once smithay adds native ext-workspace support.

#[derive(Debug, Default)]
pub struct ExtWorkspaceState {
    _private: (),
}

impl ExtWorkspaceState {
    pub fn new() -> Self {
        ExtWorkspaceState::default()
    }

    /// Notify status-bar clients that the visible tagset on `output_name` changed.
    /// No-op until ext-workspace-v1 is implemented; use dwl-ipc in the meantime.
    #[allow(dead_code)]
    pub fn update_tags(&self, _output_name: &str, _tagmask: u32) {}
}
