#![allow(dead_code)]
//! Software cursor loading and rendering.

use std::io::Read;

use drm_fourcc::DrmFourcc;
use smithay::{
    backend::renderer::{
        element::{
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            Kind,
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

pub struct CursorManager {
    image: CursorImage,
    buffer: MemoryRenderBuffer,
}

impl CursorManager {
    pub fn new() -> Self {
        let image = load_from_theme("default", 24).unwrap_or_else(embedded_arrow);
        let buffer = image.to_memory_buffer();
        Self { image, buffer }
    }

    pub fn hotspot(&self) -> Point<i32, Physical> {
        Point::from((self.image.hotspot_x, self.image.hotspot_y))
    }

    pub fn render_element(
        &self,
        renderer: &mut GlesRenderer,
        pos: Point<f64, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let hs = self.hotspot();
        let offset = Point::<f64, Logical>::from((-(hs.x as f64 / scale), -(hs.y as f64 / scale)));
        let adjusted = pos + offset;
        let render_pos = Point::<f64, Physical>::from((adjusted.x * scale, adjusted.y * scale));
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            render_pos,
            &self.buffer,
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
    std::fs::File::open(&path).ok()?.read_to_end(&mut data).ok()?;
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

    CursorImage { pixels, width: 16, height: 16, hotspot_x: 0, hotspot_y: 0 }
}
