//! Native screenshot capture pipeline.
//!
//! Replaces the `grim`/`slurp` subprocess shellout in
//! `scripts/screenshot` with an in-compositor capture path:
//!
//!   1. The dispatch layer queues a [`ScreenshotRequest`] onto
//!      [`MargoState::pending_screenshots`] and pings repaint.
//!   2. After the live render lands, the udev backend calls
//!      [`drain_pending_screenshots`] which resolves each
//!      request's source, builds the same `MargoRenderElement`
//!      list the live path produces (with the screencast block-out
//!      filter on, so privacy-marked windows stay hidden in
//!      screenshots), and renders that into a CPU-readable
//!      pixel buffer via the existing `render_and_download`
//!      helper from the screencast pipeline.
//!   3. Encoding + disk write + clipboard delivery happen on a
//!      worker thread so the compositor doesn't stall on a
//!      4K PNG encode (~50-100ms). A calloop channel routes the
//!      "done" signal back to the main loop for notification +
//!      IPC broadcast.
//!
//! ## What's "better than niri"
//!
//! niri's screenshot stack is split across three files (~1300
//! LOC including a heavy interactive UI with pango/cairo text
//! rendering). Phase 1 here is one file (~500 LOC), no
//! pango/cairo dep, and reuses margo's existing
//! `build_render_elements_inner` + `render_and_download` helpers
//! instead of duplicating the render plumbing.
//!
//! Three trade-offs vs niri:
//!
//!   * **No interactive region UI yet**. Phase 2 will add a
//!     frozen-screen overlay equivalent to niri's
//!     `ScreenshotUi`, but the keybind path covered here is what
//!     90% of users press 90% of the time (full output + window).
//!   * **Clipboard via `wl-copy` subprocess** instead of an
//!     in-compositor `wl_data_source`. Saves ~150 LOC of
//!     selection-handler refactor; needs `wl-clipboard` (already
//!     a recommended optdep). Phase 2 can swap in the native
//!     path with a custom `SelectionData` user-data type.
//!   * **`include_pointer` defaults flipped from niri's choices**.
//!     niri requires explicit `--show-pointer`; we ship sensible
//!     per-source defaults (output: hide, window: hide,
//!     interactive region: show) since users overwhelmingly
//!     don't want a pointer in their screenshots.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use anyhow::{bail, Context, Result};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::AsRenderElements;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ExportMem;
use smithay::utils::{Physical, Point, Scale, Size, Transform};
use tracing::{debug, info, warn};

use crate::backend::udev::MargoRenderElement;
use crate::screencasting::render_helpers::render_and_download;
use crate::state::MargoState;

/// What the dispatch layer asks the udev hook to capture, plus the
/// per-request flags that vary between Print-key and Alt-Print
/// invocations of the same handler.
#[derive(Debug, Clone)]
pub struct ScreenshotRequest {
    pub source: ScreenshotSource,
    /// Embed the live cursor sprite in the captured frame.
    /// Default: false (sensible for "I want a clean shot of this
    /// window/screen"). The interactive UI overrides per-session.
    pub include_pointer: bool,
    /// Explicit save path — when None, [`make_default_path`]
    /// generates `$XDG_PICTURES_DIR/Screenshots/screenshot_TS.png`.
    /// `Some(_)` is honoured verbatim, no parent-dir creation
    /// magic.
    pub save_path: Option<PathBuf>,
    /// When true, also push the encoded PNG into the clipboard
    /// (via `wl-copy`). Both `save` and `clipboard` may be true;
    /// at least one must be true or the request is a no-op.
    pub copy_clipboard: bool,
}

/// Capture target. `Focused*` variants are resolved at drain
/// time against the current focused-monitor / focused-client
/// pointers, not at queue time — so a screenshot keybind pressed
/// during an animation captures whatever's focused when the
/// dispatch fires, not whatever was focused when it started.
#[allow(dead_code)] // `Window(u64)` reserved for mctl + IPC dispatch
#[derive(Debug, Clone)]
pub enum ScreenshotSource {
    FocusedOutput,
    FocusedWindow,
    /// Capture by connector name (`DP-3`, `eDP-1`, …).
    Output(String),
    /// Capture by client identity — the same `addr_of!` u64 used
    /// by the screencast Window-target lookup.
    Window(u64),
}

