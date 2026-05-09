//! `wp-color-management-v1` (staging) handler — HDR Phase 1
//! scaffolding.
//!
//! Most of the logic lives in `crate::protocols::color_management`;
//! `MargoState` only needs to expose the manager state. Phases 2/3
//! (linear-light fp16 composite + KMS HDR scan-out) read the active
//! description's identity off the per-surface tracker the protocol
//! module maintains.

use crate::{
    delegate_color_management,
    protocols::color_management::{ColorManagementHandler, ColorManagementState},
    state::MargoState,
};

impl ColorManagementHandler for MargoState {
    fn color_management_state(&mut self) -> &mut ColorManagementState {
        &mut self.color_management_state
    }
}
delegate_color_management!(MargoState);
