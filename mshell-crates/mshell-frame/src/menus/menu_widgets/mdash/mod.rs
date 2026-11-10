//! **mdash** — the tabbed dashboard (`MenuWidget::Dashboard`, the default
//! `mdash_menu`).
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │ Good evening, Kenan                                   (⚙)(⏻) │
//! │ Tuesday 10 November · 20:23                                  │
//! │ [▮ 54%] [Wi-Fi Home] [🔊 45%] [☁ 11°] [🔔 3 new] [⬇ 3 updates]│
//! │ ( Today | Controls | System | Tools )                        │
//! │ ┌──────────────────── one size for every tab ──────────────┐ │
//! │ └──────────────────────────────────────────────────────────┘ │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! * `dashboard.rs` — the component: header, strip, segmented tabs, the
//!   homogeneous page stack, keyboard, reveal gating and the live feeds.
//! * `header.rs` / `strip.rs` — greeting + clock + Settings + the power
//!   popover; the status chips.
//! * `today/`, `controls.rs`, `system.rs`, `tools.rs` — the four tabs.
//! * `layout.rs` — where cards sit (pure; hidden cards leave no holes).
//! * `tab_state.rs` — remembered tab, keys, `mshellctl menu mdash <tab>`.
//! * `card.rs` / `sparkline.rs` — the shared card and sparkline.

mod card;
mod controls;
pub(crate) mod dashboard;
mod header;
mod layout;
mod sparkline;
mod strip;
mod system;
pub(crate) mod tab_state;
mod today;
mod tools;

#[cfg(test)]
mod render_tests;