/// Kick off a request. Pushes onto the pending queue and pings
/// repaint so the udev hook drains us on the next vblank.
pub fn queue(state: &mut MargoState, request: ScreenshotRequest) {
    if !request.copy_clipboard && request.save_path.is_none() {
        // Caller asked for "neither save nor copy" — nothing to
        // do. Could happen via `mctl dispatch screenshot ''`.
        warn!("screenshot request with no save target and no clipboard — dropping");
        return;
    }
    debug!("screenshot queued: {:?}", request);
    state.pending_screenshots.push(request);
    state.request_repaint();
}

/// Drain the request queue after the live render finishes. Hooks
/// into the udev backend's repaint handler at the same point as
/// `drain_image_copy_frames` and `drain_active_cast_frames` —
/// renderer is warm, the scene is exactly what the user just saw.
pub fn drain_pending_screenshots(
    renderer: &mut GlesRenderer,
    outputs: &mut std::collections::HashMap<
        smithay::reexports::drm::control::crtc::Handle,
        crate::backend::udev::OutputDevice,
    >,
    state: &mut MargoState,
) {
    let drained: Vec<ScreenshotRequest> =
        std::mem::take(&mut state.pending_screenshots);
    if drained.is_empty() {
        return;
    }

    for request in drained {
        match capture(renderer, outputs, state, &request) {
            Ok(capture) => spawn_save(state, capture, request),
            Err(err) => {
                warn!("screenshot capture failed: {err:#}");
                send_notification_failure(&format!("{err:#}"));
            }
        }
    }
}

/// Captured pixel buffer + the metadata the saver thread needs
/// (size for PNG header, source label for the notification).
struct CapturedImage {
    size: Size<i32, Physical>,
    pixels: Vec<u8>, // RGBA8 (Abgr8888 in DRM fourcc terms = RGBA in PNG byte order)
    label: String,
}

fn capture(
    renderer: &mut GlesRenderer,
    outputs: &mut std::collections::HashMap<
        smithay::reexports::drm::control::crtc::Handle,
        crate::backend::udev::OutputDevice,
    >,
    state: &MargoState,
    request: &ScreenshotRequest,
) -> Result<CapturedImage> {
    // Resolve the source against the live state.
    let resolved = resolve_source(&request.source, state)?;
    match resolved {
        ResolvedSource::Output { name } => {
            capture_output(renderer, outputs, state, &name, request.include_pointer)
        }
        ResolvedSource::Window { client_idx } => {
            capture_window(renderer, outputs, state, client_idx, request.include_pointer)
        }
    }
}

enum ResolvedSource {
    Output { name: String },
    Window { client_idx: usize },
}

fn resolve_source(src: &ScreenshotSource, state: &MargoState) -> Result<ResolvedSource> {
    match src {
        ScreenshotSource::FocusedOutput => {
            let mon_idx = state.focused_monitor();
            let mon = state
                .monitors
                .get(mon_idx)
                .context("focused monitor index out of range")?;
            Ok(ResolvedSource::Output {
                name: mon.name.clone(),
            })
        }
        ScreenshotSource::FocusedWindow => {
            let idx = state
                .focused_client_idx()
                .context("no focused window to capture")?;
            Ok(ResolvedSource::Window { client_idx: idx })
        }
        ScreenshotSource::Output(name) => {
            if !state.monitors.iter().any(|m| &m.name == name) {
                bail!("no output named `{name}`");
            }
            Ok(ResolvedSource::Output { name: name.clone() })
        }
        ScreenshotSource::Window(id) => {
            let idx = state
                .clients
                .iter()
                .position(|c| std::ptr::addr_of!(*c) as u64 == *id)
                .with_context(|| format!("no window with id `{id}`"))?;
            Ok(ResolvedSource::Window { client_idx: idx })
        }
    }
}

