//! `mpicker` — margo's native color picker.
//!
//! Replaces the `hyprpicker` invocation the quick-settings color
//! picker used to spawn. Spirit + UX cloned from hyprpicker:
//!   1. Freeze every output with a wlr-screencopy frame.
//!   2. Open a fullscreen layer-shell overlay per monitor with the
//!      frame painted as the background, cursor set to crosshair.
//!   3. Track the pointer; render a circular zoom lens around the
//!      cursor showing the magnified pixels underneath.
//!   4. On click → sample the pixel under the cursor, format it
//!      per `--format`, print to stdout, optionally `wl-copy` +
//!      `notify-send`, then exit.
//!   5. Escape cancels with exit code 1.
//!
//! Built on top of `mshell_screenshot::CaptureBackend` (wlr-
//! screencopy) so screen grab is a single dep with the rest of
//! margo's screenshot pipeline.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use anyhow::Result;
use clap::Parser;
use gtk4::cairo::{Format, ImageSurface};
use gtk4::prelude::*;
use gtk4::{cairo, gdk, glib};
use gtk4_layer_shell::LayerShell;
use image::{Rgba, RgbaImage};
use mshell_screenshot::{CaptureBackend, OutputInfo, query_outputs};

#[derive(Parser, Debug)]
#[command(
    name = "mpicker",
    version,
    about = "Margo color picker — freezes the screen, tracks the cursor, prints the clicked pixel."
)]
struct Cli {
    /// Auto-copy the picked color to the clipboard via wl-copy.
    #[arg(short = 'a', long)]
    autocopy: bool,
    /// Send a desktop notification when a color is picked
    /// (requires notify-send + a notification daemon).
    #[arg(short = 'n', long)]
    notify: bool,
    /// Output format: hex (default), rgb, hsl, cmyk.
    #[arg(short = 'f', long, default_value = "hex")]
    format: String,
    /// Use lowercase letters in hex output (#abcdef instead of
    /// #ABCDEF).
    #[arg(short = 'l', long)]
    lowercase_hex: bool,
    /// Disable the zoom-lens overlay around the cursor.
    #[arg(short = 'z', long)]
    no_zoom: bool,
    /// Suppress informational log lines on stderr; errors still
    /// printed.
    #[arg(short = 'q', long)]
    quiet: bool,
}

/// Final picked-color state — written by the click handler,
/// read after `app.run()` returns to drive stdout / clipboard /
/// notify-send. Wrapped in a Mutex so the GTK callbacks (single
/// thread, but stored across closure lifetimes) can mutate it
/// without explicit `Send` gymnastics.
static PICKED: Mutex<Option<String>> = Mutex::new(None);

fn main() -> Result<()> {
    let cli = Cli::parse();

    let outputs = query_outputs().map_err(|e| anyhow::anyhow!("query outputs: {e}"))?;
    if outputs.is_empty() {
        anyhow::bail!("no outputs found");
    }

    // Capture every output BEFORE we open the layer-shell
    // overlays — once the overlay covers the screen the captured
    // frame would just be a dark mask. Do this synchronously
    // (each capture is ~tens of ms), store as RgbaImage keyed
    // by output name.
    let backend = CaptureBackend::new().map_err(|e| anyhow::anyhow!("backend: {e}"))?;
    let mut frames: HashMap<String, RgbaImage> = HashMap::new();
    for out in &outputs {
        let img = backend
            .capture_output(&out.name)
            .map_err(|e| anyhow::anyhow!("capture {}: {e}", out.name))?;
        frames.insert(out.name.clone(), img);
    }
    if !cli.quiet {
        eprintln!("mpicker: captured {} output(s)", frames.len());
    }

    let app = gtk4::Application::builder()
        .application_id("com.mshell.picker")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let outputs_rc = Rc::new(outputs);
    let frames_rc = Rc::new(frames);
    let cli_rc = Rc::new(cli);
    app.connect_activate(move |app| {
        open_overlays(app, &outputs_rc, &frames_rc, &cli_rc);
    });

    // Drop argv so GTK doesn't try to parse our --autocopy etc.
    app.run_with_args::<&str>(&[]);

    // Post-run: deliver picked color (if any) to stdout / wl-copy /
    // notify, then exit with the corresponding code.
    let picked = PICKED.lock().unwrap().clone();
    let cli = Cli::parse();
    if let Some(color) = picked {
        println!("{}", color);
        if cli.autocopy {
            if let Err(e) = wl_copy(&color) {
                eprintln!("mpicker: wl-copy failed: {e}");
            }
        }
        if cli.notify {
            let _ = std::process::Command::new("notify-send")
                .arg("--app-name=Color Picker")
                .arg(format!("Picked {}", color))
                .status();
        }
        Ok(())
    } else {
        // Cancelled — exit non-zero so callers can branch on it.
        std::process::exit(1);
    }
}

