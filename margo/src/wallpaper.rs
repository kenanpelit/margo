//! Compositor-side wallpaper renderer.
//!
//! Decodes the configured wallpaper at startup (and off the event loop
//! again when a reload changes it, `state/reload.rs`), uploads it as a
//! GPU texture the first time a renderer draws it, and exposes a
//! `render_element()` helper that fits the bitmap into a given output
//! rectangle. The decoded CPU pixels (~33 MB for a 4K image) are dropped
//! after that first upload; see [`PerRendererTextures`]. The render loop in `backend/udev/frame.rs` pushes
//! this element to the bottom of the per-output element stack so it
//! sits behind every window and layer surface.
//!
//! Resolution chain when [`Config::wallpaper`] is `None`:
//!   1. `~/.local/share/margo/wallpapers/default.jpg` — user override.
//!   2. `/usr/share/margo/wallpapers/default.jpg`     — package default.
//!
//! Mirrors mlock's chain so the lock screen and the desktop pick the
//! same image on a clean install.
//!
//! Fit modes: every [`margo_config::WallpaperFit`] (see [`fit_rects`]).
//! A client can replace this image per output at runtime over IPC; see
//! [`crate::wallpaper::runtime`].
//!
//! Pixels are uploaded premultiplied, as smithay's GLES shaders blend
//! them: a PNG with transparency used to show the colour of its fully
//! transparent pixels.
//!
//! Raw backdrop: a `.raw` path is not decoded through the `image`
//! crate but read as a headed `[u32 LE width][u32 LE height][RGBA]`
//! buffer — the format `mlogind`'s theme sync bakes into
//! `/var/lib/mgreet/background.raw` and `mgreet` paints. Pointing the
//! greeter compositor at that same file makes margo's first frame
//! pixel-identical to the greeter's backdrop, so the login card fades
//! in over an unchanging wallpaper instead of over the packaged
//! default. Keep the header contract in step with `theme_sync.rs`.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};

use drm_fourcc::DrmFourcc;
use smithay::{
    backend::renderer::{
        ContextId, ImportMem, Renderer,
        element::{Id, Kind, texture::TextureRenderElement},
        gles::{GlesRenderer, GlesTexture},
    },
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};
use tracing::{info, warn};

pub mod runtime;

const FALLBACK_RELATIVE_USER: &str = ".local/share/margo/wallpapers/default.jpg";
const FALLBACK_SYSTEM: &str = "/usr/share/margo/wallpapers/default.jpg";

pub struct WallpaperState {
    /// Pixel size of the decoded image (for the fit maths).
    natural_size: (u32, u32),
    /// Element id, stable for this image's lifetime: the damage tracker
    /// repaints the wallpaper only when it changes, not every frame.
    id: Id,
    /// One texture per renderer that draws this image, uploaded on first
    /// use. One texture serves every output of that renderer — the element
    /// handles per-output scaling via the `dst_size` arg below. `RefCell`
    /// because the render path only holds `&MargoState`.
    textures: RefCell<PerRendererTextures<ContextId<GlesTexture>, GlesTexture>>,
}

/// Uploads one texture per renderer from decoded pixels that are kept only
/// until the first upload.
///
/// margo has one GLES renderer on udev (every output, screencopy and cast
/// render through it) and the winit backend doesn't draw the wallpaper, so
/// in practice the pixels are uploaded once and freed — the CPU copy used
/// to stay for the whole session (~33 MB at 4K) because smithay's
/// `MemoryRenderBuffer` keeps it for renderers it hasn't seen yet. A
/// renderer that does turn up later re-decodes the image from its file
/// instead (slow, but it never happens on a single-GPU session). Each
/// renderer gets one attempt: a failed upload or re-decode is remembered
/// rather than retried every frame (the wallpaper is then missing on that
/// renderer and the `rootcolor` clear shows).
///
/// Generic over the renderer key and texture type so the bookkeeping is
/// unit-testable without a GPU.
struct PerRendererTextures<K, T> {
    /// Decoded pixels not yet uploaded. `None` once the first renderer
    /// took them.
    pixels: Option<Vec<u8>>,
    /// File to re-decode for a renderer that comes after the pixels were
    /// dropped; `None` for an image without one.
    source: Option<PathBuf>,
    /// Per renderer: its texture, or `None` when its one attempt failed.
    entries: Vec<(K, Option<T>)>,
}

