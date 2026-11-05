//! Runtime compositor wallpaper: an image a client hands margo over IPC,
//! for every output or per output, drawn where the configured `wallpaper`
//! would be.
//!
//! Why: a shell that paints its wallpaper in a full-screen layer surface
//! keeps a whole swapchain for a static image — four output-sized buffers
//! under GTK's Vulkan renderer, about 104 MiB of GTT on a 1920x1200 +
//! 2560x1440 desk. margo draws a wallpaper from one texture with no
//! swapchain at all, so the shell gives margo its own (theme-filtered,
//! screen-sized) image and fit and destroys its windows. mshell does
//! (`mshell-wallpaper/src/handoff.rs`); it keeps its windows when margo is
//! older or refuses.
//!
//! Wire grammar (one `dispatch` line; the tail is free text):
//!
//! ```text
//! wallpaper_set    <outputs> [fit=<fit>] [bg=<colour>|bg=none] [fade=<ms>] [owner=<tag>] <path>
//! wallpaper_update <outputs> [fit=<fit>] [bg=<colour>|bg=none]
//! wallpaper_clear  [<outputs>] [owner=<tag>]
//! ```
//!
//! * `<outputs>`: `*` (every output without an entry of its own) or a
//!   comma list of connector names; a name not plugged in yet is kept for
//!   when it is. Clearing `*` clears every entry.
//! * `<path>`: absolute (`~/` expands). A headed `.raw`
//!   (`[u32 LE w][u32 LE h][RGBA]`) is read before the reply, so `ok`
//!   means the pixels are in memory and the file may be deleted. Any other
//!   image is decoded off the event loop: `ok` means queued, the previous
//!   image stays until it lands (`get wallpaper` shows `pending`/`error`).
//! * `fit`: a [`WallpaperFit`] name, kept from the previous entry when
//!   absent (else `cover`).
//! * `bg`: painted over the whole output under the image — what shows
//!   around a `contain`ed one. Kept from the previous entry when absent;
//!   `none` (the default) shows the `rootcolor` clear.
//! * `fade`: crossfade from the entry's previous image, eased like GTK's
//!   stack crossfade (ease-out cubic). `0` (the default) swaps at once.
//! * `owner`: a tag; `wallpaper_clear owner=<tag>` only clears entries
//!   with that tag, so one client never clears another's.
//!
//! Damage: each image keeps one element id for its lifetime and the
//! background a stable id and commit counter, so a static wallpaper
//! repaints nothing; a set, update or clear damages once, a fade every
//! frame until it ends.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use margo_config::WallpaperFit;
use serde_json::{Value, json};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

use crate::dispatch::DispatchOutcome;
use crate::state::MargoState;
use crate::wallpaper::{DecodedImage, WallpaperState, fit_rects};

/// Longest crossfade accepted; longer requests are clamped.
const MAX_FADE_MS: u32 = 10_000;

/// Element ids per image: one per [`Piece::slot`] — the image itself, its
/// overlap with the next image and up to four strips around it (see
/// [`plan`]).
const ID_SLOTS: usize = 6;

// ── Requests ────────────────────────────────────────────────────────────

/// Which entries a request addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Targets {
    /// `*`: the default entry (and, for a clear, every entry).
    All,
    /// Connector names.
    Named(Vec<String>),
}

impl Targets {
    fn parse(token: &str) -> Result<Self, String> {
        if token == "*" {
            return Ok(Targets::All);
        }
        let names: Vec<String> = token
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        if names.is_empty() || names.iter().any(|name| name == "*" || name.contains('=')) {
            return Err(format!(
                "bad output list `{token}` (use `*` or NAME[,NAME…])"
            ));
        }
        Ok(Targets::Named(names))
    }

    /// Keys of the entries addressed: `*` or the names.
    fn keys(&self) -> Vec<String> {
        match self {
            Targets::All => vec!["*".to_string()],
            Targets::Named(names) => names.clone(),
        }
    }
}

/// A `bg=` value: the straight-alpha colour to paint, `None` for `bg=none`.
pub type Background = Option<[f32; 4]>;

/// Options shared by the three requests; `None` = not given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    pub fit: Option<WallpaperFit>,
    pub bg: Option<Background>,
    pub fade_ms: Option<u32>,
    pub owner: Option<String>,
}

/// `wallpaper_set`.
#[derive(Debug, Clone, PartialEq)]
pub struct SetRequest {
    pub targets: Targets,
    pub options: Options,
    /// As given (unexpanded).
    pub path: String,
}

/// `wallpaper_update`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateRequest {
    pub targets: Targets,
    pub options: Options,
}

/// `wallpaper_clear`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClearRequest {
    pub targets: Targets,
    pub owner: Option<String>,
}

const SET_USAGE: &str = "usage: wallpaper_set <*|OUTPUT[,…]> [fit=…] [bg=#rrggbb[aa]|none] [fade=MS] [owner=TAG] <path>";

/// Parse `wallpaper_set`'s argument text.
pub fn parse_set(text: &str) -> Result<SetRequest, String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let (first, rest) = tokens.split_first().ok_or(SET_USAGE)?;
    let targets = Targets::parse(first)?;
    let (options, path) = take_options(rest, &["fit", "bg", "fade", "owner"])?;
    if path.is_empty() {
        return Err(SET_USAGE.to_string());
    }
    Ok(SetRequest {
        targets,
        options,
        path: path.join(" "),
    })
}

/// Parse `wallpaper_update`'s argument text.
pub fn parse_update(text: &str) -> Result<UpdateRequest, String> {
    const USAGE: &str = "usage: wallpaper_update <*|OUTPUT[,…]> [fit=…] [bg=#rrggbb[aa]|none]";
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let (first, rest) = tokens.split_first().ok_or(USAGE)?;
    let targets = Targets::parse(first)?;
    let (options, extra) = take_options(rest, &["fit", "bg"])?;
    if !extra.is_empty() || (options.fit.is_none() && options.bg.is_none()) {
        return Err(USAGE.to_string());
    }
    Ok(UpdateRequest { targets, options })
}

/// Parse `wallpaper_clear`'s argument text (empty = every entry).
pub fn parse_clear(text: &str) -> Result<ClearRequest, String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let (targets, rest) = match tokens.split_first() {
        Some((first, rest)) if !first.contains('=') => (Targets::parse(first)?, rest),
        _ => (Targets::All, &tokens[..]),
    };
    let (options, extra) = take_options(rest, &["owner"])?;
    if !extra.is_empty() {
        return Err("usage: wallpaper_clear [*|OUTPUT[,…]] [owner=TAG]".to_string());
    }
    Ok(ClearRequest {
        targets,
        owner: options.owner,
    })
}

