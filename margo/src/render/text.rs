//! Compositor-side text rendering for tabbed-group tab labels.
//!
//! margo has no font stack (no pango/cairo/freetype) — every other label
//! in the desktop is drawn by mshell (GTK). The tab strip lives in the
//! compositor, though, so to put an app name on each chip we rasterise it
//! here with [`fontdue`] (pure Rust, no system font lib) into an RGBA
//! bitmap and wrap it in a Smithay [`MemoryRenderBuffer`] — the exact same
//! upload path `wallpaper.rs` / `cursor.rs` already use, so positioning and
//! GL upload are handled by Smithay rather than raw GL.
//!
//! Rasterised buffers are cached per `(text, height, colour, max-width)` in
//! a thread-local map: the render thread re-requests the same labels every
//! frame, and Smithay caches the texture upload as long as the same
//! `MemoryRenderBuffer` instance is reused.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use drm_fourcc::DrmFourcc;
use fontdue::{Font, FontSettings};
use smithay::backend::renderer::element::{
    Kind,
    memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Buffer, Physical, Point, Size, Transform};
use tracing::{info, warn};

/// Loaded once, lazily. `None` means no usable font was found on disk —
/// labels are then silently skipped (the chips still render).
static FONT: OnceLock<Option<Font>> = OnceLock::new();

thread_local! {
    /// `(height_px, packed_rgb, max_width_px)` → (`text` → rasterised
    /// buffer; `None` when the text rasterised to nothing). Reused across
    /// frames so Smithay's texture upload stays cached. The text is the
    /// *inner* key so the common cache-hit lookup can borrow it as `&str`
    /// rather than allocating a key `String` every frame.
    static LABEL_CACHE: RefCell<
        HashMap<(i32, u32, i32), HashMap<String, Option<MemoryRenderBuffer>>>,
    > = RefCell::new(HashMap::new());
}

fn font() -> Option<&'static Font> {
    FONT.get_or_init(load_font).as_ref()
}

fn load_font() -> Option<Font> {
    // Common regular sans faces across distros (Arch/Cachy paths first).
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/TTF/Hack-Regular.ttf",
        "/usr/share/fonts/cantarell/Cantarell-Regular.otf",
        "/usr/share/fonts/ttf-dejavu/DejaVuSans.ttf",
    ];
    for p in CANDIDATES {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(f) = Font::from_bytes(bytes, FontSettings::default()) {
                info!(path = p, "group tabs: loaded font for tab labels");
                return Some(f);
            }
        }
    }
    // Last resort: first .ttf/.otf anywhere under /usr/share/fonts.
    if let Some((path, bytes)) = find_any_font("/usr/share/fonts".as_ref(), 0) {
        if let Ok(f) = Font::from_bytes(bytes, FontSettings::default()) {
            info!(path = %path.display(), "group tabs: loaded fallback font for tab labels");
            return Some(f);
        }
    }
    warn!("group tabs: no usable font found under /usr/share/fonts — tab labels disabled");
    None
}

/// Bounded recursive search for the first usable font file.
fn find_any_font(dir: &std::path::Path, depth: u32) -> Option<(std::path::PathBuf, Vec<u8>)> {
    if depth > 6 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            subdirs.push(path);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("ttf") | Some("otf")) {
            if let Ok(bytes) = std::fs::read(&path) {
                return Some((path, bytes));
            }
        }
    }
    for sub in subdirs {
        if let Some(found) = find_any_font(&sub, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// Build a render element for `text` at `pos` (physical), rasterised at
/// `height_px` tall and truncated with an ellipsis to fit `max_width_px`.
/// `rgb` is the text colour (the bitmap is premultiplied-alpha so it
/// blends correctly over the chip). Returns `None` if there's no font, the
/// text is empty, or it rasterises to nothing.
pub fn label_element(
    renderer: &mut GlesRenderer,
    text: &str,
    height_px: i32,
    max_width_px: i32,
    rgb: [u8; 3],
    pos: Point<f64, Physical>,
) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
    if text.is_empty() || height_px <= 2 || max_width_px <= 2 {
        return None;
    }
    let font = font()?;
    let packed = ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32;
    let style_key = (height_px, packed, max_width_px);

    LABEL_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        // Window titles change often (browser tabs etc.); cap the total
        // cached buffers so stale entries can't accumulate unbounded over
        // a long session.
        let total: usize = cache.values().map(HashMap::len).sum();
        if total > 512 {
            cache.clear();
        }
        let inner = cache.entry(style_key).or_default();
        // Hit path borrows `text` as `&str` — we only allocate a key
        // `String` when actually inserting a freshly-rasterised buffer.
        if !inner.contains_key(text) {
            let buf = build_buffer(font, text, height_px, max_width_px, rgb);
            inner.insert(text.to_string(), buf);
        }
        let buf = inner.get(text)?.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            pos,
            buf,
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    })
}

