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
use std::time::{Duration, Instant};
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
/// (`mlogind/src/theme_sync.rs`), which blurs a copy for `mgreet` when a
/// session starts and ends. An automatic rotation is therefore not written
/// at once (see [`Persisted`]): it waits for the next lock or quit
/// ([`flush_display_cache`]), at most [`MAX_DEFERRED`].
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
    /// The texture's own pixels (straight-alpha RGBA, `width * 4` per row),
    /// shared, not a copy: what `mshell-wallpaper` hands margo to draw.
    pub rgba: glib::Bytes,
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
    set_wallpaper_from(path, Origin::User);
}

/// [`set_wallpaper`] for a change the user didn't ask for just now.
fn set_wallpaper_from(path: &Path, origin: Origin) {
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

    if origin == Origin::Rotation {
        NEXT_IS_ROTATION.store(true, Ordering::SeqCst);
    }
    refilter();
}

// ── Daily wallpaper ──────────────────────────────────────────────────────────
// Lives in `crate::daily_wallpaper`; re-exported here for existing callers.
pub use crate::daily_wallpaper::{DailyCaption, daily_caption, fetch_daily_wallpaper};

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

/// The wallpapers next/prev/random and the rotation step through: the
/// active theme bundle's wallpapers (in bundle order) while one is active
/// and has wallpapers on disk, else every image file in the configured
/// `wallpaper_dir`, sorted by filename for a stable next/prev order.
pub fn list_wallpapers() -> Vec<PathBuf> {
    let theme = config_manager().config().theme().get_untracked();
    if let Some(list) = crate::theme_bundle::active_bundle_wallpapers(&theme) {
        return list;
    }
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
    match list_wallpapers_in(Path::new(&dir)) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("wallpaper: cannot read dir {dir}: {e}");
            Vec::new()
        }
    }
}

/// Image files (by extension, case-insensitive) directly in `dir`, sorted.
fn list_wallpapers_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| WALLPAPER_EXTENSIONS.contains(&e.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();
    Ok(entries)
}

/// Step the wallpaper in the given direction within `wallpaper_dir`
/// and apply it. No-op when the directory has no images.
pub fn cycle_wallpaper(direction: CycleDirection) {
    cycle_wallpaper_from(direction, Origin::User);
}

/// The automatic rotation's step: like [`cycle_wallpaper`], but the new
/// image is not written for the greeter until the next lock or quit (at
/// most 30 minutes later), so a rotation every few minutes doesn't rewrite
/// a screen-sized raw file each time.
pub fn rotate_wallpaper(direction: CycleDirection) {
    cycle_wallpaper_from(direction, Origin::Rotation);
}

fn cycle_wallpaper_from(direction: CycleDirection, origin: Origin) {
    let list = list_wallpapers();
    if list.is_empty() {
        return;
    }
    // Where are we now? `None` when the current wallpaper isn't (or is no
    // longer) in the directory.
    let current_idx = current_wallpaper_path().and_then(|cur| list.iter().position(|p| *p == cur));
    // Cheap, dependency-free PRNG seeded off the clock — wallpaper choice
    // doesn't need crypto randomness.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    let target_idx = cycle_target(current_idx, list.len(), direction, nanos);

    set_wallpaper_from(&list[target_idx], origin);
}

/// The index to step to in a `len`-long (non-empty) list from `current`, the
/// current wallpaper's index (`None` when it isn't in the list). From outside
/// the list, `Next` starts at the first image and `Previous` at the last.
/// `Random` picks `seed % len`, nudged forward so it never repeats the current
/// image when there is another to choose.
fn cycle_target(
    current: Option<usize>,
    len: usize,
    direction: CycleDirection,
    seed: usize,
) -> usize {
    match (direction, current) {
        (CycleDirection::Next, Some(i)) => (i + 1) % len,
        (CycleDirection::Next, None) => 0,
        (CycleDirection::Previous, Some(i)) => (i + len - 1) % len,
        (CycleDirection::Previous, None) => len - 1,
        (CycleDirection::Random, _) => {
            let idx = seed % len;
            if len > 1 && Some(idx) == current {
                (idx + 1) % len
            } else {
                idx
            }
        }
    }
}