/// Split the leading `key=value` options (of `allowed` keys) off `tokens`.
/// An absolute path (`/…`, `~…`) ends them; returns the options and the
/// tokens after them.
fn take_options<'a>(
    tokens: &'a [&'a str],
    allowed: &[&str],
) -> Result<(Options, &'a [&'a str]), String> {
    let mut options = Options::default();
    let mut used = 0;
    for token in tokens {
        if token.starts_with('/') || token.starts_with('~') {
            break;
        }
        let Some((key, value)) = token.split_once('=') else {
            return Err(format!(
                "expected key=value or an absolute path, got `{token}`"
            ));
        };
        if !allowed.contains(&key) {
            return Err(format!("unknown option `{key}`"));
        }
        match key {
            "fit" => {
                let fit = WallpaperFit::from_name(value).ok_or_else(|| {
                    format!("fit must be one of {}", WallpaperFit::names_joined())
                })?;
                options.fit = Some(fit);
            }
            "bg" => options.bg = Some(parse_background(value)?),
            "fade" => {
                let ms = value
                    .parse::<u32>()
                    .map_err(|_| format!("fade must be milliseconds, got `{value}`"))?;
                options.fade_ms = Some(ms.min(MAX_FADE_MS));
            }
            _ => {
                if value.is_empty() || value.chars().any(char::is_whitespace) {
                    return Err("owner must be one word".to_string());
                }
                options.owner = Some(value.to_string());
            }
        }
        used += 1;
    }
    Ok((options, &tokens[used..]))
}

/// `none`, or `#rrggbb` / `#rrggbbaa` (`0x` also accepted) as unit floats.
fn parse_background(value: &str) -> Result<Background, String> {
    if value.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let hex = value
        .strip_prefix('#')
        .or_else(|| value.strip_prefix("0x"))
        .unwrap_or(value);
    let bad = || format!("bg must be #rrggbb, #rrggbbaa or none, got `{value}`");
    let digits = u32::from_str_radix(hex, 16).map_err(|_| bad())?;
    let rgba = match hex.len() {
        6 => (digits << 8) | 0xff,
        8 => digits,
        _ => return Err(bad()),
    };
    let channel = |shift: u32| ((rgba >> shift) & 0xff) as f32 / 255.0;
    Ok(Some([channel(24), channel(16), channel(8), channel(0)]))
}

// ── State ───────────────────────────────────────────────────────────────

/// One image as drawn: its texture(s) and where it came from.
pub struct Image {
    pixels: WallpaperState,
    path: PathBuf,
    /// One element id per [`Piece::slot`] (slot 0 is the image's own).
    ids: [Id; ID_SLOTS],
}

impl Image {
    fn new(pixels: WallpaperState, path: PathBuf) -> Self {
        let ids = std::array::from_fn(|slot| match slot {
            0 => pixels.id().clone(),
            _ => Id::new(),
        });
        Self { pixels, path, ids }
    }
}

/// The crossfade from an entry's previous image.
struct Fade {
    from: Rc<Image>,
    from_fit: WallpaperFit,
    start: Instant,
    duration: Duration,
}

/// The colour painted under an entry's image. The id and commit counter
/// stay across frames so the damage tracker repaints it only on change.
struct Fill {
    rgba: [f32; 4],
    id: Id,
    commit: CommitCounter,
}

/// What one target (`*` or an output) shows.
struct Entry {
    image: Rc<Image>,
    fit: WallpaperFit,
    fill: Option<Fill>,
    fade: Option<Fade>,
    owner: String,
    serial: u64,
}

/// Every runtime wallpaper, keyed `*` (the default) or by output name.
#[derive(Default)]
pub struct RuntimeWallpapers {
    /// Tests: act as if the backend drew compositor wallpapers.
    #[cfg(test)]
    pub force_backend: bool,
    entries: BTreeMap<String, Entry>,
    /// Per target key, the latest set request: an off-loop decode that
    /// lands after a newer request for the same target is dropped.
    generations: BTreeMap<String, u64>,
    next_generation: u64,
    next_serial: u64,
    /// Why the last off-loop decode failed.
    last_error: Option<String>,
}

impl RuntimeWallpapers {
    /// The entry `output` shows: its own, else the `*` default.
    fn entry(&self, output: &str) -> Option<&Entry> {
        self.entries.get(output).or_else(|| self.entries.get("*"))
    }

    /// Whether `output` shows a runtime wallpaper.
    pub fn covers(&self, output: &str) -> bool {
        self.entry(output).is_some()
    }

    /// Start a set request for `targets`: supersedes their pending decodes.
    fn begin(&mut self, targets: &Targets) -> u64 {
        self.next_generation += 1;
        for key in targets.keys() {
            self.generations.insert(key, self.next_generation);
        }
        self.next_generation
    }

    /// Install `image` for the targets of request `generation` that no
    /// newer request has claimed since. Returns whether anything changed.
    fn install(
        &mut self,
        targets: &Targets,
        options: &Options,
        generation: u64,
        image: Rc<Image>,
        now: Instant,
    ) -> bool {
        let mut changed = false;
        for key in targets.keys() {
            if self.generations.get(&key) != Some(&generation) {
                continue;
            }
            self.generations.remove(&key);
            self.next_serial += 1;
            let previous = self.entries.remove(&key);
            let entry = new_entry(previous, options, image.clone(), self.next_serial, now);
            self.entries.insert(key, entry);
            changed = true;
        }
        changed
    }

    /// Change fit / background of existing entries. Returns how many.
    fn update(&mut self, request: &UpdateRequest) -> usize {
        let mut count = 0;
        for key in request.targets.keys() {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if let Some(fit) = request.options.fit {
                // GTK sets the fit on both pictures of a running crossfade.
                entry.fit = fit;
                if let Some(fade) = &mut entry.fade {
                    fade.from_fit = fit;
                }
            }
            if let Some(bg) = request.options.bg {
                set_fill(&mut entry.fill, bg);
            }
            count += 1;
        }
        count
    }

    /// Remove entries (all of them for `*`), only `owner`'s when given.
    /// Returns how many went.
    fn clear(&mut self, request: &ClearRequest) -> usize {
        let owned = |entry: &Entry| request.owner.as_ref().is_none_or(|o| *o == entry.owner);
        let before = self.entries.len();
        match &request.targets {
            Targets::All => self.entries.retain(|_, entry| !owned(entry)),
            Targets::Named(names) => {
                for name in names {
                    if self.entries.get(name).is_some_and(owned) {
                        self.entries.remove(name);
                    }
                }
            }
        }
        before - self.entries.len()
    }

