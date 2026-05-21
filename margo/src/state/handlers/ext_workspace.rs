//! Handler: maps `ext-workspace-v1` activate requests to margo tag views.
//!
//! `activate_workspace(output, tag_idx)` → warp focus to that monitor (if
//! needed) and view the `tag_idx`-th tag. Other requests (assign /
//! remove / create) are no-ops — margo's tag set is fixed per monitor.

use smithay::output::Output;

use crate::protocols::ext_workspace::{ExtWorkspaceHandler, ExtWorkspaceManagerState};
use crate::state::MargoState;

impl ExtWorkspaceHandler for MargoState {
    fn ext_workspace_manager_state(&mut self) -> &mut ExtWorkspaceManagerState {
        &mut self.ext_workspace_state
    }

    fn activate_workspace(&mut self, output: Output, tag_idx: usize) {
        let Some(mon_idx) = self.monitors.iter().position(|m| m.output == output) else {
            return;
        };
        if mon_idx != self.focused_monitor() {
            self.warp_focus_to_monitor(mon_idx);
        }
        self.view_tag(1u32 << tag_idx);
    }
}

crate::delegate_ext_workspace!(MargoState);
