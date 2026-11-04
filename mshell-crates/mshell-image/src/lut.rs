use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use image::{ImageBuffer, Rgb, RgbImage, RgbaImage};
use lutgen::GenerateLut;
use lutgen::identity::{correct_image, correct_pixel};
use lutgen::interpolation::GaussianRemapper;
use mshell_config::schema::themes::Themes;
use mshell_matugen::json_struct::MatugenTheme;
use mshell_matugen::static_theme_mapping::static_theme;
use relm4::gtk;
use relm4::gtk::gdk_pixbuf::{Pixbuf, PixbufLoader};
use relm4::gtk::prelude::{
    Cast, GskRendererExt, PixbufLoaderExt, SnapshotExt, TextureExt, TextureExtManual,
};
use tracing::warn;

/// Every static theme that owns a CLUT, paired with its stable file/cache
/// basename. `Wallpaper` is absent (it grades from the live wallpaper) and
/// `Default` is absent as its own entry — it shares Kenp's CLUT.
pub const CLUT_THEMES: &[(&str, Themes)] = &[
    ("catppuccin_mocha", Themes::CatppuccinMocha),
    ("everforest_dark_medium", Themes::EverforestDarkMedium),
    ("gruvbox_material", Themes::GruvboxMaterial),
    ("horizon", Themes::Horizon),
    ("iznik", Themes::Iznik),
    ("kanagawa_wave", Themes::KanagawaWave),
    ("kenp", Themes::Kenp),
    ("one_dark", Themes::OneDark),
    ("margo", Themes::Margo),
    ("rose_pine", Themes::RosePine),
    ("tokyo_night", Themes::TokyoNight),
];

/// Stable CLUT basename for a theme, or `None` for `Wallpaper` (which has no
/// static CLUT). `Default` aliases the house theme Kenp and shares its CLUT.
/// This is the cheap predicate for "does this theme support recolouring" — it
/// never triggers CLUT generation.
pub fn clut_name(theme: &Themes) -> Option<&'static str> {
    match theme {
        Themes::Wallpaper => None,
        Themes::Default => Some("kenp"),
        _ => CLUT_THEMES
            .iter()
            .find(|(_, t)| t == theme)
            .map(|(name, _)| *name),
    }
}

// Gaussian remapper parameters. These must stay fixed: a CLUT generated with
// different values would grade images differently from theme to theme and
// invalidate any cache written by an older build.
const GAUSSIAN_SHAPE: f64 = 96.0;
const GAUSSIAN_NEAREST: usize = 0;
const LUM_FACTOR: f64 = 1.0;
const PRESERVE: bool = false;

/// The 68 Material + base16 swatches that seed a theme's Gaussian remap.
fn extract_palette(theme: &MatugenTheme) -> Vec<[u8; 3]> {
    let c = &theme.colors;
    let b = &theme.base16;

    let colors = vec![
        c.background.default.as_rgb(),
        c.error.default.as_rgb(),
        c.error_container.default.as_rgb(),
        c.inverse_on_surface.default.as_rgb(),
        c.inverse_primary.default.as_rgb(),
        c.inverse_surface.default.as_rgb(),
        c.on_background.default.as_rgb(),
        c.on_error.default.as_rgb(),
        c.on_error_container.default.as_rgb(),
        c.on_primary.default.as_rgb(),
        c.on_primary_container.default.as_rgb(),
        c.on_primary_fixed.default.as_rgb(),
        c.on_primary_fixed_variant.default.as_rgb(),
        c.on_secondary.default.as_rgb(),
        c.on_secondary_container.default.as_rgb(),
        c.on_secondary_fixed.default.as_rgb(),
        c.on_secondary_fixed_variant.default.as_rgb(),
        c.on_surface.default.as_rgb(),
        c.on_surface_variant.default.as_rgb(),
        c.on_tertiary.default.as_rgb(),
        c.on_tertiary_container.default.as_rgb(),
        c.on_tertiary_fixed.default.as_rgb(),
        c.on_tertiary_fixed_variant.default.as_rgb(),
        c.outline.default.as_rgb(),
        c.outline_variant.default.as_rgb(),
        c.primary.default.as_rgb(),
        c.primary_container.default.as_rgb(),
        c.primary_fixed.default.as_rgb(),
        c.primary_fixed_dim.default.as_rgb(),
        c.scrim.default.as_rgb(),
        c.secondary.default.as_rgb(),
        c.secondary_container.default.as_rgb(),
        c.secondary_fixed.default.as_rgb(),
        c.secondary_fixed_dim.default.as_rgb(),
        c.shadow.default.as_rgb(),
        c.source_color.default.as_rgb(),
        c.surface.default.as_rgb(),
        c.surface_bright.default.as_rgb(),
        c.surface_container.default.as_rgb(),
        c.surface_container_high.default.as_rgb(),
        c.surface_container_highest.default.as_rgb(),
        c.surface_container_low.default.as_rgb(),
        c.surface_container_lowest.default.as_rgb(),
        c.surface_dim.default.as_rgb(),
        c.surface_tint.default.as_rgb(),
        c.surface_variant.default.as_rgb(),
        c.tertiary.default.as_rgb(),
        c.tertiary_container.default.as_rgb(),
        c.tertiary_fixed.default.as_rgb(),
        c.tertiary_fixed_dim.default.as_rgb(),
        b.base00.default.as_rgb(),
        b.base01.default.as_rgb(),
        b.base02.default.as_rgb(),
        b.base03.default.as_rgb(),
        b.base04.default.as_rgb(),
        b.base05.default.as_rgb(),
        b.base06.default.as_rgb(),
        b.base07.default.as_rgb(),
        b.base08.default.as_rgb(),
        b.base09.default.as_rgb(),
        b.base0a.default.as_rgb(),
        b.base0b.default.as_rgb(),
        b.base0c.default.as_rgb(),
        b.base0d.default.as_rgb(),
        b.base0e.default.as_rgb(),
        b.base0f.default.as_rgb(),
    ];

    colors.into_iter().map(|(r, g, b)| [r, g, b]).collect()
}