    /// End finished crossfades. `true` while any runs (and on the tick
    /// that ends one), so the caller keeps frames coming.
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut animating = false;
        for entry in self.entries.values_mut() {
            if let Some(fade) = &entry.fade {
                animating = true;
                if now.saturating_duration_since(fade.start) >= fade.duration {
                    entry.fade = None;
                }
            }
        }
        animating
    }

    /// The `wallpaper` IPC topic; `supported` = the backend draws it.
    pub fn topic(&self, supported: bool) -> Value {
        let entries: Vec<Value> = self
            .entries
            .iter()
            .map(|(key, entry)| {
                let (w, h) = entry.image.pixels.natural_size();
                json!({
                    "outputs": key,
                    "path": entry.image.path.display().to_string(),
                    "size": [w, h],
                    "fit": entry.fit.name(),
                    "bg": entry.fill.as_ref().map(|f| hex_colour(f.rgba)),
                    "owner": entry.owner,
                    "serial": entry.serial,
                    "fading": entry.fade.is_some(),
                })
            })
            .collect();
        json!({
            "supported": supported,
            "fits": WallpaperFit::ALL.iter().map(|f| f.name()).collect::<Vec<_>>(),
            "entries": entries,
            "pending": self.generations.keys().collect::<Vec<_>>(),
            "error": self.last_error,
        })
    }

    /// Elements for `output` (logical `size`, `scale`) at `place`, topmost
    /// first; `None` when no runtime wallpaper covers it.
    fn elements(
        &self,
        renderer: &mut GlesRenderer,
        output: &str,
        size: Size<i32, Logical>,
        scale: f64,
        place: &Placement,
        now: Instant,
    ) -> Option<Vec<WallpaperElement>> {
        let entry = self.entry(output)?;
        let natural = |image: &Image| {
            let (w, h) = image.pixels.natural_size();
            (f64::from(w), f64::from(h))
        };
        let (previous, progress) = match &entry.fade {
            Some(fade) => (
                Some((natural(&fade.from), fade.from_fit)),
                eased(now.saturating_duration_since(fade.start), fade.duration),
            ),
            None => (None, 1.0),
        };
        let out = (f64::from(size.w), f64::from(size.h));
        let mut elements = Vec::new();
        for piece in plan((natural(&entry.image), entry.fit), previous, progress, out) {
            let image = match (piece.layer, &entry.fade) {
                (Layer::Current, _) => &entry.image,
                (Layer::Previous, Some(fade)) => &fade.from,
                (Layer::Previous, None) => continue,
            };
            let k = place.zoom;
            let dst_size = Size::<i32, Logical>::from((
                (piece.dst.2 * k).round() as i32,
                (piece.dst.3 * k).round() as i32,
            ));
            if dst_size.w <= 0 || dst_size.h <= 0 {
                continue;
            }
            let loc = Point::<f64, Physical>::from((
                place.origin.x + piece.dst.0 * k * scale,
                place.origin.y + piece.dst.1 * k * scale,
            ));
            let src = Rectangle::<f64, Logical>::new(
                Point::from((piece.src.0, piece.src.1)),
                Size::from((piece.src.2, piece.src.3)),
            );
            let Some(base) = image.ids.get(piece.slot) else {
                continue;
            };
            let id = element_id(base, place.cell);
            if let Some(element) =
                image
                    .pixels
                    .element(renderer, id, src, loc, dst_size, piece.alpha)
            {
                elements.push(WallpaperElement::Texture(element));
            }
        }
        if let Some(fill) = &entry.fill {
            let geometry = Rectangle::<i32, Physical>::new(
                place.origin.to_i32_round(),
                Size::from((
                    (out.0 * place.zoom * scale).round() as i32,
                    (out.1 * place.zoom * scale).round() as i32,
                )),
            );
            let [r, g, b, a] = fill.rgba;
            elements.push(WallpaperElement::Solid(SolidColorRenderElement::new(
                element_id(&fill.id, place.cell),
                geometry,
                fill.commit,
                [r * a, g * a, b * a, a],
                Kind::Unspecified,
            )));
        }
        Some(elements)
    }
}

/// The entry replacing `previous` for a set request.
fn new_entry(
    previous: Option<Entry>,
    options: &Options,
    image: Rc<Image>,
    serial: u64,
    now: Instant,
) -> Entry {
    let fade_ms = options.fade_ms.unwrap_or(0);
    let (fit, mut fill, fade, owner) = match previous {
        Some(prev) => {
            let fade = (fade_ms > 0).then(|| Fade {
                from: prev.image.clone(),
                from_fit: prev.fit,
                start: now,
                duration: Duration::from_millis(u64::from(fade_ms)),
            });
            (prev.fit, prev.fill, fade, prev.owner)
        }
        // GTK's stack shows its first child without a transition.
        None => (WallpaperFit::default(), None, None, String::new()),
    };
    if let Some(bg) = options.bg {
        set_fill(&mut fill, bg);
    }
    Entry {
        image,
        fit: options.fit.unwrap_or(fit),
        fill,
        fade,
        owner: options.owner.clone().unwrap_or(owner),
        serial,
    }
}

/// Apply a `bg=` value, keeping the fill's id (and bumping its commit
/// only when the colour changes).
fn set_fill(fill: &mut Option<Fill>, bg: Background) {
    match (fill.as_mut(), bg) {
        (_, None) => *fill = None,
        (Some(current), Some(rgba)) => {
            if current.rgba != rgba {
                current.rgba = rgba;
                current.commit.increment();
            }
        }
        (None, Some(rgba)) => {
            *fill = Some(Fill {
                rgba,
                id: Id::new(),
                commit: CommitCounter::default(),
            });
        }
    }
}

/// `#rrggbbaa`.
fn hex_colour(rgba: [f32; 4]) -> String {
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        byte(rgba[0]),
        byte(rgba[1]),
        byte(rgba[2]),
        byte(rgba[3])
    )
}

// ── Geometry ────────────────────────────────────────────────────────────

/// `(x, y, w, h)`.
type Rect = (f64, f64, f64, f64);

/// Which image a [`Piece`] samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Current,
    Previous,
}

/// One textured quad of the wallpaper: `src` in image pixels, `dst` in
/// output-local logical px. `slot` tells the pieces' element ids apart.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub layer: Layer,
    pub src: Rect,
    pub dst: Rect,
    pub alpha: f32,
    pub slot: usize,
}

