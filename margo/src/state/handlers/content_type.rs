//! `wp_content_type_v1` delegate.
//!
//! Clients hint whether their surface is a game / video / photo so
//! the compositor can adjust scheduling. Pure smithay state — no
//! per-protocol handler trait; consumers look up the content type
//! per-surface via smithay's surface-data API when they need it.

use smithay::delegate_content_type;

use crate::state::MargoState;

delegate_content_type!(MargoState);