/// One layer-shell overlay per output; each one paints its
/// own captured frame plus a cursor zoom lens.
fn open_overlays(
    app: &gtk4::Application,
    outputs: &Rc<Vec<OutputInfo>>,
    frames: &Rc<HashMap<String, RgbaImage>>,
    cli: &Rc<Cli>,
) {
    let state = Rc::new(SharedState {
        cursor: Cell::new(None),
        cursor_output: RefCell::new(None),
        windows: RefCell::new(Vec::new()),
    });

    let gdk_display = gdk::Display::default().expect("no display");
    let monitors = gdk_display.monitors();

    for out in outputs.iter() {
        let frame = match frames.get(&out.name) {
            Some(f) => f.clone(),
            None => continue,
        };
        let gdk_monitor = find_gdk_monitor(&monitors, out);
        let window = build_overlay(app, out, gdk_monitor.as_ref(), frame, &state, cli);
        state.windows.borrow_mut().push(window);
    }
    for w in state.windows.borrow().iter() {
        w.present();
    }
}

struct SharedState {
    /// Cursor position in the active overlay's output-local
    /// pixels.
    cursor: Cell<Option<(f64, f64)>>,
    /// Output name that owns the cursor right now (one per
    /// motion event; switches when the user crosses monitors).
    cursor_output: RefCell<Option<String>>,
    windows: RefCell<Vec<gtk4::Window>>,
}

impl SharedState {
    fn close_all(&self) {
        for w in self.windows.borrow().iter() {
            w.close();
        }
    }
}

fn build_overlay(
    app: &gtk4::Application,
    output: &OutputInfo,
    gdk_monitor: Option<&gdk::Monitor>,
    frame: RgbaImage,
    state: &Rc<SharedState>,
    cli: &Rc<Cli>,
) -> gtk4::Window {
    let window = gtk4::ApplicationWindow::new(app).upcast::<gtk4::Window>();
    window.set_decorated(false);

    window.init_layer_shell();
    window.set_layer(gtk4_layer_shell::Layer::Overlay);
    window.set_anchor(gtk4_layer_shell::Edge::Top, true);
    window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
    window.set_anchor(gtk4_layer_shell::Edge::Left, true);
    window.set_anchor(gtk4_layer_shell::Edge::Right, true);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);
    window.set_namespace(Some("mpicker"));

    if let Some(m) = gdk_monitor {
        window.set_monitor(Some(m));
    }
    window.set_cursor_from_name(Some("crosshair"));

    let drawing_area = gtk4::DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    window.set_child(Some(&drawing_area));

    // Convert RgbaImage to a Cairo ImageSurface ONCE — Cairo's
    // ARGB32 format expects pre-multiplied BGRA in native-endian
    // order, but for opaque captures the channel layout matches
    // image::Rgba reinterpreted as ARGB premul-with-alpha=255.
    // We pre-swap R/B and pre-multiply (alpha is always 0xFF for
    // captured screen frames so this is a no-op apart from the
    // channel swap).
    let surface = rgba_image_to_cairo_surface(&frame).expect("cairo surface");
    let surface_rc = Rc::new(surface);
    let frame_rc = Rc::new(frame);
    let output_name = output.name.clone();

    let state_draw = Rc::clone(state);
    let cli_draw = Rc::clone(cli);
    let surface_for_draw = Rc::clone(&surface_rc);
    let frame_for_draw = Rc::clone(&frame_rc);
    let out_for_draw = output_name.clone();
    drawing_area.set_draw_func(move |_, cr, w, h| {
        draw_overlay(
            cr,
            w,
            h,
            &surface_for_draw,
            &frame_for_draw,
            &state_draw,
            &out_for_draw,
            cli_draw.no_zoom,
        );
    });

    // Motion: update cursor, re-render.
    let motion = gtk4::EventControllerMotion::new();
    let state_motion = Rc::clone(state);
    let out_motion = output_name.clone();
    let da_motion = drawing_area.clone();
    motion.connect_motion(move |_, x, y| {
        state_motion.cursor.set(Some((x, y)));
        *state_motion.cursor_output.borrow_mut() = Some(out_motion.clone());
        da_motion.queue_draw();
    });
    drawing_area.add_controller(motion);

    // Click: sample the pixel under the cursor + format + stash.
    let click = gtk4::GestureClick::new();
    click.set_button(0); // any button
    let state_click = Rc::clone(state);
    let cli_click = Rc::clone(cli);
    let frame_click = Rc::clone(&frame_rc);
    let out_click = output_name.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        *state_click.cursor_output.borrow_mut() = Some(out_click.clone());
        state_click.cursor.set(Some((x, y)));
        if let Some(color) = sample_at(&frame_click, x, y) {
            let formatted = format_color(color, &cli_click.format, cli_click.lowercase_hex);
            *PICKED.lock().unwrap() = Some(formatted);
        }
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        state_click.close_all();
    });
    drawing_area.add_controller(click);

    // Keyboard: Escape cancels.
    let key = gtk4::EventControllerKey::new();
    let state_key = Rc::clone(state);
    key.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape {
            state_key.close_all();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key);

    window
}

