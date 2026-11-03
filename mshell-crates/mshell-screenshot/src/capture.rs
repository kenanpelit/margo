use std::collections::HashMap;
use std::os::fd::AsFd;
use std::sync::{Arc, Mutex};

use super::{HyprlandWindow, OutputInfo, Result, ScreenshotError};
use crate::selectors::area_selector::RegionSelection;
use crate::utils::query_outputs;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

#[derive(Debug, Clone)]
pub struct ScreenshotResult {
    pub saved_path: Option<std::path::PathBuf>,
    pub in_clipboard: bool,
}

/// Low-level capture backend wrapping wlr-screencopy.
pub struct CaptureBackend;

/// Per-frame capture state driven by wlr-screencopy events.
#[derive(Debug)]
struct FrameState {
    /// Format info received from the compositor.
    format: Option<FrameFormat>,
    /// Whether the frame is ready to copy.
    ready: bool,
    /// Whether the copy completed.
    done: bool,
    /// Whether the copy failed.
    failed: bool,
}

#[derive(Debug, Clone)]
struct FrameFormat {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

/// Aggregated state for the wayland event loop.
struct CaptureState {
    outputs: HashMap<String, wl_output::WlOutput>,
    _shm: Option<wl_shm::WlShm>,
    _screencopy_mgr: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    frame_state: Arc<Mutex<FrameState>>,
}

impl CaptureBackend {
    pub fn new() -> Result<Self> {
        // Quick check that we can connect to wayland.
        Connection::connect_to_env().map_err(|e| ScreenshotError::WaylandConnect(e.to_string()))?;
        Ok(Self)
    }

    /// Capture all outputs and stitch them together (see [`stitch_plan`]
    /// for the canvas geometry).
    pub fn capture_all(&self) -> Result<image::RgbaImage> {
        let outputs = query_outputs()?;
        let plan = stitch_plan(&outputs)
            .ok_or_else(|| ScreenshotError::CaptureFailed("no outputs found".into()))?;

        let mut canvas = image::RgbaImage::new(plan.width, plan.height);
        for (output_info, tile) in outputs.iter().zip(&plan.tiles) {
            let frame = self.capture_output_raw(&output_info.name)?;
            // A frame at a lower scale than the canvas (or one whose buffer
            // size disagrees with the reported geometry) is resampled to
            // its tile, so every output covers exactly its layout rect.
            let frame = if frame.dimensions() == (tile.width, tile.height) {
                frame
            } else {
                image::imageops::resize(
                    &frame,
                    tile.width,
                    tile.height,
                    image::imageops::FilterType::CatmullRom,
                )
            };
            image::imageops::overlay(&mut canvas, &frame, tile.x, tile.y);
        }

        Ok(canvas)
    }

    /// Capture a single output by name.
    pub fn capture_output(&self, name: &str) -> Result<image::RgbaImage> {
        self.capture_output_raw(name)
    }

    /// Capture a window by cropping the relevant output.
    pub fn capture_window(&self, win: &HyprlandWindow) -> Result<image::RgbaImage> {
        let outputs = query_outputs()?;
        let output_info = outputs
            .iter()
            .find(|o| o.name == win.output)
            .ok_or_else(|| ScreenshotError::OutputNotFound(win.output.clone()))?;

        let full = self.capture_output_raw(&win.output)?;

        // Window coords are global and logical; convert to output-local
        // buffer pixels.
        let local_x = (win.x - output_info.x).max(0);
        let local_y = (win.y - output_info.y).max(0);
        let Some((x, y, w, h)) = clamp_crop(
            logical_to_buffer((local_x, local_y, win.width, win.height), output_info.scale),
            (full.width(), full.height()),
        ) else {
            return Err(ScreenshotError::CaptureFailed(
                "window has zero size".into(),
            ));
        };

        Ok(image::imageops::crop_imm(&full, x, y, w, h).to_image())
    }

    /// Capture a user-selected region by cropping the relevant output.
    pub fn capture_region(&self, region: &RegionSelection) -> Result<image::RgbaImage> {
        // The selector reports output-local logical pixels; the frame is
        // in buffer pixels.
        let scale = query_outputs()?
            .iter()
            .find(|o| o.name == region.output)
            .map_or(1.0, |o| o.scale);
        let full = self.capture_output_raw(&region.output)?;

        let Some((x, y, w, h)) = clamp_crop(
            logical_to_buffer((region.x, region.y, region.width, region.height), scale),
            (full.width(), full.height()),
        ) else {
            return Err(ScreenshotError::CaptureFailed(
                "region has zero size".into(),
            ));
        };

        Ok(image::imageops::crop_imm(&full, x, y, w, h).to_image())
    }

