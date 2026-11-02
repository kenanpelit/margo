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
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode,
    wayland::{
        seat::WaylandFocus,
        shell::xdg::{ToplevelSurface, decoration::XdgDecorationHandler},
    },
};

use crate::state::{MargoState, read_toplevel_identity};

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

impl MargoState {
    /// What decoration mode should we send to a freshly-bound or
    /// reset toplevel? Defaults to `ServerSide`; flips to
    /// `ClientSide` only when the client is in our `clients` vec
    /// and matches a window-rule that whitelists CSD. At the time
    /// `new_decoration` fires the toplevel may not even be in
    /// `clients` yet (xdg-decoration arrives before the first
    /// commit), in which case we ALSO check the raw window-rule
    /// list against the toplevel's current app_id / title — the
    /// rule machinery would otherwise only kick in at
    /// `finalize_initial_map`, too late to influence the very first
    /// configure.
    pub(super) fn decoration_mode_for(&self, toplevel: &ToplevelSurface) -> XdgDecorationMode {
        if self.client_allows_csd(toplevel) {
            XdgDecorationMode::ClientSide
        } else {
            XdgDecorationMode::ServerSide
        }
    }

    pub(super) fn client_allows_csd(&self, toplevel: &ToplevelSurface) -> bool {
        let wl_surface = toplevel.wl_surface();
        // Path A: client already mapped — read the resolved
        // `allow_csd` flag right off the `MargoClient`.
        if let Some(client) = self
            .clients
            .iter()
            .find(|c| c.window.wl_surface().as_deref() == Some(wl_surface))
        {
            return client.allow_csd;
        }
        // Path B: client is between role bind and first commit —
        // best we can do is look up the window-rule by the
        // toplevel's currently-set app_id / title. This is the
        // path that fires for the *first* `xdg_decoration.configure`
        // many compositors get wrong (Chromium / Firefox often
        // bind decoration before any role-data commit, so the
        // initial mode the user sees depends entirely on what we
        // send right now).
        let (app_id, title) = read_toplevel_identity(toplevel);
        self.config
            .window_rules
            .iter()
            .filter(|rule| self.window_rule_matches(rule, &app_id, &title))
            .any(|rule| rule.allow_csd == Some(true))
    }
}
