//! Resolve + decode + blur the user's wallpaper once at lock time,
//! so render.rs can composite it under the lock UI on every output.
//!
//! Resolution chain (matches mshell's lockscreen + matugen logic):
//!   1. `state.json` active output's `wallpaper` field (margo tagrule)
//!   2. `mshell.toml` `[wallpaper.tags][active_tag]`
//!   3. `mshell.toml` `[wallpaper.shuffle].directory` — first image
//!      (alphabetical) so the lock has *some* backdrop even when
//!      shuffle is disabled.
//!
//! On any failure returns `None`; render.rs falls back to a solid
//! dark backdrop in that case.

use std::path::PathBuf;

const BLUR_SIGMA: f32 = 18.0;
const RESIZE_LONG_EDGE: u32 = 1920;

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
    let runtime_state = read_state_json()?;
    let active = runtime_state
        .get("outputs")?
        .as_array()?
        .iter()
        .find(|o| o.get("active").and_then(|v| v.as_bool()).unwrap_or(false))?;

    // 1) state.json wallpaper field — margo tagrule passes through.
    if let Some(p) = active
        .get("wallpaper")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(expand_home(p));
    }

    // 2) mshell.toml [wallpaper.tags][active_tag]
    let mask = active.get("active_tag_mask").and_then(|v| v.as_u64())?;
    let mshell_cfg = read_mshell_toml().ok();

    if mask != 0 {
        let tag = (mask as u32).trailing_zeros() + 1;
        if let Some(cfg) = mshell_cfg.as_ref()
            && let Some(tags) = cfg.get("wallpaper").and_then(|w| w.get("tags"))
            && let Some(p) = tags.get(&tag.to_string()).and_then(|v| v.as_str())
        {
            return Some(expand_home(p));
        }
    }

    // 3) [wallpaper.shuffle].directory — first image even if shuffle is off.
    if let Some(cfg) = mshell_cfg.as_ref()
        && let Some(dir_s) = cfg
            .get("wallpaper")
            .and_then(|w| w.get("shuffle"))
            .and_then(|s| s.get("directory"))
            .and_then(|v| v.as_str())
    {
        let dir = expand_home(dir_s);
        if dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .ok()?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(is_image)
                .collect();
            if !entries.is_empty() {
                entries.sort();
                return Some(entries.into_iter().next().unwrap());
            }
        }
    }

    None
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

fn read_mshell_toml() -> Result<toml::Value, anyhow::Error> {
    let path = home_dir().join(".config").join("margo").join("mshell.toml");
    let raw = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&raw)?)
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

fn is_image(p: &PathBuf) -> bool {
    p.extension()
        .and_then(|s| s.to_str())
        .map(|s| matches!(s.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
        .unwrap_or(false)
}
