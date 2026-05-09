//! `xdg-decoration-unstable-v1` protocol handler — server-side
//! decoration policy with per-client CSD opt-in via window-rule.
//!
//! Mango's policy (and ours) is "compositor draws the decorations
//! by default" — clients are sent `ServerSide` so they suppress
//! their CSD titlebar / shadow / corner radius and the compositor's
//! `RoundedBorderElement` + `clipped_surface` shaders do the work.
//! Clients can still ask for `ClientSide` and we honour it iff the
//! client matches a window-rule with `allow_csd:1`.

use smithay::{
    delegate_xdg_decoration,
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode,
    wayland::shell::xdg::{decoration::XdgDecorationHandler, ToplevelSurface},
};

use crate::state::MargoState;

impl XdgDecorationHandler for MargoState {
    /// First time a client binds `xdg-decoration-unstable-v1` for this
    /// toplevel. Send `ServerSide` (the default policy); clients can
    /// still request CSD via [`Self::request_mode`].
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        let mode = self.decoration_mode_for(&toplevel);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        toplevel.send_configure();
    }

    /// Client asked for a specific decoration mode. Honour `ClientSide`
    /// only when the client is window-ruled with `allow_csd:1` (or the
    /// global `Config::allow_csd_default` is on). Everything else gets
    /// `ServerSide` regardless — keeps the visual identity consistent
    /// while still letting the user opt specific apps (browsers
    /// usually) into their native CSD.
    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: XdgDecorationMode) {
        let resolved = match mode {
            XdgDecorationMode::ClientSide if self.client_allows_csd(&toplevel) => {
                XdgDecorationMode::ClientSide
            }
            _ => XdgDecorationMode::ServerSide,
        };
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(resolved);
        });
        toplevel.send_configure();
    }

    /// Client cleared its decoration preference — re-evaluate from
    /// our policy (same path as `new_decoration`). Ensures the
    /// `allow_csd` whitelist is still respected if the client toggles
    /// its own decoration off via UI.
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        let mode = self.decoration_mode_for(&toplevel);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        toplevel.send_configure();
    }
}
delegate_xdg_decoration!(MargoState);