/// GTK's crossfade curve (`gtk_progress_tracker_get_ease_out_cubic`) at
/// `elapsed` of `duration`.
pub fn eased(elapsed: Duration, duration: Duration) -> f64 {
    if duration.is_zero() {
        return 1.0;
    }
    let t = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// The pieces drawing `current` (image size, fit) on an `out`-sized output,
/// crossfading from `previous` at `progress` (0..=1), topmost first.
///
/// GTK's crossfade blends the two pictures, `old·(1−p) + new·p`, before
/// they meet the window background. Drawing the new image at alpha `p`
/// over the old one gives exactly that where both cover; where only the
/// old one covers (a `contain`ed image of another aspect) the old one must
/// itself fade, so it is split into its overlap with the new image (alpha
/// 1) and the strips around it (alpha `1 − p`).
pub fn plan(
    current: ((f64, f64), WallpaperFit),
    previous: Option<((f64, f64), WallpaperFit)>,
    progress: f64,
    out: (f64, f64),
) -> Vec<Piece> {
    let Some(new) = fitted(current, out) else {
        return Vec::new();
    };
    let previous = previous
        .filter(|_| progress < 1.0)
        .and_then(|prev| fitted(prev, out));
    let Some(old) = previous else {
        return vec![Piece {
            layer: Layer::Current,
            src: new.0,
            dst: new.1,
            alpha: 1.0,
            slot: 0,
        }];
    };
    let p = progress.clamp(0.0, 1.0) as f32;
    let mut pieces = vec![Piece {
        layer: Layer::Current,
        src: new.0,
        dst: new.1,
        alpha: p,
        slot: 0,
    }];
    if let Some(overlap) = intersect(old.1, new.1) {
        pieces.push(Piece {
            layer: Layer::Previous,
            src: sub_src(old, overlap),
            dst: overlap,
            alpha: 1.0,
            slot: 1,
        });
    }
    for (i, strip) in subtract(old.1, new.1).into_iter().enumerate() {
        pieces.push(Piece {
            layer: Layer::Previous,
            src: sub_src(old, strip),
            dst: strip,
            alpha: 1.0 - p,
            slot: 2 + i,
        });
    }
    pieces
}

/// `(src, dst)` of an image fitted to `out`.
fn fitted((img, fit): ((f64, f64), WallpaperFit), out: (f64, f64)) -> Option<(Rect, Rect)> {
    let (src, loc, size) = fit_rects(img, out, fit)?;
    Some((src, (loc.0, loc.1, size.0, size.1)))
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// `a` minus `b`: up to four rectangles (top, bottom, left, right).
fn subtract(a: Rect, b: Rect) -> Vec<Rect> {
    let Some(i) = intersect(a, b) else {
        return vec![a];
    };
    let (ax1, ay1) = (a.0 + a.2, a.1 + a.3);
    let (ix1, iy1) = (i.0 + i.2, i.1 + i.3);
    [
        (a.0, a.1, a.2, i.1 - a.1),
        (a.0, iy1, a.2, ay1 - iy1),
        (a.0, i.1, i.0 - a.0, i.3),
        (ix1, i.1, ax1 - ix1, i.3),
    ]
    .into_iter()
    .filter(|r| r.2 > 0.0 && r.3 > 0.0)
    .collect()
}

/// The part of `(src, dst)`'s source that lands on `part` (inside `dst`).
fn sub_src((src, dst): (Rect, Rect), part: Rect) -> Rect {
    let sx = src.2 / dst.2;
    let sy = src.3 / dst.3;
    (
        src.0 + (part.0 - dst.0) * sx,
        src.1 + (part.1 - dst.1) * sy,
        part.2 * sx,
        part.3 * sy,
    )
}

/// The id of an element whose own id is `base` (a piece slot's, a
/// fill's), in an overview `cell` or on the output itself.
fn element_id(base: &Id, cell: Option<usize>) -> Id {
    match cell {
        None => base.clone(),
        Some(cell) => base.namespaced(cell),
    }
}

// ── Scene ───────────────────────────────────────────────────────────────

/// A wallpaper render element: an image quad or the background fill.
pub enum WallpaperElement {
    Texture(TextureRenderElement<GlesTexture>),
    Solid(SolidColorRenderElement),
}

impl From<WallpaperElement> for crate::backend::udev::MargoRenderElement {
    fn from(element: WallpaperElement) -> Self {
        match element {
            WallpaperElement::Texture(e) => Self::Wallpaper(e),
            WallpaperElement::Solid(e) => Self::Solid(e),
        }
    }
}

/// Where an output's wallpaper is drawn: `origin` (output-local physical
/// px) and `zoom` place the output-sized picture; `cell` namespaces the
/// element ids of an overview cell.
struct Placement {
    origin: Point<f64, Physical>,
    zoom: f64,
    cell: Option<usize>,
}

/// The wallpaper under `output`'s scene, topmost first: the runtime image
/// set for it, else the configured `wallpaper`.
pub fn scene_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output: &Output,
) -> Vec<WallpaperElement> {
    let Some(geometry) = state.space.output_geometry(output) else {
        return Vec::new();
    };
    let scale = output.current_scale().fractional_scale();
    let place = Placement {
        origin: Point::from((0.0, 0.0)),
        zoom: 1.0,
        cell: None,
    };
    let now = Instant::now();
    let runtime = &state.runtime_wallpaper;
    if let Some(elements) =
        runtime.elements(renderer, &output.name(), geometry.size, scale, &place, now)
    {
        return elements;
    }
    state
        .wallpaper
        .as_ref()
        .and_then(|wp| {
            wp.render_element(renderer, geometry.size, state.config.wallpaper_fit, scale)
        })
        .map(WallpaperElement::Texture)
        .into_iter()
        .collect()
}

/// The runtime wallpaper inside a scroller-overview cell whose top-left is
/// `origin` and which shows the output at `zoom`. It stands in for the
/// background layer surface the shell no longer maps, which every cell
/// showed; the configured `wallpaper` never was drawn in cells.
pub fn cell_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output: &Output,
    origin: Point<i32, Physical>,
    zoom: f64,
    cell: usize,
) -> Vec<WallpaperElement> {
    let Some(geometry) = state.space.output_geometry(output) else {
        return Vec::new();
    };
    let place = Placement {
        origin: origin.to_f64(),
        zoom,
        cell: Some(cell),
    };
    let scale = output.current_scale().fractional_scale();
    state
        .runtime_wallpaper
        .elements(
            renderer,
            &output.name(),
            geometry.size,
            scale,
            &place,
            Instant::now(),
        )
        .unwrap_or_default()
}

// ── Dispatch ────────────────────────────────────────────────────────────

impl MargoState {
    /// Whether this backend draws a compositor wallpaper: only the udev
    /// renderer does. Nested winit refuses `wallpaper_set`, so a shell
    /// keeps its own surfaces there.
    pub(crate) fn draws_compositor_wallpaper(&self) -> bool {
        #[cfg(test)]
        if self.runtime_wallpaper.force_backend {
            return true;
        }
        self.udev_backend.is_some()
    }

    /// The `wallpaper` IPC topic.
    pub(crate) fn runtime_wallpaper_topic(&self) -> Value {
        self.runtime_wallpaper
            .topic(self.draws_compositor_wallpaper())
    }

    /// Per loop iteration: keep frames coming while a crossfade runs, and
    /// one more when it ends to drop the previous image.
    pub(crate) fn tick_runtime_wallpaper(&mut self) {
        if self.runtime_wallpaper.tick(Instant::now()) {
            self.request_scene_repaint();
        }
    }

    /// `dispatch wallpaper_set …`.
    pub(crate) fn wallpaper_set(&mut self, text: &str) -> DispatchOutcome {
        if !self.draws_compositor_wallpaper() {
            return DispatchOutcome::Failed(
                "this backend draws no compositor wallpaper".to_string(),
            );
        }
        let request = match parse_set(text) {
            Ok(request) => request,
            Err(e) => return DispatchOutcome::Failed(e),
        };
        // Expanded here, not on a decode thread: it reads `HOME`.
        let path = crate::wallpaper::expand_home(&request.path);
        if !path.is_absolute() {
            return DispatchOutcome::Failed(format!(
                "{}: the path must be absolute",
                path.display()
            ));
        }
        if !crate::wallpaper::exists_and_readable(&path) {
            return DispatchOutcome::Failed(format!("{}: not a readable file", path.display()));
        }
        let generation = self.runtime_wallpaper.begin(&request.targets);
        if crate::wallpaper::is_raw_backdrop(&path) {
            return match crate::wallpaper::decode_pixels(&path) {
                Ok(image) => {
                    self.install_runtime_wallpaper(&request, generation, image, path);
                    DispatchOutcome::Handled
                }
                Err(e) => {
                    self.runtime_wallpaper
                        .generations
                        .retain(|_, g| *g != generation);
                    DispatchOutcome::Failed(format!("{}: {e}", path.display()))
                }
            };
        }
        self.decode_runtime_wallpaper(request, generation, path);
        DispatchOutcome::Handled
    }

