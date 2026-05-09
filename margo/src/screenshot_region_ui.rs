//! In-compositor region selector — niri-pattern port + improvements.
//!
//! Press a `screenshot-region-ui` keybind, the compositor freezes a
//! copy of every output's current scene to a `GlesTexture`, dims
//! everything except a pre-drawn rectangle, and lets you adjust
//! the rectangle with the mouse before confirming with Return.
//!
//! ## What the user sees
//!
//! 1. **Print pressed** → frozen scene appears, dimmed everywhere,
//!    with an immediately-visible selection rectangle:
//!      * **First time on this session**: rectangle is centred at
//!        50% of the active output's size — same as niri's default.
//!      * **Subsequent times**: rectangle restored to the last
//!        confirmed/cancelled selection on that output.
//! 2. **Drag inside the selection** → moves the rectangle as a
//!    whole, preserving size. Cursor offset is locked at click.
//! 3. **Drag anywhere else** (or click on empty area) → starts a
//!    fresh selection from the click point; further motion stretches
//!    the opposite corner.
//! 4. **Mouse release** → ends the drag; rectangle stays in place.
//! 5. **Return** → captures the picked rectangle from the FROZEN
//!    texture (not the live scene that's been replaced by the
//!    selector overlay), saves to disk + clipboard via the
//!    standard pipeline.
//! 6. **Esc** → cancels without saving; the just-drawn rectangle
//!    is still remembered as the next-open default.
//!
//! ## Why frozen-texture-capture matters
//!
//! Earlier versions captured via the standard `ScreenshotSource::
//! Region` path, which on Return queues a request that the udev
//! hook drains by calling `build_render_elements_inner` for the
//! output. By that point the region selector is still active (we
//! clear it after queuing), so `build_render_elements` returns
//! the *selector overlay* (frozen + dim + border lines), not the
//! original scene the user actually wanted to capture. The saved
//! PNG was a screenshot of the screenshot UI, which the user
//! correctly described as "boşluk" (empty / wrong content).
//!
//! Phase 4 fix: route confirmation through
//! `screenshot::save_from_frozen_texture` which crops the captured
//! frozen `GlesTexture` directly. The selector's own captured
//! image IS the user-intended content; no second capture pass is
//! needed.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::input::keyboard::Keysym;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform};
use tracing::{debug, info};

use crate::backend::udev::{
    build_cursor_elements, build_render_elements_inner, MargoRenderElement,
    OutputDevice,
};
use crate::screencasting::render_helpers::create_texture;
use crate::state::MargoState;

/// Pushed by the dispatch handler. Drained by the udev hook on
/// the next repaint, which captures every output's current scene
/// to a frozen texture and assigns the resulting selector to
/// `state.region_selector`.
#[derive(Debug, Clone)]
pub struct PendingOpen {
    pub save_to_disk: bool,
    pub save_path: Option<PathBuf>,
    pub copy_clipboard: bool,
    pub include_pointer: bool,
}

/// Per-output state for the active selector.
pub struct FrozenOutput {
    /// The captured scene at open time. Drawn under the dim
    /// overlay so the user has a stable image to crop against.
    pub texture: GlesTexture,
    /// Texture's pixel size — same as the output's physical mode.
    pub size: Size<i32, Physical>,
    /// Output's fractional scale at capture time (passed to
    /// `TextureBuffer`'s constructor as i32 — fractional gets
    /// rounded; HiDPI users get a slight scale mismatch but
    /// still see the frozen content correctly oriented).
    pub scale: f64,
    /// Output's logical-space top-left in the global compositor
    /// coordinate space. Used to map the global pointer back to
    /// output-local for the selection drag.
    pub logical_origin: Point<i32, Logical>,
    /// Output's logical size (post-scale). Same units as the
    /// dim-strip rectangles' coordinates.
    pub logical_size: Size<i32, Logical>,
}

