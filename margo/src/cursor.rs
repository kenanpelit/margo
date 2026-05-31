#![allow(dead_code)]
//! Software cursor loading and rendering.

use std::collections::HashMap;
use std::io::Read;

use drm_fourcc::DrmFourcc;
use smithay::{
    backend::renderer::{
        element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
        },
        gles::GlesRenderer,
    },
    utils::{Buffer, Logical, Physical, Point, Size, Transform},
};

// ── Cursor image data (backend-independent) ────────────────────────────────────

pub struct CursorImage {
    /// BGRA pixel data (Argb8888 little-endian format)
    pub pixels: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
}

impl CursorImage {
    pub fn to_memory_buffer(&self) -> MemoryRenderBuffer {
        MemoryRenderBuffer::from_slice(
            &self.pixels,
            DrmFourcc::Argb8888,
            Size::<i32, Buffer>::from((self.width, self.height)),
            1,
            Transform::Normal,
            None,
        )
    }
}

// ── CursorManager ─────────────────────────────────────────────────────────────

/// Renders the named cursor the focused client last requested (via
/// `wp_cursor_shape_v1` or `wl_pointer.set_cursor` → `CursorImageStatus::
/// Named`). Themed XCursor images are loaded lazily by name and cached, so
/// e.g. a browser asking for the `pointer` (hand) over a link actually
/// changes the on-screen cursor — previously only the fixed `default` arrow
/// was ever drawn.
pub struct CursorManager {
    /// Name → (image, GPU buffer), keyed by the requested cursor name.
    /// "default" is always present (theme or embedded fallback).
    cache: HashMap<String, (CursorImage, MemoryRenderBuffer)>,
    /// The name currently being drawn.
    current: String,
    /// Nominal cursor size in px (closest theme image is picked).
    size: u32,
}

impl CursorManager {
    pub fn new() -> Self {
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(24);
        let image = load_from_theme("default", size).unwrap_or_else(embedded_arrow);
        let buffer = image.to_memory_buffer();
        let mut cache = HashMap::new();
        cache.insert("default".to_string(), (image, buffer));
        Self {
            cache,
            current: "default".to_string(),
            size,
        }
    }

    /// Switch to the cursor named `primary` (with `alts` as theme fallbacks,
    /// e.g. `pointer` → `hand2`/`pointing_hand`). Loads + caches on first use;
    /// keeps the previous cursor (falling back to `default`) if no variant
    /// resolves in the active theme.
    pub fn set_named(&mut self, primary: &str, alts: &[&str]) {
        if self.current == primary && self.cache.contains_key(primary) {
            return;
        }
        if !self.cache.contains_key(primary)
            && let Some(image) = std::iter::once(primary)
                .chain(alts.iter().copied())
                .find_map(|name| load_from_theme(name, self.size))
        {
            let buffer = image.to_memory_buffer();
            self.cache.insert(primary.to_string(), (image, buffer));
        }
        self.current = if self.cache.contains_key(primary) {
            primary.to_string()
        } else {
            "default".to_string()
        };
    }

    fn current_entry(&self) -> &(CursorImage, MemoryRenderBuffer) {
        self.cache
            .get(self.current.as_str())
            .or_else(|| self.cache.get("default"))
            .expect("default cursor is always cached")
    }

    pub fn hotspot(&self) -> Point<i32, Physical> {
        let (image, _) = self.current_entry();
        Point::from((image.hotspot_x, image.hotspot_y))
    }

    pub fn render_element(
        &self,
        renderer: &mut GlesRenderer,
        pos: Point<f64, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let (_, buffer) = self.current_entry();
        let hs = self.hotspot();
        let offset = Point::<f64, Logical>::from((-(hs.x as f64 / scale), -(hs.y as f64 / scale)));
        let adjusted = pos + offset;
        let render_pos = Point::<f64, Physical>::from((adjusted.x * scale, adjusted.y * scale));
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            render_pos,
            buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )
        .ok()
    }
}

// ── xcursor theme loading ─────────────────────────────────────────────────────

fn load_from_theme(cursor_name: &str, size: u32) -> Option<CursorImage> {
    let theme_name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
    let theme = xcursor::CursorTheme::load(&theme_name);
    let path = theme.load_icon(cursor_name)?;
    let mut data = Vec::new();
    std::fs::File::open(&path)
        .ok()?
        .read_to_end(&mut data)
        .ok()?;
    let images = xcursor::parser::parse_xcursor(&data)?;

    // Pick the image closest to the requested size
    let img = images
        .iter()
        .min_by_key(|i| (i.size as i64 - size as i64).unsigned_abs())?;

    // pixels_rgba is [R, G, B, A] per pixel (4 bytes each)
    // Argb8888 little-endian memory layout is [B, G, R, A]
    let pixels: Vec<u8> = img
        .pixels_rgba
        .chunks_exact(4)
        .flat_map(|c| [c[2], c[1], c[0], c[3]])
        .collect();

    Some(CursorImage {
        pixels,
        width: img.width as i32,
        height: img.height as i32,
        hotspot_x: img.xhot as i32,
        hotspot_y: img.yhot as i32,
    })
}

// ── Embedded fallback: 16×16 white arrow ─────────────────────────────────────

fn embedded_arrow() -> CursorImage {
    #[rustfmt::skip]
    let mask: [u8; 256] = [
        1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,
        1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,
        1,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,
        1,1,1,1,1,1,1,0,0,0,0,0,0,0,0,0,
        1,1,1,1,1,1,1,1,0,0,0,0,0,0,0,0,
        1,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,
        1,1,1,0,1,1,0,0,0,0,0,0,0,0,0,0,
        1,1,0,0,0,1,1,0,0,0,0,0,0,0,0,0,
        1,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    ];

    let pixels: Vec<u8> = mask
        .iter()
        .flat_map(|&p| {
            if p == 1 {
                [255u8, 255, 255, 255] // BGRA: white opaque
            } else {
                [0u8, 0, 0, 0]
            }
        })
        .collect();

    CursorImage {
        pixels,
        width: 16,
        height: 16,
        hotspot_x: 0,
        hotspot_y: 0,
    }
}
