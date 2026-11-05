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
mod config_no_effect;
mod fixture;
mod server;
mod wlr_ftl_client;
mod wlr_output_client;

mod active_output;
mod arrange;
mod bootstrap;
mod client_removal;
mod color_management;
mod compositor_indicators;
mod cursor_repaint;
mod disabled_output;
mod dispatch;
mod dmabuf;
mod dpms;
mod floating_layout;
mod focus_index;
mod focus_mon;
mod gamma_control;
mod globals;
mod groups;
mod hot_corner;
mod idle;
mod internal_keyboard;
mod ipc;
mod keymode;
mod layer_shell;
mod lock_keys;
mod logical_output_area;
mod minimize;
mod mirror;
mod mosaic_layout;
mod move_client;
mod mru_switcher;
mod opacity;
mod output_assignment;
mod output_config;
mod output_management;
mod overview;
mod pacing;
mod pointer_constraints;
mod pointer_gestures;
mod preferred_scale;
mod present_timer;
mod reload;
mod repaint;
mod scratchpad;
mod screencopy;
mod scripting;
mod security_context;
mod selection;
mod session_lock;
mod session_regressions;
mod session_snapshot;
mod tag_mask;
mod tag_mon;
mod tag_transition_anim;
mod theme;
mod touch;
mod twilight;
mod virtual_pointer;
mod wallpaper_runtime;
mod window_rules;
mod x11;
mod xdg_activation;
mod xdg_decoration;
mod xdg_shell;