/// Active selector state.
pub struct RegionSelector {
    pub frozen: HashMap<String, FrozenOutput>,
    /// Connector name of the output the selection rectangle is
    /// being drawn on. Updates as the cursor crosses output
    /// boundaries.
    pub active_output: String,
    /// Selection rectangle in active-output's PHYSICAL pixels.
    /// Stored as (anchor, current) corner points so we know which
    /// edge is "live" during a drag-resize. Always populated —
    /// `open()` pre-fills with a centred default or restores the
    /// last-used rectangle on the active output.
    pub a: Point<i32, Physical>,
    pub b: Point<i32, Physical>,
    /// Mouse-button state machine. niri-pattern: `Up` between
    /// drags, `Down { mode }` while a button is held.
    pub button: Button,
    /// Carried into the save call when the user confirms.
    pub save_to_disk: bool,
    pub save_path: Option<PathBuf>,
    pub copy_clipboard: bool,
    /// Whether the cursor should appear in the SAVED screenshot.
    /// Toggle with P key. Live cursor in the selector overlay
    /// is independent of this — that's always shown so users
    /// can see where they're clicking.
    pub include_pointer: bool,
}

#[derive(Debug)]
pub enum Button {
    Up,
    Down { mode: DragMode },
}

/// What a press initiated. niri's selector calls these
/// "drag-existing" vs "drag-new"; they branch off
/// `is_within_selection(click_point)`.
#[derive(Debug)]
pub enum DragMode {
    /// User clicked inside the existing selection — drag-to-move.
    /// Tracks the offset between the cursor and selection's
    /// anchor at press time so motion preserves it.
    Move { cursor_offset: Point<i32, Physical> },
    /// User clicked outside or in empty space — drag-to-resize
    /// from the click point. The press resets the anchor (`a`)
    /// to the click; subsequent motion updates `b`.
    Resize,
}

/// Result of an input event handler. `Close { save: ... }` lets
/// the input handler queue the screenshot capture from the
/// frozen texture before clearing the selector.
pub enum HandleResult {
    Consumed,
    /// Selector is done — caller must clear
    /// `state.region_selector`. If `save` is `Some`, queue a
    /// frozen-texture save with those parameters.
    Close {
        save: Option<ConfirmSave>,
    },
}

pub struct ConfirmSave {
    pub texture: GlesTexture,
    pub rect_physical: Rectangle<i32, Physical>,
    pub save_to_disk: bool,
    pub save_path: Option<PathBuf>,
    pub copy_clipboard: bool,
}