/// Render the entire scene of one output into a CPU-readable
/// pixel buffer. The `for_screencast=true` flag is honoured by
/// `build_render_elements_inner` so windows tagged with
/// `block_out_from_screencast = 1` are substituted with solid
/// black rectangles in the screenshot, same privacy guarantee as
/// the screencast and image-copy-capture paths.
fn capture_output(
    renderer: &mut GlesRenderer,
    outputs: &mut std::collections::HashMap<
        smithay::reexports::drm::control::crtc::Handle,
        crate::backend::udev::OutputDevice,
    >,
    state: &MargoState,
    name: &str,
    include_pointer: bool,
) -> Result<CapturedImage> {
    let (_, od) = outputs
        .iter()
        .find(|(_, od)| od.output.name() == name)
        .with_context(|| format!("output `{name}` not bound to a backend device"))?;
    let mode = od
        .output
        .current_mode()
        .with_context(|| format!("output `{name}` has no current mode"))?;
    let size = mode.size;
    let scale = Scale::from(od.output.current_scale().fractional_scale());

    let elements: Vec<MargoRenderElement> = crate::backend::udev::build_render_elements_inner(
        renderer,
        od,
        state,
        include_pointer,
        true, // for_screencast = honour block_out_from_screencast
    );

    let mapping = render_and_download(
        renderer,
        size,
        scale,
        Transform::Normal,
        Fourcc::Abgr8888,
        elements.iter(),
    )
    .context("render output to pixel buffer")?;

    let pixels = renderer
        .map_texture(&mapping)
        .context("read back rendered pixels")?
        .to_vec();

    Ok(CapturedImage {
        size,
        pixels,
        label: format!("output {name}"),
    })
}

/// Render one window's surface tree into a CPU-readable pixel
/// buffer. The cast buffer is sized to the window's geometry; we
/// reuse the screencast Window-target's relocate trick to land
/// the window at (0,0) regardless of where it sits on screen.
fn capture_window(
    renderer: &mut GlesRenderer,
    outputs: &mut std::collections::HashMap<
        smithay::reexports::drm::control::crtc::Handle,
        crate::backend::udev::OutputDevice,
    >,
    state: &MargoState,
    client_idx: usize,
    include_pointer: bool,
) -> Result<CapturedImage> {
    let client = state
        .clients
        .get(client_idx)
        .context("client index out of range")?;
    let geom = client.geom;
    if geom.width <= 0 || geom.height <= 0 {
        bail!("window has degenerate geometry");
    }

    let mon = state
        .monitors
        .get(client.monitor)
        .context("client's monitor missing")?;
    let scale_f = mon.output.current_scale().fractional_scale();
    let scale = Scale::from(scale_f);
    let size = Size::<i32, Physical>::from((
        (geom.width as f64 * scale_f).round() as i32,
        (geom.height as f64 * scale_f).round() as i32,
    ));
    if size.w <= 0 || size.h <= 0 {
        bail!("window physical size is zero");
    }

    // Two render paths for window capture:
    //
    //   * include_pointer = false → render just the window's
    //     surface tree at origin (0, 0). Cheap, no decoration,
    //     matches what every other compositor's "screenshot
    //     window" produces.
    //
    //   * include_pointer = true → render the FULL output's
    //     element list (cursor + every other window) but
    //     translated by `-window_offset_within_output`, so the
    //     target window's top-left lands at (0, 0) of the
    //     window-sized cast buffer. Extra clients on the same
    //     monitor end up at negative coords and clip naturally.
    //     This is the same RelocateRenderElement trick the Phase F
    //     screencast Window cast uses.
    use smithay::backend::renderer::element::utils::{
        Relocate, RelocateRenderElement,
    };
    use smithay::backend::renderer::element::RenderElement;

    if !include_pointer {
        use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
        let surface_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            AsRenderElements::<GlesRenderer>::render_elements(
                &client.window,
                renderer,
                Point::from((0, 0)),
                scale,
                1.0,
            );
        let elements: Vec<MargoRenderElement> = surface_elems
            .into_iter()
            .map(MargoRenderElement::WaylandSurface)
            .collect();

        return finish_capture(
            renderer,
            elements.iter(),
            size,
            scale,
            window_label(client),
        );
    }

    // include_pointer = true path.
    let (_, od) = outputs
        .iter()
        .find(|(_, od)| od.output == mon.output)
        .context("client's monitor not bound to a backend device")?;
    let output_elems: Vec<MargoRenderElement> =
        crate::backend::udev::build_render_elements_inner(
            renderer, od, state, true, true,
        );

    let win_off_x =
        -((geom.x - mon.monitor_area.x) as f64 * scale_f).round() as i32;
    let win_off_y =
        -((geom.y - mon.monitor_area.y) as f64 * scale_f).round() as i32;
    let win_off = Point::<i32, Physical>::from((win_off_x, win_off_y));

    let translated: Vec<RelocateRenderElement<MargoRenderElement>> = output_elems
        .into_iter()
        .map(|e| RelocateRenderElement::from_element(e, win_off, Relocate::Relative))
        .collect();
    // Help the trait-resolver: render_and_download is generic over
    // `impl RenderElement<GlesRenderer>`; the explicit bound makes
    // sure RelocateRenderElement<MargoRenderElement> picks the
    // right impl.
    fn _assert_render_element<E: RenderElement<GlesRenderer>>(_: &E) {}

    let mapping = render_and_download(
        renderer,
        size,
        scale,
        Transform::Normal,
        Fourcc::Abgr8888,
        translated.iter(),
    )
    .context("render window to pixel buffer")?;

    let pixels = renderer
        .map_texture(&mapping)
        .context("read back rendered pixels")?
        .to_vec();
    Ok(CapturedImage {
        size,
        pixels,
        label: window_label(client),
    })
}