    /// Core capture logic for a single output using wlr-screencopy.
    fn capture_output_raw(&self, output_name: &str) -> Result<image::RgbaImage> {
        // Fresh connection per capture to avoid stale state when
        // capturing multiple outputs sequentially.
        let conn = Connection::connect_to_env()
            .map_err(|e| ScreenshotError::WaylandConnect(e.to_string()))?;

        let (globals, mut queue) = registry_queue_init::<CaptureState>(&conn)
            .map_err(|e| ScreenshotError::WaylandConnect(e.to_string()))?;

        let qh = queue.handle();

        // Bind globals.
        let shm: wl_shm::WlShm = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| ScreenshotError::ProtocolNotSupported)?;

        let screencopy_mgr: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 = globals
            .bind(&qh, 3..=3, ())
            .map_err(|_| ScreenshotError::ProtocolNotSupported)?;

        // Collect outputs.
        let mut state = CaptureState {
            outputs: HashMap::new(),
            _shm: Some(shm.clone()),
            _screencopy_mgr: Some(screencopy_mgr.clone()),
            frame_state: Arc::new(Mutex::new(FrameState {
                format: None,
                ready: false,
                done: false,
                failed: false,
            })),
        };

        // We need a roundtrip to discover outputs.
        // Bind all wl_output globals.
        let output_list = globals.contents().clone_list();
        for global in output_list.iter() {
            if global.interface == "wl_output" {
                let _: wl_output::WlOutput =
                    globals
                        .registry()
                        .bind(global.name, global.version.min(4), &qh, global.name);
            }
        }

        // Roundtrip to receive output name events.
        queue
            .roundtrip(&mut state)
            .map_err(|e| ScreenshotError::CaptureFailed(e.to_string()))?;

        let wl_output = state
            .outputs
            .get(output_name)
            .cloned()
            .ok_or_else(|| ScreenshotError::OutputNotFound(output_name.to_string()))?;

        // Request a screencopy frame.
        let frame = screencopy_mgr.capture_output(0, &wl_output, &qh, ());

        // Roundtrip to receive buffer format info.
        queue
            .roundtrip(&mut state)
            .map_err(|e| ScreenshotError::CaptureFailed(e.to_string()))?;

        let fmt = {
            let fs = state.frame_state.lock().unwrap();
            fs.format
                .clone()
                .ok_or_else(|| ScreenshotError::CaptureFailed("no format received".into()))?
        };

        // Create shm buffer.
        let buf_size = (fmt.stride * fmt.height) as usize;
        let file = create_shm_file(buf_size)?;
        let pool = shm.create_pool(file.as_fd(), buf_size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            fmt.width as i32,
            fmt.height as i32,
            fmt.stride as i32,
            fmt.format,
            &qh,
            (),
        );

        // Tell screencopy to copy into our buffer.
        frame.copy(&buffer);

        // Pump events until done or failed.
        loop {
            queue
                .blocking_dispatch(&mut state)
                .map_err(|e| ScreenshotError::CaptureFailed(e.to_string()))?;

            let fs = state.frame_state.lock().unwrap();
            if fs.failed {
                return Err(ScreenshotError::CaptureFailed(
                    "compositor rejected copy".into(),
                ));
            }
            if fs.done {
                break;
            }
        }

        // Read pixels from shared memory.
        let mmap = unsafe {
            memmap2::MmapOptions::new()
                .len(buf_size)
                .map(&file)
                .map_err(|e| ScreenshotError::CaptureFailed(e.to_string()))?
        };

        let image = convert_to_rgba(&mmap, fmt.width, fmt.height, fmt.stride, fmt.format)?;

        // Cleanup protocol objects.
        buffer.destroy();
        pool.destroy();
        frame.destroy();
        screencopy_mgr.destroy();

        Ok(image)
    }
}

/// Where one output's frame goes on the stitched canvas, in canvas pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i64,
    y: i64,
    width: u32,
    height: u32,
}

/// Geometry of a full-desktop capture.
#[derive(Debug, Clone, PartialEq)]
struct StitchPlan {
    width: u32,
    height: u32,
    /// Canvas pixels per logical pixel.
    scale: f64,
    /// One tile per output, in the order of the `outputs` slice.
    tiles: Vec<Tile>,
}