/// Captures every active output to a frozen `GlesTexture`,
/// composes the initial selection rectangle (default-centred or
/// restored from `state.last_screenshot_region`), and returns
/// the new selector for the caller to install onto
/// `state.region_selector`.
pub fn open(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<
        smithay::reexports::drm::control::crtc::Handle,
        OutputDevice,
    >,
    state: &MargoState,
    request: PendingOpen,
) -> Result<RegionSelector> {
    let active_output = state
        .monitors
        .get(state.focused_monitor())
        .map(|m| m.name.clone())
        .context("no focused monitor to open region selector on")?;

    let mut frozen: HashMap<String, FrozenOutput> = HashMap::new();
    for (_, od) in outputs.iter() {
        let name = od.output.name();
        let mode = match od.output.current_mode() {
            Some(m) => m,
            None => continue,
        };
        let size = mode.size;
        if size.w <= 0 || size.h <= 0 {
            continue;
        }
        let scale_f = od.output.current_scale().fractional_scale();
        let scale = Scale::from(scale_f);

        // Render the output's CURRENT scene into a fresh texture.
        // include_cursor=false because the frozen scene gets a
        // dim overlay anyway, and embedding the cursor would
        // produce a distracting still pointer in the middle of
        // the user's selection drag.
        let elements: Vec<MargoRenderElement> =
            build_render_elements_inner(renderer, od, state, false, false);

        use smithay::backend::renderer::Bind;
        use smithay::backend::renderer::damage::OutputDamageTracker;
        use smithay::backend::renderer::Color32F;
        let mut texture = create_texture(renderer, size, Fourcc::Abgr8888)
            .context("create frozen texture")?;
        {
            let mut target = renderer
                .bind(&mut texture)
                .context("bind frozen texture")?;
            let mut dt = OutputDamageTracker::new(size, scale, Transform::Normal);
            dt.render_output(
                renderer,
                &mut target,
                0,
                &elements,
                Color32F::TRANSPARENT,
            )
            .context("render frozen scene")?;
        }

        let logical_origin = state
            .monitors
            .iter()
            .find(|m| m.name == name)
            .map(|m| Point::<i32, Logical>::from((m.monitor_area.x, m.monitor_area.y)))
            .unwrap_or_default();
        let logical_size = state
            .monitors
            .iter()
            .find(|m| m.name == name)
            .map(|m| Size::<i32, Logical>::from((
                m.monitor_area.width.max(0),
                m.monitor_area.height.max(0),
            )))
            .unwrap_or_default();

        frozen.insert(
            name,
            FrozenOutput {
                texture,
                size,
                scale: scale_f,
                logical_origin,
                logical_size,
            },
        );
    }

    if frozen.is_empty() {
        bail!("region selector: no captureable outputs");
    }

    // Initial selection: niri-pattern.
    //   - If there's a stashed last-selection on the active output, restore it.
    //   - Otherwise centred 50% rect.
    let active_size = frozen
        .get(&active_output)
        .map(|f| f.size)
        .unwrap_or(Size::from((1920, 1080)));
    let (a, b) = if let Some((last_output, last_rect)) = &state.last_screenshot_region {
        if last_output == &active_output
            && last_rect.size.w > 0
            && last_rect.size.h > 0
            && last_rect.loc.x >= 0
            && last_rect.loc.y >= 0
            && last_rect.loc.x + last_rect.size.w <= active_size.w
            && last_rect.loc.y + last_rect.size.h <= active_size.h
        {
            (
                last_rect.loc,
                Point::from((
                    last_rect.loc.x + last_rect.size.w - 1,
                    last_rect.loc.y + last_rect.size.h - 1,
                )),
            )
        } else {
            default_selection(active_size)
        }
    } else {
        default_selection(active_size)
    };

    info!(
        "region selector opened: {} output(s), active = `{}`, rect = {}x{}+{}+{}",
        frozen.len(),
        active_output,
        (a.x - b.x).abs() + 1,
        (a.y - b.y).abs() + 1,
        a.x.min(b.x),
        a.y.min(b.y),
    );

    Ok(RegionSelector {
        frozen,
        active_output,
        a,
        b,
        button: Button::Up,
        save_to_disk: request.save_to_disk,
        save_path: request.save_path,
        copy_clipboard: request.copy_clipboard,
        include_pointer: request.include_pointer,
    })
}

/// Default selection: 50% × 50% rect, centred on the output. Same
/// shape niri uses for its first-time-open default.
fn default_selection(size: Size<i32, Physical>) -> (Point<i32, Physical>, Point<i32, Physical>) {
    let w = size.w / 2;
    let h = size.h / 2;
    let x = size.w / 4;
    let y = size.h / 4;
    (
        Point::from((x, y)),
        Point::from((x + w - 1, y + h - 1)),
    )
}

/// Normalise (a, b) corner points into a positive-size Rectangle
/// in physical pixels. Used for both rendering (which wants
/// loc + size) and capture (which wants the cropped extent).
pub fn rect_from_corners(
    a: Point<i32, Physical>,
    b: Point<i32, Physical>,
) -> Rectangle<i32, Physical> {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let w = (a.x - b.x).abs() + 1;
    let h = (a.y - b.y).abs() + 1;
    Rectangle::new(Point::from((x, y)), Size::from((w, h)))
}