impl<K: PartialEq, T> PerRendererTextures<K, T> {
    fn new(pixels: Vec<u8>, source: Option<PathBuf>) -> Self {
        Self {
            pixels: Some(pixels),
            source,
            entries: Vec::new(),
        }
    }

    /// The texture for renderer `key`, uploading it on the first call for
    /// that renderer: from the kept pixels (which are then dropped), or
    /// from `redecode(source)` when they're already gone.
    fn get_or_upload(
        &mut self,
        key: K,
        upload: impl FnOnce(&[u8]) -> Option<T>,
        redecode: impl FnOnce(&Path) -> Option<Vec<u8>>,
    ) -> Option<&T> {
        let index = match self.entries.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                let pixels = match self.pixels.take() {
                    Some(pixels) => Some(pixels),
                    None => self.source.as_deref().and_then(redecode),
                };
                let texture = pixels.as_deref().and_then(upload);
                self.entries.push((key, texture));
                self.entries.len() - 1
            }
        };
        self.entries[index].1.as_ref()
    }

    /// Whether the CPU copy is still held (tests).
    #[cfg(test)]
    fn holds_pixels(&self) -> bool {
        self.pixels.is_some()
    }
}

/// A decoded image, RGBA in `Abgr8888` byte order. Plain data, so the
/// decode can run off the event loop and be turned into a
/// [`WallpaperState`] on it.
pub struct DecodedImage {
    rgba: Vec<u8>,
    size: (u32, u32),
    /// The file it was decoded from, for a later re-decode.
    source: Option<PathBuf>,
}

impl DecodedImage {
    /// Resolve exactly `path` (with `~` expansion), without the wallpaper
    /// fallback chain — the overview backdrop's semantics. `None` (logged)
    /// when the path is empty or missing. Reads `HOME`, so call it on the
    /// event-loop thread: margo mutates its environment at runtime (e.g.
    /// `DISPLAY` once XWayland is up), and getenv racing setenv is unsound.
    pub fn resolve_exact(path: &str) -> Option<PathBuf> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return None;
        }
        let expanded = expand_home(trimmed);
        if !exists_and_readable(&expanded) {
            warn!(path = %expanded.display(), "overview backdrop: image not found");
            return None;
        }
        Some(expanded)
    }

    /// The configured wallpaper's path through the fallback chain (see the
    /// module docs), `None` when nothing is readable. Reads `HOME`: call it
    /// on the event-loop thread, like [`Self::resolve_exact`].
    pub fn resolve_wallpaper(explicit: Option<&str>) -> Option<PathBuf> {
        resolve_path(explicit)
    }

    /// Decode an already-resolved path. Touches no environment, so it may
    /// run on any thread. `None` (logged) when undecodable.
    pub fn decode(path: &Path) -> Option<Self> {
        match decode_pixels(path) {
            Ok(image) => {
                info!(path = %path.display(), "image: loaded");
                Some(image.with_source(path))
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "image: decode failed");
                None
            }
        }
    }
}

impl DecodedImage {
    /// A `w`×`h` image of one premultiplied `rgba` colour (tests).
    #[cfg(test)]
    pub(crate) fn filled_for_test(w: u32, h: u32, rgba: [u8; 4]) -> Self {
        Self {
            rgba: rgba.repeat(w as usize * h as usize),
            size: (w, h),
            source: None,
        }
    }

    pub(crate) fn with_source(mut self, path: &Path) -> Self {
        self.source = Some(path.to_path_buf());
        self
    }
}

