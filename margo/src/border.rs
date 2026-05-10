//! Per-window border rendering.
//!
//! Uses `RoundedBorderElement` (a GLES pixel shader) to draw anti-aliased
//! rounded rectangles.

use smithay::{
    backend::renderer::element::Id,
    backend::renderer::utils::CommitCounter,
    utils::{Logical, Point, Rectangle, Scale},
};

use crate::{
    layout::Rect,
    render::rounded_border::RoundedBorderElement,
    state::{MargoClient, MargoState},
};

#[derive(Debug)]
pub struct ClientBorder {
    id: Id,
    commit: CommitCounter,
    geom: Rect,
    width: f32,
    radius: f32,
    color: [f32; 4],
    /// Secondary (inner band) colour. Equal to `color` when the
    /// dual-band feature is unused — see `render/rounded_border.rs`
    /// shader for the degeneracy that keeps single-colour render
    /// output bit-identical to pre-dual builds.
    color_secondary: [f32; 4],
    /// Width of the secondary band in logical pixels. Zero collapses
    /// the rendering to single-colour mode.
    secondary_width: f32,
}

impl Default for ClientBorder {
    fn default() -> Self {
        Self {
            id: Id::new(),
            commit: CommitCounter::default(),
            geom: Rect::default(),
            width: 0.0,
            radius: 0.0,
            color: [0.0; 4],
            color_secondary: [0.0; 4],
            secondary_width: 0.0,
        }
    }
}

impl ClientBorder {
    pub fn update(
        &mut self,
        geom: Rect,
        width: f32,
        radius: f32,
        color: [f32; 4],
        color_secondary: [f32; 4],
        secondary_width: f32,
    ) {
        if self.geom != geom
            || self.width != width
            || self.radius != radius
            || self.color != color
            || self.color_secondary != color_secondary
            || self.secondary_width != secondary_width
        {
            self.geom = geom;
            self.width = width;
            self.radius = radius;
            self.color = color;
            self.color_secondary = color_secondary;
            self.secondary_width = secondary_width;
            self.commit.increment();
        }
    }
}

fn color_for(state: &MargoState, client_idx: usize, focused: bool) -> [f32; 4] {
    let c = &state.clients[client_idx];
    let cfg = &state.config;
    if c.is_urgent {
        cfg.urgentcolor.0
    } else if c.is_overlay {
        cfg.overlaycolor.0
    } else if c.is_global {
        cfg.globalcolor.0
    } else if c.is_fullscreen {
        cfg.maximizescreencolor.0
    } else if focused {
        cfg.focuscolor.0
    } else {
        cfg.bordercolor.0
    }
}

/// Pick the secondary (inner) band colour to pair with the primary
/// returned by `color_for`. The secondary set deliberately covers
/// only the **focus / unfocus / urgent** cases — these are the
/// states a dual-tone border accentuates (the user wants their
/// focused window to stand out, or a screaming `urgent` flag to
/// have an unmistakable inner ring). Overlay / global / fullscreen
/// keep their existing single-tone look so opt-in users don't lose
/// the at-a-glance recognition those colours provide.
///
/// Falls back to the primary colour when no secondary is configured
/// — paired with the shader's two-band degeneracy this means the
/// render output stays bit-identical for single-colour configs.
fn secondary_color_for(state: &MargoState, client_idx: usize, focused: bool, primary: [f32; 4]) -> [f32; 4] {
    let c = &state.clients[client_idx];
    let cfg = &state.config;
    let secondary = if c.is_urgent {
        cfg.urgentcolor_secondary
    } else if c.is_overlay || c.is_global || c.is_fullscreen {
        None
    } else if focused {
        cfg.focuscolor_secondary
    } else {
        cfg.bordercolor_secondary
    };
    secondary.map(|c| c.0).unwrap_or(primary)
}

