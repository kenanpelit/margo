//! In-compositor region selector for screenshots.
//!
//! When the user binds `screenshot-region-ui` (or invokes it via
//! `mctl dispatch screenshot-region-ui`), this module takes over
//! the pointer + keyboard until the user picks a rectangle or
//! cancels. The selector renders a *frozen* copy of every output's
//! current scene as the background, with a dim overlay everywhere
//! except the dragged-out selection rect — so the user sees a
//! stable image to crop against (no chasing animations / cursor
//! ghosts).
//!
//! Two-stage flow:
//!
//!   1. **Open**: dispatch handler queues a [`PendingOpen`] request
//!      onto `MargoState`. The udev repaint hook drains that on the
//!      next frame, captures every output's current render-element
//!      list into a `GlesTexture`, builds the selector state, and
//!      assigns it to `MargoState::region_selector`.
//!   2. **Active**: the live render path swaps to
//!      [`build_render_elements`] (this module) which produces a
//!      `Vec<MargoRenderElement>` of frozen-texture backgrounds +
//!      dim-strip overlays + selection-border lines. Pointer and
//!      keyboard events are intercepted at the top of
//!      `input_handler::handle_pointer_*` / `handle_keyboard` and
//!      routed through this module's [`handle_pointer`] /
//!      [`handle_key`] handlers instead of the normal client path.
//!   3. **Confirm / cancel**: Return finalises and queues a regular
//!      `ScreenshotSource::Region` capture; Esc clears the state
//!      without saving.
//!
//! ## Design choices
//!
//! * **Single-output selection**. The selection rect lives in the
//!   coordinates of *one* output (the one the cursor was on at
//!   open time). Niri makes the same choice for the same reason —
//!   cross-output rectangles are well-defined for a global render
//!   space but fall apart on mixed-scale outputs (a rect that
//!   crosses a 1.0-scale and a 1.5-scale output has no clean
//!   physical pixel meaning). Other outputs still freeze + dim, so
//!   moving the cursor reveals the second monitor's frozen scene
//!   too — but the active rectangle stays on one screen.
//!
//! * **No animation / no help text**. niri's selector does pango-
//!   rendered Pango help and an open/close fade animation. We skip
//!   both: the keybind hint comes from the user's config (they
//!   already know they pressed Print to get here), and the
//!   open/close is instant. Cuts ~600 LOC of pango+animation
//!   plumbing.
//!
//! * **Mouse-only drag, keyboard-only confirm/cancel**. niri also
//!   supports keyboard nudge (`Shift+arrow`) for sub-pixel tweaks.
//!   Phase 4 territory; not worth the code today.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::input::keyboard::Keysym;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform};
use tracing::{debug, info};

use crate::backend::udev::{
    build_render_elements_inner, MargoRenderElement, OutputDevice,
};
use crate::screencasting::render_helpers::create_texture;
use crate::screenshot::{ScreenshotRequest, ScreenshotSource};
use crate::state::MargoState;

/// Pushed by the dispatch handler when the user invokes
/// `screenshot-region-ui`. The udev hook drains this on the next
/// repaint and turns it into a [`RegionSelector`] (which then
/// lives on `state.region_selector` until the user finishes).
#[derive(Debug, Clone)]
pub struct PendingOpen {
    pub save_to_disk: bool,
    pub save_path: Option<PathBuf>,
    pub copy_clipboard: bool,
    pub include_pointer: bool,
}

/// Per-output frozen scene captured at open time. The texture is
/// the output's full render-element tree rasterised into a single
/// GLES texture; we draw it as a `MargoRenderElement::Texture`
/// underneath the dim overlay.
#[allow(dead_code)] // size + logical_origin are reserved for Phase 4 multi-output drags
pub struct FrozenOutput {
    pub texture: GlesTexture,
    pub size: Size<i32, Physical>,
    pub scale: f64,
    /// The output's logical-space top-left in the global
    /// coordinate space. Used to convert pointer global coords
    /// back to output-local for the selection.
    pub logical_origin: Point<i32, Logical>,
    /// Output's logical size — matches the dim-strip rectangles.
    pub logical_size: Size<i32, Logical>,
}

