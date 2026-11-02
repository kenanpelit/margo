#![allow(dead_code)]
//! Software cursor loading and rendering.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::rc::Rc;

use drm_fourcc::DrmFourcc;
use smithay::{
    backend::renderer::{
        element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
        },
        gles::GlesRenderer,
    },
    input::pointer::CursorIcon,
    utils::{Buffer, Logical, Physical, Point, Size, Transform},
};

/// Shapes loaded when the manager is created, so the first hover over a
/// link, text field or window edge doesn't parse the theme file inside
/// input handling.
const PRELOAD: &[CursorIcon] = &[
    CursorIcon::Pointer,
    CursorIcon::Text,
    CursorIcon::Grab,
    CursorIcon::Grabbing,
    CursorIcon::Move,
    CursorIcon::EwResize,
    CursorIcon::NsResize,
    CursorIcon::NwseResize,
    CursorIcon::NeswResize,
    CursorIcon::ColResize,
    CursorIcon::RowResize,
    CursorIcon::NotAllowed,
    CursorIcon::Wait,
    CursorIcon::Progress,
    CursorIcon::Crosshair,
];

/// Theme lookup: `(icon name, nominal size) → image`.
type Loader<'a> = dyn FnMut(&str, u32) -> Option<CursorImage> + 'a;

// ── Cursor image data (backend-independent) ────────────────────────────────────

pub struct CursorImage {
    /// BGRA pixel data (Argb8888 little-endian format)
    pub pixels: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    /// The XCursor image's nominal size (what the theme calls it, e.g. 48
    /// for the 48 px variant); `width`/`height` may differ.
    pub nominal_size: u32,
}

impl CursorImage {
    pub fn to_memory_buffer(&self, buffer_scale: i32) -> MemoryRenderBuffer {
        MemoryRenderBuffer::from_slice(
            &self.pixels,
            DrmFourcc::Argb8888,
            Size::<i32, Buffer>::from((self.width, self.height)),
            buffer_scale,
            Transform::Normal,
            None,
        )
    }
}

// ── Scale / hotspot geometry (pure) ───────────────────────────────────────────

/// Integer buffer scale to load a cursor at for an output of `output_scale`:
/// the next whole scale up, so a 1.5× output gets a 2× image downscaled
/// (sharp) rather than a 1× image upscaled (blurry). Capped at 8×.
pub fn cursor_buffer_scale(output_scale: f64) -> i32 {
    if !output_scale.is_finite() || output_scale <= 1.0 {
        return 1;
    }
    (output_scale.ceil() as i32).clamp(1, 8)
}

/// Buffer scale an image loaded for `wanted` actually has. Themes don't
/// ship every size: asking for 48 px (24 px at 2×) may return the 24 px
/// image, which must then be drawn at buffer scale 1 to keep its logical
/// size — upscaled, but not shrunk to half size.
pub fn effective_buffer_scale(nominal_size: u32, image_nominal_size: u32, wanted: i32) -> i32 {
    if nominal_size == 0 {
        return 1;
    }
    let ratio = (f64::from(image_nominal_size) / f64::from(nominal_size)).round() as i32;
    ratio.clamp(1, wanted.max(1))
}

/// Physical location of the cursor image's top-left corner so that its
/// hotspot pixel lands exactly on the pointer at `pos`.
///
/// The image is drawn at its logical size (buffer px / `buffer_scale`)
/// times `output_scale`, so the hotspot sits `hotspot / buffer_scale`
/// logical units from the corner. It does not depend on the output scale.
pub fn cursor_render_location(
    pos: Point<f64, Logical>,
    hotspot: Point<i32, Buffer>,
    buffer_scale: i32,
    output_scale: f64,
) -> Point<f64, Physical> {
    let bs = f64::from(buffer_scale.max(1));
    let corner = Point::<f64, Logical>::from((
        pos.x - f64::from(hotspot.x) / bs,
        pos.y - f64::from(hotspot.y) / bs,
    ));
    corner.to_physical(output_scale)
}

// ── CursorManager ─────────────────────────────────────────────────────────────

/// One loaded cursor image, uploaded at a given buffer scale.
#[derive(Clone)]
struct CursorSprite {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, Buffer>,
    buffer_scale: i32,
}

impl CursorSprite {
    fn new(image: &CursorImage, buffer_scale: i32) -> Self {
        Self {
            buffer: image.to_memory_buffer(buffer_scale),
            hotspot: Point::from((image.hotspot_x, image.hotspot_y)),
            buffer_scale,
        }
    }
}

