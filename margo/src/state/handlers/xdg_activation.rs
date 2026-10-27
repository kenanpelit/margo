//! `xdg-activation-v1` handler — strict-by-default anti-focus-steal
//! policy with tag-aware view jumps on legitimate activation.
//!
//! Tokens must be (serial, seat)-bundled, the seat must match ours,
//! and the serial must be no older than the keyboard's last enter
//! (i.e. the requester was actually focused when it generated the
//! token). Activations older than 10 s are dropped. Browsers
//! (Helium / Chromium) self-activate constantly via the same path
//! they use for "click a link inside the page" — calling `view_tag`
//! when the surface is already visible would deliberately toggle to
//! the previous tag (dwl's "press tag-N again to flip back"
//! semantic), so we gate the tag switch on `!already_visible`.

use smithay::{
    input::Seat,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        seat::WaylandFocus,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
    },
};

use crate::state::MargoState;

impl XdgActivationHandler for MargoState {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        // A token without a (serial, seat) bundle is suspicious —
        // someone scripted activation without going through a real
        // user interaction. Reject.
        let Some((serial, seat)) = data.serial else {
            return false;
        };
        // Different seat? Don't trust.
        if Seat::<MargoState>::from_resource(&seat).as_ref() != Some(&self.seat) {
            return false;
        }
        // Serial must be no older than the seat keyboard's last enter
        // — i.e. the requesting client was the keyboard-focused one
        // when it generated the token.
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        let Some(last_enter) = keyboard.last_enter() else {
            return false;
        };
        serial.is_no_older_than(&last_enter)
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        // A valid token still must not mutate the hidden desktop while the
        // session is locked.  `activate_window_idx` has the same central
        // guard for foreign-toplevel and IPC activation paths.
        if self.session_locked {
            return;
        }

        // Token expires after 10 s — older requests are stale.
        if token_data.timestamp.elapsed().as_secs() >= 10 {
            return;
        }

        // Find which client owns the surface.
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&surface))
        else {
            return;
        };

        // `focus_on_xdg_activate:0` (niri port, d89a3eb): this client
        // opted out of focus-steal from a — still cryptographically
        // valid — activation request. Common case: a Picture-in-Picture
        // popup re-activating itself on every frame. Mark urgent
        // instead of jumping the view/focus to it.
        // Global `focus_on_activate = 0` (mango) does the same for every
        // client.
        if !self.config.focus_on_activate || self.clients[idx].block_activation_focus {
            self.clients[idx].is_urgent = true;
            self.mark_state_dirty();
            self.request_repaint();
            tracing::info!(
                "xdg_activation: blocked focus-steal, marked urgent app_id={} idx={}",
                self.clients[idx].app_id,
                idx,
            );
            return;
        }

        // Same path as foreign-toplevel / IPC activation: switch to the
        // client's tag iff it isn't already visible (see the module
        // comment about browsers and the toggle-back semantic), keep a
        // scratchpad a scratchpad, swap in a parked group member, then
        // focus + raise + arrange.
        let mask = self.clients[idx].tags;
        self.activate_window_idx(idx);

        tracing::info!(
            "xdg_activation: activated app_id={} idx={} tag={:#x}",
            self.clients[idx].app_id,
            idx,
            mask,
        );
    }
}