/// Generate a theme's level-8 Hald CLUT by remapping the identity LUT onto its
/// static Material palette. `None` for `Wallpaper` (dynamic) or a theme whose
/// static palette is missing. Deterministic — the same theme always yields the
/// same bytes, so a cached CLUT stays valid.
pub fn generate_clut(theme: &Themes) -> Option<Vec<u8>> {
    clut_name(theme)?;
    let matugen = static_theme(theme, None)?;
    let palette = extract_palette(&matugen);
    let remapper = GaussianRemapper::new(
        &palette,
        GAUSSIAN_SHAPE,
        GAUSSIAN_NEAREST,
        LUM_FACTOR,
        PRESERVE,
    );
    Some(remapper.par_generate_lut(HALD_LEVEL).into_raw())
}

/// CLUT cache directory: `$XDG_CACHE_HOME/mshell/cluts`, falling back to
/// `$HOME/.cache/mshell/cluts`. `None` when neither is set.
fn clut_cache_dir() -> Option<PathBuf> {
    // design-lint: allow-xdg (testable seam; resolves via margo_fs::xdg::resolve)
    clut_cache_dir_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

/// [`clut_cache_dir`] for explicit `XDG_CACHE_HOME` / `HOME` values.
fn clut_cache_dir_from(
    xdg_cache_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    margo_fs::xdg::resolve(xdg_cache_home, home, ".cache").map(|d| d.join("mshell").join("cluts"))
}

// Unique suffix source for cache temp files so concurrent writers never share
// a temp path.
static CACHE_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn write_clut_cache(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "cache path has no parent")
    })?;
    fs::create_dir_all(dir)?;
    let seq = CACHE_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".clut.{}.{seq}.tmp", std::process::id()));
    // Write to a temp file and rename so a reader never sees a partial CLUT.
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Load a theme's Hald CLUT, generating and caching it on first use.
///
/// Returns `None` for `Wallpaper` or when the CLUT cannot be produced —
/// callers treat `None` as "skip the theme filter", never a hard error. The
/// first use of a theme costs one generation (~20 ms); every later use is a
/// cache read.
pub fn load_clut(theme: &Themes) -> Option<Vec<u8>> {
    load_clut_in(clut_cache_dir().as_deref(), theme)
}

