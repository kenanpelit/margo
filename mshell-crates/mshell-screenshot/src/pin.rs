//! Pinned captures — a screenshot kept floating above every window, like a
//! sticky note of the screen (Snipaste-style "pin to screen").
//!
//! `OutputTarget::Pin` (`mshellctl screenshot region --pin`, or the
//! screenshot menu's "Pin on screen" save option) shows the capture in an
//! overlay-layer surface. A region pin opens exactly where the region was,
//! at 1:1, so it reads as a frozen piece of the screen; other captures open
//! near the output's top-left corner at the output's own scale.
//!
//! Interaction (also in the tooltip): drag to move · scroll to zoom ·
//! Ctrl+scroll for opacity · Ctrl+C copy · Ctrl+S save · `0` resets zoom and
//! opacity · Esc or right-click closes. Each pin is independent; nothing is
//! written to disk unless the user saves.

use crate::common::OutputInfo;
use crate::utils::{default_screenshot_path, find_gdk_monitor};
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::LayerShell;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// Zoom bounds and one scroll step (multiplicative).
const ZOOM_MIN: f64 = 0.1;
const ZOOM_MAX: f64 = 4.0;
const ZOOM_STEP: f64 = 1.1;
/// Opacity bounds and one Ctrl+scroll step.
const OPACITY_MIN: f64 = 0.2;
const OPACITY_STEP: f64 = 0.1;
/// How much of a pin must stay on its output while dragging (px).
const KEEP_VISIBLE: i32 = 48;
/// Where a pin without a known region opens (px from the top-left).
const DEFAULT_INSET: i32 = 48;

/// Where and how large a pin opens: output-local logical origin + size.
#[derive(Clone, Debug, PartialEq)]
pub struct PinPlacement {
    pub output: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PinPlacement {
    /// A pin for a capture with no region (window / output / full): inset
    /// from the top-left of `output`, sized from the image at its scale.
    pub fn inset(output: Option<&OutputInfo>, image_w: u32, image_h: u32) -> Self {
        let scale = output.map(|o| o.scale).filter(|s| *s > 0.0).unwrap_or(1.0);
        Self {
            output: output.map(|o| o.name.clone()),
            x: DEFAULT_INSET,
            y: DEFAULT_INSET,
            width: (f64::from(image_w) / scale).round() as i32,
            height: (f64::from(image_h) / scale).round() as i32,
        }
    }
}

/// Next zoom after one scroll notch (`up` = zoom in).
pub fn next_zoom(zoom: f64, up: bool) -> f64 {
    let z = if up {
        zoom * ZOOM_STEP
    } else {
        zoom / ZOOM_STEP
    };
    z.clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Next opacity after one Ctrl+scroll notch (`up` = more opaque).
pub fn next_opacity(opacity: f64, up: bool) -> f64 {
    let o = if up {
        opacity + OPACITY_STEP
    } else {
        opacity - OPACITY_STEP
    };
    // Round away float drift so repeated steps land on tenths.
    ((o * 10.0).round() / 10.0).clamp(OPACITY_MIN, 1.0)
}

/// Clamp a pin origin so at least [`KEEP_VISIBLE`] px of a `w × h` pin stay
/// on an `out_w × out_h` output (a pin can't be dragged out of reach).
pub fn clamp_origin(x: i32, y: i32, w: i32, h: i32, out_w: i32, out_h: i32) -> (i32, i32) {
    let keep_w = KEEP_VISIBLE.min(w.max(1));
    let keep_h = KEEP_VISIBLE.min(h.max(1));
    let min_x = keep_w - w;
    let min_y = keep_h - h;
    let max_x = (out_w - keep_w).max(min_x);
    let max_y = (out_h - keep_h).max(min_y);
    // Layer-shell margins can't go negative on an anchored edge; the
    // compositor would push the surface back. Keep it ≥ 0 there.
    (
        x.clamp(min_x.max(0), max_x.max(0)),
        y.clamp(min_y.max(0), max_y.max(0)),
    )
}

/// Show `image` pinned per `placement`. GTK main thread only.
pub fn show_pin(image: image::RgbaImage, placement: PinPlacement, outputs: &[OutputInfo]) {
    let (iw, ih) = image.dimensions();
    if iw == 0 || ih == 0 {
        return;
    }
    let image = Arc::new(image);
    let bytes = glib::Bytes::from(image.as_raw().as_slice());
    let texture = gdk::MemoryTexture::new(
        iw as i32,
        ih as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        iw as usize * 4,
    );

    let output = placement
        .output
        .as_deref()
        .and_then(|n| outputs.iter().find(|o| o.name == n));
    let (out_w, out_h) = output
        .map(|o| (o.width, o.height))
        .unwrap_or((i32::MAX / 4, i32::MAX / 4));

    let window = gtk4::Window::new();
    window.set_decorated(false);
    window.add_css_class("screenshot-pin-window");
    window.init_layer_shell();
    window.set_namespace(Some("mshell-pin"));
    window.set_layer(gtk4_layer_shell::Layer::Overlay);
    window.set_anchor(gtk4_layer_shell::Edge::Top, true);
    window.set_anchor(gtk4_layer_shell::Edge::Left, true);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);
    if let (Some(o), Some(display)) = (output, gdk::Display::default())
        && let Some(monitor) = find_gdk_monitor(&display.monitors(), o)
    {
        window.set_monitor(Some(&monitor));
    }

    let picture = gtk4::Picture::for_paintable(&texture);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk4::ContentFit::Fill);
    picture.add_css_class("screenshot-pin");
    picture.set_cursor_from_name(Some("grab"));
    picture.set_tooltip_text(Some(
        "Pinned capture — drag to move · scroll to zoom · Ctrl+scroll opacity · \
         Ctrl+C copy · Ctrl+S save · 0 reset · Esc or right-click to close",
    ));
    picture.update_property(&[gtk4::accessible::Property::Label("Pinned screen capture")]);
    window.set_child(Some(&picture));