impl WallpaperState {
    /// Resolve and decode the wallpaper (the upload happens on first draw). Returns `None` when
    /// no candidate path is readable or the decode fails — callers
    /// fall through to the compositor's solid `rootcolor` clear.
    pub fn load(explicit: Option<&str>) -> Option<Self> {
        let path = resolve_path(explicit)?;
        match decode_pixels(&path) {
            Ok(image) => {
                info!(
                    path = %path.display(),
                    width = image.size.0,
                    height = image.size.1,
                    "wallpaper: loaded"
                );
                Some(Self::from_decoded(image.with_source(&path)))
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "wallpaper: decode failed");
                None
            }
        }
    }

    /// Like [`Self::load`] but **without** the wallpaper fallback chain:
    /// decode exactly `path` (with `~` expansion) or return `None`. Used
    /// for the overview backdrop image, which must stay unset (so the
    /// solid `overview_backdrop_color` shows) when its configured path is
    /// missing — rather than silently borrowing the desktop wallpaper
    /// default like [`Self::load`] would.
    pub fn load_exact(path: &str) -> Option<Self> {
        let path = DecodedImage::resolve_exact(path)?;
        DecodedImage::decode(&path).map(Self::from_decoded)
    }

    /// Wrap a [`DecodedImage`]; its pixels are uploaded on first draw.
    pub fn from_decoded(image: DecodedImage) -> Self {
        Self {
            natural_size: image.size,
            id: Id::new(),
            textures: RefCell::new(PerRendererTextures::new(image.rgba, image.source)),
        }
    }

    /// Pixel size of the image.
    pub fn natural_size(&self) -> (u32, u32) {
        self.natural_size
    }

    /// The image's element id (see [`Self::element`]).
    pub fn id(&self) -> &Id {
        &self.id
    }

    /// This renderer's texture of the image, uploaded on first use.
    fn texture(&self, renderer: &mut GlesRenderer) -> Option<GlesTexture> {
        let (w, h) = self.natural_size;
        let size = Size::<i32, Buffer>::from((i32::try_from(w).ok()?, i32::try_from(h).ok()?));
        let mut textures = self.textures.borrow_mut();
        let key = renderer.context_id();
        textures
            .get_or_upload(
                key,
                |pixels| {
                    // `Abgr8888` is RGBA byte order little-endian, which is
                    // what the decoders produce (see `decode_pixels`).
                    match renderer.import_memory(pixels, DrmFourcc::Abgr8888, size, false) {
                        Ok(texture) => Some(texture),
                        Err(e) => {
                            warn!(error = ?e, "wallpaper: texture upload failed");
                            None
                        }
                    }
                },
                |path| {
                    info!(path = %path.display(), "wallpaper: re-decoding for a new renderer");
                    match decode_pixels(path) {
                        Ok(image) if image.size == (w, h) => Some(image.rgba),
                        Ok(_) => {
                            warn!(path = %path.display(), "wallpaper: file changed size since load");
                            None
                        }
                        Err(e) => {
                            warn!(path = %path.display(), error = %e, "wallpaper: re-decode failed");
                            None
                        }
                    }
                },
            )
            .cloned()
    }

    /// Build a render element for an output of `output_size` (logical)
    /// placed per `fit` (`wallpaper_fit`): `Cover` crops the centred part
    /// whose aspect matches the output, `Contain` letterboxes the whole
    /// image, `Fill` stretches it, `Center` shows it at native size (cropped
    /// if larger). Uncovered strips show the `rootcolor` clear.
    ///
    /// Coordinates are output-**local**: the udev renderer composites each
    /// output in its own space, so the image is anchored at the output's
    /// top-left (a global origin here once shoved the wallpaper 1920 px
    /// right on a second monitor). `scale` converts the letterbox offset to
    /// the physical location smithay wants.
    pub fn render_element(
        &self,
        renderer: &mut GlesRenderer,
        output_size: Size<i32, Logical>,
        fit: margo_config::WallpaperFit,
        scale: f64,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let (src, dst_loc, dst_size) = fit_rects(
            (self.natural_size.0 as f64, self.natural_size.1 as f64),
            (output_size.w as f64, output_size.h as f64),
            fit,
        )?;
        // The element takes the sample region and destination size in
        // logical units (it resolves physical scaling itself); the location
        // is physical.
        let src =
            Rectangle::<f64, Logical>::new(Point::from((src.0, src.1)), Size::from((src.2, src.3)));
        let dst_size =
            Size::<i32, Logical>::from((dst_size.0.round() as i32, dst_size.1.round() as i32));
        self.element(
            renderer,
            self.id.clone(),
            src,
            Point::from((dst_loc.0 * scale, dst_loc.1 * scale)),
            dst_size,
            1.0,
        )
    }

    /// An element drawing the `src` part of the image (image pixels) at
    /// `loc` (output-local physical px), `size` logical px large, under
    /// `id`. The building block for [`Self::render_element`] and the
    /// runtime wallpaper's crossfade pieces and overview cells, which each
    /// need their own id for the same texture.
    pub fn element(
        &self,
        renderer: &mut GlesRenderer,
        id: Id,
        src: Rectangle<f64, Logical>,
        loc: Point<f64, Physical>,
        size: Size<i32, Logical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let texture = self.texture(renderer)?;
        Some(TextureRenderElement::from_static_texture(
            id,
            renderer.context_id(),
            loc,
            texture,
            1,
            Transform::Normal,
            Some(alpha),
            Some(src),
            Some(size),
            None,
            Kind::Unspecified,
        ))
    }
}

