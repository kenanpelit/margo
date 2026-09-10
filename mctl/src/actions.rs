//! The dispatch-action catalogue moved to `margo-config` (the config
//! validator needs it, and `margo-config` is the shared leaf crate).
//! Re-exported here so `mctl::actions::{ACTIONS, Group, all_names, …}`
//! keeps resolving.

pub use margo_config::actions::*;