    /// `dispatch wallpaper_update …`.
    pub(crate) fn wallpaper_update(&mut self, text: &str) -> DispatchOutcome {
        let request = match parse_update(text) {
            Ok(request) => request,
            Err(e) => return DispatchOutcome::Failed(e),
        };
        if self.runtime_wallpaper.update(&request) == 0 {
            return DispatchOutcome::Failed("no runtime wallpaper set there".to_string());
        }
        self.runtime_wallpaper_changed();
        DispatchOutcome::Handled
    }

    /// `dispatch wallpaper_clear …`. Clearing nothing is not an error.
    pub(crate) fn wallpaper_clear(&mut self, text: &str) -> DispatchOutcome {
        let request = match parse_clear(text) {
            Ok(request) => request,
            Err(e) => return DispatchOutcome::Failed(e),
        };
        // A decode still in flight for these targets must not land later.
        let generations = &mut self.runtime_wallpaper.generations;
        match &request.targets {
            Targets::All => generations.clear(),
            Targets::Named(names) => generations.retain(|key, _| !names.contains(key)),
        }
        if self.runtime_wallpaper.clear(&request) > 0 {
            self.runtime_wallpaper_changed();
        }
        DispatchOutcome::Handled
    }

    fn install_runtime_wallpaper(
        &mut self,
        request: &SetRequest,
        generation: u64,
        image: DecodedImage,
        path: PathBuf,
    ) {
        let image = Rc::new(Image::new(
            WallpaperState::from_decoded(image.with_source(&path)),
            path,
        ));
        let now = Instant::now();
        let runtime = &mut self.runtime_wallpaper;
        if runtime.install(&request.targets, &request.options, generation, image, now) {
            runtime.last_error = None;
            self.runtime_wallpaper_changed();
        }
    }