/// `(src x, y, w, h)` in image pixels, destination location and size in
/// output-local logical px, for an `img`-sized image on an `out`-sized
/// output under `fit`. `None` for a degenerate image or output.
pub(crate) fn fit_rects(
    img: (f64, f64),
    out: (f64, f64),
    fit: margo_config::WallpaperFit,
) -> Option<((f64, f64, f64, f64), (f64, f64), (f64, f64))> {
    use margo_config::WallpaperFit;
    let ((iw, ih), (ow, oh)) = (img, out);
    if iw <= 0.0 || ih <= 0.0 || ow <= 0.0 || oh <= 0.0 {
        return None;
    }
    Some(match fit {
        WallpaperFit::Cover => {
            let (sw, sh) = if iw / ih > ow / oh {
                (ih * ow / oh, ih)
            } else {
                (iw, iw * oh / ow)
            };
            (
                ((iw - sw) / 2.0, (ih - sh) / 2.0, sw, sh),
                (0.0, 0.0),
                (ow, oh),
            )
        }
        WallpaperFit::Contain => {
            let k = (ow / iw).min(oh / ih);
            let (dw, dh) = (iw * k, ih * k);
            (
                (0.0, 0.0, iw, ih),
                ((ow - dw) / 2.0, (oh - dh) / 2.0),
                (dw, dh),
            )
        }
        WallpaperFit::Fill => ((0.0, 0.0, iw, ih), (0.0, 0.0), (ow, oh)),
        // GTK: native size when the image fits on both axes, else contain.
        WallpaperFit::ScaleDown if iw <= ow && ih <= oh => {
            return fit_rects(img, out, WallpaperFit::Center);
        }
        WallpaperFit::ScaleDown => return fit_rects(img, out, WallpaperFit::Contain),
        WallpaperFit::Center => {
            let (vw, vh) = (iw.min(ow), ih.min(oh));
            (
                ((iw - vw) / 2.0, (ih - vh) / 2.0, vw, vh),
                ((ow - vw) / 2.0, (oh - vh) / 2.0),
                (vw, vh),
            )
        }
    })
}