/// Live selector state. Pointer + keyboard events are routed
/// through `handle_pointer` / `handle_key`; render path queries
/// `build_render_elements` to get the overlay element list.
pub struct RegionSelector {
    /// Frozen scene per output (keyed by connector name).
    pub frozen: HashMap<String, FrozenOutput>,
    /// Connector name of the output the active selection is on.
    /// Updated as the cursor enters another output (the new
    /// output becomes "active" and the old selection clears).
    pub active_output: String,
    /// Current selection in active-output local logical coords.
    /// `None` until the user has clicked once.
    pub selection: Option<LogicalRect>,
    /// True while a mouse button is held (drag-out in progress).
    pub dragging: bool,
    /// Carried into the save when the user hits Return.
    pub save_to_disk: bool,
    pub save_path: Option<PathBuf>,
    pub copy_clipboard: bool,
    pub include_pointer: bool,
}

/// Output-local logical rect (x1, y1, x2, y2 — top-left + bottom-
/// right). Stored as raw points rather than a normalised
/// `Rectangle` so we know which corner the user clicked first and
/// can render the drag direction correctly.
#[derive(Debug, Clone, Copy)]
pub struct LogicalRect {
    pub anchor: (i32, i32),
    pub current: (i32, i32),
}

impl LogicalRect {
    fn normalised(self) -> Rectangle<i32, Logical> {
        let (ax, ay) = self.anchor;
        let (bx, by) = self.current;
        let x = ax.min(bx);
        let y = ay.min(by);
        let w = (ax - bx).unsigned_abs() as i32;
        let h = (ay - by).unsigned_abs() as i32;
        Rectangle::new((x, y).into(), (w.max(1), h.max(1)).into())
    }
}

/// What the active-state input handlers return so the caller can
/// decide whether to fall through to normal client routing.
pub enum HandleResult {
    /// Selector consumed the event; don't deliver to clients.
    Consumed,
    /// Selector wants to be torn down (Esc / Return). Caller
    /// must clear `state.region_selector` and, if `Some(_)`,
    /// queue the screenshot.
    Close(Option<ScreenshotRequest>),
}

/// Capture every active output's current scene into a frozen
/// `GlesTexture` + build a [`RegionSelector`]. Called from the
/// udev repaint hook when [`PendingOpen`] has been pushed onto
/// state. Returns the new selector so the caller can install it
/// onto `state.region_selector`.
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

        let elements: Vec<MargoRenderElement> =
            build_render_elements_inner(renderer, od, state, true, false);

        // Render into a fresh GLES texture sized to the output's
        // mode. Using `render_helpers::create_texture` + smithay's
        // damage-tracker render is overkill here (we only do this
        // once per open), so we inline the bind+render+forget
        // path.
        use smithay::backend::renderer::Bind;
        let mut texture = create_texture(renderer, size, Fourcc::Abgr8888)
            .context("create frozen texture")?;
        {
            let mut target = renderer
                .bind(&mut texture)
                .context("bind frozen texture")?;
            // Use a damage tracker per call so we get a clean
            // first-frame render (None damage = full output).
            use smithay::backend::renderer::damage::OutputDamageTracker;
            let mut dt = OutputDamageTracker::new(size, scale, Transform::Normal);
            dt.render_output(
                renderer,
                &mut target,
                0,
                &elements,
                smithay::backend::renderer::Color32F::TRANSPARENT,
            )
            .context("render frozen scene")?;
        }

        // Logical origin/size: needed to map global pointer
        // coords back to output-local for the selection state.
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

    info!(
        "region selector opened: {} output(s), active = `{}`",
        frozen.len(),
        active_output
    );

    Ok(RegionSelector {
        frozen,
        active_output,
        selection: None,
        dragging: false,
        save_to_disk: request.save_to_disk,
        save_path: request.save_path,
        copy_clipboard: request.copy_clipboard,
        include_pointer: request.include_pointer,
    })
}

