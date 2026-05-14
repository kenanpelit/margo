use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ThemeStoreFields, WallpaperStoreFields};
use mshell_config::schema::themes::Themes;
use mshell_image::lut::apply_theme_filter;
use reactive_graph::effect::Effect;
use reactive_graph::prelude::{Get, GetUntracked, Update};
use reactive_stores::{ArcStore, Store};
use relm4::gtk::glib;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use tracing::info;

// ── Cache paths ──────────────────────────────────────────────────────────────

fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("mshell")
}

/// The original wallpaper as provided by the user (raw bytes, any format).
pub fn source_path() -> PathBuf {
    cache_dir().join("wallpaper_source")
}

/// Persisted display wallpaper on disk: [u32 width][u32 height][RGBA pixels]
fn display_cache_path() -> PathBuf {
    cache_dir().join("wallpaper.raw")
}

/// Records the *original* path of the current wallpaper (the file
/// in the user's wallpaper dir), so cycling can find its position
/// in the directory listing — `source_path()` is only a cache
/// copy and loses the original location.
fn current_path_file() -> PathBuf {
    cache_dir().join("wallpaper_path")
}

// ── In-memory wallpaper buffer ───────────────────────────────────────────────

/// Shared RGBA image data ready for direct use with MemoryTexture.
#[derive(Debug, Clone)]
pub struct WallpaperImage {
    pub buf: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

// ── Store ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Store)]
pub struct WallpaperState {
    /// Monotonic counter bumped every time the wallpaper is updated.
    /// Consumers watch this to know when to reload.
    pub revision: u64,
}

struct WallpaperInner {
    cancel_token: Arc<AtomicBool>,
    image: Option<WallpaperImage>,
}

static WALLPAPER: LazyLock<ArcStore<WallpaperState>> =
    LazyLock::new(|| ArcStore::new(WallpaperState { revision: 0 }));

static WALLPAPER_INNER: LazyLock<std::sync::Mutex<WallpaperInner>> = LazyLock::new(|| {
    // Load persisted image from disk if available
    let image = load_from_disk();

    // React to theme changes
    Effect::new(move |_| {
        let _theme = config_manager().config().theme().theme().get();
        refilter();
    });

    // React to filter toggle
    Effect::new(move |_| {
        let _apply = config_manager()
            .config()
            .wallpaper()
            .apply_theme_filter()
            .get();
        refilter();
    });

    // React to filter strength changes
    Effect::new(move |_| {
        let _strength = config_manager()
            .config()
            .wallpaper()
            .theme_filter_strength()
            .get();
        refilter();
    });

    let has_image = image.is_some();

    let inner = std::sync::Mutex::new(WallpaperInner {
        cancel_token: Arc::new(AtomicBool::new(false)),
        image,
    });

    // If we loaded a persisted image, bump revision so consumers pick it up
    if has_image {
        WALLPAPER.update(|s| s.revision = 1);
    }

    inner
});

pub fn wallpaper_store() -> ArcStore<WallpaperState> {
    // Ensure effects are initialized
    let _ = &*WALLPAPER_INNER;
    WALLPAPER.clone()
}

/// Get the current in-memory wallpaper image, if any.
pub fn current_wallpaper_image() -> Option<WallpaperImage> {
    WALLPAPER_INNER.lock().ok()?.image.clone()
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Set a new wallpaper from a source image. Copies it to the cache dir
/// and applies the current theme filter if enabled.
pub fn set_wallpaper(path: &Path) {
    info!("set wallpaper to {}", path.display());
    let dir = cache_dir();
    fs::create_dir_all(&dir).ok();

    if let Err(e) = fs::copy(path, source_path()) {
        eprintln!("Failed to copy wallpaper source: {e}");
        return;
    }

    // Remember the original location so next/prev rotation knows
    // where we are in the directory.
    if let Some(p) = path.to_str() {
        let _ = fs::write(current_path_file(), p);
    }

    refilter();
}

// ── Rotation / cycling ───────────────────────────────────────────────────────

/// Which way to step through the wallpaper directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDirection {
    Next,
    Previous,
    Random,
}

const WALLPAPER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];

/// The original path of the wallpaper currently set, if recorded.
pub fn current_wallpaper_path() -> Option<PathBuf> {
    let raw = fs::read_to_string(current_path_file()).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Every image file in the configured `wallpaper_dir`, sorted by
/// filename for a stable next/prev order.
pub fn list_wallpapers() -> Vec<PathBuf> {
    let dir = config_manager()
        .config()
        .wallpaper()
        .wallpaper_dir()
        .get_untracked();
    if dir.is_empty() {
        return Vec::new();
    }
    let mut entries: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| WALLPAPER_EXTENSIONS.contains(&e.to_lowercase().as_str()))
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            eprintln!("wallpaper: cannot read dir {dir}: {e}");
            Vec::new()
        }
    };
    entries.sort();
    entries
}

