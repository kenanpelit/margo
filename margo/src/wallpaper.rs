//! Compositor-side wallpaper renderer.
//!
//! Decodes the configured wallpaper once at startup (or on config
//! reload), uploads it into a Smithay `MemoryRenderBuffer`, and exposes
//! a `render_element()` helper that fits the bitmap into a given
//! output rectangle. The render loop in `backend/udev/frame.rs` pushes
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
//! Fit modes: only `Cover` is currently wired through the element
//! builder. The other variants are parsed by the config crate so a
//! config line that picks them doesn't fail validation; they will
//! engage once `render_element()` grows the corresponding code paths.

use std::path::{Path, PathBuf};

use drm_fourcc::DrmFourcc;
use smithay::{
    backend::renderer::{
        element::{
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            Kind,
        },
        gles::GlesRenderer,
    },
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};
use tracing::{info, warn};

const FALLBACK_RELATIVE_USER: &str = ".local/share/margo/wallpapers/default.jpg";
const FALLBACK_SYSTEM: &str = "/usr/share/margo/wallpapers/default.jpg";

pub struct WallpaperState {
    /// The decoded source — kept around so we can rebuild the buffer
    /// if the renderer ever needs different byte orderings or scales.
    /// Drop it once stable to save ~24 MB on a 4K image.
    #[allow(dead_code)]
    natural_size: (u32, u32),
    /// Whatever buffer we hand to `MemoryRenderBufferRenderElement`.
    /// One global buffer is reused across every output — Smithay's
    /// element handles per-output scaling via the `dst_size` arg below.
    buffer: MemoryRenderBuffer,
}

impl WallpaperState {
    /// Resolve, decode, and upload the wallpaper. Returns `None` when
    /// no candidate path is readable or the decode fails — callers
    /// fall through to the compositor's solid `rootcolor` clear.
    pub fn load(explicit: Option<&str>) -> Option<Self> {
        let path = resolve_path(explicit)?;
        match decode_to_buffer(&path) {
            Ok((buffer, size)) => {
                info!(
                    path = %path.display(),
                    width = size.0,
                    height = size.1,
                    "wallpaper: loaded"
                );
                Some(Self {
                    natural_size: size,
                    buffer,
                })
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "wallpaper: decode failed");
                None
            }
        }
    }

    /// Build a render element sized to fit `output_geom` (logical
    /// coordinates, scaled to `output_scale`). Currently implements
    /// `Cover` fit only: the image is upscaled until both output
    /// dimensions are filled, then a centred sub-region is sampled
    /// so the visible area is exactly the output rectangle.
    pub fn render_element(
        &self,
        renderer: &mut GlesRenderer,
        output_loc: Point<f64, Logical>,
        output_size: Size<i32, Logical>,
        output_scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let (img_w, img_h) = (self.natural_size.0 as f64, self.natural_size.1 as f64);
        if img_w <= 0.0 || img_h <= 0.0 {
            return None;
        }

        // Cover-fit src rect: choose the cropped sub-rectangle of the
        // source image whose aspect ratio matches the output, then let
        // Smithay scale it into the dst region.
        let out_w = output_size.w as f64;
        let out_h = output_size.h as f64;
        let src_aspect = img_w / img_h;
        let out_aspect = out_w / out_h;
        let (src_w, src_h) = if src_aspect > out_aspect {
            // Source wider than output → crop horizontally.
            (img_h * out_aspect, img_h)
        } else {
            // Source taller than output → crop vertically.
            (img_w, img_w / out_aspect)
        };
        let src_x = (img_w - src_w) / 2.0;
        let src_y = (img_h - src_h) / 2.0;
        // Smithay's `from_buffer` takes the sample region and the
        // destination size in *logical* units — it treats the buffer
        // dimensions as logical 1:1, and resolves physical scaling
        // internally via the renderer's output scale. We can stay in
        // logical coords throughout.
        let src = Rectangle::<f64, Logical>::new(
            Point::from((src_x, src_y)),
            Size::from((src_w, src_h)),
        );
        let dst_size = Size::<i32, Logical>::from((output_size.w, output_size.h));
        let render_pos = Point::<f64, Physical>::from((
            output_loc.x * output_scale,
            output_loc.y * output_scale,
        ));

        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            render_pos,
            &self.buffer,
            None,
            Some(src),
            Some(dst_size),
            Kind::Unspecified,
        )
        .ok()
    }
}

fn resolve_path(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
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

fn decode_to_buffer(path: &Path) -> Result<(MemoryRenderBuffer, (u32, u32)), image::ImageError> {
    let img = image::open(path)?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());

    // Smithay's MemoryRenderBuffer with `DrmFourcc::Abgr8888` expects
    // RGBA byte order in little-endian (R first byte, A last byte) —
    // which is exactly what `image::RgbaImage` gives us. No swizzle
    // needed. (Cursor uses Argb8888 because libxcursor outputs that
    // ordering; we deliberately diverge here.)
    let buffer = MemoryRenderBuffer::from_slice(
        &rgba.into_raw(),
        DrmFourcc::Abgr8888,
        Size::<i32, Buffer>::from((w as i32, h as i32)),
        1,
        Transform::Normal,
        None,
    );
    Ok((buffer, (w, h)))
}

fn exists_and_readable(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
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
