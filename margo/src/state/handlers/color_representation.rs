//! `wp-color-representation-v1` (staging) handler.
//!
//! The logic lives in `crate::protocols::color_representation`;
//! `MargoState` only exposes the manager state (surface-exists
//! bookkeeping).

use crate::{
    delegate_color_representation,
    protocols::color_representation::{ColorRepresentationHandler, ColorRepresentationState},
    state::MargoState,
};

impl ColorRepresentationHandler for MargoState {
    fn color_representation_state(&mut self) -> &mut ColorRepresentationState {
        &mut self.color_representation_state
    }
}
delegate_color_representation!(MargoState);