/// Build the render-element list for ONE output while the
/// selector is active. Called from `udev::build_render_elements`
/// (the live render path) for every output every frame.
///
/// Element order (first-pushed = top-most z):
///   1. Live cursor (so the user can see where they're clicking)
///   2. Help bar at the bottom-centre of the active output
///   3. Selection corner handles (4 small white squares)
///   4. Selection border (4 thin white rects)
///   5. Dim strips (4 black-50% rects around selection)
///   6. Frozen scene background
pub fn build_render_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    selector: &RegionSelector,
) -> Vec<MargoRenderElement> {
    let name = od.output.name();
    let Some(frozen) = selector.frozen.get(&name) else {
        let mut v = cover_dark(od);
        // Even on outputs that came online after the selector
        // opened, render the cursor so the user isn't lost.
        let mut cursor = build_cursor_elements(renderer, od, state, true);
        cursor.append(&mut v);
        return cursor;
    };

    let scale = Scale::from(frozen.scale);
    let mut elements: Vec<MargoRenderElement> = Vec::with_capacity(20);

    // 1. Cursor on top.
    let cursor = build_cursor_elements(renderer, od, state, true);
    elements.extend(cursor);

    let active = selector.active_output == name;
    let rect_phys = if active {
        Some(rect_from_corners(selector.a, selector.b))
    } else {
        None
    };

    // 2. Help bar — only on the active output, anchored at the
    //    bottom-centre. Doesn't render on inactive outputs to
    //    avoid duplication.
    if active {
        push_help_bar(
            &mut elements,
            frozen.logical_size,
            scale,
            selector.include_pointer,
        );
    }

    if let Some(rect_phys) = rect_phys {
        let rect_logical = phys_to_logical(rect_phys, scale, frozen.logical_size);
        // 3. Corner handles — 4 small filled squares at each
        //    corner. Reinforces the selection edges visually,
        //    especially when the rect is tiny or near monitor
        //    edges where dim strips collapse to zero.
        push_corner_handles(&mut elements, rect_logical, scale);
        // 4. Border lines.
        push_selection_border(&mut elements, rect_logical, scale);
        // 5. Dim strips.
        push_dim_strips(&mut elements, rect_logical, frozen.logical_size, scale);
    } else {
        push_full_dim(&mut elements, frozen.logical_size, scale);
    }

    // 6. Frozen background.
    elements.push(frozen_background(renderer, frozen));
    elements
}

fn cover_dark(od: &OutputDevice) -> Vec<MargoRenderElement> {
    let Some(mode) = od.output.current_mode() else {
        return Vec::new();
    };
    let scale_f = od.output.current_scale().fractional_scale();
    let scale = Scale::from(scale_f);
    let size = Size::<i32, Logical>::from((
        (mode.size.w as f64 / scale_f).round() as i32,
        (mode.size.h as f64 / scale_f).round() as i32,
    ));
    let mut v = Vec::new();
    push_full_dim(&mut v, size, scale);
    v
}

fn phys_to_logical(
    rect: Rectangle<i32, Physical>,
    scale: Scale<f64>,
    output_logical: Size<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let s = scale.x.max(0.001);
    let x = (rect.loc.x as f64 / s).round() as i32;
    let y = (rect.loc.y as f64 / s).round() as i32;
    let w = (rect.size.w as f64 / s).round() as i32;
    let h = (rect.size.h as f64 / s).round() as i32;
    let x = x.clamp(0, output_logical.w);
    let y = y.clamp(0, output_logical.h);
    let w = w.min(output_logical.w - x).max(1);
    let h = h.min(output_logical.h - y).max(1);
    Rectangle::new(Point::from((x, y)), Size::from((w, h)))
}

fn push_full_dim(
    out: &mut Vec<MargoRenderElement>,
    logical_size: Size<i32, Logical>,
    scale: Scale<f64>,
) {
    let dim = solid(
        (0, 0),
        (logical_size.w, logical_size.h),
        scale,
        [0.0, 0.0, 0.0, 0.5],
    );
    out.push(MargoRenderElement::Solid(dim));
}

fn push_dim_strips(
    out: &mut Vec<MargoRenderElement>,
    sel: Rectangle<i32, Logical>,
    logical_size: Size<i32, Logical>,
    scale: Scale<f64>,
) {
    let dim = [0.0, 0.0, 0.0, 0.5];
    let w = logical_size.w;
    let h = logical_size.h;
    let sx = sel.loc.x.clamp(0, w);
    let sy = sel.loc.y.clamp(0, h);
    let sx2 = (sel.loc.x + sel.size.w).clamp(0, w);
    let sy2 = (sel.loc.y + sel.size.h).clamp(0, h);

    if sy > 0 {
        out.push(MargoRenderElement::Solid(solid((0, 0), (w, sy), scale, dim)));
    }
    if sy2 < h {
        out.push(MargoRenderElement::Solid(solid(
            (0, sy2),
            (w, h - sy2),
            scale,
            dim,
        )));
    }
    if sx > 0 {
        out.push(MargoRenderElement::Solid(solid(
            (0, sy),
            (sx, sy2 - sy),
            scale,
            dim,
        )));
    }
    if sx2 < w {
        out.push(MargoRenderElement::Solid(solid(
            (sx2, sy),
            (w - sx2, sy2 - sy),
            scale,
            dim,
        )));
    }
}

