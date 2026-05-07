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

            // The border has to wrap the *actual* on-screen content, not
            // the layout-reserved rect. Electron clients (Helium browser,
            // Spotify) silently clamp our `xdg_toplevel.configure(size)`
            // against their internal min-size and end up rendering
            // narrower than we asked, which leaves a 10–15px wallpaper
            // strip between the visible window and the border drawn at
            // `c.geom`. Reading `window.geometry().size` gives us the size
            // the client actually committed; we shrink the border rect to
            // match (never grow past `c.geom` so we don't bleed into
            // adjacent scroller columns).
            let actual = c.window.geometry().size;
            let mut g = c.geom;
            if actual.w > 0 && actual.w < g.width {
                g.width = actual.w;
            }
            if actual.h > 0 && actual.h < g.height {
                g.height = actual.h;
            }

            (g, c.border_width as f32, hide)
        };
        let color = if hide { [0.0; 4] } else { color_for(state, idx, focused == Some(idx)) };
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