fn resolve_path(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let expanded = expand_home(p);
        if exists_and_readable(&expanded) {
            return Some(expanded);
        }
        warn!(path = %expanded.display(), "wallpaper: configured path missing, trying fallbacks");
    }

    let user_fallback = home_dir().join(FALLBACK_RELATIVE_USER);
    if exists_and_readable(&user_fallback) {
        return Some(user_fallback);
    }

    let system_fallback = PathBuf::from(FALLBACK_SYSTEM);
    if exists_and_readable(&system_fallback) {
        return Some(system_fallback);
    }

    None
}

/// The greeter backdrop's header size, in bytes: two little-endian `u32`s.
const RAW_HEADER: usize = 8;

/// Decode `path` (a headed `.raw` backdrop or any image the `image` crate
/// opens) to premultiplied RGBA.
pub(crate) fn decode_pixels(path: &Path) -> Result<DecodedImage, image::ImageError> {
    let mut image = decode_straight(path)?;
    premultiply(&mut image.rgba);
    Ok(image)
}

/// Premultiply straight-alpha RGBA in place, rounding like GL does. A
/// no-op (one pass, no writes) for an opaque image.
pub(crate) fn premultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        if a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
}

fn decode_straight(path: &Path) -> Result<DecodedImage, image::ImageError> {
    if is_raw_backdrop(path) {
        // `image::ImageError: From<io::Error>`, so a raw failure surfaces
        // through the same `warn!(error = %e)` path as a decode failure.
        return Ok(decode_raw(path)?);
    }

    let img = image::open(path)?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());

    // A texture imported as `DrmFourcc::Abgr8888` expects
    // RGBA byte order in little-endian (R first byte, A last byte) —
    // which is exactly what `image::RgbaImage` gives us. No swizzle
    // needed. (Cursor uses Argb8888 because libxcursor outputs that
    // ordering; we deliberately diverge here.)
    Ok(DecodedImage {
        rgba: rgba.into_raw(),
        size: (w, h),
        source: None,
    })
}

/// A `.raw` path is the headed greeter backdrop, not an image the `image`
/// crate can open. Match by extension, case-insensitively.
pub(crate) fn is_raw_backdrop(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("raw"))
}

/// Largest raw image accepted, per side. GL's usual texture limit; also
/// bounds what a stray file can make margo allocate.
const RAW_MAX_SIDE: u32 = 16384;

/// Read a `[u32 LE width][u32 LE height][RGBA]` backdrop. The trailing
/// RGBA is already in `Abgr8888` byte order (R first, A last), so — like
/// the `image` path above — no swizzle is needed. The header is checked
/// against the file's length before the body is allocated, so a huge or
/// truncated file is refused without reading it.
fn decode_raw(path: &Path) -> io::Result<DecodedImage> {
    use std::io::Read as _;
    let invalid =
        |msg: &str| io::Error::new(io::ErrorKind::InvalidData, format!("raw backdrop: {msg}"));
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; RAW_HEADER];
    file.read_exact(&mut header)?;
    let (w, h) = raw_dimensions(&header).ok_or_else(|| invalid("zero size"))?;
    if w > RAW_MAX_SIDE || h > RAW_MAX_SIDE {
        return Err(invalid("larger than 16384 px"));
    }
    let len = usize::try_from(file.metadata()?.len()).map_err(|_| invalid("too large"))?;
    let body = raw_body_len(w, h).ok_or_else(|| invalid("too large"))?;
    if len != body + RAW_HEADER {
        return Err(invalid("header disagrees with byte count"));
    }
    let mut rgba = vec![0u8; body];
    file.read_exact(&mut rgba)?;
    Ok(DecodedImage {
        rgba,
        size: (w, h),
        source: None,
    })
}

/// The non-zero `(width, height)` of a raw header.
fn raw_dimensions(header: &[u8; RAW_HEADER]) -> Option<(u32, u32)> {
    let width = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let height = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    (width != 0 && height != 0).then_some((width, height))
}

