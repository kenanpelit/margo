use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ThemeStoreFields, WallpaperStoreFields};
use mshell_config::schema::themes::Themes;
use mshell_image::lut::{apply_theme_filter_rgba, decode_rgba_scaled};
use reactive_graph::effect::Effect;
use reactive_graph::prelude::{Get, GetUntracked, Update};
use reactive_stores::{ArcStore, Store};
use relm4::gtk::prelude::{Cast, DisplayExt, ListModelExt, MonitorExt};
use relm4::gtk::{gdk, glib};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, mpsc};
use tracing::{info, warn};

// ── Cache paths ──────────────────────────────────────────────────────────────

fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("mshell")
}

/// The original wallpaper as provided by the user (raw bytes, any format).
pub fn source_path() -> PathBuf {
    cache_dir().join("wallpaper_source")
}

/// The displayed (screen-sized, theme-filtered) wallpaper on disk:
/// `[u32 LE width][u32 LE height][RGBA pixels]`. mshell itself never reads it
/// back; it is the hand-off to `mlogind`'s greeter-backdrop sync
/// (`mlogind/src/theme_sync.rs`), which blurs a copy for `mgreet`.
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

/// Path to the bundled margo default wallpaper (`margo-hero.png`), if it's
/// installed. Resolved in priority order: the user data-dir override, then the
/// system data dirs (`/usr/share/margo/wallpapers/…`). `None` on an
/// uninstalled dev tree where the asset hasn't been deployed.
pub fn default_wallpaper_path() -> Option<PathBuf> {
    const REL: &str = "margo/wallpapers/margo-hero.png";
    let mut candidates = vec![glib::user_data_dir().join(REL)];
    candidates.extend(glib::system_data_dirs().into_iter().map(|d| d.join(REL)));
    candidates.into_iter().find(|p| p.exists())
}

// ── In-memory wallpaper ──────────────────────────────────────────────────────

/// The current wallpaper as one shared GPU-uploadable texture. Every
/// per-monitor wallpaper window and every lock-screen surface paints this same
/// `gdk::Texture` (cloning it is a refcount bump), so the pixels exist once in
/// memory instead of once per surface. It is decoded at the size the largest
/// monitor needs (see [`cover_decode_size`]), not the source resolution.
#[derive(Debug, Clone)]
pub struct WallpaperImage {
    pub texture: gdk::Texture,
    pub width: u32,
    pub height: u32,
}

/// Decode target used when no monitor geometry is known (no display yet).
const FALLBACK_TARGET: (u32, u32) = (3840, 2160);

/// The size to decode a `src_w`×`src_h` image at so it still *covers* every
/// target (monitor size in device pixels) without upscaling past the source.
/// Cover is the most demanding fit: `Contain`/`ScaleDown` need less, and
/// `Fill` needs at most the cover scale on each axis, so one cover-sized
/// image serves every `content_fit`. With no targets, [`FALLBACK_TARGET`].
fn cover_decode_size(src_w: u32, src_h: u32, targets: &[(u32, u32)]) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (src_w, src_h);
    }
    let fallback = [FALLBACK_TARGET];
    let targets = if targets.is_empty() {
        &fallback[..]
    } else {
        targets
    };
    let (sw, sh) = (f64::from(src_w), f64::from(src_h));
    let scale = targets
        .iter()
        .map(|&(tw, th)| f64::max(f64::from(tw) / sw, f64::from(th) / sh))
        .fold(0.0, f64::max);
    if scale >= 1.0 {
        return (src_w, src_h);
    }
    let dim = |v: f64, max: u32| ((v * scale).ceil() as u32).clamp(1, max);
    (dim(sw, src_w), dim(sh, src_h))
}

/// Every connected monitor's size in device pixels (logical size × scale).
/// Main thread only (GDK).
fn monitor_pixel_sizes() -> Vec<(u32, u32)> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let monitors = display.monitors();
    (0..monitors.n_items())
        .filter_map(|i| monitors.item(i)?.downcast::<gdk::Monitor>().ok())
        .map(|m| {
            let geometry = m.geometry();
            let scale = m.scale();
            (
                (f64::from(geometry.width()) * scale).ceil() as u32,
                (f64::from(geometry.height()) * scale).ceil() as u32,
            )
        })
        .filter(|&(w, h)| w > 0 && h > 0)
        .collect()
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

