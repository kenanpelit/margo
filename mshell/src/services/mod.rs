//! Stateless readers for kernel / filesystem-exposed system info.
//! D-Bus-backed services (audio, brightness, network, notifications,
//! tray, MPRIS, …) get their own submodules in later stages.

pub mod audio;
pub mod battery;
pub mod bluetooth;
pub mod brightness;
pub mod cpu;
pub mod cpu_temp;
pub mod keymode;
pub mod memory;
pub mod mpris;
pub mod network;
pub mod notes;
pub mod podman;
pub mod power_profile;
pub mod public_ip;
pub mod twilight;
pub mod ufw;
pub mod updates;
pub mod wallpaper;
pub mod weather;
