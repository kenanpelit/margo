//! Dormant `wp_fifo_v1` + `wp_commit_timing_v1` dispatch support.
//!
//! Smithay's dispatch implementations remain available at compile time, but
//! Margo deliberately creates neither manager global. The managed protocol
//! states install commit blockers; they must not be advertised until the
//! presentation loop signals FIFO barriers and commit deadlines and then
//! notifies the client compositor state that each blocker was cleared.