/// Build the per-output element list for the live render path
/// while the selector is active. Replaces
/// `build_render_elements_inner`'s output for *every* output —
/// the call site already iterates outputs, so we accept one
/// `OutputDevice` and return its overlay.
pub fn build_render_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    selector: &RegionSelector,
    pointer_global: (f64, f64),
) -> Vec<MargoRenderElement> {
    let name = od.output.name();
    let Some(frozen) = selector.frozen.get(&name) else {
        // Output came online after the selector opened —
        // nothing frozen to draw. Render an opaque dim cover
        // so the live scene doesn't bleed through and confuse
        // the user; they can still cancel with Esc.
        return cover_dark(od);
    };

    let scale = Scale::from(frozen.scale);
    let mut elements: Vec<MargoRenderElement> = Vec::with_capacity(10);

    // Top → bottom in element order. First push = top-most.
    //
    // 1. Selection border lines (4 thin rects).
    // 2. Dim overlay strips (4 rects around the selection).
    //    These sit BELOW the border so the border edge looks
    //    crisp.
    // 3. Frozen-scene texture (full-output background).
    let active = selector.active_output == name;
    if active {
        if let Some(rect) = selector.selection {
            push_selection_border(&mut elements, rect.normalised(), scale);
            push_dim_strips(&mut elements, rect.normalised(), frozen.logical_size, scale);
        } else {
            // No drag yet on this output — full dim cover so
            // the user knows they're in capture mode but
            // haven't picked a rect.
            push_full_dim(&mut elements, frozen.logical_size, scale);
        }
    } else {
        // Inactive output: full-cover dim. Cursor moving over
        // this output activates it on the next pointer event.
        push_full_dim(&mut elements, frozen.logical_size, scale);
    }

    // Frozen background (always last → bottom-most).
    let _ = pointer_global; // reserved for cursor render — Phase 4
    elements.push(frozen_background(renderer, frozen, scale));
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
    // Four strips around the selection; each is one
    // SolidColorRenderElement at 50% black.
    let dim = [0.0, 0.0, 0.0, 0.5];
    let w = logical_size.w;
    let h = logical_size.h;
    let sx = sel.loc.x.clamp(0, w);
    let sy = sel.loc.y.clamp(0, h);
    let sx2 = (sel.loc.x + sel.size.w).clamp(0, w);
    let sy2 = (sel.loc.y + sel.size.h).clamp(0, h);

    // Top strip: full width × (sy)
    if sy > 0 {
        out.push(MargoRenderElement::Solid(solid((0, 0), (w, sy), scale, dim)));
    }
    // Bottom strip: full width × (h - sy2)
    if sy2 < h {
        out.push(MargoRenderElement::Solid(solid(
            (0, sy2),
            (w, h - sy2),
            scale,
            dim,
        )));
    }
    // Left strip: (sx) × selection-height
    if sx > 0 {
        out.push(MargoRenderElement::Solid(solid(
            (0, sy),
            (sx, sy2 - sy),
            scale,
            dim,
        )));
    }
    // Right strip: (w - sx2) × selection-height
    if sx2 < w {
        out.push(MargoRenderElement::Solid(solid(
            (sx2, sy),
            (w - sx2, sy2 - sy),
            scale,
            dim,
        )));
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

    // Top
    out.push(MargoRenderElement::Solid(solid(
        (x, y),
        (w, bw),
        scale,
        stroke,
    )));
    // Bottom
    out.push(MargoRenderElement::Solid(solid(
        (x, y + h - bw),
        (w, bw),
        scale,
        stroke,
    )));
    // Left
    out.push(MargoRenderElement::Solid(solid(
        (x, y),
        (bw, h),
        scale,
        stroke,
    )));
    // Right
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
    _renderer: &mut GlesRenderer,
    frozen: &FrozenOutput,
    _scale: Scale<f64>,
) -> MargoRenderElement {
    // Wrap the captured GLES texture in a TextureBuffer + place
    // it at output origin (0, 0). The texture is already sized
    // to the output's physical mode, so we draw at scale 1 in
    // physical space — but the surrounding overlay rectangles
    // are computed in logical space and converted via `scale`.
    // To make both line up, we draw the texture at logical
    // (0, 0) with size = output's logical size and let the
    // physical-rounding match.
    let buffer = TextureBuffer::from_texture(
        _renderer,
        frozen.texture.clone(),
        1,
        Transform::Normal,
        None,
    );
    let pos = Point::<f64, Physical>::from((0.0, 0.0));
    MargoRenderElement::Texture(TextureRenderElement::from_texture_buffer(
        pos,
        &buffer,
        Some(1.0),
        None,
        None,
        Kind::Unspecified,
    ))
}

/// Pointer event handler. Coords are global (the same coords the
/// rest of margo's pointer logic uses). Returns `Consumed` for
/// most events; the caller suppresses normal client routing on
/// that.
pub fn handle_pointer(
    selector: &mut RegionSelector,
    pointer_global: (f64, f64),
    button_press: Option<bool>,
) -> HandleResult {
    // Resolve which output the pointer is on now. We snap to the
    // first frozen output whose logical rect contains the cursor.
    let (px, py) = (pointer_global.0.round() as i32, pointer_global.1.round() as i32);
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
            // Cursor crossed onto a different output — clear
            // the in-progress selection so the next click
            // starts fresh on the new output.
            selector.active_output = name;
            selector.selection = None;
            selector.dragging = false;
        }
    }

    let frozen = match selector.frozen.get(&selector.active_output) {
        Some(f) => f,
        None => return HandleResult::Consumed,
    };
    let local = (
        px - frozen.logical_origin.x,
        py - frozen.logical_origin.y,
    );

    match button_press {
        Some(true) => {
            // Button down — start a fresh selection at the
            // cursor.
            selector.selection = Some(LogicalRect {
                anchor: local,
                current: local,
            });
            selector.dragging = true;
        }
        Some(false) => {
            // Button up — finalise the drag but keep the
            // selection visible (Return confirms, Esc cancels).
            if let Some(rect) = selector.selection.as_mut() {
                rect.current = local;
            }
            selector.dragging = false;
        }
        None => {
            // Plain motion. Update the live edge while
            // dragging; just track the cursor otherwise (so
            // crossing outputs re-arms).
            if selector.dragging {
                if let Some(rect) = selector.selection.as_mut() {
                    rect.current = local;
                }
            }
        }
    }

    HandleResult::Consumed
}

