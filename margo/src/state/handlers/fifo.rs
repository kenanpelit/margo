//! `wp_fifo_v1` + `wp_commit_timing_v1` dispatch support.
//!
//! Both managers are advertised (created in `MargoState::new`) and their
//! Dispatch2 impls arrive through the blanket `delegate_dispatch2!`. The
//! managed protocol states install commit blockers; `state/pacing.rs` is the
//! scheduler that releases them — FIFO barriers at every per-output present
//! (plus the hidden-surface fallback tick), commit-timing barriers up to the
//! next present deadline or via the exact one-shot wake armed from the
//! surface pre-commit hook. Do not advertise these globals without that
//! scheduler: an unreleased barrier freezes the client's commit queue (the
//! historical hidden-tag Chromium stall).