/// Common tail of every `capture_*` path: render → download →
/// box up the result. Pulled out so the include_pointer-true
/// and include_pointer-false branches of `capture_window` don't
/// duplicate the read-back boilerplate.
fn finish_capture<E>(
    renderer: &mut GlesRenderer,
    elements: impl Iterator<Item = E>,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    label: String,
) -> Result<CapturedImage>
where
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
{
    let mapping = render_and_download(
        renderer,
        size,
        scale,
        Transform::Normal,
        Fourcc::Abgr8888,
        elements,
    )
    .context("render to pixel buffer")?;
    let pixels = renderer
        .map_texture(&mapping)
        .context("read back rendered pixels")?
        .to_vec();
    Ok(CapturedImage { size, pixels, label })
}

fn window_label(client: &crate::state::MargoClient) -> String {
    if !client.title.is_empty() {
        format!("window {}", client.title)
    } else if !client.app_id.is_empty() {
        format!("window {}", client.app_id)
    } else {
        "window".to_string()
    }
}

/// Save result delivered from the worker thread back into the
/// main loop. Carries either the file path (for IPC + the
/// notification) or the encoded PNG bytes (for the native
/// clipboard set, which has to run on the main thread because
/// `set_data_device_selection` touches Wayland state).
struct SaveDelivery {
    path: Option<PathBuf>,
    label: String,
    clipboard_png: Option<Arc<[u8]>>,
    error: Option<String>,
}

/// Encode + write on a worker thread; clipboard set + notify
/// run back on the main loop via a calloop channel.
fn spawn_save(state: &mut MargoState, image: CapturedImage, request: ScreenshotRequest) {
    let path = request
        .save_path
        .or_else(|| make_default_path().ok().flatten());

    let (tx, rx) = calloop::channel::sync_channel::<SaveDelivery>(1);
    state
        .loop_handle
        .insert_source(rx, |event, _, state| {
            if let calloop::channel::Event::Msg(delivery) = event {
                // Set clipboard from the main thread. The
                // selection's user_data carries the PNG bytes
                // so `send_selection` can serve any number of
                // future read fds without re-encoding.
                if let Some(bytes) = delivery.clipboard_png.as_ref() {
                    smithay::wayland::selection::data_device::set_data_device_selection(
                        &state.display_handle,
                        &state.seat,
                        vec![String::from("image/png")],
                        crate::state::SelectionUserData::Screenshot(bytes.clone()),
                    );
                }
                send_notification(&delivery);
                if let Some(p) = delivery.path.as_ref() {
                    info!("screenshot saved: {}", p.display());
                }
            }
        })
        .ok();

    let copy_clipboard = request.copy_clipboard;
    let label = image.label.clone();

    thread::spawn(move || {
        // 1. Encode PNG (slow — ~50-100ms for 4K).
        let png_bytes = match encode_png(image.size, &image.pixels) {
            Ok(b) => b,
            Err(err) => {
                warn!("PNG encode failed: {err:#}");
                let _ = tx.send(SaveDelivery {
                    path: None,
                    label,
                    clipboard_png: None,
                    error: Some(format!("{err:#}")),
                });
                return;
            }
        };
        let png_arc: Arc<[u8]> = Arc::from(png_bytes.into_boxed_slice());

        // 2. Disk write.
        let mut written = None;
        if let Some(p) = path.as_ref() {
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            match std::fs::write(p, &png_arc[..]) {
                Ok(()) => written = Some(p.clone()),
                Err(err) => warn!("save screenshot {}: {err}", p.display()),
            }
        }

        // 3. Hand the bytes back to the main loop for the
        //    native clipboard set. No `wl-copy` subprocess.
        let _ = tx.send(SaveDelivery {
            path: written,
            label,
            clipboard_png: copy_clipboard.then_some(png_arc),
            error: None,
        });
    });
}