/// `w * h * 4`, `None` on overflow.
fn raw_body_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

/// `(width, height)` of a `[u32 LE w][u32 LE h][RGBA]` buffer whose length
/// agrees. Overflow is a rejection, not a wrap: the file is machine-written
/// by `mlogind`, but a stray or truncated one must never make us index past
/// the slice. Mirrors `theme_sync.rs::parse_header` — one shared contract.
#[cfg(test)]
fn parse_raw_header(bytes: &[u8]) -> Option<(u32, u32)> {
    let header: &[u8; RAW_HEADER] = bytes.get(..RAW_HEADER)?.try_into().ok()?;
    let (width, height) = raw_dimensions(header)?;
    let body = raw_body_len(width, height)?;
    if bytes.len() != body.checked_add(RAW_HEADER)? {
        return None;
    }
    Some((width, height))
}

pub(crate) fn exists_and_readable(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub(crate) fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        home_dir().join(rest)
    } else if p == "~" {
        home_dir()
    } else {
        PathBuf::from(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upload stub: records each call, returns the pixel count as the
    /// "texture".
    fn upload_counting(calls: &mut u32) -> impl FnOnce(&[u8]) -> Option<usize> + '_ {
        move |pixels| {
            *calls += 1;
            Some(pixels.len())
        }
    }

    #[test]
    fn pixels_are_dropped_after_the_first_upload_and_reused_per_renderer() {
        let mut textures = PerRendererTextures::new(vec![7; 16], Some(PathBuf::from("/w.png")));
        let (mut uploads, mut decodes) = (0, 0);
        let mut redecode = |_: &Path| {
            decodes += 1;
            Some(vec![7; 16])
        };
        assert_eq!(
            textures.get_or_upload(1, upload_counting(&mut uploads), &mut redecode),
            Some(&16)
        );
        assert!(!textures.holds_pixels(), "CPU copy freed after the upload");
        // Every later frame on the same renderer reuses its texture.
        for _ in 0..3 {
            assert_eq!(
                textures.get_or_upload(1, upload_counting(&mut uploads), &mut redecode),
                Some(&16)
            );
        }
        assert_eq!((uploads, decodes), (1, 0));
    }

    #[test]
    fn a_renderer_seen_later_re_decodes_from_the_file() {
        let mut textures = PerRendererTextures::new(vec![1; 8], Some(PathBuf::from("/w.png")));
        let mut uploads = 0;
        let mut decoded_from = Vec::new();
        let mut redecode = |path: &Path| {
            decoded_from.push(path.to_path_buf());
            Some(vec![2; 4])
        };
        textures.get_or_upload("gpu0", upload_counting(&mut uploads), &mut redecode);
        assert_eq!(
            textures.get_or_upload("gpu1", upload_counting(&mut uploads), &mut redecode),
            Some(&4)
        );
        assert_eq!(decoded_from, vec![PathBuf::from("/w.png")]);
        assert!(
            !textures.holds_pixels(),
            "the re-decoded copy isn't kept either"
        );
        assert_eq!(uploads, 2);
    }

    #[test]
    fn render_element_uploads_once_and_keeps_a_stable_element() {
        use smithay::backend::renderer::element::Element;
        let Some(mut renderer) = crate::render::blur::gpu_tests::renderer() else {
            eprintln!("skipped: no render node / EGL");
            return;
        };
        let wallpaper = WallpaperState::from_decoded(DecodedImage {
            rgba: vec![255; 4 * 8 * 4],
            size: (8, 4),
            source: None,
        });
        let fit = margo_config::WallpaperFit::Cover;
        let first = wallpaper
            .render_element(&mut renderer, (16, 8).into(), fit, 1.0)
            .expect("element");
        assert!(!wallpaper.textures.borrow().holds_pixels());
        let second = wallpaper
            .render_element(&mut renderer, (16, 8).into(), fit, 2.0)
            .expect("element from the uploaded texture, no pixels needed");
        // Same id across frames and outputs, so the damage tracker doesn't
        // repaint the whole wallpaper every frame.
        assert_eq!(first.id(), second.id());
    }

    #[test]
    fn a_failed_attempt_is_not_retried_every_frame() {
        let mut textures = PerRendererTextures::<u8, usize>::new(vec![1; 8], None);
        let mut attempts = 0;
        for _ in 0..3 {
            let got = textures.get_or_upload(
                1,
                |_| {
                    attempts += 1;
                    None
                },
                |_| None,
            );
            assert_eq!(got, None);
        }
        assert_eq!(attempts, 1);
        // No source file: a second renderer gets no texture, and no decode.
        assert_eq!(
            textures.get_or_upload(2, |_| Some(1), |_| panic!("no source to decode")),
            None
        );
    }

    /// A `[w][h][RGBA]` buffer whose body is `fill` — the shape
    /// `theme_sync.rs` writes to `background.raw`.
    fn raw(width: u32, height: u32, fill: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&width.to_le_bytes());
        buf.extend_from_slice(&height.to_le_bytes());
        buf.resize(RAW_HEADER + (width as usize) * (height as usize) * 4, fill);
        buf
    }

    #[test]
    fn a_well_formed_backdrop_header_round_trips() {
        assert_eq!(parse_raw_header(&raw(3, 5, 0)), Some((3, 5)));
    }

    #[test]
    fn a_header_that_disagrees_with_the_byte_count_is_rejected() {
        let mut buf = raw(3, 5, 0);
        buf.pop();
        assert_eq!(parse_raw_header(&buf), None);
    }

    #[test]
    fn a_truncated_file_is_rejected_rather_than_indexed() {
        assert_eq!(parse_raw_header(&[1, 2, 3]), None);
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        assert_eq!(parse_raw_header(&raw(0, 5, 0)), None);
        assert_eq!(parse_raw_header(&raw(5, 0, 0)), None);
    }

    #[test]
    fn a_dimension_product_that_would_overflow_is_rejected_not_wrapped() {
        // The file is machine-written but still untrusted: `w*h*4` must not
        // wrap into a small number that happens to match the real length.
        let mut buf = Vec::new();
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.resize(RAW_HEADER + 4, 0);
        assert_eq!(parse_raw_header(&buf), None);
    }

    #[test]
    fn transparent_pixels_are_premultiplied_for_the_blend() {
        // smithay's GLES shaders blend premultiplied colour: a straight
        // white, fully transparent pixel used to paint white.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alpha.png");
        let mut img = image::RgbaImage::new(3, 1);
        img.put_pixel(0, 0, image::Rgba([255, 255, 255, 0]));
        img.put_pixel(1, 0, image::Rgba([200, 100, 50, 128]));
        img.put_pixel(2, 0, image::Rgba([9, 8, 7, 255]));
        img.save(&path).expect("png");
        let decoded = decode_pixels(&path).expect("decode");
        assert_eq!(decoded.size, (3, 1));
        assert_eq!(
            decoded.rgba,
            [0, 0, 0, 0, 100, 50, 25, 128, 9, 8, 7, 255],
            "premultiplied, opaque pixels untouched"
        );
    }

    #[test]
    fn a_raw_file_is_checked_against_its_length_before_it_is_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("w.raw");
        std::fs::write(&path, raw(3, 5, 9)).expect("write");
        let ok = decode_raw(&path).expect("well-formed");
        assert_eq!((ok.size, ok.rgba.len()), ((3, 5), 3 * 5 * 4));
        let mut short = raw(3, 5, 9);
        short.pop();
        std::fs::write(&path, short).expect("write");
        assert!(decode_raw(&path).is_err(), "truncated");
        // A header claiming an enormous image is refused without
        // allocating its body.
        let mut huge = Vec::new();
        huge.extend_from_slice(&16385u32.to_le_bytes());
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.resize(RAW_HEADER + 16385 * 4, 0);
        std::fs::write(&path, huge).expect("write");
        assert!(decode_raw(&path).is_err(), "over the side limit");
        std::fs::write(&path, [1, 0]).expect("write");
        assert!(decode_raw(&path).is_err(), "no header");
    }

    #[test]
    fn only_a_raw_extension_takes_the_headed_path() {
        assert!(is_raw_backdrop(Path::new("/var/lib/mgreet/background.raw")));
        assert!(is_raw_backdrop(Path::new("/tmp/x.RAW")));
        assert!(!is_raw_backdrop(Path::new(
            "/usr/share/margo/wallpapers/default.jpg"
        )));
        assert!(!is_raw_backdrop(Path::new("/home/u/wall.png")));
    }
}