/// Rasterise `text` into a premultiplied-alpha RGBA buffer.
fn build_buffer(
    font: &Font,
    text: &str,
    height_px: i32,
    max_width_px: i32,
    rgb: [u8; 3],
) -> Option<MemoryRenderBuffer> {
    let px = height_px as f32;
    let lm = font.horizontal_line_metrics(px)?;
    let ascent = lm.ascent;
    // descent is negative in fontdue; line height = ascent - descent.
    let img_h = (lm.ascent - lm.descent).ceil().max(1.0) as usize;

    let fitted = fit_text(font, text, px, max_width_px as f32);
    if fitted.is_empty() {
        return None;
    }

    let total_w: f32 = fitted
        .chars()
        .map(|ch| font.metrics(ch, px).advance_width)
        .sum();
    let img_w = total_w.ceil().max(1.0) as usize;
    if img_w == 0 || img_h == 0 {
        return None;
    }

    let mut rgba = vec![0u8; img_w * img_h * 4];
    let mut pen_x = 0.0f32;
    for ch in fitted.chars() {
        let (m, bitmap) = font.rasterize(ch, px);
        let gx0 = (pen_x + m.xmin as f32).round() as i32;
        // Glyph bitmap top, measured down from the image top: baseline is
        // at `ascent`; the bitmap's top sits `height + ymin` above the
        // baseline (ymin can be negative for descenders).
        let top = (ascent - (m.height as f32 + m.ymin as f32)).round() as i32;
        for gy in 0..m.height {
            let iy = top + gy as i32;
            if iy < 0 || iy as usize >= img_h {
                continue;
            }
            for gxp in 0..m.width {
                let ix = gx0 + gxp as i32;
                if ix < 0 || ix as usize >= img_w {
                    continue;
                }
                let cov = bitmap[gy * m.width + gxp] as u32;
                if cov == 0 {
                    continue;
                }
                let idx = (iy as usize * img_w + ix as usize) * 4;
                // Premultiplied alpha, RGBA byte order (Abgr8888 LE).
                rgba[idx] = ((rgb[0] as u32 * cov) / 255) as u8;
                rgba[idx + 1] = ((rgb[1] as u32 * cov) / 255) as u8;
                rgba[idx + 2] = ((rgb[2] as u32 * cov) / 255) as u8;
                rgba[idx + 3] = cov as u8;
            }
        }
        pen_x += m.advance_width;
    }

    Some(MemoryRenderBuffer::from_slice(
        &rgba,
        DrmFourcc::Abgr8888,
        Size::<i32, Buffer>::from((img_w as i32, img_h as i32)),
        1,
        Transform::Normal,
        None,
    ))
}

/// Truncate `text` with a trailing ellipsis so its advance width fits
/// `max_w`. Returns the full string when it already fits.
fn fit_text(font: &Font, text: &str, px: f32, max_w: f32) -> String {
    let width_of = |s: &str| -> f32 { s.chars().map(|c| font.metrics(c, px).advance_width).sum() };
    if width_of(text) <= max_w {
        return text.to_string();
    }
    let ell_w = font.metrics('…', px).advance_width;
    let mut out = String::new();
    let mut w = ell_w;
    for ch in text.chars() {
        let cw = font.metrics(ch, px).advance_width;
        if w + cw > max_w {
            break;
        }
        out.push(ch);
        w += cw;
    }
    if out.is_empty() {
        return String::new();
    }
    out.push('…');
    out
}
