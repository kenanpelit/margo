//! `wp_security_context_v1` handler.
//!
//! Sandboxed clients (Flatpak, Snap-style isolation engines) call
//! through this protocol to ask the compositor to listen on a
//! separate socket the sandbox engine pre-allocates, so the
//! sandboxed app talks Wayland through it. The compositor inserts
//! the listener source into its calloop and accepts incoming
//! connections from there.
//!
//! Clients that connect through such a socket get
//! `MargoClientData { restricted: true }`, and every privileged global
//! (screen capture, clipboard managers, virtual input, session lock,
//! layer shell, output management, window lists, …) filters them out via
//! `state::client_is_unrestricted` — the same split niri makes.

use smithay::wayland::security_context::{
    SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
};

use crate::state::MargoState;

impl SecurityContextHandler for MargoState {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        let res = self
            .loop_handle
            .insert_source(source, move |client_stream, _, state| {
                tracing::debug!(?context, "security-context: new sandboxed client");
                // Must be MargoClientData (not `()`): every client needs a
                // CompositorClientState, and `client_compositor_state` panics
                // ("unknown client data type") on any other data type. With `()`
                // the sandboxed client's first `wl_output` bind crashed the
                // whole compositor — i.e. launching any Flatpak / sandboxed app
                // through this socket took margo (and every client) down.
                if let Err(err) = state.display_handle.insert_client(
                    client_stream,
                    std::sync::Arc::new(crate::state::MargoClientData {
                        restricted: true,
                        ..Default::default()
                    }),
                ) {
                    tracing::warn!(
                        error = %err,
                        "security-context: failed to insert client",
                    );
                }
            });
        if let Err(err) = res {
            tracing::warn!(
                error = %err,
                "security-context: failed to register listener with calloop",
            );
        }
    }
}