/// Clear the wallpaper entirely.
pub fn clear_wallpaper() {
    fs::remove_file(source_path()).ok();
    if let Ok(mut p) = PERSISTED.lock() {
        *p = Persisted::default();
        fs::remove_file(display_cache_path()).ok();
    }
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

/// Set by [`rotate_wallpaper`]: the next refilter shows a rotated image.
static NEXT_IS_ROTATION: AtomicBool = AtomicBool::new(false);

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
    origin: Origin,
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

    let origin = if NEXT_IS_ROTATION.swap(false, Ordering::SeqCst) {
        Origin::Rotation
    } else {
        Origin::User
    };
    let job = Job {
        path,
        filter: should_filter.then_some((theme, strength)),
        targets: monitor_pixel_sizes(),
        cancel,
        origin,
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
        origin,
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
        rgba: bytes.clone(),
    });
    drop(inner);

    glib::idle_add_once(|| {
        bump_revision();
    });

    if let Ok(mut p) = PERSISTED.lock() {
        p.offer(
            width,
            height,
            bytes,
            origin,
            Instant::now(),
            &mut write_display_cache,
        );
    }
}

// ── Disk persistence ─────────────────────────────────────────────────────────
// Format: [u32 LE width][u32 LE height][width*height*4 bytes RGBA]

/// Why a new image is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Picked, refiltered, or shown at start: write it for the greeter now.
    User,
    /// The automatic rotation: the write can wait (see [`Persisted`]).
    Rotation,
}

/// The longest a rotated image goes unwritten. The next lock or quit
/// writes it sooner; this bounds how stale the greeter's copy can be when
/// the machine is rebooted or powered off without either.
const MAX_DEFERRED: Duration = Duration::from_secs(30 * 60);

/// Whether an image from `origin` is written now, `since_write` after the
/// last write (`None` = nothing written yet).
fn write_now(origin: Origin, since_write: Option<Duration>) -> bool {
    match origin {
        Origin::User => true,
        Origin::Rotation => since_write.is_none_or(|d| d >= MAX_DEFERRED),
    }
}

/// What reached [`display_cache_path`], and a rotated image waiting for it.
///
/// The file only matters when `mlogind` reads it, at the start and end of
/// a session. Writing every rotation (5 minutes by default) rewrote a
/// screen-sized raw file 12 times an hour, about 4 GB a day of SSD writes,
/// for one read per login.
#[derive(Default)]
struct Persisted {
    /// Hash of the pixels last written, so an identical image (a theme
    /// switch with the filter off, a monitor hotplug) isn't rewritten.
    last_hash: Option<u64>,
    last_write: Option<Instant>,
    /// A rotated image not written yet. `glib::Bytes` is the texture's own
    /// buffer, shared, not a copy.
    pending: Option<(u32, u32, glib::Bytes)>,
}

static PERSISTED: LazyLock<Mutex<Persisted>> = LazyLock::new(Mutex::default);

impl Persisted {
    /// A new image is on screen: write it now or keep it for [`Self::flush`].
    fn offer(
        &mut self,
        width: u32,
        height: u32,
        rgba: glib::Bytes,
        origin: Origin,
        now: Instant,
        write: &mut impl FnMut(u32, u32, &[u8]) -> std::io::Result<()>,
    ) {
        let since = self.last_write.map(|t| now.saturating_duration_since(t));
        if write_now(origin, since) {
            self.pending = None;
            self.write(width, height, &rgba, now, write);
        } else {
            self.pending = Some((width, height, rgba));
        }
    }