/// Lay the outputs out on one canvas. Output positions *and* sizes are in
/// layout (logical) coordinates; screencopy frames are buffer pixels
/// (logical × each output's own scale). With mixed scales those can't be
/// stitched 1:1, so the canvas is the logical bounding box rendered at the
/// **largest** output scale (as `grim` does): the sharpest output keeps
/// every pixel, lower-scale outputs are upscaled into their rect, and the
/// arrangement matches the layout exactly. Tile edges are rounded from
/// the absolute logical edges so neighbouring outputs neither overlap nor
/// leave a gap. `None` for an empty output list.
fn stitch_plan(outputs: &[OutputInfo]) -> Option<StitchPlan> {
    let min_x = outputs.iter().map(|o| o.x).min()?;
    let min_y = outputs.iter().map(|o| o.y).min()?;
    let max_x = outputs.iter().map(|o| o.x + o.width.max(0)).max()?;
    let max_y = outputs.iter().map(|o| o.y + o.height.max(0)).max()?;
    let scale = outputs
        .iter()
        .map(|o| o.scale)
        .filter(|s| s.is_finite() && *s > 0.0)
        .fold(1.0_f64, f64::max);
    let px = |logical: i32, origin: i32| (f64::from(logical - origin) * scale).round() as i64;

    let tiles = outputs
        .iter()
        .map(|o| {
            let (x0, y0) = (px(o.x, min_x), px(o.y, min_y));
            let x1 = px(o.x + o.width.max(0), min_x);
            let y1 = px(o.y + o.height.max(0), min_y);
            Tile {
                x: x0,
                y: y0,
                width: (x1 - x0).max(0) as u32,
                height: (y1 - y0).max(0) as u32,
            }
        })
        .collect();
    Some(StitchPlan {
        width: px(max_x, min_x).max(0) as u32,
        height: px(max_y, min_y).max(0) as u32,
        scale,
        tiles,
    })
}

/// Logical rect `(x, y, w, h)` → the buffer-pixel rect covering it on an
/// output with the given scale (screencopy frames are buffer pixels).
/// Edges are rounded outwards so a fractional scale never drops the
/// partly covered pixel row/column. A non-positive or non-finite scale
/// is treated as 1.
fn logical_to_buffer((x, y, w, h): (i32, i32, i32, i32), scale: f64) -> (i32, i32, i32, i32) {
    let s = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let x0 = (x as f64 * s).floor();
    let y0 = (y as f64 * s).floor();
    let x1 = ((x + w) as f64 * s).ceil();
    let y1 = ((y + h) as f64 * s).ceil();
    (x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32)
}

/// Clamp crop rect `(x, y, w, h)` to a `(width, height)` image. `None`
/// when nothing of it is left.
fn clamp_crop(
    (x, y, w, h): (i32, i32, i32, i32),
    (full_w, full_h): (u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let x = (x as u32).min(full_w);
    let y = (y as u32).min(full_h);
    let w = (w as u32).min(full_w.saturating_sub(x));
    let h = (h as u32).min(full_h.saturating_sub(y));
    (w > 0 && h > 0).then_some((x, y, w, h))
}

/// Convert raw pixel data from the compositor's format to RGBA8.
fn convert_to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
) -> Result<image::RgbaImage> {
    let mut img = image::RgbaImage::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let offset = (y * stride + x * 4) as usize;
            if offset + 3 >= data.len() {
                break;
            }
            let pixel = match format {
                // ARGB8888 — most common from compositors.
                wl_shm::Format::Argb8888 => {
                    let b = data[offset];
                    let g = data[offset + 1];
                    let r = data[offset + 2];
                    let a = data[offset + 3];
                    image::Rgba([r, g, b, a])
                }
                // XRGB8888 — like ARGB but alpha is unused (opaque).
                wl_shm::Format::Xrgb8888 => {
                    let b = data[offset];
                    let g = data[offset + 1];
                    let r = data[offset + 2];
                    image::Rgba([r, g, b, 255])
                }
                // ABGR8888
                wl_shm::Format::Abgr8888 => {
                    let r = data[offset];
                    let g = data[offset + 1];
                    let b = data[offset + 2];
                    let a = data[offset + 3];
                    image::Rgba([r, g, b, a])
                }
                // XBGR8888
                wl_shm::Format::Xbgr8888 => {
                    let r = data[offset];
                    let g = data[offset + 1];
                    let b = data[offset + 2];
                    image::Rgba([r, g, b, 255])
                }
                other => {
                    return Err(ScreenshotError::CaptureFailed(format!(
                        "unsupported shm format: {:?}",
                        other
                    )));
                }
            };
            img.put_pixel(x, y, pixel);
        }
    }

    Ok(img)
}