/// Resolve which `gdk::Monitor` matches the wayland output by
/// connector name — same heuristic the screenshot area_selector
/// uses but copied locally since the helper is crate-private.
fn find_gdk_monitor(
    monitors: &gtk4::gio::ListModel,
    output: &OutputInfo,
) -> Option<gdk::Monitor> {
    for i in 0..monitors.n_items() {
        let obj = monitors.item(i)?;
        let mon: gdk::Monitor = obj.downcast().ok()?;
        if mon.connector().map(|s| s.to_string()).as_deref() == Some(output.name.as_str()) {
            return Some(mon);
        }
    }
    None
}

fn draw_overlay(
    cr: &cairo::Context,
    width: i32,
    height: i32,
    surface: &ImageSurface,
    frame: &RgbaImage,
    state: &SharedState,
    this_output: &str,
    no_zoom: bool,
) {
    // Frozen screen as the background.
    cr.set_source_surface(surface, 0.0, 0.0).ok();
    cr.paint().ok();

    // Cursor + zoom lens only on the overlay that owns the cursor.
    let same_owner = state
        .cursor_output
        .borrow()
        .as_deref()
        .map(|o| o == this_output)
        .unwrap_or(false);
    if !same_owner {
        return;
    }
    let Some((cx, cy)) = state.cursor.get() else {
        return;
    };

    let _ = (width, height); // unused but useful to keep signature consistent

    if !no_zoom {
        draw_zoom_lens(cr, cx, cy, frame);
    }

    // Hex chip floating below the cursor. Sample current pixel.
    if let Some(color) = sample_at(frame, cx, cy) {
        draw_hex_chip(cr, cx, cy, color);
    }
}

const LENS_RADIUS: f64 = 100.0;
const LENS_ZOOM: f64 = 10.0;

/// Cairo-render a circular magnifier centered on the cursor.
/// Inside the circle: the captured pixels scaled `LENS_ZOOM` ×,
/// with a 1-pixel grid + a central crosshair so the user can
/// see exactly which pixel they're about to click.
fn draw_zoom_lens(cr: &cairo::Context, cx: f64, cy: f64, frame: &RgbaImage) {
    cr.save().ok();

    // Clip to the lens circle.
    cr.new_path();
    cr.arc(cx, cy, LENS_RADIUS, 0.0, std::f64::consts::TAU);
    cr.clip();

    // Translate + scale around the cursor so the captured image
    // is magnified about the cursor pixel.
    cr.translate(cx, cy);
    cr.scale(LENS_ZOOM, LENS_ZOOM);
    cr.translate(-cx, -cy);

    if let Ok(surface) = rgba_image_to_cairo_surface(frame) {
        cr.set_source_surface(&surface, 0.0, 0.0).ok();
        // Nearest-neighbour so individual pixels render as crisp
        // squares — the whole point of a magnifier is seeing the
        // pixel grid.
        cr.source().set_filter(cairo::Filter::Nearest);
        cr.paint().ok();
    }

    cr.restore().ok();

    // Crosshair on the centre pixel.
    cr.save().ok();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    cr.set_line_width(2.0);
    let half = LENS_RADIUS * 0.18;
    cr.move_to(cx - half, cy);
    cr.line_to(cx + half, cy);
    cr.move_to(cx, cy - half);
    cr.line_to(cx, cy + half);
    cr.stroke().ok();
    cr.restore().ok();

    // Lens border.
    cr.save().ok();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    cr.set_line_width(2.0);
    cr.new_path();
    cr.arc(cx, cy, LENS_RADIUS, 0.0, std::f64::consts::TAU);
    cr.stroke().ok();
    cr.restore().ok();
}