/// [`load_clut`] with the cache in `cache_dir` (`None` = don't cache).
fn load_clut_in(cache_dir: Option<&Path>, theme: &Themes) -> Option<Vec<u8>> {
    let name = clut_name(theme)?;
    let cached = cache_dir.map(|d| d.join(format!("{name}.bin")));

    if let Some(path) = &cached {
        match fs::read(path) {
            Ok(bytes) if bytes.len() == CLUT_BYTE_LEN => return Some(bytes),
            Ok(bytes) => warn!(
                "regenerating {name} CLUT: cache {} is {} bytes, expected {CLUT_BYTE_LEN}",
                path.display(),
                bytes.len()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("reading CLUT cache {}: {e}", path.display()),
        }
    }

    let bytes = generate_clut(theme)?;
    if let Some(path) = &cached
        && let Err(e) = write_clut_cache(path, &bytes)
    {
        warn!("caching CLUT {}: {e}", path.display());
    }
    Some(bytes)
}

/// Hald CLUT level used for all CLUTs.
pub const HALD_LEVEL: u8 = 8;

/// Image dimensions for a level-8 Hald CLUT (8^3 = 512).
const HALD8_DIM: u32 = 512;

/// Raw byte length of a level-8 Hald CLUT (512×512 RGB).
pub const CLUT_BYTE_LEN: usize = (HALD8_DIM * HALD8_DIM * 3) as usize;

/// Result of a successful palette remap operation.
pub struct RemapResult {
    pub buf: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Apply a theme's Hald CLUT to an image file.
///
/// The CLUT is generated on first use and cached (see [`load_clut`]).
/// Returns `None` if:
/// - The theme has no CLUT (Wallpaper), or it could not be generated
/// - The image cannot be opened
/// - The operation was cancelled
pub fn apply_theme_filter(
    path: &Path,
    theme: &Themes,
    strength: f64,
    contrast_adjustment: f64,
    monochrome: f64,
    cancel: &AtomicBool,
) -> Option<RemapResult> {
    let hald_clut = load_hald_clut(theme)?;

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    let img = decode_pixbuf_rgba(path)?;
    let img = filter_with_clut(
        img,
        &hald_clut,
        theme,
        strength,
        contrast_adjustment,
        monochrome,
        cancel,
    )?;
    let (width, height) = img.dimensions();
    Some(RemapResult {
        buf: img.into_raw(),
        width,
        height,
    })
}

/// [`apply_theme_filter`] for an already-decoded image (e.g. one decoded at
/// screen size with [`decode_rgba_scaled`]). Grades `img` in place and hands it
/// back; `None` when the theme has no CLUT or `cancel` was raised mid-way.
pub fn apply_theme_filter_rgba(
    img: RgbaImage,
    theme: &Themes,
    strength: f64,
    contrast_adjustment: f64,
    monochrome: f64,
    cancel: &AtomicBool,
) -> Option<RgbaImage> {
    let hald_clut = load_hald_clut(theme)?;
    filter_with_clut(
        img,
        &hald_clut,
        theme,
        strength,
        contrast_adjustment,
        monochrome,
        cancel,
    )
}

fn load_hald_clut(theme: &Themes) -> Option<RgbImage> {
    ImageBuffer::<Rgb<u8>, _>::from_raw(HALD8_DIM, HALD8_DIM, load_clut(theme)?)
}

fn filter_with_clut(
    mut img: RgbaImage,
    hald_clut: &RgbImage,
    theme: &Themes,
    strength: f64,
    contrast_adjustment: f64,
    monochrome: f64,
    cancel: &AtomicBool,
) -> Option<RgbaImage> {
    if monochrome > 0.0 {
        let tint = static_theme(theme, None)
            .map(|t| {
                let rgb = t.colors.on_surface.default.as_rgb();
                [rgb.0, rgb.1, rgb.2]
            })
            .unwrap_or([255, 255, 255]);
        apply_monochrome(img.as_mut(), tint, monochrome);
    }

    grade_in_place(
        img.as_mut(),
        hald_clut,
        strength,
        contrast_adjustment,
        cancel,
    )
    .then_some(img)
}

/// Pixels graded between two cancel checks (~1 ms of work).
const GRADE_CHUNK_PIXELS: usize = 64 * 1024;

/// CLUT-correct, contrast-adjust and blend an RGBA buffer back toward its
/// original colours by `strength`, all in one pass per pixel. Fusing the
/// stages means a partial-strength grade no longer needs a full copy of the
/// image to blend against (a 4K wallpaper is 33 MB of RGBA). Alpha is left
/// untouched. Returns `false` when `cancel` was raised part-way (the buffer is
/// then partly graded and should be thrown away).
fn grade_in_place(
    buf: &mut [u8],
    hald_clut: &RgbImage,
    strength: f64,
    contrast: f64,
    cancel: &AtomicBool,
) -> bool {
    let blend = strength < 1.0;
    for chunk in buf.chunks_mut(GRADE_CHUNK_PIXELS * 4) {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        for px in chunk.chunks_exact_mut(4) {
            let original = [px[0], px[1], px[2]];
            let graded = correct_pixel(&original, hald_clut, HALD_LEVEL);
            for i in 0..3 {
                let v = contrast_channel(graded[i], contrast);
                px[i] = if blend {
                    lerp_u8(original[i], v, strength)
                } else {
                    v
                };
            }
        }
    }
    !cancel.load(Ordering::Relaxed)
}

/// Decode an image file to RGBA using gdk-pixbuf.
pub fn decode_pixbuf_rgba(path: &Path) -> Option<RgbaImage> {
    let pixbuf = Pixbuf::from_file(path).ok()?;
    pixbuf_to_rgba(&pixbuf)
}

/// Bytes fed to the pixbuf loader between two cancel checks.
const DECODE_CHUNK_BYTES: usize = 256 * 1024;

/// Decode an image file to RGBA at the size `size_for(source_w, source_h)`
/// returns. The loader is told the target size before it decodes, so JPEGs
/// are scaled during the DCT and the full-resolution image never exists in
/// memory. `cancel` is checked between file chunks; `None` on cancel or
/// decode failure.
pub fn decode_rgba_scaled(
    path: &Path,
    size_for: impl Fn(u32, u32) -> (u32, u32) + 'static,
    cancel: &AtomicBool,
) -> Option<RgbaImage> {
    let mut file = fs::File::open(path).ok()?;
    let loader = PixbufLoader::new();
    loader.connect_size_prepared(move |loader, w, h| {
        let (src_w, src_h) = (w.max(0) as u32, h.max(0) as u32);
        let (tw, th) = size_for(src_w, src_h);
        if (tw, th) != (src_w, src_h) && tw > 0 && th > 0 {
            loader.set_size(tw as i32, th as i32);
        }
    });

    let mut chunk = vec![0u8; DECODE_CHUNK_BYTES];
    let fed = loop {
        if cancel.load(Ordering::Relaxed) {
            break false;
        }
        match file.read(&mut chunk) {
            Ok(0) => break true,
            Ok(n) => {
                if loader.write(&chunk[..n]).is_err() {
                    break false;
                }
            }
            Err(_) => break false,
        }
    };
    // Always close: a loader finalized open logs a GLib critical.
    let closed = loader.close().is_ok();
    if !fed || !closed || cancel.load(Ordering::Relaxed) {
        return None;
    }
    pixbuf_to_rgba(&loader.pixbuf()?)
}

fn pixbuf_to_rgba(pixbuf: &Pixbuf) -> Option<RgbaImage> {
    let width = pixbuf.width() as u32;
    let height = pixbuf.height() as u32;
    let n_channels = pixbuf.n_channels() as u32;
    let rowstride = pixbuf.rowstride() as u32;
    let has_alpha = pixbuf.has_alpha();
    let pixels = unsafe { pixbuf.pixels() };

    let mut rgba_buf = vec![0u8; (width * height * 4) as usize];

    for y in 0..height {
        let row_offset = (y * rowstride) as usize;
        for x in 0..width {
            let src = row_offset + (x * n_channels) as usize;
            let dst = ((y * width + x) * 4) as usize;

            rgba_buf[dst] = pixels[src]; // R
            rgba_buf[dst + 1] = pixels[src + 1]; // G
            rgba_buf[dst + 2] = pixels[src + 2]; // B
            rgba_buf[dst + 3] = if has_alpha { pixels[src + 3] } else { 255 };
        }
    }

    RgbaImage::from_raw(width, height, rgba_buf)
}

/// Average Rec. 709 luma of an image, normalized to `0.0..=1.0`
/// (0 = black, 1 = white). Used to auto-derive a light/dark Material You
/// polarity from the wallpaper. The image is decoded at a small fixed size
/// (a full average doesn't need the native resolution, and this avoids
/// decoding a 4K wallpaper on the main thread). `None` on decode failure.
pub fn average_luminance(path: &Path) -> Option<f64> {
    let pixbuf = Pixbuf::from_file_at_scale(path, 64, 64, true).ok()?;
    let width = pixbuf.width() as usize;
    let height = pixbuf.height() as usize;
    if width == 0 || height == 0 {
        return None;
    }
    let n_channels = pixbuf.n_channels() as usize;
    let rowstride = pixbuf.rowstride() as usize;
    let pixels = unsafe { pixbuf.pixels() };

    let mut sum = 0.0f64;
    let mut count = 0u64;
    for y in 0..height {
        let row = y * rowstride;
        for x in 0..width {
            let i = row + x * n_channels;
            let r = pixels[i] as f64;
            let g = pixels[i + 1] as f64;
            let b = pixels[i + 2] as f64;
            sum += (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f64)
}

/// Blend `a` toward `b` by `t`, rounded to the nearest value (truncating
/// would bias every blend dark by up to one step).
fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Render a paintable to pixels via snapshot, apply the CLUT, return a new texture.
pub fn snapshot_and_recolor(
    paintable: &gtk::IconPaintable,
    color_theme: &Themes,
) -> Option<gtk::gdk::Texture> {
    use gtk::graphene;
    use gtk::prelude::PaintableExt;

    let w = paintable.intrinsic_width().max(48) as f64;
    let h = paintable.intrinsic_height().max(48) as f64;

    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(snapshot.upcast_ref::<gtk::gdk::Snapshot>(), w, h);
    let node = snapshot.to_node()?;

    // Simpler approach: use CairoRenderer which doesn't need a surface
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gtk::gdk::Surface>).ok()?;

    let rect = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
    let texture = renderer.render_texture(&node, Some(&rect));
    renderer.unrealize();

    let width = texture.width() as u32;
    let height = texture.height() as u32;
    let stride = width * 4;
    let mut pixels = vec![0u8; (stride * height) as usize];
    texture.download(&mut pixels, stride as usize);

    // Build an RgbaImage from the downloaded pixels and apply the CLUT
    let mut img = RgbaImage::from_raw(width, height, pixels)?;
    let clut_bytes = load_clut(color_theme)?;
    let hald_clut = ImageBuffer::<Rgb<u8>, _>::from_raw(HALD8_DIM, HALD8_DIM, clut_bytes)?;
    correct_image(&mut img, &hald_clut);

    rgba_to_texture(img.as_raw(), width, height)
}

pub fn rgba_to_texture(buf: &[u8], width: u32, height: u32) -> Option<gtk::gdk::Texture> {
    let bytes = gtk::glib::Bytes::from(buf);
    let texture = gtk::gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        (width * 4) as usize,
    );
    Some(texture.upcast())
}

/// One channel of the contrast curve: `factor` > 1.0 increases contrast,
/// < 1.0 decreases it, 1.0 is no change. The result is rounded: the float
/// round trip lands a hair below many integers, and truncating it made 1.0
/// darken 20 of the 256 values by one step.
fn contrast_channel(v: u8, factor: f64) -> u8 {
    ((((v as f64 / 255.0) - 0.5) * factor + 0.5) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Desaturate toward a tint color.
/// `factor` 0.0 = no change, 1.0 = fully desaturated and tinted.
fn apply_monochrome(buf: &mut [u8], tint: [u8; 3], factor: f64) {
    for chunk in buf.chunks_exact_mut(4) {
        // Rec. 709 luminance
        let luma = (chunk[0] as f32 * 0.2126 + chunk[1] as f32 * 0.7152 + chunk[2] as f32 * 0.0722)
            .clamp(0.0, 255.0);

        // Blend tint color by luma (preserves light/dark variation)
        let tinted_r = tint[0] as f32 * (luma / 255.0);
        let tinted_g = tint[1] as f32 * (luma / 255.0);
        let tinted_b = tint[2] as f32 * (luma / 255.0);

        // Rounded, not truncated, so a white tint keeps greys as they were.
        chunk[0] = lerp_u8(chunk[0], tinted_r.round() as u8, factor);
        chunk[1] = lerp_u8(chunk[1], tinted_g.round() as u8, factor);
        chunk[2] = lerp_u8(chunk[2], tinted_b.round() as u8, factor);
        // alpha untouched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A non-identity level-8 Hald CLUT: the identity with every channel
    /// rotated (r,g,b) → (g,b,255-r), so a no-op grade can't pass by accident.
    fn test_clut() -> RgbImage {
        let mut clut = lutgen::identity::generate(HALD_LEVEL);
        for px in clut.pixels_mut() {
            let [r, g, b] = px.0;
            px.0 = [g, b, 255 - r];
        }
        clut
    }

    fn test_image(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([
                (x * 37 % 256) as u8,
                (y * 91 % 256) as u8,
                ((x + y) * 13 % 256) as u8,
                (x * y % 256) as u8,
            ])
        })
    }

    /// The pre-fusion pipeline: clone, CLUT, contrast, blend over full buffers.
    fn reference_grade(img: &mut RgbaImage, clut: &RgbImage, strength: f64, contrast: f64) {
        let original = (strength < 1.0).then(|| img.clone());
        correct_image(img, clut);
        for chunk in img.as_mut().chunks_exact_mut(4) {
            for c in chunk.iter_mut().take(3) {
                *c = ((((*c as f64 / 255.0) - 0.5) * contrast + 0.5) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
        if let Some(original) = original {
            for (d, s) in img.as_mut().iter_mut().zip(original.as_raw().iter()) {
                *d = lerp_u8(*s, *d, strength);
            }
        }
    }

    #[test]
    fn fused_grade_matches_reference_pipeline() {
        let clut = test_clut();
        let cancel = AtomicBool::new(false);
        for (strength, contrast) in [(1.0, 1.0), (0.5, 1.0), (0.3, 1.4), (1.0, 0.7), (0.0, 1.0)] {
            let mut expected = test_image(67, 41);
            reference_grade(&mut expected, &clut, strength, contrast);
            let mut got = test_image(67, 41);
            assert!(grade_in_place(
                got.as_mut(),
                &clut,
                strength,
                contrast,
                &cancel
            ));
            assert_eq!(
                got.as_raw(),
                expected.as_raw(),
                "strength {strength} contrast {contrast}"
            );
        }
    }

    #[test]
    fn contrast_factor_one_is_the_identity() {
        for v in 0..=255u8 {
            assert_eq!(contrast_channel(v, 1.0), v, "value {v}");
        }
    }

    #[test]
    fn contrast_rounds_to_the_nearest_value() {
        // 0.5 + (200/255 - 0.5) * 1.5 = 0.926… → 236.25 → 236
        assert_eq!(contrast_channel(200, 1.5), 236);
        // 0.5 + (100/255 - 0.5) * 0.5 = 0.446… → 113.75 rounds up, not down.
        assert_eq!(contrast_channel(100, 0.5), 114);
        // Clamped at both ends.
        assert_eq!(contrast_channel(0, 3.0), 0);
        assert_eq!(contrast_channel(255, 3.0), 255);
        // Factor 0 flattens everything to mid-grey.
        assert_eq!(contrast_channel(17, 0.0), 128);
        assert_eq!(contrast_channel(240, 0.0), 128);
    }

    #[test]
    fn lerp_rounds_and_is_symmetric() {
        assert_eq!(lerp_u8(10, 20, 0.0), 10);
        assert_eq!(lerp_u8(10, 20, 1.0), 20);
        // Midpoint 127.5 rounds to 128 from either side.
        assert_eq!(lerp_u8(0, 255, 0.5), 128);
        assert_eq!(lerp_u8(255, 0, 0.5), 128);
        for (a, b) in [(0u8, 1u8), (10, 11), (3, 250), (200, 7)] {
            for t in [0.1, 0.25, 0.5, 0.7, 0.9] {
                assert_eq!(lerp_u8(a, b, t), lerp_u8(b, a, 1.0 - t), "{a} {b} {t}");
            }
        }
    }

    #[test]
    fn contrast_one_adds_nothing_to_the_clut_lookup() {
        // The wallpaper grade calls with contrast 1.0: the result must be the
        // CLUT lookup itself, for every channel value.
        let clut = test_clut();
        let cancel = AtomicBool::new(false);
        let mut img = RgbaImage::from_fn(256, 3, |x, y| {
            let v = x as u8;
            let w = (x as u8).wrapping_mul(7).wrapping_add(y as u8 * 50);
            image::Rgba([v, w, 255 - v, 255])
        });
        let expected: Vec<u8> = img
            .pixels()
            .flat_map(|p| {
                let [r, g, b] = correct_pixel(&[p.0[0], p.0[1], p.0[2]], &clut, HALD_LEVEL);
                [r, g, b, p.0[3]]
            })
            .collect();
        assert!(grade_in_place(img.as_mut(), &clut, 1.0, 1.0, &cancel));
        assert_eq!(img.as_raw(), &expected);
    }

    #[test]
    fn white_monochrome_keeps_greys() {
        // Fully tinting a grey toward white leaves it as it was.
        let mut buf: Vec<u8> = (0..=255u8).flat_map(|v| [v, v, v, 7]).collect();
        let original = buf.clone();
        apply_monochrome(&mut buf, [255, 255, 255], 1.0);
        assert_eq!(buf, original);
    }

    #[test]
    fn monochrome_tints_by_luma_and_keeps_alpha() {
        let mut buf = vec![255, 0, 0, 9, 0, 0, 0, 200];
        apply_monochrome(&mut buf, [100, 200, 50], 1.0);
        // Pure red has luma 0.2126 → 21/43/11 of the tint (rounded).
        assert_eq!(buf, vec![21, 43, 11, 9, 0, 0, 0, 200]);
        // Factor 0 is a no-op.
        let mut buf = vec![12, 34, 56, 78];
        apply_monochrome(&mut buf, [1, 2, 3], 0.0);
        assert_eq!(buf, vec![12, 34, 56, 78]);
    }

    #[test]
    fn grade_stops_when_cancelled() {
        let clut = test_clut();
        let cancel = AtomicBool::new(true);
        let mut img = test_image(8, 8);
        assert!(!grade_in_place(img.as_mut(), &clut, 1.0, 1.0, &cancel));
    }

    /// A fresh, empty directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "mshell-image-test-{tag}-{}-{seq}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .map(|e| {
                e.expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_static_theme_has_a_unique_clut_name() {
        for (name, theme) in CLUT_THEMES {
            assert_eq!(clut_name(theme), Some(*name));
        }
        let mut names: Vec<_> = CLUT_THEMES.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CLUT_THEMES.len(), "duplicate CLUT name");
        // Every theme but Wallpaper recolours; Default shares Kenp's CLUT.
        for theme in [
            Themes::Kenp,
            Themes::CatppuccinMocha,
            Themes::EverforestDarkMedium,
            Themes::GruvboxMaterial,
            Themes::Horizon,
            Themes::Iznik,
            Themes::KanagawaWave,
            Themes::OneDark,
            Themes::Margo,
            Themes::RosePine,
            Themes::TokyoNight,
        ] {
            assert!(clut_name(&theme).is_some(), "{theme:?} has no CLUT");
        }
        assert_eq!(clut_name(&Themes::Default), Some("kenp"));
        assert_eq!(clut_name(&Themes::Wallpaper), None);
        assert!(!CLUT_THEMES.iter().any(|(_, t)| *t == Themes::Default));
    }

    #[test]
    fn cache_dir_prefers_an_absolute_xdg_cache_home() {
        let some = |s: &str| Some(std::ffi::OsString::from(s));
        assert_eq!(
            clut_cache_dir_from(some("/x/cache"), some("/home/u")),
            Some(PathBuf::from("/x/cache/mshell/cluts"))
        );
        // Relative or unset XDG_CACHE_HOME falls back to $HOME/.cache.
        for xdg in [some("rel/cache"), None] {
            assert_eq!(
                clut_cache_dir_from(xdg, some("/home/u")),
                Some(PathBuf::from("/home/u/.cache/mshell/cluts"))
            );
        }
        assert_eq!(clut_cache_dir_from(some("rel"), None), None);
        assert_eq!(clut_cache_dir_from(None, None), None);
    }

    #[test]
    fn cache_write_is_atomic_and_overwrites() {
        let tmp = TempDir::new("write");
        let path = tmp.0.join("nested").join("kenp.bin");
        write_clut_cache(&path, b"first").expect("write");
        write_clut_cache(&path, b"second").expect("overwrite");
        assert_eq!(fs::read(&path).expect("read"), b"second");
        // No temp files are left beside the cache entry.
        assert_eq!(file_names(&tmp.0.join("nested")), vec!["kenp.bin"]);
        let err = write_clut_cache(Path::new("/"), b"x").expect_err("no parent");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn load_clut_reads_a_valid_cache_without_regenerating() {
        let tmp = TempDir::new("hit");
        // Not a real CLUT: proves the cached bytes are returned as they are.
        let fake = vec![7u8; CLUT_BYTE_LEN];
        fs::write(tmp.0.join("kenp.bin"), &fake).expect("seed cache");
        assert_eq!(
            load_clut_in(Some(&tmp.0), &Themes::Kenp),
            Some(fake.clone())
        );
        // Default resolves to the same cache entry.
        assert_eq!(load_clut_in(Some(&tmp.0), &Themes::Default), Some(fake));
    }

    #[test]
    fn load_clut_generates_caches_and_repairs_a_corrupt_cache() {
        let tmp = TempDir::new("gen");
        let dir = tmp.0.join("cluts");
        let path = dir.join("tokyo_night.bin");

        // Missing cache: generated, then written (directory created).
        let generated = load_clut_in(Some(&dir), &Themes::TokyoNight).expect("generate");
        assert_eq!(generated.len(), CLUT_BYTE_LEN);
        assert_eq!(fs::read(&path).expect("cached"), generated);
        assert_eq!(file_names(&dir), vec!["tokyo_night.bin"]);

        // A truncated cache is regenerated and rewritten; generation is
        // deterministic, so the bytes match the first run.
        fs::write(&path, &generated[..1000]).expect("corrupt cache");
        let repaired = load_clut_in(Some(&dir), &Themes::TokyoNight).expect("regenerate");
        assert_eq!(repaired, generated);
        assert_eq!(fs::read(&path).expect("rewritten"), generated);

        // Wallpaper has no CLUT and must not touch the cache.
        assert_eq!(load_clut_in(Some(&dir), &Themes::Wallpaper), None);
        assert_eq!(file_names(&dir), vec!["tokyo_night.bin"]);
    }

    #[test]
    fn load_clut_without_a_cache_dir_still_generates() {
        let clut = load_clut_in(None, &Themes::Horizon).expect("generate");
        assert_eq!(clut.len(), CLUT_BYTE_LEN);
        assert_eq!(Some(clut), generate_clut(&Themes::Horizon));
        assert_eq!(generate_clut(&Themes::Wallpaper), None);
    }

    fn write_test_png(name: &str, w: u32, h: u32) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mshell-image-{name}-{}-{w}x{h}.png",
            std::process::id()
        ));
        test_image(w, h).save(&path).expect("write test png");
        path
    }

    #[test]
    fn scaled_decode_produces_the_requested_size() {
        let path = write_test_png("scaled", 400, 200);
        let cancel = AtomicBool::new(false);
        let img = decode_rgba_scaled(
            &path,
            |w, h| {
                assert_eq!((w, h), (400, 200), "size_for sees the source size");
                (100, 50)
            },
            &cancel,
        )
        .expect("decode");
        assert_eq!(img.dimensions(), (100, 50));
        // Unscaled request keeps the source size and pixels.
        let full = decode_rgba_scaled(&path, |w, h| (w, h), &cancel).expect("decode");
        assert_eq!(full.dimensions(), (400, 200));
        assert_eq!(full.as_raw(), test_image(400, 200).as_raw());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn scaled_decode_honours_cancel() {
        let path = write_test_png("cancel", 16, 16);
        let cancel = AtomicBool::new(true);
        assert!(decode_rgba_scaled(&path, |w, h| (w, h), &cancel).is_none());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn pixbuf_decode_keeps_alpha_and_fills_it_for_rgb() {
        let path = write_test_png("rgba", 23, 9);
        let img = decode_pixbuf_rgba(&path).expect("decode rgba");
        assert_eq!(img.as_raw(), test_image(23, 9).as_raw());
        let _ = fs::remove_file(&path);

        let tmp = TempDir::new("rgb");
        let rgb_path = tmp.0.join("rgb.png");
        let rgb = RgbImage::from_fn(5, 3, |x, y| Rgb([x as u8 * 40, y as u8 * 70, 9]));
        rgb.save(&rgb_path).expect("write rgb png");
        let img = decode_pixbuf_rgba(&rgb_path).expect("decode rgb");
        assert_eq!(img.dimensions(), (5, 3));
        for (x, y, p) in img.enumerate_pixels() {
            assert_eq!(p.0, [x as u8 * 40, y as u8 * 70, 9, 255]);
        }
    }

    #[test]
    fn decoders_reject_missing_and_garbage_files() {
        let tmp = TempDir::new("garbage");
        let missing = tmp.0.join("missing.png");
        let garbage = tmp.0.join("garbage.png");
        fs::write(&garbage, b"definitely not an image").expect("write garbage");
        let cancel = AtomicBool::new(false);
        for path in [&missing, &garbage] {
            assert!(decode_pixbuf_rgba(path).is_none());
            assert!(decode_rgba_scaled(path, |w, h| (w, h), &cancel).is_none());
            assert!(average_luminance(path).is_none());
        }
    }

    #[test]
    fn scaled_decode_ignores_a_zero_target_size() {
        let path = write_test_png("zero", 30, 20);
        let cancel = AtomicBool::new(false);
        for target in [(0, 10), (10, 0), (0, 0)] {
            let img = decode_rgba_scaled(&path, move |_, _| target, &cancel).expect("decode");
            assert_eq!(img.dimensions(), (30, 20), "target {target:?}");
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn average_luminance_spans_black_to_white() {
        let tmp = TempDir::new("luma");
        let lum = |name: &str, rgb: [u8; 3]| {
            let path = tmp.0.join(name);
            RgbImage::from_pixel(100, 80, Rgb(rgb))
                .save(&path)
                .expect("write png");
            average_luminance(&path).expect("luminance")
        };
        assert!(lum("black.png", [0, 0, 0]).abs() < 1e-9);
        assert!((lum("white.png", [255, 255, 255]) - 1.0).abs() < 1e-6);
        // Rec. 709 weights: green dominates, blue barely counts.
        assert!((lum("green.png", [0, 255, 0]) - 0.7152).abs() < 1e-6);
        assert!((lum("blue.png", [0, 0, 255]) - 0.0722).abs() < 1e-6);
    }

    #[test]
    fn theme_filter_skips_wallpaper_and_honours_cancel() {
        let path = write_test_png("filter", 12, 6);
        let idle = AtomicBool::new(false);
        let cancelled = AtomicBool::new(true);
        assert!(apply_theme_filter(&path, &Themes::Wallpaper, 1.0, 1.0, 0.0, &idle).is_none());
        assert!(
            apply_theme_filter_rgba(test_image(4, 4), &Themes::Wallpaper, 1.0, 1.0, 0.0, &idle)
                .is_none()
        );
        let clut = test_clut();
        assert!(
            filter_with_clut(
                test_image(4, 4),
                &clut,
                &Themes::Kenp,
                1.0,
                1.0,
                0.0,
                &cancelled
            )
            .is_none()
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn filter_with_clut_grades_and_keeps_size_and_alpha() {
        let clut = test_clut();
        let cancel = AtomicBool::new(false);
        let img = test_image(9, 5);
        let mut expected = img.clone();
        reference_grade(&mut expected, &clut, 0.6, 1.2);
        let got =
            filter_with_clut(img, &clut, &Themes::Kenp, 0.6, 1.2, 0.0, &cancel).expect("graded");
        assert_eq!(got.dimensions(), (9, 5));
        assert_eq!(got.as_raw(), expected.as_raw());
    }
}