    /// Decode an encoded image on a short-lived thread and install it when
    /// it arrives, unless a newer request claimed its targets meanwhile.
    fn decode_runtime_wallpaper(&mut self, request: SetRequest, generation: u64, path: PathBuf) {
        let (tx, rx) = calloop::channel::channel::<Result<DecodedImage, String>>();
        let thread_path = path.clone();
        let inserted = self.loop_handle.insert_source(rx, move |event, _, state| {
            let calloop::channel::Event::Msg(result) = event else {
                return;
            };
            match result {
                Ok(image) => {
                    state.install_runtime_wallpaper(&request, generation, image, path.clone())
                }
                Err(error) => {
                    tracing::warn!(%error, "runtime wallpaper: decode failed");
                    let runtime = &mut state.runtime_wallpaper;
                    runtime.generations.retain(|_, g| *g != generation);
                    runtime.last_error = Some(error);
                    state.mark_state_dirty();
                }
            }
        });
        if let Err(error) = inserted {
            tracing::warn!(?error, "runtime wallpaper: channel insert failed");
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("margo-wallpaper".into())
            .spawn(move || {
                let result = crate::wallpaper::decode_pixels(&thread_path)
                    .map_err(|e| format!("{}: {e}", thread_path.display()));
                // The receiver may be gone (compositor shutting down).
                let _ = tx.send(result);
            });
        if let Err(error) = spawned {
            tracing::warn!(?error, "runtime wallpaper: decode thread failed");
        }
    }

    fn runtime_wallpaper_changed(&mut self) {
        self.mark_state_dirty();
        self.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(names: &[&str]) -> Targets {
        Targets::Named(names.iter().map(|n| (*n).to_string()).collect())
    }

    #[test]
    fn set_parses_targets_options_and_a_path_with_spaces() {
        let req = parse_set(
            "eDP-1,DP-3 fit=scale-down bg=#11223344 fade=200 owner=mshell /run/u/a b.raw",
        )
        .expect("parses");
        assert_eq!(req.targets, named(&["eDP-1", "DP-3"]));
        assert_eq!(req.options.fit, Some(WallpaperFit::ScaleDown));
        let bg = req.options.bg.flatten().expect("colour");
        assert!(
            (bg[0] - 0x11 as f32 / 255.0).abs() < 1e-6
                && (bg[3] - 0x44 as f32 / 255.0).abs() < 1e-6
        );
        assert_eq!(req.options.fade_ms, Some(200));
        assert_eq!(req.options.owner.as_deref(), Some("mshell"));
        assert_eq!(req.path, "/run/u/a b.raw");

        let bare = parse_set("* ~/w.png").expect("parses");
        assert_eq!(
            (bare.targets, bare.options, bare.path.as_str()),
            (Targets::All, Options::default(), "~/w.png")
        );
        assert_eq!(
            parse_set("* bg=none /w.raw").expect("none").options.bg,
            Some(None)
        );
        assert_eq!(
            parse_set("* bg=#ffffff /w.raw").expect("rgb").options.bg,
            Some(Some([1.0; 4]))
        );
        assert_eq!(
            parse_set("* fade=999999 /w.raw")
                .expect("clamped")
                .options
                .fade_ms,
            Some(MAX_FADE_MS)
        );
    }

    #[test]
    fn malformed_requests_are_refused_with_a_reason() {
        for bad in [
            "",
            "*",
            "* fit=cover",
            "* fit=tile /w.raw",
            "* bg=#12345 /w.raw",
            "* bg=red /w.raw",
            "* fade=-1 /w.raw",
            "* colour=red /w.raw",
            "* relative.raw",
            ",, /w.raw",
            "eDP-1,* /w.raw",
        ] {
            assert!(parse_set(bad).is_err(), "`{bad}` should be refused");
        }
        assert!(parse_update("*").is_err(), "nothing to change");
        assert!(parse_update("* fade=10").is_err(), "fade is set-only");
        assert!(parse_update("* fit=fill /w.raw").is_err(), "no path");
        assert!(parse_clear("* fit=cover").is_err());
    }

    #[test]
    fn clear_defaults_to_every_entry() {
        assert_eq!(
            parse_clear("").expect("empty"),
            ClearRequest {
                targets: Targets::All,
                owner: None
            }
        );
        assert_eq!(
            parse_clear("owner=mshell").expect("owner only"),
            ClearRequest {
                targets: Targets::All,
                owner: Some("mshell".into())
            }
        );
        assert_eq!(
            parse_clear("DP-3").expect("named").targets,
            named(&["DP-3"])
        );
    }

    #[test]
    fn gtk_crossfade_curve_is_ease_out_cubic() {
        let d = Duration::from_millis(200);
        assert_eq!(eased(Duration::ZERO, d), 0.0);
        assert!((eased(Duration::from_millis(100), d) - 0.875).abs() < 1e-12);
        assert_eq!(eased(d, d), 1.0);
        assert_eq!(eased(Duration::from_secs(9), d), 1.0);
        assert_eq!(eased(Duration::ZERO, Duration::ZERO), 1.0);
    }

    const OUT: (f64, f64) = (1920.0, 1080.0);

    #[test]
    fn a_settled_wallpaper_is_one_opaque_piece() {
        let pieces = plan(((3840.0, 2160.0), WallpaperFit::Cover), None, 1.0, OUT);
        assert_eq!(
            pieces,
            [Piece {
                layer: Layer::Current,
                src: (0.0, 0.0, 3840.0, 2160.0),
                dst: (0.0, 0.0, 1920.0, 1080.0),
                alpha: 1.0,
                slot: 0
            }]
        );
        // A finished fade draws no previous image.
        let done = plan(
            ((3840.0, 2160.0), WallpaperFit::Cover),
            Some(((100.0, 100.0), WallpaperFit::Cover)),
            1.0,
            OUT,
        );
        assert_eq!(done.len(), 1);
    }

    #[test]
    fn covering_images_crossfade_as_new_over_an_opaque_old() {
        let pieces = plan(
            ((3840.0, 2160.0), WallpaperFit::Cover),
            Some(((1920.0, 1200.0), WallpaperFit::Cover)),
            0.25,
            OUT,
        );
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        assert_eq!((pieces[0].layer, pieces[0].alpha), (Layer::Current, 0.25));
        assert_eq!(
            (pieces[1].layer, pieces[1].alpha, pieces[1].dst),
            (Layer::Previous, 1.0, (0.0, 0.0, 1920.0, 1080.0))
        );
    }

    /// Composite `pieces` over `bg` at output point `(x, y)`, one channel,
    /// with images `new` / `old` of uniform value: what the GPU blends.
    fn blend_at(pieces: &[Piece], bg: f32, new: f32, old: f32, x: f64, y: f64) -> f32 {
        let mut c = bg;
        for piece in pieces.iter().rev() {
            let (px, py, pw, ph) = piece.dst;
            if x >= px && x < px + pw && y >= py && y < py + ph {
                let v = if piece.layer == Layer::Current {
                    new
                } else {
                    old
                };
                c = v * piece.alpha + c * (1.0 - piece.alpha);
            }
        }
        c
    }

    /// GTK: `cross_fade(old, new, p)` composited over the window background.
    fn gtk_at(
        new_dst: Rect,
        old_dst: Rect,
        p: f32,
        bg: f32,
        new: f32,
        old: f32,
        x: f64,
        y: f64,
    ) -> f32 {
        let inside = |r: Rect| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
        let (new_a, old_a) = (
            if inside(new_dst) { 1.0 } else { 0.0 },
            if inside(old_dst) { 1.0 } else { 0.0 },
        );
        let colour = new * new_a * p + old * old_a * (1.0 - p);
        let alpha = new_a * p + old_a * (1.0 - p);
        colour + bg * (1.0 - alpha)
    }

    #[test]
    fn letterboxed_crossfade_matches_gtk_everywhere() {
        // Old: a wide image contained (bars top and bottom); new: a tall one
        // contained (bars left and right). GTK fades the old image's bars
        // out; drawing the old one opaque would pop them at the end.
        let new = ((1000.0, 2000.0), WallpaperFit::Contain);
        let old = ((4000.0, 1000.0), WallpaperFit::Contain);
        let new_dst = fitted(new, OUT).unwrap().1;
        let old_dst = fitted(old, OUT).unwrap().1;
        for p in [0.0, 0.3, 0.875, 0.999] {
            let pieces = plan(new, Some(old), f64::from(p), OUT);
            for x in (0..1920).step_by(37) {
                for y in (0..1080).step_by(29) {
                    let (x, y) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                    let ours = blend_at(&pieces, 0.2, 0.9, 0.5, x, y);
                    let gtk = gtk_at(new_dst, old_dst, p, 0.2, 0.9, 0.5, x, y);
                    assert!(
                        (ours - gtk).abs() < 1e-5,
                        "p {p} at ({x}, {y}): {ours} vs {gtk}"
                    );
                }
            }
        }
    }

    #[test]
    fn previous_pieces_sample_the_matching_part_of_the_old_image() {
        let pieces = plan(
            ((1000.0, 2000.0), WallpaperFit::Contain),
            Some(((1920.0, 1080.0), WallpaperFit::Fill)),
            0.5,
            OUT,
        );
        // The old image is 1:1 with the output, so each piece's source
        // equals its destination.
        for piece in pieces.iter().filter(|p| p.layer == Layer::Previous) {
            let (s, d) = (piece.src, piece.dst);
            assert!(
                (s.0 - d.0).abs() < 1e-9
                    && (s.1 - d.1).abs() < 1e-9
                    && (s.2 - d.2).abs() < 1e-9
                    && (s.3 - d.3).abs() < 1e-9,
                "{piece:?}"
            );
        }
        // Slots tell every piece apart.
        let mut slots: Vec<usize> = pieces.iter().map(|p| p.slot).collect();
        slots.dedup();
        assert_eq!(slots.len(), pieces.len());
        assert!(slots.iter().all(|s| *s < ID_SLOTS));
    }

    #[test]
    fn subtract_covers_exactly_the_difference() {
        let a = (0.0, 0.0, 10.0, 10.0);
        assert_eq!(subtract(a, (20.0, 20.0, 1.0, 1.0)), [a]);
        assert!(subtract(a, a).is_empty());
        let strips = subtract(a, (2.0, 3.0, 4.0, 5.0));
        let area: f64 = strips.iter().map(|r| r.2 * r.3).sum();
        assert_eq!(area, 100.0 - 20.0);
        for (i, r) in strips.iter().enumerate() {
            assert!(intersect(*r, (2.0, 3.0, 4.0, 5.0)).is_none());
            for s in &strips[i + 1..] {
                assert!(intersect(*r, *s).is_none());
            }
        }
    }

    #[test]
    fn element_ids_differ_per_slot_and_cell_and_stay_stable() {
        let img = image(1, 1);
        let mut ids = Vec::new();
        for base in &img.ids {
            ids.push(element_id(base, None));
            for cell in [0, 1, 7, usize::MAX] {
                ids.push(element_id(base, Some(cell)));
            }
        }
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "duplicate id at {i}");
        }
        assert_eq!(
            &img.ids[0],
            img.pixels.id(),
            "the main piece keeps the image id"
        );
        assert_eq!(
            element_id(&img.ids[2], Some(3)),
            element_id(&img.ids[2], Some(3))
        );
    }

    fn image(w: u32, h: u32) -> Rc<Image> {
        let path = PathBuf::from(format!("/w-{w}x{h}.raw"));
        let pixels = WallpaperState::from_decoded(crate::wallpaper::DecodedImage::filled_for_test(
            w, h, [255; 4],
        ));
        Rc::new(Image::new(pixels, path))
    }

    fn set(rt: &mut RuntimeWallpapers, text: &str, img: Rc<Image>, now: Instant) -> bool {
        let req = parse_set(text).expect("request");
        let generation = rt.begin(&req.targets);
        rt.install(&req.targets, &req.options, generation, img, now)
    }

