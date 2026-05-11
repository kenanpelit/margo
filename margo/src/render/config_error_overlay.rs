//! On-screen banner shown when a config reload trips the validator.
//! Niri-style "your config is broken, run `mctl check-config`" hint —
//! a red-bordered dark box pinned to the top-right of the active
//! output. Solid-colour only (no text yet) so the overlay can ship
//! without dragging a font rasterizer in; the message itself is
//! delivered via `notify-send` and `mctl config-errors`, the banner
//! is the visual cue that something is wrong.
//!
//! Lifecycle:
//!   * `MargoState::reload_config` sets
//!     `config_error_overlay_until = Some(now + 10s)` when the
//!     validator catches errors.
//!   * Every frame, `build_render_elements_inner` asks
//!     `ConfigErrorOverlay::render_elements` for elements while the
//!     deadline is in the future.
//!   * `MargoState::tick_animations` clears the deadline once it's
//!     passed and requests one final repaint so the banner doesn't
//!     visually linger on a stale frame.

use smithay::{
    backend::renderer::element::{
        solid::{SolidColorBuffer, SolidColorRenderElement},
        Kind,
    },
    utils::{Physical, Point, Scale},
};

// Sizing — chosen so a 480 × 90 banner reads at a glance on a
// 1080p display without dominating the corner.
const BANNER_W: i32 = 480;
const BANNER_H: i32 = 90;
const MARGIN: i32 = 24;
const BORDER_PX: i32 = 4;

// Catppuccin-ish "red alarm + dark fill" palette. Tinted slightly
// translucent so the overlay reads as overlay, not opaque chrome.
//
// NOTE: SolidColorBuffer takes pre-multiplied RGBA in 0.0–1.0.
const BG_COLOR: [f32; 4] = [0.10, 0.05, 0.06, 0.92];
const BORDER_COLOR: [f32; 4] = [0.95, 0.34, 0.45, 1.00]; // mauve-red

pub struct ConfigErrorOverlay {
    bg: SolidColorBuffer,
    border_top: SolidColorBuffer,
    border_bottom: SolidColorBuffer,
    border_left: SolidColorBuffer,
    border_right: SolidColorBuffer,
}

impl Default for ConfigErrorOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigErrorOverlay {
    pub fn new() -> Self {
        Self {
            bg: SolidColorBuffer::new((BANNER_W, BANNER_H), BG_COLOR),
            border_top: SolidColorBuffer::new((BANNER_W, BORDER_PX), BORDER_COLOR),
            border_bottom: SolidColorBuffer::new((BANNER_W, BORDER_PX), BORDER_COLOR),
            border_left: SolidColorBuffer::new((BORDER_PX, BANNER_H), BORDER_COLOR),
            border_right: SolidColorBuffer::new((BORDER_PX, BANNER_H), BORDER_COLOR),
        }
    }

    /// Build the banner's render elements for one output. `output_origin_logical`
    /// is where the output's top-left lives in the global logical coordinate
    /// space (matches `Monitor::monitor_area.{x,y}`); `output_size_logical` is
    /// the output's logical width × height. The banner anchors at the
    /// top-right corner of the output minus a `MARGIN` px gutter.
    /// Render the banner. Takes `&self` because the SolidColorBuffers
    /// were sized at construction time and never need to change —
    /// the banner is a fixed `BANNER_W × BANNER_H` rectangle, only
    /// its physical position changes per output.
    pub fn render_elements(
        &self,
        output_origin_logical: (i32, i32),
        output_size_logical: (i32, i32),
        output_scale: f64,
    ) -> Vec<SolidColorRenderElement> {
        let (ox, oy) = output_origin_logical;
        let (ow, _oh) = output_size_logical;
        let scale: Scale<f64> = Scale::from(output_scale);

        let to_phys = |x: i32, y: i32| -> Point<i32, Physical> {
            let px = ((ox + x) as f64 * output_scale).round() as i32;
            let py = ((oy + y) as f64 * output_scale).round() as i32;
            Point::from((px, py))
        };

        // Banner top-left in logical, output-local coords.
        let banner_x = ow - BANNER_W - MARGIN;
        let banner_y = MARGIN;

        // Background first, borders on top within the returned vec.
        // (vec[0] is highest z-order in the DRM compositor, so the
        // earlier we push, the more "on top" the element draws.)
        vec![
            SolidColorRenderElement::from_buffer(
                &self.bg,
                to_phys(banner_x, banner_y),
                scale,
                1.0,
                Kind::Unspecified,
            ),
            SolidColorRenderElement::from_buffer(
                &self.border_top,
                to_phys(banner_x, banner_y),
                scale,
                1.0,
                Kind::Unspecified,
            ),
            SolidColorRenderElement::from_buffer(
                &self.border_bottom,
                to_phys(banner_x, banner_y + BANNER_H - BORDER_PX),
                scale,
                1.0,
                Kind::Unspecified,
            ),
            SolidColorRenderElement::from_buffer(
                &self.border_left,
                to_phys(banner_x, banner_y),
                scale,
                1.0,
                Kind::Unspecified,
            ),
            SolidColorRenderElement::from_buffer(
                &self.border_right,
                to_phys(banner_x + BANNER_W - BORDER_PX, banner_y),
                scale,
                1.0,
                Kind::Unspecified,
            ),
        ]
    }
}
