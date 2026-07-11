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

/// What goes behind the auth column.
///
/// The distinction is not cosmetic. A photograph needs a dim and a vignette to
/// keep the clock and the password field legible over whatever happens to be in
/// it. A flat colour has nothing to separate — dimming it only darkens the
/// colour the user picked, which is why `#1e1e2e` used to reach the screen as
/// roughly `#0d0d15`. So the renderer asks which one it is rather than treating
/// a solid colour as a very small photograph.
pub enum Backdrop {
    /// A flat colour, painted exactly as chosen.
    Solid((f64, f64, f64)),
    /// A blurred image, dimmed and vignetted.
    Image(PremulImage),
}

/// A backdrop already converted to cairo's premultiplied ARgb32 (BGRA) layout.
///
/// The lock screen re-renders on every keystroke, clock tick and pointer wake.
/// Converting the ~8 MB blurred wallpaper from RGBA to premultiplied BGRA on
/// each of those frames was pure repeated work — the pixels never change once
/// resolved. Do it once here and let the renderer blit the cached buffer.
pub struct PremulImage {
    /// Premultiplied BGRA bytes, `width * height * 4`.
    pub bgra: Vec<u8>,
    pub width: i32,
    pub height: i32,
}

impl PremulImage {
    /// Premultiply an RGBA image into cairo's ARgb32 (BGRA, alpha-premultiplied)
    /// layout, once, at resolve time.
    pub fn from_rgba(img: &image::RgbaImage) -> Self {
        let (width, height) = (img.width() as i32, img.height() as i32);
        let mut bgra: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
        for px in img.chunks_exact(4) {
            let a = px[3] as u16;
            let pm = |c: u8| ((c as u16 * a + 127) / 255) as u8;
            bgra.push(pm(px[2])); // b
            bgra.push(pm(px[1])); // g
            bgra.push(pm(px[0])); // r
            bgra.push(px[3]); // a, unchanged
        }
        Self {
            bgra,
            width,
            height,
        }
    }

    pub fn stride(&self) -> i32 {
        self.width * 4
    }
}

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
        match std::fs::read(p)
            .and_then(|bytes| image::load_from_memory(&bytes).map_err(std::io::Error::other))
        {
            Ok(img) => {
                tracing::info!(path = %p.display(), "avatar: decoded source");
                let sized = img.resize_to_fill(192, 192, image::imageops::FilterType::Lanczos3);
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

/// Resolve the lock background per `~/.config/margo/mlock.conf`:
/// a solid colour, a specific image, or (default) the desktop wallpaper.
/// `None` only when a wallpaper/image source is wanted but unavailable —
/// render.rs then paints the palette's solid backdrop.
pub fn load_background() -> Option<Backdrop> {
    use crate::background::BgMode;
    let bg = crate::background::read();
    match bg.mode {
        BgMode::Color => Some(Backdrop::Solid(bg.color)),
        // Custom image, falling back to the desktop wallpaper if the
        // configured path is missing/unreadable.
        BgMode::Image => bg
            .image
            .as_deref()
            .and_then(load_blurred_path)
            .or_else(load_blurred)
            .map(|img| Backdrop::Image(PremulImage::from_rgba(&img))),
        BgMode::Wallpaper => {
            load_blurred().map(|img| Backdrop::Image(PremulImage::from_rgba(&img)))
        }
    }
}

pub fn load_blurred() -> Option<image::RgbaImage> {
    load_blurred_path(&resolve_path()?)
}

/// Decode + downscale + blur an image file for use as the backdrop.
pub fn load_blurred_path(path: &Path) -> Option<image::RgbaImage> {
    let img = image::open(path).ok()?;

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

/// Fetch the compositor state snapshot over margo's IPC socket
/// (`$MARGO_SOCKET` / `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`) with a
/// one-shot `get state`. Returns `None` when margo isn't reachable —
/// the caller falls back to the built-in wallpaper paths.
fn read_state_json() -> Option<serde_json::Value> {
    use std::io::{BufRead, BufReader, Write};
    let sock_path = std::env::var_os("MARGO_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    let uid = unsafe { libc::getuid() };
                    PathBuf::from(format!("/run/user/{uid}"))
                });
            runtime.join("margo").join("margo-ipc.sock")
        });
    let mut sock = std::os::unix::net::UnixStream::connect(sock_path).ok()?;
    sock.write_all(b"get state\n").ok()?;
    let mut reader = BufReader::new(sock);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    serde_json::from_str(line.trim()).ok()
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
