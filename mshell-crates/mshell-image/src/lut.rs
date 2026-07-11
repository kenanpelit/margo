use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use image::{ImageBuffer, Rgb, RgbaImage};
use lutgen::GenerateLut;
use lutgen::identity::correct_image;
use lutgen::interpolation::GaussianRemapper;
use mshell_config::schema::themes::Themes;
use mshell_matugen::json_struct::MatugenTheme;
use mshell_matugen::static_theme_mapping::static_theme;
use relm4::gtk;
use relm4::gtk::gdk_pixbuf::Pixbuf;
use relm4::gtk::prelude::{Cast, GskRendererExt, SnapshotExt, TextureExt, TextureExtManual};
use tracing::warn;

/// Every static theme that owns a CLUT, paired with its stable file/cache
/// basename. `Wallpaper` is absent (it grades from the live wallpaper) and
/// `Default` is absent as its own entry — it shares Kenp's CLUT.
pub const CLUT_THEMES: &[(&str, Themes)] = &[
    ("ayu_dark", Themes::AyuDark),
    ("catppuccin_mocha", Themes::CatppuccinMocha),
    ("dracula", Themes::Dracula),
    ("everforest_dark_medium", Themes::EverforestDarkMedium),
    ("flexoki", Themes::Flexoki),
    ("github_dark", Themes::GithubDark),
    ("gruvbox_dark_medium", Themes::GruvboxDarkMedium),
    ("gruvbox_material", Themes::GruvboxMaterial),
    ("horizon", Themes::Horizon),
    ("kanagawa_wave", Themes::KanagawaWave),
    ("kenp", Themes::Kenp),
    ("margo", Themes::Margo),
    ("monokai_classic", Themes::MonokaiClassic),
    ("nord_dark", Themes::NordDark),
    ("one_dark", Themes::OneDark),
    ("oxocarbon", Themes::Oxocarbon),
    ("rose_pine", Themes::RosePine),
    ("solarized_dark", Themes::SolarizedDark),
    ("tokyo_night", Themes::TokyoNight),
    ("vesper", Themes::Vesper),
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
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
        let dir = PathBuf::from(dir);
        // A relative XDG_CACHE_HOME is invalid per spec; ignore it.
        if dir.is_absolute() {
            return Some(dir.join("mshell").join("cluts"));
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("mshell")
            .join("cluts"),
    )
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
    let name = clut_name(theme)?;
    let cached = clut_cache_dir().map(|d| d.join(format!("{name}.bin")));

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
    let clut_bytes = load_clut(theme)?;

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    let hald_clut = ImageBuffer::<Rgb<u8>, _>::from_raw(HALD8_DIM, HALD8_DIM, clut_bytes)?;

    let mut img = decode_pixbuf_rgba(path)?;
    let (width, height) = img.dimensions();

    if monochrome > 0.0 {
        let tint = static_theme(theme, None)
            .map(|t| {
                let rgb = t.colors.on_surface.default.as_rgb();
                [rgb.0, rgb.1, rgb.2]
            })
            .unwrap_or([255, 255, 255]);
        apply_monochrome(img.as_mut(), tint, monochrome);
    }

    let original = if strength < 1.0 {
        Some(img.clone())
    } else {
        None
    };

    correct_image(&mut img, &hald_clut);

    adjust_contrast(img.as_mut(), contrast_adjustment);

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    if let Some(original) = original {
        blend_buffers(img.as_mut(), original.as_raw(), strength);
    }

    Some(RemapResult {
        buf: img.into_raw(),
        width,
        height,
    })
}

/// Decode an image file to RGBA using gdk-pixbuf.
pub fn decode_pixbuf_rgba(path: &Path) -> Option<RgbaImage> {
    let pixbuf = Pixbuf::from_file(path).ok()?;

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

fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t).clamp(0.0, 255.0) as u8
}

fn blend_buffers(dst: &mut [u8], src: &[u8], t: f64) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d = lerp_u8(*s, *d, t);
    }
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

/// Adjust contrast of an RGBA buffer in-place.
/// `factor` > 1.0 increases contrast, < 1.0 decreases it.
/// 1.0 is no change.
fn adjust_contrast(buf: &mut [u8], factor: f64) {
    for chunk in buf.chunks_exact_mut(4) {
        chunk[0] =
            ((((chunk[0] as f64 / 255.0) - 0.5) * factor + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
        chunk[1] =
            ((((chunk[1] as f64 / 255.0) - 0.5) * factor + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
        chunk[2] =
            ((((chunk[2] as f64 / 255.0) - 0.5) * factor + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
        // leave alpha (chunk[3]) untouched
    }
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

        chunk[0] = lerp_u8(chunk[0], tinted_r as u8, factor);
        chunk[1] = lerp_u8(chunk[1], tinted_g as u8, factor);
        chunk[2] = lerp_u8(chunk[2], tinted_b as u8, factor);
        // alpha untouched
    }
}
