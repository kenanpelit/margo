//! Native "VPN" layer-shell menu — the Mullvad control surface the `mvpn`
//! bar pill opens. Mirrors the DNS/VPN menu's chrome (DESIGN.md panel header
//! + cards + `ok-button-*` actions) but exposes the full `mvpn` feature set:
//! connect / random / fastest, lockdown / auto-connect / quantum-resistant
//! toggles, anti-censorship mode, and the favourites list — plus a live
//! rx/tx throughput graph for the tunnel interface (`throughput.rs`).

pub(crate) mod locations;
pub(crate) mod multihop;
pub(crate) mod throughput;
pub(crate) mod vpn_menu_widget;
