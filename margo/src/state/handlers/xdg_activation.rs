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
    delegate_xdg_activation,
    input::Seat,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        seat::WaylandFocus,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
    },
};

use crate::state::{FocusTarget, MargoState};

impl XdgActivationHandler for MargoState {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(
        &mut self,
        _token: XdgActivationToken,
        data: XdgActivationTokenData,
    ) -> bool {
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

        // Switch to the client's tag iff it isn't already visible —
        // see the module comment about browsers and the toggle-back
        // semantic.
        let mask = self.clients[idx].tags;
        let mon_idx = self.clients[idx].monitor;
        let already_visible = self
            .monitors
            .get(mon_idx)
            .map(|m| (mask & m.current_tagset()) != 0)
            .unwrap_or(false);
        if !already_visible {
            let one_bit = mask & mask.wrapping_neg();
            let target = if one_bit != 0 { one_bit } else { mask };
            self.view_tag(target);
        }

        // Focus + raise. focus_surface tracks selected/prev-selected
        // history per monitor.
        let window = self.clients[idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window.clone())));
        self.space.raise_element(&window, true);
        self.enforce_z_order();
        self.request_repaint();

        tracing::info!(
            "xdg_activation: activated app_id={} idx={} tag={:#x} already_visible={}",
            self.clients[idx].app_id,
            idx,
            mask,
            already_visible,
        );
    }
}
delegate_xdg_activation!(MargoState);