/// Renders the named cursor the focused client last requested (via
/// `wp_cursor_shape_v1` or `wl_pointer.set_cursor` → `CursorImageStatus::
/// Named`). Themed XCursor images are loaded lazily by name and cached, so
/// e.g. a browser asking for the `pointer` (hand) over a link actually
/// changes the on-screen cursor — previously only the fixed `default` arrow
/// was ever drawn. Each name is loaded once per buffer scale in use
/// (see [`cursor_buffer_scale`]), so scaled outputs get a sharp image.
pub struct CursorManager {
    /// Requested name → theme icon that resolved it (`None`: the embedded
    /// arrow). "default" is always present.
    resolved: HashMap<String, Option<String>>,
    /// (requested name, wanted buffer scale) → uploaded sprite. Behind a
    /// `RefCell` because the render path only has `&self` and loads a
    /// missing scale on first use.
    sprites: RefCell<HashMap<(String, i32), CursorSprite>>,
    /// Requested names the theme had no image for, so asking again (every
    /// pointer motion over such a surface) doesn't re-read the theme.
    missing: HashSet<String>,
    /// The active XCursor theme, resolved once (its directory search is
    /// not free).
    theme: Rc<xcursor::CursorTheme>,
    /// The name currently being drawn.
    current: String,
    /// Nominal cursor size in logical px (closest theme image is picked).
    size: u32,
}

impl CursorManager {
    pub fn new() -> Self {
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(24);
        let theme_name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let theme = Rc::new(xcursor::CursorTheme::load(&theme_name));
        let loader_theme = theme.clone();
        Self::build(size, theme, &mut |name, size| {
            load_from_theme(&loader_theme, name, size)
        })
    }

    /// Load `default` (embedded arrow when the theme has none) and the
    /// [`PRELOAD`] shapes through `load`.
    fn build(size: u32, theme: Rc<xcursor::CursorTheme>, load: &mut Loader<'_>) -> Self {
        let (theme_name, image) = match load("default", size) {
            Some(image) => (Some("default".to_string()), image),
            None => (None, embedded_arrow()),
        };
        let mut resolved = HashMap::new();
        resolved.insert("default".to_string(), theme_name);
        let mut sprites = HashMap::new();
        sprites.insert(("default".to_string(), 1), CursorSprite::new(&image, 1));
        let mut manager = Self {
            resolved,
            sprites: RefCell::new(sprites),
            missing: HashSet::new(),
            theme,
            current: "default".to_string(),
            size,
        };
        for icon in PRELOAD {
            manager.resolve(icon.name(), icon.alt_names(), load);
        }
        manager
    }

    /// Make sure `primary` is resolved (theme image cached) or known to be
    /// missing, loading through `load` only the first time it is asked for.
    fn resolve(&mut self, primary: &str, alts: &[&str], load: &mut Loader<'_>) {
        if self.resolved.contains_key(primary) || self.missing.contains(primary) {
            return;
        }
        let size = self.size;
        match std::iter::once(primary)
            .chain(alts.iter().copied())
            .find_map(|name| load(name, size).map(|image| (name, image)))
        {
            Some((name, image)) => {
                self.resolved
                    .insert(primary.to_string(), Some(name.to_string()));
                self.sprites
                    .get_mut()
                    .insert((primary.to_string(), 1), CursorSprite::new(&image, 1));
            }
            None => {
                self.missing.insert(primary.to_string());
            }
        }
    }

    /// Switch to the cursor named `primary` (with `alts` as theme fallbacks,
    /// e.g. `pointer` → `hand2`/`pointing_hand`). Loads + caches on first use
    /// (common shapes are preloaded); keeps drawing `default` if no variant
    /// resolves in the active theme.
    pub fn set_named(&mut self, primary: &str, alts: &[&str]) {
        if self.current == primary && self.resolved.contains_key(primary) {
            return;
        }
        let theme = self.theme.clone();
        self.resolve(primary, alts, &mut |name, size| {
            load_from_theme(&theme, name, size)
        });
        self.current = if self.resolved.contains_key(primary) {
            primary.to_string()
        } else {
            "default".to_string()
        };
    }

    /// The current cursor's sprite for buffer scale `wanted`, loading it from
    /// the theme on first use. Falls back to the 1× sprite when the theme
    /// can't provide it.
    fn sprite(&self, wanted: i32) -> CursorSprite {
        let key = (self.current.clone(), wanted);
        if let Some(sprite) = self.sprites.borrow().get(&key) {
            return sprite.clone();
        }
        let loaded = match self.resolved.get(&self.current) {
            Some(Some(theme_name)) if wanted > 1 => load_from_theme(
                &self.theme,
                theme_name,
                self.size.saturating_mul(wanted as u32),
            )
            .map(|image| {
                let bs = effective_buffer_scale(self.size, image.nominal_size, wanted);
                CursorSprite::new(&image, bs)
            }),
            _ => None,
        };
        let mut sprites = self.sprites.borrow_mut();
        let sprite = match loaded {
            Some(sprite) => sprite,
            None => sprites
                .get(&(self.current.clone(), 1))
                .or_else(|| sprites.get(&("default".to_string(), 1)))
                .cloned()
                .unwrap_or_else(|| CursorSprite::new(&embedded_arrow(), 1)),
        };
        // Cache the fallback too, so a theme without the size isn't
        // re-read on every frame.
        sprites.insert(key, sprite.clone());
        sprite
    }