pub fn refresh(state: &mut MargoState) {
    let focused = state.focused_client_idx();
    let n = state.clients.len();
    for idx in 0..n {
        let (geom, width, hide) = {
            let c = &state.clients[idx];
            let mon_idx = c.monitor;
            let visible = mon_idx < state.monitors.len()
                && c.is_visible_on(mon_idx, state.monitors[mon_idx].current_tagset());
            // Hide the border for the duration of an open transition.
            // The OpenCloseRenderElement is drawing the surface scaled
            // around `c.geom`'s centre, so a full-slot border around it
            // would visibly precede the window into existence by ~180 ms.
            // Letting the border pop in at the end of the open
            // animation matches niri/Hyprland behaviour and is far less
            // jarring than the alternative.
            let hide = c.no_border
                || c.is_fullscreen
                || !visible
                || c.opening_animation.is_some();

            // The border has to wrap the *actual* on-screen content,
            // not the layout-reserved rect: Electron clients (Helium
            // browser, Spotify, Discord) silently clamp our
            // `xdg_toplevel.configure(size)` against their internal
            // min-size and render narrower than we asked, leaving a
            // wallpaper strip between the visible window and the
            // border drawn at `c.geom`.
            //
            // Three cases:
            //
            //   1. A resize-snapshot is in flight. The renderer is
            //      drawing a captured texture *scaled to `c.geom`* and
            //      hiding the live surface, so the visible content
            //      fills the interpolated slot exactly. Border tracks
            //      `c.geom`.
            //
            //   2. A move/spring animation is running with no snapshot
            //      (pure translate, dimensions unchanged). The slot's
            //      `width/height` already match the buffer; tracking
            //      `c.geom` is the same as tracking the buffer. Either
            //      works, but `c.geom` is cheaper.
            //
            //   3. Steady state OR mid-animation when the buffer
            //      doesn't match the slot. Always clamp the border
            //      box to `min(actual, slot)`. This is the case the
            //      old `!anim.running` gate left uncovered: when
            //      Helium/Spotify settle at a buffer size smaller
            //      than the slot we requested, the post-settle frame
            //      would draw the border at the slot rect even though
            //      the live surface only covered a sub-rectangle.
            // Read the client-declared geometry rect. `loc` is the
            // offset of the "drawable area" within the wl_buffer
            // (Electron clients sometimes report a non-zero loc to
            // exclude shadow / titlebar). `size` is the declared
            // visible size — what the client thinks is on-screen.
            let geom_rect = c.window.geometry();
            let actual = geom_rect.size;
            // `snapshot_pending` covers the gap between
            // arrange_monitor flagging a resize transition and the
            // renderer actually capturing a texture for it: arrange
            // runs `border::refresh` synchronously, but the snapshot
            // capture only fires on the next render. Without rolling
            // `pending` into `active`, the arrange-time refresh would
            // shrink the border to `actual` while the next render's
            // `ResizeRenderElement` paints the snapshot stretched to
            // the full slot — that's the "border ve pencere bağımsız
            // haraket ediyor" mismatch the user sees on Spotify
            // during super+r.
            let snapshot_active = c.resize_snapshot.is_some() || c.snapshot_pending;
            let mut g = c.geom;
            let mut border_shrunk_to_actual = false;
            if !snapshot_active {
                // Clamp to the visible bounds: a buffer rendered at
                // c.geom.loc covers (c.geom.loc, geometry.size); the
                // rest is clipped to the slot anyway. Clipping the
                // border to `min(actual, slot)` makes the frame hug
                // whatever the user actually sees.
                if actual.w > 0 && actual.w < g.width {
                    g.width = actual.w;
                    border_shrunk_to_actual = true;
                }
                if actual.h > 0 && actual.h < g.height {
                    g.height = actual.h;
                    border_shrunk_to_actual = true;
                }
            }

            // Diagnostic — fires when something interesting is
            // happening (deviation from slot, mid-flight animation,
            // non-zero geometry offset, or active snapshot). Includes
            // `loc` so we can spot Electron-style buffer offsets that
            // would otherwise be invisible to the existing
            // `slot vs actual` summary.
            if border_shrunk_to_actual
                || c.animation.running
                || snapshot_active
                || geom_rect.loc.x != 0
                || geom_rect.loc.y != 0
                || (actual.w != 0 && actual.w != c.geom.width)
                || (actual.h != 0 && actual.h != c.geom.height)
            {
                tracing::info!(
                    "border[{}]: slot={}x{}+{}+{} geom_loc={}+{} geom_size={}x{} drawn={}x{} \
                     anim={} snap={} shrunk={}",
                    c.app_id.as_str(),
                    c.geom.width,
                    c.geom.height,
                    c.geom.x,
                    c.geom.y,
                    geom_rect.loc.x,
                    geom_rect.loc.y,
                    actual.w,
                    actual.h,
                    g.width,
                    g.height,
                    c.animation.running,
                    snapshot_active,
                    border_shrunk_to_actual,
                );
            }

            (g, c.border_width as f32, hide)
        };
        let color = if hide {
            [0.0; 4]
        } else if state.clients[idx].opacity_animation.running
            && state.clients[idx].opacity_animation.current_border_color != [0.0, 0.0, 0.0, 0.0]
        {
            // Focus crossfade is in flight — use the interpolated
            // colour from `tick_animations`. Reading directly off the
            // animation struct keeps the cross-fade visible *between*
            // arrange passes too (every render driven by an
            // animation tick refreshes this).
            state.clients[idx].opacity_animation.current_border_color
        } else {
            color_for(state, idx, focused == Some(idx))
        };
        // Secondary band: solid colour resolved from config; not
        // crossfaded during focus transitions (the inner accent is
        // typically a calmer base — leaving it steady while the
        // primary fades in / out reads as "the highlight is moving",
        // which matches what the user sees).
        let color_secondary = if hide {
            [0.0; 4]
        } else {
            secondary_color_for(state, idx, focused == Some(idx), color)
        };
        let effective = if hide { 0.0 } else { width };
        let radius = state.config.border_radius as f32;
        // Secondary band width — clamp to the effective border width
        // so a config typo (`border_secondary_px = 99` against
        // `borderpx = 4`) can't paint past the inner edge.
        let secondary_width = if hide {
            0.0
        } else {
            (state.config.border_secondary_px as f32).min(effective)
        };
        state.clients[idx].border.update(
            geom,
            effective,
            radius,
            color,
            color_secondary,
            secondary_width,
        );
    }
}