static WALLPAPER_INNER: LazyLock<Mutex<WallpaperInner>> = LazyLock::new(|| {
    // Nothing is decoded here: the first (debounced) refilter below decodes
    // the source — or, when none is set (fresh profile / reset / never
    // configured), the bundled margo default — off the main thread at screen
    // size, and bumps the revision when it lands.
    refilter();

    // A new monitor may need a larger decode (or allow a smaller one).
    if let Some(display) = gdk::Display::default() {
        display
            .monitors()
            .connect_items_changed(|_, _, _, _| refilter());
    }

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

    Mutex::new(WallpaperInner {
        cancel_token: Arc::new(AtomicBool::new(false)),
        image: None,
    })
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

// ── Daily wallpaper (Bing / NASA "image of the day") ─────────────────────────
//
// Port of the noctalia `daily-wallpaper` plugin: fetch today's Bing or NASA
// image-of-the-day, cache it under ~/Pictures/Wallpapers, apply it, and prune
// downloads older than 5 days. Synchronous (blocking `curl`) — call off the UI
// thread.

/// Downloaded dailies live here (matches the original plugin).
fn daily_wallpaper_dir() -> PathBuf {
    glib::home_dir().join("Pictures").join("Wallpapers")
}

fn curl_text(url: &str) -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "20", url])
        .output()
        .map_err(|e| format!("curl spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl failed for {url}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn curl_download(url: &str, dest: &Path) -> bool {
    matches!(
        std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "60", "-o"])
            .arg(dest)
            .arg(url)
            .status(),
        Ok(s) if s.success()
    )
}

/// The substring between `start` and the first `end` following it.
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let rest = &s[i..];
    rest.find(end).map(|j| &rest[..j])
}

/// `(primary, fallback)` URLs for Bing's image of the day.
fn bing_urls(locale: &str) -> Result<(String, String), String> {
    let url = format!("https://www.bing.com/HPImageArchive.aspx?format=js&idx=0&n=1&mkt={locale}");
    let body = curl_text(&url)?;
    let urlbase = between(&body, "\"urlbase\":\"", "\"")
        .ok_or_else(|| "Bing: urlbase not found".to_string())?;
    Ok((
        format!("https://www.bing.com{urlbase}_UHD.jpg"),
        format!("https://www.bing.com{urlbase}_1920x1080.jpg"),
    ))
}

/// First `content="…"` after `key` (for `og:image` / `twitter:image` metas).
fn meta_content(body: &str, key: &str) -> Option<String> {
    let i = body.find(key)?;
    between(&body[i..], "content=\"", "\"").map(str::to_string)
}

/// `(primary, fallback)` URLs for NASA's image of the day (HTML-scraped).
fn nasa_urls() -> Result<(String, String), String> {
    let body = curl_text("https://www.nasa.gov/image-of-the-day/")?;
    let primary = body
        .find("hds-gallery-image")
        .and_then(|i| between(&body[i..], "src=\"", "\""))
        .unwrap_or("")
        .to_string();
    let fallback = meta_content(&body, "og:image")
        .or_else(|| meta_content(&body, "twitter:image"))
        .unwrap_or_default();
    if primary.is_empty() && fallback.is_empty() {
        return Err("NASA: no image URL found".to_string());
    }
    Ok((primary, fallback))
}

/// Ensure today's Bing/NASA image-of-the-day is downloaded and return its
/// path (reusing the cached file if already present), pruning downloads older
/// than 5 days. `source` is `"bing"` or `"nasa"`; `locale` is the Bing market
/// (ignored for NASA). Blocking (curl) — call off the UI thread, then apply
/// the returned path with [`set_wallpaper`] on the main thread (`set_wallpaper`
/// touches the reactive config store and must not run on a worker thread).
pub fn fetch_daily_wallpaper(source: &str, locale: &str) -> Result<PathBuf, String> {
    let source = if source.eq_ignore_ascii_case("nasa") {
        "nasa"
    } else {
        "bing"
    };
    let dir = daily_wallpaper_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    let date = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "today".to_string());
    let file = dir.join(format!("{source}-{date}.jpg"));

    if file.is_file() {
        prune_daily(&dir, source);
        return Ok(file);
    }

    let loc = {
        let l = locale.trim().to_lowercase().replace('_', "-");
        if l.is_empty() { "en-us".to_string() } else { l }
    };
    let (primary, fallback) = if source == "nasa" {
        nasa_urls()?
    } else {
        bing_urls(&loc)?
    };

    let ok = (!primary.is_empty() && curl_download(&primary, &file))
        || (!fallback.is_empty() && curl_download(&fallback, &file));
    if !ok {
        let _ = fs::remove_file(&file); // drop any truncated partial
        return Err(format!("{source}: download failed"));
    }

    prune_daily(&dir, source);
    Ok(file)
}