#[cfg(test)]
mod fit_tests {
    use super::fit_rects;
    use margo_config::WallpaperFit;

    #[test]
    fn cover_crops_the_wider_image_to_the_output_aspect() {
        let (src, loc, size) =
            fit_rects((4000.0, 1000.0), (1920.0, 1080.0), WallpaperFit::Cover).unwrap();
        assert!((src.2 / src.3 - 1920.0 / 1080.0).abs() < 1e-9);
        assert_eq!((loc, size), ((0.0, 0.0), (1920.0, 1080.0)));
    }

    #[test]
    fn contain_letterboxes_and_centres() {
        let (src, loc, size) =
            fit_rects((1000.0, 1000.0), (1920.0, 1080.0), WallpaperFit::Contain).unwrap();
        assert_eq!(src, (0.0, 0.0, 1000.0, 1000.0));
        assert_eq!(size, (1080.0, 1080.0));
        assert_eq!(loc, (420.0, 0.0));
    }

    #[test]
    fn fill_stretches_the_whole_image() {
        let (src, loc, size) =
            fit_rects((800.0, 600.0), (1920.0, 1080.0), WallpaperFit::Fill).unwrap();
        assert_eq!(
            (src, loc, size),
            ((0.0, 0.0, 800.0, 600.0), (0.0, 0.0), (1920.0, 1080.0))
        );
    }