fn push_corner_handles(
    out: &mut Vec<MargoRenderElement>,
    sel: Rectangle<i32, Logical>,
    scale: Scale<f64>,
) {
    // 8×8 logical-pixel filled squares at each corner. Drawn
    // OUTSIDE the selection rect so they don't obscure the
    // captured pixels — but they still indicate the corners
    // clearly.
    let handle: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    let s: i32 = 8;
    let x0 = sel.loc.x;
    let y0 = sel.loc.y;
    let x1 = sel.loc.x + sel.size.w;
    let y1 = sel.loc.y + sel.size.h;
    out.push(MargoRenderElement::Solid(solid(
        (x0 - s / 2, y0 - s / 2),
        (s, s),
        scale,
        handle,
    )));
    out.push(MargoRenderElement::Solid(solid(
        (x1 - s / 2, y0 - s / 2),
        (s, s),
        scale,
        handle,
    )));
    out.push(MargoRenderElement::Solid(solid(
        (x0 - s / 2, y1 - s / 2),
        (s, s),
        scale,
        handle,
    )));
    out.push(MargoRenderElement::Solid(solid(
        (x1 - s / 2, y1 - s / 2),
        (s, s),
        scale,
        handle,
    )));
}

/// 5×7 bitmap font for ASCII characters used in the help bar.
/// Each glyph is 7 rows of 5 bits (high bit first). Embedded
/// directly so we don't need pango/cairo or fontdue. Saves
/// ~500KB of binary growth and ~200KB of font asset.
const FONT_WIDTH: i32 = 5;
const FONT_HEIGHT: i32 = 7;

const fn glyph(rows: [u8; 7]) -> [u8; 7] {
    rows
}

