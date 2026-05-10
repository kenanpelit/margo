//! Integration test fixture for margo (W1.6 — see `road_map.md`).
//!
//! Imports both halves of the harness ([`fixture::Fixture`],
//! [`server::Server`], [`client::Client`]) plus the test modules
//! that exercise the just-split protocol handlers in
//! `state/handlers/`.
//!
//! This file is gated on `#[cfg(test)]` so the harness vanishes
//! from release builds — `wayland-client::Connection` would
//! otherwise force a runtime dep on the system `libwayland-client`
//! at the wrong scope.
//!
//! # Adding a test
//!
//! 1. Write the test in a new file at `tests/<name>.rs`.
//! 2. Declare `mod <name>;` below.
//! 3. The body looks like:
//!
//!    ```ignore
//!    use super::fixture::Fixture;
//!
//!    #[test]
//!    fn descriptive_invariant_name() {
//!        let mut fx = Fixture::new();
//!        let id = fx.add_client();
//!        fx.roundtrip(id);
//!        // ... assertions on fx.client(id) and fx.server.state ...
//!    }
//!    ```

mod client;
mod fixture;
mod server;

mod color_management;
mod dmabuf;
mod gamma_control;
mod globals;
mod idle;
mod layer_shell;
mod output_management;
mod overview;
mod pointer_constraints;
mod screencopy;
mod selection;
mod session_lock;
mod x11;
mod xdg_activation;
mod xdg_decoration;
mod xdg_shell;
