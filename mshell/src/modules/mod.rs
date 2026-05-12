//! Bar modules. Each submodule exposes a `build()` constructor that
//! returns a GTK widget ready to be appended into the bar's left,
//! center or right region.

pub mod battery;
pub mod bluetooth;
pub mod brightness;
pub mod keymode;
pub mod media;
pub mod memory;
pub mod microphone;
pub mod network;
pub mod notes;
pub mod notifications;
pub mod podman;
pub mod power;
pub mod public_ip;
pub mod system_info;
pub mod tempo;
pub mod tray;
pub mod twilight;
pub mod ufw;
pub mod updates;
pub mod volume;
pub mod weather;
pub mod window_title;
pub mod workspaces;