    #[test]
    fn per_output_entries_override_the_default() {
        let mut rt = RuntimeWallpapers::default();
        let now = Instant::now();
        assert!(!rt.covers("eDP-1"));
        assert!(set(&mut rt, "* fit=contain /a", image(4, 2), now));
        assert!(
            rt.covers("eDP-1") && rt.covers("HDMI-A-9"),
            "* covers outputs plugged in later"
        );
        assert!(set(&mut rt, "DP-3 /b", image(2, 2), now));
        assert_eq!(
            rt.entry("DP-3").map(|e| e.image.pixels.natural_size()),
            Some((2, 2))
        );
        assert_eq!(
            rt.entry("eDP-1").map(|e| e.image.pixels.natural_size()),
            Some((4, 2))
        );
        assert_eq!(
            rt.entry("DP-3").map(|e| e.fit),
            Some(WallpaperFit::Cover),
            "a new entry starts at cover"
        );
    }

    #[test]
    fn a_set_keeps_fit_background_and_owner_unless_given_and_fades_only_when_asked() {
        let mut rt = RuntimeWallpapers::default();
        let t0 = Instant::now();
        set(
            &mut rt,
            "* fit=scale-down bg=#000000 owner=mshell /a",
            image(4, 2),
            t0,
        );
        let first_fill_id = rt.entries["*"].fill.as_ref().unwrap().id.clone();
        assert!(
            rt.entries["*"].fade.is_none(),
            "the first image appears without a fade"
        );
        set(&mut rt, "* fade=200 /b", image(2, 2), t0);
        let entry = &rt.entries["*"];
        assert_eq!(
            (entry.fit, entry.owner.as_str()),
            (WallpaperFit::ScaleDown, "mshell")
        );
        assert_eq!(
            entry.fill.as_ref().unwrap().id,
            first_fill_id,
            "same fill element"
        );
        let fade = entry.fade.as_ref().expect("fading");
        assert_eq!(
            (fade.from.pixels.natural_size(), fade.from_fit),
            ((4, 2), WallpaperFit::ScaleDown)
        );
        // The tick keeps frames coming until the fade ends, then drops it.
        assert!(rt.tick(t0 + Duration::from_millis(100)));
        assert!(rt.entries["*"].fade.is_some());
        assert!(
            rt.tick(t0 + Duration::from_millis(200)),
            "one more frame without the old image"
        );
        assert!(rt.entries["*"].fade.is_none());
        assert!(!rt.tick(t0 + Duration::from_millis(216)));
    }

    #[test]
    fn update_changes_fit_and_background_in_place() {
        let mut rt = RuntimeWallpapers::default();
        set(&mut rt, "* bg=#000000 /a", image(4, 2), Instant::now());
        let commit = rt.entries["*"].fill.as_ref().unwrap().commit;
        let same = parse_update("* bg=#000000").unwrap();
        assert_eq!(rt.update(&same), 1);
        assert_eq!(
            rt.entries["*"].fill.as_ref().unwrap().commit,
            commit,
            "an unchanged colour damages nothing"
        );
        assert_eq!(
            rt.update(&parse_update("* fit=fill bg=#ff0000").unwrap()),
            1
        );
        let entry = &rt.entries["*"];
        assert_eq!(entry.fit, WallpaperFit::Fill);
        assert_ne!(
            entry.fill.as_ref().unwrap().commit,
            commit,
            "a new colour is a new commit"
        );
        assert_eq!(
            rt.update(&parse_update("DP-3 fit=fill").unwrap()),
            0,
            "nothing set there"
        );
        assert_eq!(rt.update(&parse_update("* bg=none").unwrap()), 1);
        assert!(rt.entries["*"].fill.is_none());
    }

    #[test]
    fn clear_honours_targets_and_owner() {
        let mut rt = RuntimeWallpapers::default();
        let now = Instant::now();
        set(&mut rt, "* owner=mshell /a", image(1, 1), now);
        set(&mut rt, "DP-3 owner=me /b", image(1, 1), now);
        assert_eq!(rt.clear(&parse_clear("owner=mshell").unwrap()), 1);
        assert!(
            !rt.covers("eDP-1") && rt.covers("DP-3"),
            "another client's entry stays"
        );
        assert_eq!(rt.clear(&parse_clear("DP-3 owner=mshell").unwrap()), 0);
        assert_eq!(rt.clear(&parse_clear("DP-3").unwrap()), 1);
        assert!(rt.entries.is_empty());
    }

    #[test]
    fn a_decode_that_lands_after_a_newer_request_is_dropped() {
        let mut rt = RuntimeWallpapers::default();
        let now = Instant::now();
        let slow = parse_set("eDP-1,DP-3 /slow.png").unwrap();
        let slow_generation = rt.begin(&slow.targets);
        // A newer set for DP-3 lands first.
        assert!(set(&mut rt, "DP-3 /fast.raw", image(2, 2), now));
        assert!(rt.install(
            &slow.targets,
            &slow.options,
            slow_generation,
            image(9, 9),
            now
        ));
        assert_eq!(
            rt.entry("eDP-1").map(|e| e.image.pixels.natural_size()),
            Some((9, 9))
        );
        assert_eq!(
            rt.entry("DP-3").map(|e| e.image.pixels.natural_size()),
            Some((2, 2)),
            "not overwritten by the older request"
        );
        assert!(rt.generations.is_empty(), "nothing left pending");
    }

    #[test]
    fn topic_reports_support_entries_and_fits() {
        let mut rt = RuntimeWallpapers::default();
        assert_eq!(rt.topic(false)["supported"], json!(false));
        set(
            &mut rt,
            "* fit=contain bg=#102030 owner=mshell /a",
            image(4, 2),
            Instant::now(),
        );
        let topic = rt.topic(true);
        assert_eq!(topic["supported"], json!(true));
        assert_eq!(
            topic["fits"],
            json!(["cover", "contain", "fill", "center", "scale-down"])
        );
        let entry = &topic["entries"][0];
        assert_eq!(entry["outputs"], json!("*"));
        assert_eq!(entry["fit"], json!("contain"));
        assert_eq!(entry["bg"], json!("#102030ff"));
        assert_eq!(entry["owner"], json!("mshell"));
        assert_eq!(entry["size"], json!([4, 2]));
        assert_eq!(entry["path"], json!("/w-4x2.raw"));
    }

    fn element_ids(
        renderer: &mut GlesRenderer,
        rt: &RuntimeWallpapers,
        place: &Placement,
        now: Instant,
    ) -> Vec<Id> {
        use smithay::backend::renderer::element::Element;
        rt.elements(renderer, "DP-1", Size::from((16, 8)), 1.5, place, now)
            .expect("covered")
            .iter()
            .map(|e| match e {
                WallpaperElement::Texture(t) => t.id().clone(),
                WallpaperElement::Solid(s) => s.id().clone(),
            })
            .collect()
    }

