//! `wp_fifo_v1` + `wp_commit_timing_v1` delegates.
//!
//! Newer presentation-pacing protocols. Clients use these to request
//! FIFO commit ordering and explicit commit-time targets. Pure
//! smithay state — no per-protocol handler trait.

use smithay::{delegate_commit_timing, delegate_fifo};

use crate::state::MargoState;

delegate_fifo!(MargoState);
delegate_commit_timing!(MargoState);
