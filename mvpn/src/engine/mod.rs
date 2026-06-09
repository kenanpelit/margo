//! mvpn engine — GTK-free Mullvad VPN logic, shared by the CLI and the panel.
//!
//! Everything here is a thin layer over the `mullvad` CLI + on-disk files; it
//! holds no persistent state of its own (the daemon + files are the source of
//! truth), so the CLI and GUI can both call straight in.

pub mod actions;
pub mod blocky;
pub mod diag;
pub mod favorites;
pub mod latency;
pub mod obf;
pub mod relays;
pub mod slot;
pub mod status;
pub mod sys;
pub mod timer;