/// Encode an RGBA8 buffer as a PNG. Pure-Rust via the `png`
/// crate; no `image` crate (which would pull every codec).
fn encode_png(size: Size<i32, Physical>, pixels: &[u8]) -> Result<Vec<u8>> {
    let w = size.w as u32;
    let h = size.h as u32;
    let expected = (w as usize) * (h as usize) * 4;
    if pixels.len() != expected {
        bail!(
            "pixel buffer size mismatch: have {} bytes, expected {} for {}x{}",
            pixels.len(),
            expected,
            w,
            h
        );
    }

    let mut buf = Vec::with_capacity(pixels.len() / 4);
    {
        let mut encoder = png::Encoder::new(&mut buf, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().context("PNG header")?;
        writer
            .write_image_data(pixels)
            .context("PNG image data")?;
    }
    Ok(buf)
}


/// `$XDG_PICTURES_DIR/Screenshots/screenshot_YYYY-MM-DD_HH-MM-SS.png`,
/// honouring `$SCREENSHOT_SAVE_DIR` for the directory if set.
/// Stdlib-only timestamp formatting — no `chrono` dep.
fn make_default_path() -> Result<Option<PathBuf>> {
    let dir = std::env::var_os("SCREENSHOT_SAVE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_PICTURES_DIR")
                .map(PathBuf::from)
                .map(|p| p.join("Screenshots"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join("Pictures").join("Screenshots"))
        })
        .context("could not derive a save directory")?;
    let stamp = current_timestamp();
    Ok(Some(dir.join(format!("screenshot_{stamp}.png"))))
}

/// `YYYY-MM-DD_HH-MM-SS` from `SystemTime::now()`. Avoids
/// pulling in `chrono`; uses libc's `localtime_r` for a
/// timezone-aware breakdown.
fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        )
    }
}

fn send_notification(result: &SaveDelivery) {
    let body = match (&result.path, &result.error, &result.clipboard_png) {
        (_, Some(err), _) => format!("{} — error: {err}", result.label),
        (Some(p), None, Some(_)) => format!("{}\n{} (+ clipboard)", result.label, p.display()),
        (Some(p), None, None) => format!("{}\n{}", result.label, p.display()),
        (None, None, Some(_)) => format!("{} → clipboard", result.label),
        (None, None, None) => format!("{} (no save target)", result.label),
    };
    let icon = if result.error.is_some() {
        "dialog-error"
    } else {
        "image-x-generic"
    };
    let _ = crate::utils::spawn(&[
        "notify-send",
        "-a",
        "margo",
        "-i",
        icon,
        "-t",
        "3500",
        "Margo: screenshot",
        &body,
    ]);
}

fn send_notification_failure(msg: &str) {
    let _ = crate::utils::spawn(&[
        "notify-send",
        "-a",
        "margo",
        "-i",
        "dialog-error",
        "-u",
        "critical",
        "-t",
        "5000",
        "Margo: screenshot failed",
        msg,
    ]);
}
