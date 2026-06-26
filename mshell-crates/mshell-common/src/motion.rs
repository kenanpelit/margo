//! Shared motion timings for the shell frame's menu surfaces.
//!
//! These are the Rust-side counterparts to the CSS `--motion-*` tokens in
//! `mshell-frame/DESIGN.md` §1. GTK widgets (revealers, the custom
//! [`DiagonalRevealer`](crate::diagonal_revealer::DiagonalRevealer)) take
//! their durations in code rather than CSS, so the values live here in one
//! place — every menu opens *and closes* with the same timing, edge and
//! corner menus included. Keeping a single constant is what stops the
//! durations drifting apart again (they used to: 250 ms edge vs 200 ms
//! corner, which read as the corner menus being "snappier" than the rest).
//!
//! `MENU_REVEAL_MS` is a *surface reveal* in DESIGN.md terms, so it stays
//! within the `--motion-slow` 320 ms budget while leaning snappy — the
//! menus are opened constantly, and frequent interactions want the shorter
//! end of the range (DESIGN.md §13.4).

/// Duration, in milliseconds, of a menu surface sliding/scaling in or out.
/// Used by both the edge `gtk::Revealer`s and the corner `DiagonalRevealer`
/// so all menu reveals share one timing. Symmetric: reveal and unreveal
/// run for the same duration.
pub const MENU_REVEAL_MS: u32 = 220;
