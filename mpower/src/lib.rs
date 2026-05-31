//! mpower — automatic power-profile manager for margo.
//!
//! Library half of the `mpower` crate: the config schema (also consumed by
//! the shell's Settings → Power page so there is a single source of truth),
//! the CPU sampler, the `/sys` AC/battery readers, the `powerprofilesctl`
//! wrapper, and the pure decision policy. The daemon + CLI live in
//! `src/main.rs`.
//!
//! Design notes are in [`README.md`](../README.md). In short: a long-lived
//! user daemon that, every tick, samples CPU load + power source and drives
//! power-profiles-daemon per `~/.config/margo/mpower.toml`. It replaces the
//! external `ppp-auto-profile` timer/script.

pub mod config;
pub mod cpu;
pub mod policy;
pub mod ppd;
pub mod syspower;

pub use config::{config_path, Config};
