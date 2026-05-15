//! `wp_security_context_v1` handler.
//!
//! Sandboxed clients (Flatpak, Snap-style isolation engines) call
//! through this protocol to ask the compositor to listen on a
//! separate socket the sandbox engine pre-allocates, so the
//! sandboxed app talks Wayland through it. The compositor inserts
//! the listener source into its calloop and accepts incoming
//! connections from there.
//!
//! Margo currently doesn't track "restricted" client state (niri
//! does, to gate sensitive protocols). For now we just accept the
//! socket — the protocol-level access boundary (the sandbox engine
//! holds the fd) is what does the heavy lifting. Restricted-client
//! enforcement is a follow-up enhancement.

use smithay::{
    delegate_security_context,
    wayland::security_context::{
        SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
    },
};

use crate::state::MargoState;

impl SecurityContextHandler for MargoState {
    fn context_created(
        &mut self,
        source: SecurityContextListenerSource,
        context: SecurityContext,
    ) {
        let res = self
            .loop_handle
            .insert_source(source, move |client_stream, _, state| {
                tracing::debug!(?context, "security-context: new sandboxed client");
                if let Err(err) = state
                    .display_handle
                    .insert_client(client_stream, std::sync::Arc::new(()))
                {
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
delegate_security_context!(MargoState);