/// Delete `<prefix>-*.jpg` in `dir` older than 5 days.
fn prune_daily(dir: &Path, prefix: &str) {
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 24 * 60 * 60);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let needle = format!("{prefix}-");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(&needle) && name.ends_with(".jpg")) {
            continue;
        }
        if let Ok(meta) = entry.metadata()
            && let Ok(modified) = meta.modified()
            && modified < cutoff
        {
            let _ = fs::remove_file(entry.path());
        }
    }
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
        // No directory configured — the bundled default is the only entry, so
        // next/prev cycling and the menu still have something to land on.
        return default_wallpaper_path().into_iter().collect();
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
        inner.cancel_token.store(true, Ordering::Relaxed);
        inner.image = None;
    }
    bump_revision();
}

/// Returns true if a wallpaper is currently set.
pub fn has_wallpaper() -> bool {
    source_path().exists()
}

// ── Internal ─────────────────────────────────────────────────────────────────

/// Set while a refilter is queued on the main loop, so a burst of triggers
/// (the three config effects all fire on creation; a theme switch can flip
/// several keys at once) collapses into one decode.
static REFILTER_QUEUED: AtomicBool = AtomicBool::new(false);

/// Schedule a re-decode of the current wallpaper with the current filter
/// settings. Debounced to one run per main-loop idle. Never touches
/// `WALLPAPER_INNER` itself, so it is safe to call from its initialiser.
fn refilter() {
    if REFILTER_QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    glib::idle_add_once(|| {
        REFILTER_QUEUED.store(false, Ordering::SeqCst);
        start_refilter();
    });
}

/// One decode request for the wallpaper worker.
struct Job {
    path: PathBuf,
    /// `Some((theme, strength))` when the theme filter applies.
    filter: Option<(Themes, f64)>,
    /// Monitor sizes in device pixels; the decode covers the largest.
    targets: Vec<(u32, u32)>,
    cancel: Arc<AtomicBool>,
}

/// The single wallpaper worker thread. Jobs queue on the channel; the worker
/// skips straight to the newest one, and every job's cancel token is raised as
/// soon as a newer one is issued, so at most one decode is ever in flight.
static WORKER: LazyLock<Option<mpsc::Sender<Job>>> = LazyLock::new(|| {
    let (tx, rx) = mpsc::channel::<Job>();
    match std::thread::Builder::new()
        .name("mshell-wallpaper".into())
        .spawn(move || {
            while let Ok(mut job) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    job = newer;
                }
                run_job(job);
            }
        }) {
        Ok(_) => Some(tx),
        Err(e) => {
            warn!("wallpaper: cannot spawn worker thread: {e}");
            None
        }
    }
});

/// Main thread: snapshot the config and monitor sizes, cancel any decode in
/// flight and hand a fresh job to the worker.
fn start_refilter() {
    let source = source_path();
    let (path, user_set) = if source.exists() {
        (source, true)
    } else if let Some(default) = default_wallpaper_path() {
        info!(
            "no wallpaper set — using bundled default {}",
            default.display()
        );
        (default, false)
    } else {
        return;
    };

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

    // `Default` is now an alias for the static house theme (Kenp), so it
    // grades the wallpaper like any other static theme; only `Wallpaper`
    // (the dynamic-from-image mode) is excluded. The bundled default is shown
    // as shipped.
    let should_filter = user_set && apply && strength != 0.0 && theme != Themes::Wallpaper;

    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut inner) = WALLPAPER_INNER.lock() {
        inner.cancel_token.store(true, Ordering::Relaxed);
        inner.cancel_token = cancel.clone();
    }

    let job = Job {
        path,
        filter: should_filter.then_some((theme, strength)),
        targets: monitor_pixel_sizes(),
        cancel,
    };
    if let Some(tx) = WORKER.as_ref()
        && tx.send(job).is_err()
    {
        warn!("wallpaper: worker thread is gone");
    }
}

/// Worker thread: decode at screen size, filter, wrap in one shared texture,
/// publish, persist.
fn run_job(job: Job) {
    let Job {
        path,
        filter,
        targets,
        cancel,
    } = job;
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    let Some(img) = decode_rgba_scaled(
        &path,
        move |w, h| cover_decode_size(w, h, &targets),
        &cancel,
    ) else {
        return;
    };
    let img = match filter {
        Some((theme, strength)) => {
            match apply_theme_filter_rgba(img, &theme, strength, 1.0, 0.0, &cancel) {
                Some(img) => img,
                None => return,
            }
        }
        None => img,
    };

    let (width, height) = img.dimensions();
    // `from_owned` hands the Vec to GLib without copying; the texture keeps it.
    let bytes = glib::Bytes::from_owned(img.into_raw());
    let texture: gdk::Texture = gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        (width * 4) as usize,
    )
    .upcast();

    let Ok(mut inner) = WALLPAPER_INNER.lock() else {
        return;
    };
    // Checked under the lock `start_refilter` cancels under, so a superseded
    // job can never overwrite a newer image.
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    inner.image = Some(WallpaperImage {
        texture,
        width,
        height,
    });
    drop(inner);

    glib::idle_add_once(|| {
        bump_revision();
    });

    persist_if_changed(width, height, &bytes);
}