    let base_w = placement.width.max(1);
    let base_h = placement.height.max(1);
    let zoom = Rc::new(Cell::new(1.0_f64));
    let origin = Rc::new(Cell::new(clamp_origin(
        placement.x,
        placement.y,
        base_w,
        base_h,
        out_w,
        out_h,
    )));

    let apply_size = {
        let picture = picture.clone();
        let window = window.clone();
        let zoom = zoom.clone();
        move || {
            let z = zoom.get();
            let w = (f64::from(base_w) * z).round().max(1.0) as i32;
            let h = (f64::from(base_h) * z).round().max(1.0) as i32;
            picture.set_size_request(w, h);
            // Shrinking a layer surface needs a fresh size negotiation.
            window.set_default_size(w, h);
        }
    };
    let apply_origin = {
        let window = window.clone();
        let origin = origin.clone();
        move || {
            let (x, y) = origin.get();
            window.set_margin(gtk4_layer_shell::Edge::Left, x);
            window.set_margin(gtk4_layer_shell::Edge::Top, y);
        }
    };
    apply_size();
    apply_origin();

    // Drag to move.
    let drag = gtk4::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    let drag_start = Rc::new(Cell::new((0, 0)));
    {
        let origin = origin.clone();
        let drag_start = drag_start.clone();
        let picture = picture.clone();
        drag.connect_drag_begin(move |_, _, _| {
            drag_start.set(origin.get());
            picture.set_cursor_from_name(Some("grabbing"));
        });
    }
    {
        let origin = origin.clone();
        let zoom = zoom.clone();
        let apply_origin = apply_origin.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let (sx, sy) = drag_start.get();
            let z = zoom.get();
            let w = (f64::from(base_w) * z).round() as i32;
            let h = (f64::from(base_h) * z).round() as i32;
            origin.set(clamp_origin(
                sx + dx.round() as i32,
                sy + dy.round() as i32,
                w,
                h,
                out_w,
                out_h,
            ));
            apply_origin();
        });
    }
    {
        let picture = picture.clone();
        drag.connect_drag_end(move |_, _, _| {
            picture.set_cursor_from_name(Some("grab"));
        });
    }
    picture.add_controller(drag);

    // Scroll: zoom; Ctrl+scroll: opacity.
    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::VERTICAL | gtk4::EventControllerScrollFlags::DISCRETE,
    );
    {
        let zoom = zoom.clone();
        let window = window.clone();
        let apply_size = apply_size.clone();
        scroll.connect_scroll(move |ctl, _, dy| {
            if dy == 0.0 {
                return glib::Propagation::Proceed;
            }
            let up = dy < 0.0;
            if ctl
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                window.set_opacity(next_opacity(window.opacity(), up));
            } else {
                zoom.set(next_zoom(zoom.get(), up));
                apply_size();
            }
            glib::Propagation::Stop
        });
    }
    picture.add_controller(scroll);

    // Right-click closes.
    let click = gtk4::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    {
        let window = window.clone();
        click.connect_released(move |_, _, _, _| window.close());
    }
    picture.add_controller(click);

    // Keys.
    let keys = gtk4::EventControllerKey::new();
    {
        let window = window.clone();
        let image = image.clone();
        let zoom = zoom.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::Escape => window.close(),
                gdk::Key::c if ctrl => {
                    let img = image.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = crate::copy_to_clipboard(&img) {
                            eprintln!("pin: copy failed: {e}");
                        }
                    });
                }
                gdk::Key::s if ctrl => {
                    let img = image.clone();
                    std::thread::spawn(move || {
                        let path = default_screenshot_path();
                        match crate::save_to_file(&img, &path) {
                            Ok(()) => notify_saved(&path),
                            Err(e) => eprintln!("pin: save failed: {e}"),
                        }
                    });
                }
                gdk::Key::_0 | gdk::Key::KP_0 => {
                    zoom.set(1.0);
                    window.set_opacity(1.0);
                    apply_size();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    window.present();
}

