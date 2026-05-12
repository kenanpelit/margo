//! Wallpaper driver — drives the in-process renderer thread.
//!
//! Polls margo's state.json every 2 s; on any output's `wallpaper`
//! field change, posts a `Set` command into the `WallpaperRenderer`
//! channel. The renderer owns its own Wayland connection (separate
//! thread) and pushes ARGB8888 buffers onto a wlr-layer-shell
//! Background surface. No external daemon, no swww.
//!
//! Pre-condition: nothing. The renderer creates its own surface.
//! Logs go through mshell's own tracing/log pipeline.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::Value;

use crate::wallpaper::{WallpaperFit, WallpaperRenderer};

const POLL_SECS: u32 = 2;
/// Painted as the buffer behind / around the image, and as the
/// whole buffer when no `wallpaper` path is set. Dracula crust.
const FALLBACK_RGB: [u8; 3] = [0x11, 0x11, 0x1b];

/// Spawn the renderer thread + the polling loop. Call once at
/// startup, after the GtkApplication has activated so the bar
/// surface is already in place.
pub fn start() {
    let renderer = WallpaperRenderer::spawn();
    let last: Rc<RefCell<HashMap<String, String>>> = Rc::new(RefCell::new(HashMap::new()));

    // First sweep applies whatever margo currently advertises.
    apply(&renderer, &last);

    let last_tick = last.clone();
    glib::timeout_add_seconds_local(POLL_SECS, move || {
        apply(&renderer, &last_tick);
        glib::ControlFlow::Continue
    });
}

fn apply(renderer: &WallpaperRenderer, last: &Rc<RefCell<HashMap<String, String>>>) {
    let Some(json) = load_state() else {
        return;
    };
    let Some(outputs) = json.get("outputs").and_then(Value::as_array) else {
        return;
    };

    let mut last = last.borrow_mut();
    for o in outputs {
        let name = o
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let path = o
            .get("wallpaper")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let entry = last.entry(name.clone()).or_default();
        if entry.as_str() == path {
            continue;
        }
        *entry = path.clone();

        let pb = if path.is_empty() {
            None
        } else {
            // Honour `~/…` paths (margo writes them as-is into
            // state.json on some setups).
            Some(expand_home(Path::new(&path)))
        };
        tracing::info!(output = %name, path = %path, "wallpaper: change requested");
        renderer.set(name, pb, WallpaperFit::Cover, FALLBACK_RGB);
    }
}

fn load_state() -> Option<Value> {
    let path = state_path();
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn state_path() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("margo").join("state.json");
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/margo/state.json"))
}

fn expand_home(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    p.to_path_buf()
}
