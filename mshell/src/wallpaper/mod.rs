//! Wallpaper renderer — independent Wayland client thread.
//!
//! Why a separate thread? Earlier attempts went through iced's
//! `Image` widget on a Background-layer surface; the surface itself
//! composed (proven by setting a bright red fallback colour and
//! seeing red appear across the output) but iced silently failed
//! to upload image pixels. RGBA-decoded handles, byte slices, and
//! raw paths all produced the same empty surface. wpaperd and
//! pandora both ship the same pattern instead: a dedicated Wayland
//! connection that decodes the image, allocates a `wl_shm` pool +
//! `wl_buffer`, and `attach`/`commit`s onto a `zwlr_layer_shell_v1`
//! Background surface directly. That's the path with years of
//! shipping history against wlroots compositors; mshell adopts it.
//!
//! The renderer runs on its own thread, owns its own
//! `wayland-client` connection, and accepts commands over an
//! `mpsc::Sender`. The main mshell process keeps its iced loop for
//! the bar / OSD / toast / menus and is otherwise unchanged.

mod state;

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

/// How the renderer fits a wallpaper into the output. Matches the
/// names of the iced port's old `[wallpaper] fit` config knob so
/// strings parse 1:1 once mshell grows a config file again.
/// `Contain` / `Fill` / `None` aren't wired into the current
/// caller (Cover is always used) but they're kept here so a
/// future config layer can flip them without an API change.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperFit {
    /// Crop-fill: scale so the image covers the output; overhang
    /// is trimmed off the edges.
    Cover,
    /// Letterbox: scale so the whole image fits; surplus space
    /// painted with `fallback_color`.
    Contain,
    /// Stretch: ignore aspect ratio.
    Fill,
    /// Native pixel size, centred; pads with `fallback_color`.
    None,
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Set / clear the wallpaper for a specific output.
    ///
    /// `output_name` is the bare Wayland output name
    /// (`eDP-1`, `DP-3`, …). `path = None` (or a path that can't
    /// be read) renders a solid `fallback_color` buffer instead.
    Set {
        output_name: String,
        path: Option<PathBuf>,
        fit: WallpaperFit,
        /// Solid RGB shown behind the image (visible with `Contain`/
        /// `None` letterboxing) or as the whole buffer when `path`
        /// is `None`.
        fallback_color: [u8; 3],
    },
    /// Shut the renderer down — destroys all surfaces and exits the
    /// thread. mshell rarely needs to call this (margo session
    /// teardown handles the process exit) but it's exposed for
    /// completeness.
    #[allow(dead_code)]
    Quit,
}

/// Handle to the wallpaper renderer thread. Cheap to clone (just
/// wraps an `mpsc::Sender`).
#[derive(Debug, Clone)]
pub struct WallpaperRenderer {
    tx: mpsc::Sender<Command>,
}

impl WallpaperRenderer {
    /// Spawn the wallpaper thread and return a handle. Connects to
    /// the same Wayland display as the host process via
    /// `Connection::connect_to_env`.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("mshell-wallpaper".to_owned())
            .spawn(move || {
                if let Err(e) = state::run(rx) {
                    log::error!("wallpaper thread exited: {e:#}");
                }
            })
            .expect("spawn mshell-wallpaper thread");
        Self { tx }
    }

    /// Queue a wallpaper change for an output. `path = None` paints
    /// a solid `fallback_color` buffer (also used when decode/read
    /// fails). Drops silently if the renderer thread has exited.
    pub fn set(
        &self,
        output_name: impl Into<String>,
        path: Option<PathBuf>,
        fit: WallpaperFit,
        fallback_color: [u8; 3],
    ) {
        let _ = self.tx.send(Command::Set {
            output_name: output_name.into(),
            path,
            fit,
            fallback_color,
        });
    }
}