#[rustfmt::skip]
fn glyph_for(c: char) -> Option<[u8; 7]> {
    Some(match c {
        ' ' => glyph([0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000]),
        '[' => glyph([0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110]),
        ']' => glyph([0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110]),
        '·' => glyph([0b00000, 0b00000, 0b00000, 0b00100, 0b00000, 0b00000, 0b00000]),
        '•' => glyph([0b00000, 0b00000, 0b01110, 0b01110, 0b01110, 0b00000, 0b00000]),
        ':' => glyph([0b00000, 0b00100, 0b00000, 0b00000, 0b00000, 0b00100, 0b00000]),
        ',' => glyph([0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000]),
        '.' => glyph([0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00000]),
        // Capital letters used in keyboard hints.
        'E' => glyph([0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
        'P' => glyph([0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
        'R' => glyph([0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
        'S' => glyph([0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
        'C' => glyph([0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
        // Lowercase used in 'cancel', 'save', 'pointer', 'toggle'.
        'a' => glyph([0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111]),
        'b' => glyph([0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110]),
        'c' => glyph([0b00000, 0b00000, 0b01110, 0b10001, 0b10000, 0b10001, 0b01110]),
        'd' => glyph([0b00001, 0b00001, 0b01111, 0b10001, 0b10001, 0b10001, 0b01111]),
        'e' => glyph([0b00000, 0b00000, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110]),
        'f' => glyph([0b00110, 0b01001, 0b01000, 0b11110, 0b01000, 0b01000, 0b01000]),
        'g' => glyph([0b00000, 0b00000, 0b01111, 0b10001, 0b01111, 0b00001, 0b01110]),
        'h' => glyph([0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001]),
        'i' => glyph([0b00100, 0b00000, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110]),
        'k' => glyph([0b10000, 0b10000, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010]),
        'l' => glyph([0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
        'n' => glyph([0b00000, 0b00000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001]),
        'o' => glyph([0b00000, 0b00000, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110]),
        'p' => glyph([0b00000, 0b00000, 0b11110, 0b10001, 0b11110, 0b10000, 0b10000]),
        'r' => glyph([0b00000, 0b00000, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000]),
        's' => glyph([0b00000, 0b00000, 0b01111, 0b10000, 0b01110, 0b00001, 0b11110]),
        't' => glyph([0b01000, 0b01000, 0b11110, 0b01000, 0b01000, 0b01001, 0b00110]),
        'u' => glyph([0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b10001, 0b01111]),
        'v' => glyph([0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
        'w' => glyph([0b00000, 0b00000, 0b10001, 0b10001, 0b10101, 0b11011, 0b10001]),
        'x' => glyph([0b00000, 0b00000, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001]),
        'y' => glyph([0b00000, 0b00000, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110]),
        'z' => glyph([0b00000, 0b00000, 0b11111, 0b00010, 0b00100, 0b01000, 0b11111]),
        _ => return None,
    })
}

/// Push the help bar at the bottom-centre of the output. A dark
/// rounded panel containing keyboard hints rendered via the
/// embedded bitmap font.
fn push_help_bar(
    out: &mut Vec<MargoRenderElement>,
    output_logical_size: Size<i32, Logical>,
    scale: Scale<f64>,
    show_pointer: bool,
) {
    let pointer_text = if show_pointer { "P hide pointer" } else { "P show pointer" };
    let text = format!(
        "Enter save   Esc cancel   {}",
        pointer_text
    );

    // Each glyph is 5×7 logical pixels at scale 1; we draw at
    // scale 4 (20×28) so the help bar reads cleanly even on
    // 1080p screens. Bumped from 2 after user feedback that
    // the bar was invisible at 14 px tall.
    let glyph_scale: i32 = 4;
    let glyph_w = FONT_WIDTH * glyph_scale;
    let glyph_h = FONT_HEIGHT * glyph_scale;
    let advance = glyph_w + 4; // 4-px space between glyphs

    let chars: Vec<char> = text.chars().collect();
    let text_w = chars.len() as i32 * advance;
    let text_h = glyph_h;

    let pad_x: i32 = 28;
    let pad_y: i32 = 18;
    let panel_w = text_w + pad_x * 2;
    let panel_h = text_h + pad_y * 2;
    let panel_x = (output_logical_size.w - panel_w) / 2;
    let panel_y = output_logical_size.h - panel_h - 40; // 40 px from bottom

    // Panel background — semi-opaque dark.
    let bg: [f32; 4] = [0.10, 0.10, 0.12, 0.88];
    out.push(MargoRenderElement::Solid(solid(
        (panel_x, panel_y),
        (panel_w, panel_h),
        scale,
        bg,
    )));

    // Top thin highlight line for contrast.
    let highlight: [f32; 4] = [1.0, 1.0, 1.0, 0.10];
    out.push(MargoRenderElement::Solid(solid(
        (panel_x, panel_y),
        (panel_w, 1),
        scale,
        highlight,
    )));

    // Render each glyph as a stack of solid rectangles for the
    // ON pixels. Cap text at output width.
    let fg: [f32; 4] = [0.95, 0.95, 0.95, 1.0];
    let text_x_start = panel_x + pad_x;
    let text_y_start = panel_y + pad_y;
    for (i, c) in chars.iter().enumerate() {
        let Some(g) = glyph_for(*c) else { continue };
        let gx = text_x_start + (i as i32) * advance;
        for (row_idx, row_bits) in g.iter().enumerate() {
            for col in 0..FONT_WIDTH {
                if row_bits & (1 << (FONT_WIDTH - 1 - col)) != 0 {
                    let px = gx + col * glyph_scale;
                    let py = text_y_start + (row_idx as i32) * glyph_scale;
                    out.push(MargoRenderElement::Solid(solid(
                        (px, py),
                        (glyph_scale, glyph_scale),
                        scale,
                        fg,
                    )));
                }
            }
        }
    }
}

fn push_selection_border(
    out: &mut Vec<MargoRenderElement>,
    sel: Rectangle<i32, Logical>,
    scale: Scale<f64>,
) {
    // 2-px white border, four thin rects.
    let stroke: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    let bw: i32 = 2;
    let x = sel.loc.x;
    let y = sel.loc.y;
    let w = sel.size.w;
    let h = sel.size.h;

    out.push(MargoRenderElement::Solid(solid((x, y), (w, bw), scale, stroke)));
    out.push(MargoRenderElement::Solid(solid(
        (x, y + h - bw),
        (w, bw),
        scale,
        stroke,
    )));
    out.push(MargoRenderElement::Solid(solid((x, y), (bw, h), scale, stroke)));
    out.push(MargoRenderElement::Solid(solid(
        (x + w - bw, y),
        (bw, h),
        scale,
        stroke,
    )));
}

fn solid(
    loc: (i32, i32),
    size: (i32, i32),
    scale: Scale<f64>,
    colour: [f32; 4],
) -> SolidColorRenderElement {
    let buffer = SolidColorBuffer::new(
        Size::<i32, Logical>::from((size.0.max(0), size.1.max(0))),
        colour,
    );
    let geo = Rectangle::<i32, Logical>::new(
        Point::from((loc.0, loc.1)),
        Size::from((size.0.max(0), size.1.max(0))),
    )
    .to_physical_precise_round::<f64, i32>(scale);
    SolidColorRenderElement::from_buffer(
        &buffer,
        geo.loc,
        scale,
        1.0,
        Kind::Unspecified,
    )
}

fn frozen_background(
    renderer: &mut GlesRenderer,
    frozen: &FrozenOutput,
) -> MargoRenderElement {
    // The texture was rendered at the output's physical mode
    // size, so its buffer-pixel-to-physical-pixel mapping is
    // 1:1. We pass scale=1 to TextureBuffer (the smithay
    // signature is `scale: i32`); this tells smithay "1 buffer
    // pixel = 1 logical pixel". The rendered location is (0, 0)
    // in physical coords — the texture fills the output exactly.
    //
    // For HiDPI outputs (scale > 1), we override the rendered
    // size so the texture fills the output's LOGICAL area and
    // gets implicitly upscaled at composite time.
    let buffer = TextureBuffer::from_texture(
        renderer,
        frozen.texture.clone(),
        1,
        Transform::Normal,
        None,
    );
    let logical_size = Size::<i32, Logical>::from((
        frozen.logical_size.w.max(1),
        frozen.logical_size.h.max(1),
    ));
    MargoRenderElement::Texture(TextureRenderElement::from_texture_buffer(
        Point::<f64, Physical>::from((0.0, 0.0)),
        &buffer,
        Some(1.0),
        None,
        Some(logical_size),
        Kind::Unspecified,
    ))
}

/// Pointer event handler. `pointer_global` is the global
/// compositor cursor position in logical pixels.
/// `button_press`: `Some(true)` = down, `Some(false)` = up,
/// `None` = motion only.
pub fn handle_pointer(
    selector: &mut RegionSelector,
    pointer_global: (f64, f64),
    button_press: Option<bool>,
) -> HandleResult {
    let (px, py) = (
        pointer_global.0.round() as i32,
        pointer_global.1.round() as i32,
    );

    // Snap the active output to wherever the cursor is.
    let mut new_active: Option<String> = None;
    for (name, frozen) in &selector.frozen {
        let x0 = frozen.logical_origin.x;
        let y0 = frozen.logical_origin.y;
        let x1 = x0 + frozen.logical_size.w;
        let y1 = y0 + frozen.logical_size.h;
        if px >= x0 && px < x1 && py >= y0 && py < y1 {
            new_active = Some(name.clone());
            break;
        }
    }
    if let Some(name) = new_active {
        if name != selector.active_output {
            // Cursor crossed onto a different output. Reset to
            // a centred default on the new output and abort any
            // in-progress drag.
            selector.active_output = name.clone();
            if let Some(frozen) = selector.frozen.get(&name) {
                let (a, b) = default_selection(frozen.size);
                selector.a = a;
                selector.b = b;
            }
            selector.button = Button::Up;
        }
    }

    // Map cursor → active-output PHYSICAL coords.
    let frozen = match selector.frozen.get(&selector.active_output) {
        Some(f) => f,
        None => return HandleResult::Consumed,
    };
    let scale_f = frozen.scale.max(0.001);
    let local_logical = (
        px - frozen.logical_origin.x,
        py - frozen.logical_origin.y,
    );
    let local_phys = Point::<i32, Physical>::from((
        ((local_logical.0 as f64) * scale_f).round() as i32,
        ((local_logical.1 as f64) * scale_f).round() as i32,
    ));
    let clamped = Point::<i32, Physical>::from((
        local_phys.x.clamp(0, frozen.size.w - 1),
        local_phys.y.clamp(0, frozen.size.h - 1),
    ));

    match button_press {
        Some(true) => {
            // Press: branch on whether the click is inside the
            // existing selection (move) or outside (resize from
            // click point).
            let current = rect_from_corners(selector.a, selector.b);
            let inside = clamped.x >= current.loc.x
                && clamped.x < current.loc.x + current.size.w
                && clamped.y >= current.loc.y
                && clamped.y < current.loc.y + current.size.h;
            if inside {
                let cursor_offset = clamped - current.loc;
                selector.button = Button::Down {
                    mode: DragMode::Move { cursor_offset },
                };
            } else {
                selector.a = clamped;
                selector.b = clamped;
                selector.button = Button::Down {
                    mode: DragMode::Resize,
                };
            }
        }
        Some(false) => {
            // Release: keep the selection where it is, exit
            // drag state.
            selector.button = Button::Up;
        }
        None => {
            // Motion: translate or resize per button mode.
            if let Button::Down { mode } = &selector.button {
                match mode {
                    DragMode::Move { cursor_offset } => {
                        let current = rect_from_corners(selector.a, selector.b);
                        let new_loc = clamped - *cursor_offset;
                        // Clamp so the rect can't escape the
                        // output bounds.
                        let max_x = (frozen.size.w - current.size.w).max(0);
                        let max_y = (frozen.size.h - current.size.h).max(0);
                        let nx = new_loc.x.clamp(0, max_x);
                        let ny = new_loc.y.clamp(0, max_y);
                        selector.a = Point::from((nx, ny));
                        selector.b = Point::from((
                            nx + current.size.w - 1,
                            ny + current.size.h - 1,
                        ));
                    }
                    DragMode::Resize => {
                        selector.b = clamped;
                    }
                }
            }
        }
    }

    HandleResult::Consumed
}

/// Keyboard event handler. `Esc` cancels. `Return` confirms.
/// `P` toggles whether the saved screenshot embeds the live
/// pointer cursor (default: off). Anything else is consumed
/// silently so compositor keybinds don't fire while the
/// selector is open.
pub fn handle_key(selector: &mut RegionSelector, keysym: Keysym, pressed: bool) -> HandleResult {
    if !pressed {
        return HandleResult::Consumed;
    }

    if keysym == Keysym::Escape {
        debug!("region selector: cancelled by Esc");
        return HandleResult::Close { save: None };
    }

    // P → toggle "include pointer" hint shown in the help bar
    // (and used in the SAVED screenshot when we wire it up).
    if keysym == Keysym::p || keysym == Keysym::P {
        selector.include_pointer = !selector.include_pointer;
        debug!(
            "region selector: include_pointer toggled → {}",
            selector.include_pointer
        );
        return HandleResult::Consumed;
    }

    if keysym == Keysym::Return || keysym == Keysym::KP_Enter {
        let rect = rect_from_corners(selector.a, selector.b);
        if rect.size.w <= 0 || rect.size.h <= 0 {
            debug!("region selector: empty selection on Return — cancelling");
            return HandleResult::Close { save: None };
        }

        let Some(frozen) = selector.frozen.get(&selector.active_output) else {
            return HandleResult::Close { save: None };
        };

        let confirm = ConfirmSave {
            texture: frozen.texture.clone(),
            rect_physical: rect,
            save_to_disk: selector.save_to_disk,
            save_path: selector.save_path.clone(),
            copy_clipboard: selector.copy_clipboard,
        };
        info!(
            "region selector: confirmed {}x{} on `{}`",
            rect.size.w, rect.size.h, selector.active_output
        );
        return HandleResult::Close {
            save: Some(confirm),
        };
    }

    HandleResult::Consumed
}

/// Public helper for the input handler: given a closing
/// selector, stash its current rectangle as the
/// `last_screenshot_region` for next-time-default. Called both
/// on confirm and on cancel — niri-pattern: the last drawn rect
/// survives Esc cancellation so the user can re-open and
/// re-confirm if they hit Esc by mistake.
pub fn stash_last_selection(selector: &RegionSelector, state: &mut MargoState) {
    let rect = rect_from_corners(selector.a, selector.b);
    state.last_screenshot_region =
        Some((selector.active_output.clone(), rect));
}