    pub fn render_element(
        &self,
        renderer: &mut GlesRenderer,
        pos: Point<f64, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let sprite = self.sprite(cursor_buffer_scale(scale));
        let render_pos = cursor_render_location(pos, sprite.hotspot, sprite.buffer_scale, scale);
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            render_pos,
            &sprite.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )
        .ok()
    }
}

// ── xcursor theme loading ─────────────────────────────────────────────────────

fn load_from_theme(
    theme: &xcursor::CursorTheme,
    cursor_name: &str,
    size: u32,
) -> Option<CursorImage> {
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
        nominal_size: img.size,
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
        nominal_size: 16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the hotspot pixel ends up on screen, in physical px.
    fn hotspot_on_screen(
        pos: Point<f64, Logical>,
        hotspot: Point<i32, Buffer>,
        buffer_scale: i32,
        output_scale: f64,
    ) -> Point<f64, Physical> {
        let corner = cursor_render_location(pos, hotspot, buffer_scale, output_scale);
        // Buffer px → logical (÷ buffer scale) → physical (× output scale).
        let k = output_scale / f64::from(buffer_scale);
        Point::from((
            corner.x + f64::from(hotspot.x) * k,
            corner.y + f64::from(hotspot.y) * k,
        ))
    }

    #[test]
    fn hotspot_lands_on_the_pointer_at_every_scale() {
        // Regression: the offset was hotspot / output_scale, so on a 2×
        // output with a 1× image the click point drew half a hotspot off.
        let pos = Point::<f64, Logical>::from((100.0, 50.0));
        let hotspot = Point::<i32, Buffer>::from((8, 6));
        for (buffer_scale, output_scale) in
            [(1, 1.0), (1, 2.0), (1, 1.5), (2, 2.0), (2, 1.5), (3, 3.0)]
        {
            let on_screen = hotspot_on_screen(pos, hotspot, buffer_scale, output_scale);
            let want = pos.to_physical(output_scale);
            assert!(
                (on_screen.x - want.x).abs() < 1e-9 && (on_screen.y - want.y).abs() < 1e-9,
                "buffer scale {buffer_scale}, output scale {output_scale}: hotspot at {on_screen:?}, pointer at {want:?}"
            );
        }
    }

    fn fake_image() -> CursorImage {
        CursorImage {
            pixels: vec![0; 4],
            width: 1,
            height: 1,
            hotspot_x: 0,
            hotspot_y: 0,
            nominal_size: 24,
        }
    }

    #[test]
    fn common_shapes_are_preloaded_and_misses_are_cached() {
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let mut load = |name: &str, _size: u32| {
            calls.borrow_mut().push(name.to_string());
            (name != "bogus").then(fake_image)
        };
        let theme = Rc::new(xcursor::CursorTheme::load("margo-test-no-such-theme"));
        let mut m = CursorManager::build(24, theme, &mut load);
        assert!(m.resolved.contains_key(CursorIcon::Pointer.name()));
        assert!(m.resolved.contains_key(CursorIcon::Text.name()));

        // Switching to a preloaded shape (the input path) loads nothing.
        let before = calls.borrow().len();
        m.resolve(
            CursorIcon::Pointer.name(),
            CursorIcon::Pointer.alt_names(),
            &mut load,
        );
        assert_eq!(calls.borrow().len(), before);

        // An unknown shape is looked up once, then remembered as missing.
        m.resolve("bogus", &[], &mut load);
        let after_miss = calls.borrow().len();
        assert_eq!(after_miss, before + 1);
        m.resolve("bogus", &[], &mut load);
        assert_eq!(
            calls.borrow().len(),
            after_miss,
            "a miss must not re-read the theme"
        );
    }

    #[test]
    fn buffer_scale_rounds_up_for_sharp_cursors() {
        assert_eq!(cursor_buffer_scale(1.0), 1);
        assert_eq!(cursor_buffer_scale(0.5), 1);
        assert_eq!(cursor_buffer_scale(1.25), 2);
        assert_eq!(cursor_buffer_scale(2.0), 2);
        assert_eq!(cursor_buffer_scale(2.5), 3);
        assert_eq!(cursor_buffer_scale(f64::NAN), 1);
        assert_eq!(cursor_buffer_scale(100.0), 8);
    }

    #[test]
    fn effective_scale_keeps_the_logical_size() {
        // The theme had the 48 px image for a 24 px cursor at 2×.
        assert_eq!(effective_buffer_scale(24, 48, 2), 2);
        // Only the 24 px image exists: draw it at 1× (upscaled), not as a
        // half-size cursor.
        assert_eq!(effective_buffer_scale(24, 24, 2), 1);
        // A bigger image than asked for never exceeds the wanted scale.
        assert_eq!(effective_buffer_scale(24, 96, 2), 2);
        assert_eq!(effective_buffer_scale(0, 48, 2), 1);
    }
}
