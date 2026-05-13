//! Resolve + decode + blur the user's wallpaper once at lock time,
//! so render.rs can composite it under the lock UI on every output.
//!
//! Resolution chain:
//!   1. `state.json` active output's `wallpaper` field (margo tagrule
//!      passes through; typically populated by the user's external
//!      shell — noctalia, swww, swaybg, …).
//!   2. `~/.local/share/margo/wallpapers/default.jpg`  — user override.
//!   3. `/usr/share/margo/wallpapers/default.jpg`      — package default
//!      (shipped by `margo-git`; reasonable 4K image so the lock screen
//!      never falls back to a flat dark backdrop on a clean install).
//!
//! Returns `None` only when every candidate is missing or unreadable;
//! render.rs paints a solid dark backdrop in that case.

use std::path::{Path, PathBuf};

const BLUR_SIGMA: f32 = 18.0;
const RESIZE_LONG_EDGE: u32 = 1920;

/// Built-in fallback paths, tried in order after `state.json`. User
/// override comes first so a copy in `~/.local/share/margo/...` wins
/// against the system-shipped image without touching the package.
const FALLBACK_RELATIVE_USER: &str = ".local/share/margo/wallpapers/default.jpg";
const FALLBACK_SYSTEM: &str = "/usr/share/margo/wallpapers/default.jpg";

/// User avatar — looks at `~/.face` first (de-facto desktop standard),
/// then AccountsService's icon. Returns a 192×192 RGBA buffer; the
/// renderer clips it into a circle.
pub fn load_avatar(user: &str) -> Option<image::RgbaImage> {
    let candidates = [
        home_dir().join(".face"),
        home_dir().join(".face.icon"),
        PathBuf::from(format!("/var/lib/AccountsService/icons/{user}")),
    ];
    for p in &candidates {
        if !p.exists() {
            tracing::debug!("avatar: not present at {}", p.display());
            continue;
        }
        // `image::open` infers format from the file extension. `.face`
        // and AccountsService's bare `<username>` have no extension,
        // so we read the bytes and let the image crate sniff magic
        // numbers via `load_from_memory`.
        match std::fs::read(p).and_then(|bytes| {
            image::load_from_memory(&bytes).map_err(std::io::Error::other)
        }) {
            Ok(img) => {
                tracing::info!(path = %p.display(), "avatar: decoded source");
                let sized =
                    img.resize_to_fill(192, 192, image::imageops::FilterType::Lanczos3);
                return Some(sized.to_rgba8());
            }
            Err(e) => {
                tracing::warn!(path = %p.display(), error = %e, "avatar: decode failed");
            }
        }
    }
    tracing::warn!(user = %user, home = ?home_dir(), "avatar: no candidate found");
    None
}

pub fn load_blurred() -> Option<image::RgbaImage> {
    let path = resolve_path()?;
    let img = image::open(&path).ok()?;

    // Resize before blur — image::blur is O(w·h·σ), 1920 long-edge
    // keeps it under ~150 ms for a 4K wallpaper.
    let (w, h) = (img.width(), img.height());
    let scale = (RESIZE_LONG_EDGE as f32 / w.max(h) as f32).min(1.0);
    let work = if scale < 1.0 {
        img.resize(
            (w as f32 * scale) as u32,
            (h as f32 * scale) as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };

    Some(work.blur(BLUR_SIGMA).to_rgba8())
}

fn resolve_path() -> Option<PathBuf> {
    if let Some(p) = state_json_path() {
        tracing::info!(path = %p.display(), "wallpaper: state.json");
        return Some(p);
    }

    let user_fallback = home_dir().join(FALLBACK_RELATIVE_USER);
    if exists_and_readable(&user_fallback) {
        tracing::info!(path = %user_fallback.display(), "wallpaper: user fallback");
        return Some(user_fallback);
    }

    let system_fallback = PathBuf::from(FALLBACK_SYSTEM);
    if exists_and_readable(&system_fallback) {
        tracing::info!(path = %system_fallback.display(), "wallpaper: system fallback");
        return Some(system_fallback);
    }

    tracing::warn!("wallpaper: no source found, lock will use solid backdrop");
    None
}

fn state_json_path() -> Option<PathBuf> {
    let runtime_state = read_state_json()?;
    let active = runtime_state
        .get("outputs")?
        .as_array()?
        .iter()
        .find(|o| o.get("active").and_then(|v| v.as_bool()).unwrap_or(false))?;

    let p = active
        .get("wallpaper")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    let expanded = expand_home(p);
    exists_and_readable(&expanded).then_some(expanded)
}

fn exists_and_readable(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

fn read_state_json() -> Option<serde_json::Value> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    let path = runtime.join("margo").join("state.json");
    let raw = std::fs::read(&path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        home_dir().join(rest)
    } else if p == "~" {
        home_dir()
    } else {
        PathBuf::from(p)
    }
}