    /// Write the image a rotation left pending, if any.
    fn flush(
        &mut self,
        now: Instant,
        write: &mut impl FnMut(u32, u32, &[u8]) -> std::io::Result<()>,
    ) {
        if let Some((width, height, rgba)) = self.pending.take() {
            self.write(width, height, &rgba, now, write);
        }
    }

    fn write(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        now: Instant,
        write: &mut impl FnMut(u32, u32, &[u8]) -> std::io::Result<()>,
    ) {
        let hash = image_hash(width, height, rgba);
        if self.last_hash == Some(hash) {
            return;
        }
        match write(width, height, rgba) {
            Ok(()) => {
                self.last_hash = Some(hash);
                self.last_write = Some(now);
            }
            Err(e) => warn!("failed to persist wallpaper: {e}"),
        }
    }
}

/// Write a rotated wallpaper that is still waiting for the greeter's copy.
/// Call when the session locks and before the shell quits. Blocking (one
/// screen-sized file write); a no-op when nothing is pending.
pub fn flush_display_cache() {
    if let Ok(mut p) = PERSISTED.lock() {
        p.flush(Instant::now(), &mut write_display_cache);
    }
}

fn write_display_cache(width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    write_raw_atomic(&display_cache_path(), width, height, rgba)
}

fn image_hash(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    (width, height).hash(&mut hasher);
    rgba.hash(&mut hasher);
    hasher.finish()
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

    /// Records each write's first pixel byte.
    fn recorder(log: &mut Vec<u8>) -> impl FnMut(u32, u32, &[u8]) -> std::io::Result<()> + '_ {
        move |_, _, rgba| {
            log.push(rgba[0]);
            Ok(())
        }
    }

    fn image(fill: u8) -> glib::Bytes {
        glib::Bytes::from_owned(vec![fill; 2 * 2 * 4])
    }

    #[test]
    fn rotations_wait_for_a_lock_and_user_changes_write_at_once() {
        let t0 = Instant::now();
        let min = |m: u64| t0 + Duration::from_secs(m * 60);
        let mut log = Vec::new();
        let mut p = Persisted::default();
        // Start-up / a picked wallpaper: written now.
        p.offer(2, 2, image(1), Origin::User, t0, &mut recorder(&mut log));
        assert_eq!(log, [1]);
        // Two rotations without a lock: nothing written.
        p.offer(
            2,
            2,
            image(2),
            Origin::Rotation,
            min(5),
            &mut recorder(&mut log),
        );
        p.offer(
            2,
            2,
            image(3),
            Origin::Rotation,
            min(10),
            &mut recorder(&mut log),
        );
        assert_eq!(log, [1]);
        // The lock writes the newest one, once.
        p.flush(min(11), &mut recorder(&mut log));
        p.flush(min(12), &mut recorder(&mut log));
        assert_eq!(log, [1, 3]);
        // A rotation long after the last write goes straight out.
        p.offer(
            2,
            2,
            image(4),
            Origin::Rotation,
            min(45),
            &mut recorder(&mut log),
        );
        assert_eq!(log, [1, 3, 4]);
        // A user change drops a pending rotation and writes itself.
        p.offer(
            2,
            2,
            image(5),
            Origin::Rotation,
            min(50),
            &mut recorder(&mut log),
        );
        p.offer(
            2,
            2,
            image(6),
            Origin::User,
            min(51),
            &mut recorder(&mut log),
        );
        p.flush(min(52), &mut recorder(&mut log));
        assert_eq!(log, [1, 3, 4, 6]);
        // The same pixels again are not rewritten.
        p.offer(
            2,
            2,
            image(6),
            Origin::User,
            min(53),
            &mut recorder(&mut log),
        );
        assert_eq!(log, [1, 3, 4, 6]);
    }

    #[test]
    fn a_failed_write_is_retried_by_the_next_change() {
        let t0 = Instant::now();
        let mut p = Persisted::default();
        let mut fail = |_: u32, _: u32, _: &[u8]| Err(std::io::Error::other("disk full"));
        p.offer(2, 2, image(1), Origin::User, t0, &mut fail);
        let mut log = Vec::new();
        // Nothing was written, so a rotation isn't deferred behind it.
        p.offer(
            2,
            2,
            image(1),
            Origin::Rotation,
            t0,
            &mut recorder(&mut log),
        );
        assert_eq!(log, [1]);
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

    #[test]
    fn cycle_steps_and_wraps() {
        use CycleDirection::*;
        assert_eq!(cycle_target(Some(0), 3, Next, 0), 1);
        assert_eq!(cycle_target(Some(2), 3, Next, 0), 0);
        assert_eq!(cycle_target(Some(0), 3, Previous, 0), 2);
        assert_eq!(cycle_target(Some(1), 3, Previous, 0), 0);
        assert_eq!(cycle_target(Some(0), 1, Next, 0), 0);
        assert_eq!(cycle_target(Some(0), 1, Previous, 0), 0);
        assert_eq!(cycle_target(Some(0), 1, Random, 12345), 0);
    }

    #[test]
    fn cycle_from_outside_the_directory_starts_at_either_end() {
        // The current wallpaper isn't in the list (the bundled default, a
        // daily download, or a file since deleted): Next must land on the
        // first image, not skip it, and Previous on the last.
        use CycleDirection::*;
        assert_eq!(cycle_target(None, 3, Next, 0), 0);
        assert_eq!(cycle_target(None, 3, Previous, 0), 2);
        assert_eq!(cycle_target(None, 1, Next, 0), 0);
        for seed in 0..6 {
            assert!(cycle_target(None, 3, Random, seed) < 3);
        }
        // With a single image, Random may pick it when it isn't current.
        assert_eq!(cycle_target(None, 1, Random, 7), 0);
    }

    #[test]
    fn cycle_random_never_repeats_the_current_image() {
        for len in 2..6 {
            for cur in 0..len {
                for seed in 0..20 {
                    let idx = cycle_target(Some(cur), len, CycleDirection::Random, seed);
                    assert!(idx < len);
                    assert_ne!(idx, cur, "len {len} cur {cur} seed {seed}");
                }
            }
        }
    }

    #[test]
    fn list_wallpapers_filters_by_extension_and_sorts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for name in [
            "b.PNG",
            "a.jpg",
            "c.webp",
            "notes.txt",
            "noext",
            "d.JpEg",
            "e.gif",
        ] {
            fs::write(tmp.path().join(name), b"x").expect("write");
        }
        fs::create_dir(tmp.path().join("sub.png.d")).expect("mkdir");
        let names: Vec<String> = list_wallpapers_in(tmp.path())
            .expect("list")
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.jpg", "b.PNG", "c.webp", "d.JpEg", "e.gif"]);
        assert!(list_wallpapers_in(&tmp.path().join("missing")).is_err());
    }

    #[test]
    fn image_hash_tells_dimensions_apart() {
        let px = [0u8; 16];
        assert_eq!(image_hash(2, 2, &px), image_hash(2, 2, &px));
        assert_ne!(image_hash(2, 2, &px), image_hash(4, 1, &px));
        assert_ne!(image_hash(2, 2, &px), image_hash(2, 2, &[1u8; 16]));
    }

    #[test]
    fn raw_file_overwrite_replaces_the_previous_image() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nested").join("wallpaper.raw");
        write_raw_atomic(&path, 1, 1, &[1, 2, 3, 4]).expect("first");
        write_raw_atomic(&path, 2, 1, &[5; 8]).expect("second");
        let bytes = fs::read(&path).expect("read");
        assert_eq!(bytes.len(), 8 + 8);
        assert_eq!(&bytes[..4], &2u32.to_le_bytes());
        assert_eq!(&bytes[8..], &[5; 8]);
    }
}