/// A plain "saved" notification for a pin saved with Ctrl+S.
fn notify_saved(path: &std::path::Path) {
    use mshell_process::desktop::{Notice, notify};
    notify(
        Notice::new("mshell", "Pinned capture saved", path.display().to_string())
            .icon("image-x-generic-symbolic"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_and_clamps() {
        assert!((next_zoom(1.0, true) - 1.1).abs() < 1e-9);
        assert!((next_zoom(1.1, false) - 1.0).abs() < 1e-9);
        assert_eq!(next_zoom(ZOOM_MAX, true), ZOOM_MAX);
        assert_eq!(next_zoom(ZOOM_MIN, false), ZOOM_MIN);
    }

    #[test]
    fn opacity_steps_on_tenths_and_clamps() {
        let mut o = 1.0;
        for _ in 0..3 {
            o = next_opacity(o, false);
        }
        assert_eq!(o, 0.7);
        for _ in 0..20 {
            o = next_opacity(o, false);
        }
        assert_eq!(o, OPACITY_MIN);
        assert_eq!(next_opacity(1.0, true), 1.0);
    }

    #[test]
    fn origin_keeps_part_of_the_pin_on_screen() {
        // In bounds: untouched.
        assert_eq!(clamp_origin(100, 50, 200, 100, 1920, 1080), (100, 50));
        // Far right / bottom: at least KEEP_VISIBLE px stay visible.
        assert_eq!(
            clamp_origin(5000, 5000, 200, 100, 1920, 1080),
            (1920 - KEEP_VISIBLE, 1080 - KEEP_VISIBLE)
        );
        // Negative margins are not representable on an anchored edge.
        assert_eq!(clamp_origin(-500, -5, 200, 100, 1920, 1080), (0, 0));
        // A pin smaller than KEEP_VISIBLE keeps itself whole.
        assert_eq!(clamp_origin(5000, 0, 20, 20, 1920, 1080), (1900, 0));
    }

    #[test]
    fn inset_placement_uses_the_output_scale() {
        let o = OutputInfo {
            name: "DP-3".into(),
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
            scale: 2.0,
        };
        let p = PinPlacement::inset(Some(&o), 2560, 1440);
        assert_eq!((p.width, p.height), (1280, 720));
        assert_eq!(p.output.as_deref(), Some("DP-3"));
        let p = PinPlacement::inset(None, 300, 200);
        assert_eq!(
            (p.x, p.y, p.width, p.height),
            (DEFAULT_INSET, DEFAULT_INSET, 300, 200)
        );
    }
}
