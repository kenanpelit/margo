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
        }
    }
}

impl ClientBorder {
    pub fn update(&mut self, geom: Rect, width: f32, radius: f32, color: [f32; 4]) {
        if self.geom != geom || self.width != width || self.radius != radius || self.color != color {
            self.geom = geom;
            self.width = width;
            self.radius = radius;
            self.color = color;
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

pub fn refresh(state: &mut MargoState) {
    let focused = state.focused_client_idx();
    let n = state.clients.len();
    for idx in 0..n {
        let (geom, width, hide) = {
            let c = &state.clients[idx];
            let mon_idx = c.monitor;
            let visible = mon_idx < state.monitors.len()
                && c.is_visible_on(mon_idx, state.monitors[mon_idx].current_tagset());
            let hide = c.no_border || c.is_fullscreen || !visible;

            // The border normally has to wrap the *actual* on-screen
            // content, not the layout-reserved rect: Electron clients
            // (Helium browser, Spotify, Discord) silently clamp our
            // `xdg_toplevel.configure(size)` against their internal
            // min-size and render narrower than we asked, leaving a
            // 10–15 px wallpaper strip between the visible window and
            // the border drawn at `c.geom`. So in the steady state we
            // shrink the border to `window.geometry().size` whenever
            // that's smaller than the slot.
            //
            // Move animation is the exception. While `c.animation.running`
            // is true, `c.geom` is interpolated from old → target every
            // tick, but the client's buffer is locked to whichever size
            // it last acked (typically the new target, committed within a
            // frame or two of the configure). If we let the buffer-size
            // fallback fire here, the border would snap to the FINAL size
            // mid-animation while the slot is still half-way there, and
            // the user would see the border jump while the surrounding
            // tiles are still sliding. Niri's resize animation avoids
            // this by smoothly tweening the buffer size visually; we
            // can't do that yet, so the next-best thing is: trust the
            // animation, draw the border at the interpolated slot, and
            // only fall back to the actual buffer rect once the
            // animation has settled.
            let actual = c.window.geometry().size;
            let mut g = c.geom;
            let mut border_shrunk_to_actual = false;
            if !c.animation.running {
                if actual.w > 0 && actual.w < g.width {
                    g.width = actual.w;
                    border_shrunk_to_actual = true;
                }
                if actual.h > 0 && actual.h < g.height {
                    g.height = actual.h;
                    border_shrunk_to_actual = true;
                }
            }

            // Diagnostic — only fires when border deviates from the
            // layout slot or when an animation is mid-flight, so the
            // log line count stays bounded in the steady state.
            if border_shrunk_to_actual
                || c.animation.running
                || (actual.w != 0 && actual.w != c.geom.width)
            {
                tracing::info!(
                    "border[{}]: slot={}x{} actual={}x{} drawn={}x{} anim={} shrunk={}",
                    c.app_id.as_str(),
                    c.geom.width,
                    c.geom.height,
                    actual.w,
                    actual.h,
                    g.width,
                    g.height,
                    c.animation.running,
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
        let effective = if hide { 0.0 } else { width };
        let radius = state.config.border_radius as f32;
        state.clients[idx].border.update(geom, effective, radius, color);
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
        b.radius,
        b.width,
        1.0,
        b.commit,
        program,
    ))
}