pub fn render_elements(
    state: &MargoState,
    output_loc: Point<i32, Logical>,
    _scale: Scale<f64>,
    program: smithay::backend::renderer::gles::GlesPixelProgram,
) -> Vec<RoundedBorderElement> {
    let mut out = Vec::with_capacity(state.clients.len());
    for client in state.clients.iter() {
        if let Some(element) = render_element_for_client(client, output_loc, program.clone()) {
            out.push(element);
        }
    }
    out
}

pub fn render_element_for_client(
    client: &MargoClient,
    output_loc: Point<i32, Logical>,
    program: smithay::backend::renderer::gles::GlesPixelProgram,
) -> Option<RoundedBorderElement> {
    if client.border.width <= 0.0 || client.border.color[3] <= 0.0 {
        return None;
    }

    let b = &client.border;
    let w = b.width.ceil() as i32;

    let geom = Rectangle::new(
        (b.geom.x - w - output_loc.x, b.geom.y - w - output_loc.y).into(),
        (b.geom.width + 2 * w, b.geom.height + 2 * w).into(),
    );

    Some(RoundedBorderElement::new(
        b.id.clone(),
        geom,
        b.color,
        b.color_secondary,
        b.radius,
        b.width,
        b.secondary_width,
        1.0,
        b.commit,
        program,
    ))
}