/// Create an anonymous shared memory file for the wl_shm_pool.
fn create_shm_file(size: usize) -> Result<std::fs::File> {
    use rustix::shm;

    let name = format!("/mshell-screenshot-{}", std::process::id());
    let fd = shm::open(
        &name,
        shm::OFlags::CREATE | shm::OFlags::EXCL | shm::OFlags::RDWR,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .map_err(|e| ScreenshotError::CaptureFailed(format!("shm_open: {e}")))?;

    // Immediately unlink so it's cleaned up.
    let _ = shm::unlink(&name);

    rustix::fs::ftruncate(&fd, size as u64)
        .map_err(|e| ScreenshotError::CaptureFailed(format!("ftruncate: {e}")))?;

    Ok(std::fs::File::from(fd))
}

// ── Wayland dispatch implementations ──────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handled by GlobalListContents
    }
}

impl Dispatch<wl_shm::WlShm, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_shm::WlShm,
        _event: wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_shm_pool::WlShmPool,
        _event: wl_shm_pool::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_buffer::WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, u32> for CaptureState {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.outputs.insert(name, proxy.clone());
        }
    }
}

impl Dispatch<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _event: zwlr_screencopy_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let mut fs = state.frame_state.lock().unwrap();
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                fs.format = Some(FrameFormat {
                    format: format.into_result().unwrap_or(wl_shm::Format::Argb8888),
                    width,
                    height,
                    stride,
                });
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                fs.ready = true;
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                fs.done = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                fs.failed = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(name: &str, x: i32, y: i32, w: i32, h: i32, scale: f64) -> OutputInfo {
        OutputInfo {
            name: name.into(),
            x,
            y,
            width: w,
            height: h,
            scale,
        }
    }

    #[test]
    fn stitch_plan_of_same_scale_outputs_is_one_to_one() {
        let plan = stitch_plan(&[
            out("A", 0, 0, 1920, 1080, 1.0),
            out("B", 1920, 0, 2560, 1440, 1.0),
        ])
        .unwrap();
        assert_eq!((plan.width, plan.height, plan.scale), (4480, 1440, 1.0));
        assert_eq!(
            plan.tiles,
            vec![
                Tile {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080
                },
                Tile {
                    x: 1920,
                    y: 0,
                    width: 2560,
                    height: 1440
                },
            ]
        );
    }

    #[test]
    fn stitch_plan_with_mixed_scales_uses_the_largest_and_keeps_the_layout() {
        // A 4K laptop panel at scale 2 (1920x1080 logical) left of a 1080p
        // monitor at scale 1, top-aligned. The old canvas summed the mode
        // sizes against logical positions: B was drawn on top of A's
        // right half and the canvas was 3840 + … wide.
        let plan = stitch_plan(&[
            out("eDP-1", 0, 0, 1920, 1080, 2.0),
            out("DP-1", 1920, 0, 1920, 1080, 1.0),
        ])
        .unwrap();
        assert_eq!((plan.width, plan.height, plan.scale), (7680, 2160, 2.0));
        assert_eq!(
            plan.tiles[0],
            Tile {
                x: 0,
                y: 0,
                width: 3840,
                height: 2160
            }
        );
        // B's 1920x1080 frame is upscaled into the 3840x2160 tile.
        assert_eq!(
            plan.tiles[1],
            Tile {
                x: 3840,
                y: 0,
                width: 3840,
                height: 2160
            }
        );
    }

    #[test]
    fn stitch_plan_fractional_scale_tiles_abut_and_negative_origins_shift() {
        let plan = stitch_plan(&[
            out("L", -1366, 100, 1366, 768, 1.0),
            out("R", 0, 0, 1707, 1067, 1.5),
        ])
        .unwrap();
        assert_eq!(plan.scale, 1.5);
        let (l, r) = (plan.tiles[0], plan.tiles[1]);
        assert_eq!((l.x, l.y), (0, 150));
        assert_eq!(l.x + i64::from(l.width), r.x, "no gap or overlap");
        assert_eq!(r.y, 0);
        assert_eq!(plan.width as i64, r.x + i64::from(r.width));
        assert_eq!(plan.height, r.height.max(l.height + 150));
    }

    #[test]
    fn stitch_plan_of_no_outputs_is_none_and_bogus_scale_counts_as_one() {
        assert!(stitch_plan(&[]).is_none());
        let plan = stitch_plan(&[out("A", 0, 0, 100, 50, f64::NAN)]).unwrap();
        assert_eq!((plan.width, plan.height, plan.scale), (100, 50, 1.0));
    }

    #[test]
    fn clamp_crop_keeps_rects_inside_the_frame() {
        assert_eq!(
            clamp_crop((10, 20, 30, 40), (100, 100)),
            Some((10, 20, 30, 40))
        );
        assert_eq!(
            clamp_crop((90, 95, 30, 40), (100, 100)),
            Some((90, 95, 10, 5)),
            "overhang is trimmed"
        );
        assert_eq!(
            clamp_crop((0, 0, 100, 100), (100, 100)),
            Some((0, 0, 100, 100))
        );
    }

    #[test]
    fn clamp_crop_rejects_empty_results() {
        assert_eq!(
            clamp_crop((100, 0, 10, 10), (100, 100)),
            None,
            "starts past the edge"
        );
        assert_eq!(clamp_crop((0, 0, 0, 10), (100, 100)), None);
        assert_eq!(clamp_crop((0, 0, 10, 10), (0, 0)), None);
    }

    #[test]
    fn logical_rects_scale_to_buffer_pixels() {
        let r = (100, 50, 640, 480);
        assert_eq!(logical_to_buffer(r, 1.0), r);
        assert_eq!(logical_to_buffer(r, 2.0), (200, 100, 1280, 960));
        // 1.25: 100*1.25 = 125, (100+640)*1.25 = 925 → 800 wide.
        assert_eq!(logical_to_buffer(r, 1.25), (125, 62, 800, 601));
        // Fractional edges round outwards.
        assert_eq!(logical_to_buffer((1, 1, 1, 1), 1.5), (1, 1, 2, 2));
    }

    #[test]
    fn bogus_scale_is_treated_as_one() {
        let r = (10, 20, 30, 40);
        for s in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(logical_to_buffer(r, s), r, "{s}");
        }
    }

    #[test]
    fn hidpi_region_crops_the_selected_pixels() {
        // A 1920x1080 logical output at 200 % yields a 3840x2160 frame;
        // the bottom-right logical quarter must map to the bottom-right
        // buffer quarter, not a quarter-sized patch in the middle.
        assert_eq!(
            clamp_crop(logical_to_buffer((960, 540, 960, 540), 2.0), (3840, 2160)),
            Some((1920, 1080, 1920, 1080))
        );
    }

    fn convert(data: &[u8], w: u32, h: u32, stride: u32, f: wl_shm::Format) -> Vec<[u8; 4]> {
        convert_to_rgba(data, w, h, stride, f)
            .unwrap()
            .pixels()
            .map(|p| p.0)
            .collect()
    }

    #[test]
    fn converts_each_supported_shm_format() {
        let px = [0x11, 0x22, 0x33, 0x80];
        assert_eq!(
            convert(&px, 1, 1, 4, wl_shm::Format::Argb8888),
            vec![[0x33, 0x22, 0x11, 0x80]]
        );
        assert_eq!(
            convert(&px, 1, 1, 4, wl_shm::Format::Xrgb8888),
            vec![[0x33, 0x22, 0x11, 0xFF]],
            "X formats are opaque whatever the padding byte holds"
        );
        assert_eq!(
            convert(&px, 1, 1, 4, wl_shm::Format::Abgr8888),
            vec![[0x11, 0x22, 0x33, 0x80]]
        );
        assert_eq!(
            convert(&px, 1, 1, 4, wl_shm::Format::Xbgr8888),
            vec![[0x11, 0x22, 0x33, 0xFF]]
        );
    }

    #[test]
    fn honours_row_stride_padding() {
        // 1x2 image, 8-byte stride: the second pixel row starts at 8.
        let data = [1, 2, 3, 255, 9, 9, 9, 9, 4, 5, 6, 255, 9, 9, 9, 9];
        assert_eq!(
            convert(&data, 1, 2, 8, wl_shm::Format::Xbgr8888),
            vec![[1, 2, 3, 255], [4, 5, 6, 255]]
        );
    }

    #[test]
    fn short_buffer_leaves_missing_pixels_blank_instead_of_panicking() {
        let data = [1, 2, 3, 255, 4, 5];
        assert_eq!(
            convert(&data, 2, 1, 8, wl_shm::Format::Xbgr8888),
            vec![[1, 2, 3, 255], [0, 0, 0, 0]]
        );
    }

    #[test]
    fn unsupported_format_is_an_error() {
        assert!(matches!(
            convert_to_rgba(&[0; 4], 1, 1, 4, wl_shm::Format::Rgb565),
            Err(ScreenshotError::CaptureFailed(_))
        ));
        // An empty frame has no pixel to reject.
        assert!(convert_to_rgba(&[], 0, 0, 0, wl_shm::Format::Rgb565).is_ok());
    }
}