// ── Disk persistence ─────────────────────────────────────────────────────────
// Format: [u32 LE width][u32 LE height][width*height*4 bytes RGBA]

/// Hash of the pixels last written to [`display_cache_path`], so a refilter
/// that produces the same image (a theme switch with the filter off, a
/// monitor hotplug) doesn't rewrite the file.
static LAST_PERSISTED: Mutex<Option<u64>> = Mutex::new(None);

fn image_hash(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    (width, height).hash(&mut hasher);
    rgba.hash(&mut hasher);
    hasher.finish()
}

fn persist_if_changed(width: u32, height: u32, rgba: &[u8]) {
    let hash = image_hash(width, height, rgba);
    let Ok(mut last) = LAST_PERSISTED.lock() else {
        return;
    };
    if *last == Some(hash) {
        return;
    }
    match write_raw_atomic(&display_cache_path(), width, height, rgba) {
        Ok(()) => *last = Some(hash),
        Err(e) => warn!("failed to persist wallpaper: {e}"),
    }
}

/// Write the raw `[w][h][RGBA]` file via a temp file + rename, so a reader
/// (`mlogind`'s sync) never sees a half-written image.
fn write_raw_atomic(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "cache path has no parent")
    })?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".wallpaper.raw.{}.tmp", std::process::id()));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&width.to_le_bytes())?;
    file.write_all(&height.to_le_bytes())?;
    file.write_all(rgba)?;
    drop(file);
    fs::rename(&tmp, path)
}

fn bump_revision() {
    info!("bumping revision");
    WALLPAPER.update(|s| s.revision += 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_size_fits_the_largest_monitor() {
        // The measured case: a 4896×3264 photo on a 1920×1200 panel.
        assert_eq!(cover_decode_size(4896, 3264, &[(1920, 1200)]), (1920, 1280));
        // Two monitors: the 2560-wide one dominates.
        assert_eq!(
            cover_decode_size(4896, 3264, &[(1920, 1200), (2560, 1440)]),
            (2560, 1707)
        );
        // A tall monitor needs height coverage.
        assert_eq!(cover_decode_size(4000, 2000, &[(1080, 1920)]), (3840, 1920));
    }

    #[test]
    fn cover_size_never_upscales() {
        assert_eq!(cover_decode_size(1280, 720, &[(1920, 1080)]), (1280, 720));
        assert_eq!(cover_decode_size(1920, 1080, &[(1920, 1080)]), (1920, 1080));
    }

    #[test]
    fn cover_size_without_monitors_uses_fallback() {
        assert_eq!(cover_decode_size(7680, 4320, &[]), (3840, 2160));
        assert_eq!(cover_decode_size(0, 10, &[]), (0, 10));
    }

    #[test]
    fn cover_size_covers_every_target() {
        for &(sw, sh) in &[(4896, 3264), (6000, 4000), (5000, 1000), (1000, 5000)] {
            let targets = [(1920, 1200), (3840, 2160), (1366, 768)];
            let (w, h) = cover_decode_size(sw, sh, &targets);
            for (tw, th) in targets {
                let scale = f64::max(f64::from(tw) / f64::from(w), f64::from(th) / f64::from(h));
                // Covering never needs more than the decoded pixels allow,
                // unless we are already at the source size.
                assert!(scale <= 1.0 || (w, h) == (sw, sh), "{sw}x{sh} -> {w}x{h}");
            }
        }
    }

    #[test]
    fn raw_file_round_trips_the_mlogind_format() {
        let dir = std::env::temp_dir().join(format!("mshell-cache-raw-{}", std::process::id()));
        let path = dir.join("wallpaper.raw");
        let pixels: Vec<u8> = (0..3 * 2 * 4).map(|i| i as u8).collect();
        write_raw_atomic(&path, 3, 2, &pixels).expect("write");
        let bytes = fs::read(&path).expect("read");
        assert_eq!(&bytes[..4], &3u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
        assert_eq!(&bytes[8..], &pixels[..]);
        assert_eq!(bytes.len(), 8 + 3 * 2 * 4);
        // No temp file left behind.
        let leftovers = fs::read_dir(&dir).expect("dir").count();
        assert_eq!(leftovers, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