    #[test]
    fn center_keeps_native_size_and_crops_overflow() {
        let (_, loc, size) =
            fit_rects((800.0, 600.0), (1920.0, 1080.0), WallpaperFit::Center).unwrap();
        assert_eq!((loc, size), ((560.0, 240.0), (800.0, 600.0)));
        let (src, loc, size) =
            fit_rects((4000.0, 2000.0), (1920.0, 1080.0), WallpaperFit::Center).unwrap();
        assert_eq!(src, (1040.0, 460.0, 1920.0, 1080.0));
        assert_eq!((loc, size), ((0.0, 0.0), (1920.0, 1080.0)));
    }

    #[test]
    fn scale_down_keeps_a_small_image_native_and_contains_a_large_one() {
        // GTK's `ContentFit::ScaleDown`: never upscaled...
        assert_eq!(
            fit_rects((800.0, 600.0), (1920.0, 1080.0), WallpaperFit::ScaleDown),
            fit_rects((800.0, 600.0), (1920.0, 1080.0), WallpaperFit::Center),
        );
        // ...exactly fitting is still native...
        assert_eq!(
            fit_rects((1920.0, 1080.0), (1920.0, 1080.0), WallpaperFit::ScaleDown),
            Some(((0.0, 0.0, 1920.0, 1080.0), (0.0, 0.0), (1920.0, 1080.0))),
        );
        // ...and one larger on either axis is letterboxed like `Contain`.
        for img in [(4000.0, 2000.0), (1000.0, 1200.0)] {
            assert_eq!(
                fit_rects(img, (1920.0, 1080.0), WallpaperFit::ScaleDown),
                fit_rects(img, (1920.0, 1080.0), WallpaperFit::Contain),
            );
        }
    }
}