    #[test]
    fn elements_keep_their_ids_across_frames_and_namespace_cells() {
        let Some(mut renderer) = crate::render::blur::gpu_tests::renderer() else {
            eprintln!("skipped: no render node / EGL");
            return;
        };
        let mut rt = RuntimeWallpapers::default();
        let t0 = Instant::now();
        set(&mut rt, "* fit=contain bg=#000000 /a", image(4, 2), t0);
        let output = Placement {
            origin: Point::from((0.0, 0.0)),
            zoom: 1.0,
            cell: None,
        };
        let cell = Placement {
            origin: Point::from((30.0, 5.0)),
            zoom: 0.5,
            cell: Some(0),
        };
        let settled = element_ids(&mut renderer, &rt, &output, t0);
        assert_eq!(settled.len(), 2, "image over its fill");
        assert_eq!(
            element_ids(&mut renderer, &rt, &output, t0),
            settled,
            "static: nothing to repaint"
        );
        let in_cell = element_ids(&mut renderer, &rt, &cell, t0);
        assert!(
            in_cell.iter().all(|id| !settled.contains(id)),
            "a cell never reuses the output's ids"
        );

        set(&mut rt, "* fade=200 fit=cover /b", image(2, 4), t0);
        let fading = element_ids(&mut renderer, &rt, &output, t0 + Duration::from_millis(50));
        assert!(fading.len() > 2, "{fading:?}");
        for (i, id) in fading.iter().enumerate() {
            assert!(
                !fading[i + 1..].contains(id),
                "duplicate element id in one frame"
            );
        }
        rt.tick(t0 + Duration::from_millis(250));
        let after = element_ids(&mut renderer, &rt, &output, t0 + Duration::from_millis(250));
        assert_eq!(after.len(), 2, "the previous image is gone");
        assert_ne!(after[0], settled[0], "a new image is a new element");
        assert_eq!(after[1], settled[1], "the fill element stays");
    }

    fn coloured(w: u32, h: u32, rgba: [u8; 4], name: &str) -> Rc<Image> {
        let pixels = WallpaperState::from_decoded(crate::wallpaper::DecodedImage::filled_for_test(
            w, h, rgba,
        ));
        Rc::new(Image::new(pixels, PathBuf::from(name)))
    }

    smithay::backend::renderer::element::render_elements! {
        Scene<=GlesRenderer>;
        Texture=TextureRenderElement<GlesTexture>,
        Solid=SolidColorRenderElement,
    }

    /// Render `rt`'s wallpaper for an 8x4 output into an offscreen buffer
    /// through `tracker`; returns the RGBA pixels and whether anything was
    /// damaged.
    fn render_frame(
        renderer: &mut GlesRenderer,
        tracker: &mut smithay::backend::renderer::damage::OutputDamageTracker,
        target: &mut GlesTexture,
        rt: &RuntimeWallpapers,
        now: Instant,
        age: usize,
    ) -> (Vec<u8>, bool) {
        use smithay::backend::allocator::Fourcc;
        use smithay::backend::renderer::{Bind, ExportMem};
        let place = Placement {
            origin: Point::from((0.0, 0.0)),
            zoom: 1.0,
            cell: None,
        };
        let elements: Vec<Scene> = rt
            .elements(renderer, "DP-1", Size::from((8, 4)), 1.0, &place, now)
            .expect("covered")
            .into_iter()
            .map(|e| match e {
                WallpaperElement::Texture(t) => Scene::Texture(t),
                WallpaperElement::Solid(s) => Scene::Solid(s),
            })
            .collect();
        let mut fb = renderer.bind(target).expect("bind");
        let damaged = tracker
            .render_output(renderer, &mut fb, age, &elements, [0.0, 0.0, 0.0, 1.0])
            .expect("render")
            .damage
            .is_some_and(|d| !d.is_empty());
        let mapping = renderer
            .copy_framebuffer(&fb, Rectangle::from_size((8, 4).into()), Fourcc::Abgr8888)
            .expect("copy");
        (
            renderer.map_texture(&mapping).expect("map").to_vec(),
            damaged,
        )
    }

    #[test]
    fn a_contained_crossfade_renders_like_gtk_and_a_settled_frame_damages_nothing() {
        use smithay::backend::allocator::Fourcc;
        use smithay::backend::renderer::Offscreen;
        use smithay::backend::renderer::damage::OutputDamageTracker;
        let Some(mut renderer) = crate::render::blur::gpu_tests::renderer() else {
            eprintln!("skipped: no render node / EGL");
            return;
        };
        let mut target: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (8, 4).into())
            .expect("offscreen buffer");
        let mut tracker = OutputDamageTracker::new((8, 4), 1.0, smithay::utils::Transform::Normal);
        let t0 = Instant::now();
        // Old: a wide red image contained (rows 1-2); new: a tall blue one
        // contained (columns 3-4); the window background is green. Both
        // are symmetric about the middle, so the read-back's row order
        // doesn't matter.
        let mut rt = RuntimeWallpapers::default();
        let req = parse_set("* fit=contain bg=#00ff00 /old").unwrap();
        let generation = rt.begin(&req.targets);
        rt.install(
            &req.targets,
            &req.options,
            generation,
            coloured(4, 1, [255, 0, 0, 255], "/old"),
            t0,
        );
        let req = parse_set("* fade=200 /new").unwrap();
        let generation = rt.begin(&req.targets);
        rt.install(
            &req.targets,
            &req.options,
            generation,
            coloured(1, 2, [0, 0, 255, 255], "/new"),
            t0,
        );

        let at = t0 + Duration::from_millis(100);
        let p = eased(Duration::from_millis(100), Duration::from_millis(200)) as f32;
        let (pixels, _) = render_frame(&mut renderer, &mut tracker, &mut target, &rt, at, 0);
        let new_dst = (3.0, 0.0, 2.0, 4.0);
        let old_dst = (0.0, 1.0, 8.0, 2.0);
        for y in 0..4 {
            for x in 0..8 {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let i = (y * 8 + x) * 4;
                for (c, (bg, new, old)) in [(0.0, 0.0, 1.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0)]
                    .into_iter()
                    .enumerate()
                {
                    let want = gtk_at(new_dst, old_dst, p, bg, new, old, cx, cy);
                    let got = f32::from(pixels[i + c]) / 255.0;
                    assert!(
                        (got - want).abs() <= 2.0 / 255.0,
                        "({x}, {y}) channel {c}: {got} vs GTK {want}"
                    );
                }
            }
        }

        // Settled: the same scene again repaints nothing...
        rt.tick(t0 + Duration::from_millis(200));
        let settled = t0 + Duration::from_millis(250);
        let (_, damaged) = render_frame(&mut renderer, &mut tracker, &mut target, &rt, settled, 1);
        assert!(damaged, "the fade's end is a change");
        let (pixels, damaged) =
            render_frame(&mut renderer, &mut tracker, &mut target, &rt, settled, 1);
        assert!(!damaged, "a static wallpaper damages nothing");
        assert_eq!(
            &pixels[..4],
            &[0, 255, 0, 255],
            "background beside the image"
        );
        assert_eq!(
            &pixels[(8 + 3) * 4..(8 + 3) * 4 + 4],
            &[0, 0, 255, 255],
            "the new image"
        );
        // ...and a new background colour is one damaged frame.
        assert_eq!(rt.update(&parse_update("* bg=#ffffff").unwrap()), 1);
        let (pixels, damaged) =
            render_frame(&mut renderer, &mut tracker, &mut target, &rt, settled, 1);
        assert!(damaged);
        assert_eq!(&pixels[..4], &[255, 255, 255, 255]);
    }
}
