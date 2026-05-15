//! `org_kde_kwin_server_decoration` handler.
//!
//! Legacy KDE protocol kept around so older Qt5 / KDE apps that
//! predate xdg-decoration negotiate decorations correctly. Margo's
//! global decoration policy is SSD-first (compositor draws the
//! decorations), so the default mode passed to the state is
//! `Mode::Server` and the trait's default `request_mode` body
//! (acknowledge whatever the client asked for) is enough — margo
//! doesn't enforce a stricter policy here, since clients ignoring
//! the suggested mode is allowed by the protocol.

use smithay::{
    delegate_kde_decoration,
    wayland::shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState},
};

use crate::state::MargoState;

impl KdeDecorationHandler for MargoState {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.kde_decoration_state
    }
}
delegate_kde_decoration!(MargoState);