/// Small chip showing the cursor's hex underneath the lens.
fn draw_hex_chip(cr: &cairo::Context, cx: f64, cy: f64, color: Rgba<u8>) {
    let label = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
    cr.set_font_size(14.0);
    let Ok(extents) = cr.text_extents(&label) else {
        return;
    };
    let pad = 6.0;
    let bw = extents.width() + pad * 2.0;
    let bh = extents.height() + pad * 2.0;
    let bx = cx - bw / 2.0;
    let by = cy + LENS_RADIUS + 12.0;

    cr.save().ok();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.7);
    cr.rectangle(bx, by, bw, bh);
    cr.fill().ok();

    // Colour swatch on the left edge.
    cr.set_source_rgba(
        color[0] as f64 / 255.0,
        color[1] as f64 / 255.0,
        color[2] as f64 / 255.0,
        1.0,
    );
    cr.rectangle(bx + 2.0, by + 2.0, bh - 4.0, bh - 4.0);
    cr.fill().ok();

    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    cr.move_to(bx + bh + 4.0, by + bh - pad - 1.0);
    cr.show_text(&label).ok();
    cr.restore().ok();
}

/// Sample the pixel at output-local coordinates `(x, y)`. Clamps
/// to image bounds; returns `None` for empty images.
fn sample_at(frame: &RgbaImage, x: f64, y: f64) -> Option<Rgba<u8>> {
    let w = frame.width() as i32;
    let h = frame.height() as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    let xi = (x.round() as i32).clamp(0, w - 1);
    let yi = (y.round() as i32).clamp(0, h - 1);
    Some(*frame.get_pixel(xi as u32, yi as u32))
}

fn format_color(c: Rgba<u8>, format: &str, lowercase: bool) -> String {
    let (r, g, b) = (c[0], c[1], c[2]);
    match format.to_ascii_lowercase().as_str() {
        "rgb" => format!("rgb({}, {}, {})", r, g, b),
        "hsl" => {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            format!(
                "hsl({}, {}%, {}%)",
                h.round() as i32,
                (s * 100.0).round() as i32,
                (l * 100.0).round() as i32
            )
        }
        "cmyk" => {
            let (c_, m, y, k) = rgb_to_cmyk(r, g, b);
            format!(
                "cmyk({}%, {}%, {}%, {}%)",
                (c_ * 100.0).round() as i32,
                (m * 100.0).round() as i32,
                (y * 100.0).round() as i32,
                (k * 100.0).round() as i32
            )
        }
        // Default: hex.
        _ => {
            if lowercase {
                format!("#{:02x}{:02x}{:02x}", r, g, b)
            } else {
                format!("#{:02X}{:02X}{:02X}", r, g, b)
            }
        }
    }
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f64::EPSILON {
        ((g - b) / d) + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f64::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn rgb_to_cmyk(r: u8, g: u8, b: u8) -> (f64, f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;
    let k = 1.0 - r.max(g).max(b);
    if (1.0 - k).abs() < f64::EPSILON {
        return (0.0, 0.0, 0.0, 1.0);
    }
    let c = (1.0 - r - k) / (1.0 - k);
    let m = (1.0 - g - k) / (1.0 - k);
    let y = (1.0 - b - k) / (1.0 - k);
    (c, m, y, k)
}

fn rgba_image_to_cairo_surface(img: &RgbaImage) -> Result<ImageSurface, cairo::Error> {
    let w = img.width() as i32;
    let h = img.height() as i32;
    let mut surface = ImageSurface::create(Format::ARgb32, w, h)?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface
            .data()
            .map_err(|_| cairo::Error::WriteError)?;
        for y in 0..(h as usize) {
            for x in 0..(w as usize) {
                let p = img.get_pixel(x as u32, y as u32);
                let r = p[0];
                let g = p[1];
                let b = p[2];
                let a = p[3];
                // Cairo ARGB32 native-endian on little-endian is
                // [B, G, R, A].
                let offs = y * stride + x * 4;
                data[offs] = b;
                data[offs + 1] = g;
                data[offs + 2] = r;
                data[offs + 3] = a;
            }
        }
    }
    Ok(surface)
}

fn wl_copy(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy").stdin(Stdio::piped()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("wl-copy exited non-zero");
    }
    Ok(())
}
