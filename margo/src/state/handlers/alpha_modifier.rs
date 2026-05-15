//! `wp_alpha_modifier_v1` delegate.
//!
//! Per-surface alpha hint — apps tag themselves as faded / dimmed
//! without going through compositor effects. Pure smithay state; no
//! handler trait.

use smithay::delegate_alpha_modifier;

use crate::state::MargoState;

delegate_alpha_modifier!(MargoState);