/// Keyboard event handler. Esc cancels, Return confirms.
/// Anything else → Consumed (so passthrough doesn't accidentally
/// trigger compositor keybinds while the selector is open).
pub fn handle_key(
    selector: &mut RegionSelector,
    keysym: Keysym,
    pressed: bool,
) -> HandleResult {
    if !pressed {
        return HandleResult::Consumed;
    }

    if keysym == Keysym::Escape {
        return HandleResult::Close(None);
    }

    if keysym == Keysym::Return || keysym == Keysym::KP_Enter {
        let Some(rect) = selector.selection else {
            // No selection drawn → Esc-equivalent.
            return HandleResult::Close(None);
        };
        let r = rect.normalised();
        if r.size.w <= 0 || r.size.h <= 0 {
            return HandleResult::Close(None);
        }
        let request = ScreenshotRequest {
            source: ScreenshotSource::Region {
                output: selector.active_output.clone(),
                x: r.loc.x,
                y: r.loc.y,
                width: r.size.w,
                height: r.size.h,
            },
            include_pointer: selector.include_pointer,
            save_to_disk: selector.save_to_disk,
            save_path: selector.save_path.clone(),
            copy_clipboard: selector.copy_clipboard,
        };
        debug!("region selector confirm: {:?}", request);
        return HandleResult::Close(Some(request));
    }

    HandleResult::Consumed
}