/// Step the wallpaper in the given direction within `wallpaper_dir`
/// and apply it. No-op when the directory has no images.
pub fn cycle_wallpaper(direction: CycleDirection) {
    let list = list_wallpapers();
    if list.is_empty() {
        return;
    }
    let len = list.len();

    // Where are we now? Default to the first image when the
    // current wallpaper isn't (or is no longer) in the directory.
    let current_idx = current_wallpaper_path()
        .and_then(|cur| list.iter().position(|p| *p == cur))
        .unwrap_or(0);

    let target_idx = match direction {
        CycleDirection::Next => (current_idx + 1) % len,
        CycleDirection::Previous => (current_idx + len - 1) % len,
        CycleDirection::Random => {
            if len == 1 {
                0
            } else {
                // Cheap, dependency-free PRNG seeded off the clock —
                // wallpaper choice doesn't need crypto randomness.
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as usize)
                    .unwrap_or(0);
                let mut idx = nanos % len;
                if idx == current_idx {
                    idx = (idx + 1) % len;
                }
                idx
            }
        }
    };

    set_wallpaper(&list[target_idx]);
}

/// Clear the wallpaper entirely.
pub fn clear_wallpaper() {
    fs::remove_file(source_path()).ok();
    fs::remove_file(display_cache_path()).ok();
    if let Ok(mut inner) = WALLPAPER_INNER.lock() {
        inner.image = None;
    }
    bump_revision();
}

/// Returns true if a wallpaper is currently set.
pub fn has_wallpaper() -> bool {
    source_path().exists()
}

// ── Internal ─────────────────────────────────────────────────────────────────

fn refilter() {
    let source = source_path();
    if !source.exists() {
        return;
    }

    if let Ok(mut inner) = WALLPAPER_INNER.lock() {
        inner.cancel_token.store(true, Ordering::Relaxed);
        let new_token = Arc::new(AtomicBool::new(false));
        inner.cancel_token = new_token.clone();

        let cancel_token = new_token;

        let theme = config_manager().config().theme().theme().get_untracked();
        let apply = config_manager()
            .config()
            .wallpaper()
            .apply_theme_filter()
            .get_untracked();
        let strength = config_manager()
            .config()
            .wallpaper()
            .theme_filter_strength()
            .get_untracked()
            .get();

        let should_filter =
            apply && strength != 0.0 && theme != Themes::Default && theme != Themes::Wallpaper;

        std::thread::spawn(move || {
            let result = if should_filter {
                apply_theme_filter(&source, &theme, strength, 1.0, 0.0, &cancel_token).map(
                    |remap| WallpaperImage {
                        buf: Arc::new(remap.buf),
                        width: remap.width,
                        height: remap.height,
                    },
                )
            } else if !cancel_token.load(Ordering::Relaxed) {
                decode_source(&source)
            } else {
                None
            };

            if cancel_token.load(Ordering::Relaxed) {
                return;
            }

            if let Some(image) = result {
                // Store in memory
                if let Ok(mut inner) = WALLPAPER_INNER.lock() {
                    inner.image = Some(image.clone());
                }

                glib::idle_add_once(|| {
                    bump_revision();
                });

                // Persist to disk in background (cheap — just a write)
                persist_to_disk(&image);
            }
        });
    }
}

/// Decode any image format into an RGBA WallpaperImage.
fn decode_source(path: &Path) -> Option<WallpaperImage> {
    let img = mshell_image::lut::decode_pixbuf_rgba(path)?;
    let (width, height) = img.dimensions();
    Some(WallpaperImage {
        buf: Arc::new(img.into_raw()),
        width,
        height,
    })
}

// ── Disk persistence ─────────────────────────────────────────────────────────
// Format: [u32 LE width][u32 LE height][width*height*4 bytes RGBA]

fn persist_to_disk(image: &WallpaperImage) {
    let path = display_cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let mut file = match fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to persist wallpaper: {e}");
            return;
        }
    };
    file.write_all(&image.width.to_le_bytes()).ok();
    file.write_all(&image.height.to_le_bytes()).ok();
    file.write_all(&image.buf).ok();
}

fn load_from_disk() -> Option<WallpaperImage> {
    let path = display_cache_path();
    let mut file = fs::File::open(&path).ok()?;

    let mut header = [0u8; 8];
    file.read_exact(&mut header).ok()?;
    let width = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let height = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);

    let expected_len = (width as usize) * (height as usize) * 4;
    let mut buf = vec![0u8; expected_len];
    file.read_exact(&mut buf).ok()?;

    Some(WallpaperImage {
        buf: Arc::new(buf),
        width,
        height,
    })
}

fn bump_revision() {
    info!("bumping revision");
    WALLPAPER.update(|s| s.revision += 1);
}
