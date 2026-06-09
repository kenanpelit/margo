//! Native "VPN" layer-shell menu — the Mullvad control surface the `mvpn`
//! bar pill opens. Mirrors the DNS/VPN menu's chrome (DESIGN.md panel header
//! + cards + `ok-button-*` actions) but exposes the full `mvpn` feature set:
//! connect / random / fastest, lockdown / auto-connect / quantum-resistant
//! toggles, anti-censorship mode, and the favourites list.

pub(crate) mod vpn_menu_widget;
