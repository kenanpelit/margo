mod capture;
mod common;
pub mod editor;
pub mod pin;
pub mod record;
mod selectors;
mod utils;
pub mod webcam;

pub use capture::{CaptureBackend, ScreenshotResult};
pub use common::OutputInfo;
pub use editor::{
    editor_available, launch_editor_blocking, pick_editor, pick_editor_with_override,
};
pub use selectors::area_selector::select_region;
pub use utils::query_outputs;

use crate::common::*;
use crate::record::{RecordBackend, RecordHandle, RecordResult, WfRecorderArgs, start_recording};
use crate::utils::default_screenshot_path;
use gtk4::glib;
use image::ImageEncoder;
use selectors::area_selector::RegionSelection;
use selectors::{monitor_selector, window_selector};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum CaptureArea {
    All,
    SelectRegion,
    /// Re-capture the last region committed via `SelectRegion` (same
    /// process, no reselect). Falls back to `SelectRegion` when
    /// nothing has been captured yet this session.
    RepeatRegion,
    SelectMonitor,
    SelectWindow,
}

/// Geometry of the last region a user committed through the area
/// selector, shared by every caller of `take_screenshot` (keybind,
/// launcher, menu). Backs `CaptureArea::RepeatRegion`. Lives only for
/// the daemon's lifetime — there is no cross-session persistence.
static LAST_REGION: OnceLock<Mutex<Option<RegionSelection>>> = OnceLock::new();

fn store_last_region(region: RegionSelection) {
    // A panic while holding this lock elsewhere would be a bug worth
    // surfacing, but it shouldn't also brick every future screenshot —
    // recover the poisoned guard instead of unwrap()'s hard abort.
    *LAST_REGION
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(region);
}

fn load_last_region() -> Option<RegionSelection> {
    LAST_REGION
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[derive(Debug, Clone)]
pub enum OutputTarget {
    FileAndClipboard,
    File,
    Clipboard,
    /// Capture → open in an annotation editor → when the editor
    /// writes its output, copy that file to the clipboard too. Falls
    /// back to the original capture for the clipboard step when the
    /// editor exits without saving.
    ///
    /// The `Option<String>` is an explicit editor override (e.g.
    /// `mshellctl screenshot region satty`). `None` uses the standard
    /// preference chain (`SCREENSHOT_EDITOR` env, then satty → swappy
    /// → gimp → krita) via [`editor::pick_editor`].
    EditAndSave(Option<String>),
    /// Capture → pin it on screen as a floating, always-on-top image
    /// ([`pin`]). Nothing is saved unless the user presses Ctrl+S on it.
    Pin,
}

#[derive(Debug, Clone)]
pub struct ScreenshotRequest {
    pub area: CaptureArea,
    pub target: OutputTarget,
}

#[derive(Debug, Clone)]
pub struct ScreenRecordRequest {
    pub area: CaptureArea,
    /// Audio spec: a source name, `default_output` / `default_input` /
    /// `both`, or several joined with `|` (mixed into one track). `None`
    /// records silently. See [`record::parse_audio`].
    pub audio: Option<String>,
    pub backend: RecordBackend,
    /// Webcam picture-in-picture bubble ([`webcam`]); `Off` = none.
    pub webcam: webcam::WebcamSize,
}

#[derive(Debug, Clone)]
pub enum ScreenSelectAreaRequest {
    SelectRegion,
    SelectMonitor,
}

#[derive(Debug, Clone)]
pub enum ScreenSelection {
    Region(RegionSelection),
    Monitor(String),
}

pub fn select_screen<F>(request: ScreenSelectAreaRequest, on_done: F)
where
    F: FnOnce(Result<ScreenSelection>) + Send + 'static,
{
    match request {
        ScreenSelectAreaRequest::SelectRegion => {
            let outputs = match query_outputs() {
                Ok(o) => o,
                Err(e) => return on_done(Err(e)),
            };
            select_region(&outputs, move |region_result| {
                // Pure preview / select API has no use for the
                // shortcut's output-target override — just unwrap
                // the RegionSelection half of the commit tuple.
                on_done(region_result.map(|(region, _override)| ScreenSelection::Region(region)));
            });
        }
        ScreenSelectAreaRequest::SelectMonitor => {
            let outputs = match query_outputs() {
                Ok(o) => o,
                Err(e) => return on_done(Err(e)),
            };
            if outputs.len() == 1 {
                on_done(Ok(ScreenSelection::Monitor(outputs[0].name.clone())));
            } else {
                monitor_selector::select_monitor(&outputs, move |monitor_result| {
                    on_done(monitor_result.map(ScreenSelection::Monitor));
                });
            }
        }
    }
}

/// Open the area selector and hand back the committed region as a
/// `grim -g` / slurp geometry (`"X,Y WxH"`) in **global** layout
/// coordinates — empty on cancel or error. Backs `mshellctl screenshot
/// select-region`, the region bridge `mscreenshot` uses; the selector
/// itself reports coordinates local to the output the region is on, which
/// grim would read as a rectangle on the output at the layout origin.
pub fn select_region_geometry<F>(on_done: F)
where
    F: FnOnce(String) + Send + 'static,
{
    let outputs = match query_outputs() {
        Ok(o) => o,
        Err(_) => return on_done(String::new()),
    };
    let layout = outputs.clone();
    select_region(&outputs, move |result| {
        on_done(match result {
            Ok((region, _override)) => region_geometry(&region, &layout),
            Err(_) => String::new(),
        })
    });
}

/// `"X,Y WxH"` for `region` in global layout coordinates.
fn region_geometry(region: &RegionSelection, outputs: &[OutputInfo]) -> String {
    let (x, y) = region_global_origin(region, outputs);
    format!("{x},{y} {}x{}", region.width, region.height)
}

pub fn record_screen<S, D>(
    request: ScreenRecordRequest,
    _delay: Duration,
    on_started: S,
    on_done: D,
) where
    S: FnOnce(anyhow::Result<RecordHandle>) + Send + 'static,
    D: FnOnce(anyhow::Result<RecordResult>) + Send + 'static,
{
    let outputs = match query_outputs() {
        Ok(o) => o,
        Err(e) => return on_started(Err(e.into())),
    };
    let ScreenRecordRequest {
        area,
        audio,
        backend,
        webcam,
    } = request;
    let launch = move |args: WfRecorderArgs,
                       target: Option<webcam::Target>,
                       outputs: Vec<OutputInfo>,
                       on_started: S,
                       on_done: D| {
        launch_recording(
            backend, audio, webcam, args, target, &outputs, on_started, on_done,
        );
    };
    match area {
        // `RepeatRegion` has no meaning for recording (there's no
        // "repeat the last clip's region" use case) — it degrades to
        // a normal selection rather than failing to compile.
        CaptureArea::SelectRegion | CaptureArea::RepeatRegion => {
            let layout = outputs.clone();
            select_region(&outputs, move |region_result| match region_result {
                // Recording cares only about the rect — Ctrl+S /
                // Ctrl+E shortcut overrides are screenshot-only,
                // so the second tuple element is discarded.
                Ok((region, _override)) => {
                    let (x, y) = region_global_origin(&region, &layout);
                    let args = WfRecorderArgs::Region {
                        x,
                        y,
                        width: region.width,
                        height: region.height,
                    };
                    let target = webcam::Target {
                        output: region.output.clone(),
                        x: region.x,
                        y: region.y,
                        width: region.width,
                        height: region.height,
                    };
                    launch(args, Some(target), layout, on_started, on_done);
                }
                Err(e) => on_started(Err(e.into())),
            });
        }
        CaptureArea::SelectMonitor => {
            let layout = outputs.clone();
            monitor_selector::select_monitor(
                &outputs,
                move |monitor_result| match monitor_result {
                    Ok(name) => {
                        let target = output_target(&name, &layout);
                        let args = WfRecorderArgs::Monitor { name };
                        launch(args, target, layout, on_started, on_done);
                    }
                    Err(e) => on_started(Err(e.into())),
                },
            );
        }
        CaptureArea::SelectWindow => {
            let layout = outputs.clone();
            window_selector::select_window(&outputs, move |window_result| match window_result {
                Ok(win) => {
                    let target =
                        layout
                            .iter()
                            .find(|o| o.name == win.output)
                            .map(|o| webcam::Target {
                                output: o.name.clone(),
                                x: win.x - o.x,
                                y: win.y - o.y,
                                width: win.width,
                                height: win.height,
                            });
                    let args = WfRecorderArgs::Window {
                        x: win.x,
                        y: win.y,
                        width: win.width,
                        height: win.height,
                    };
                    launch(args, target, layout, on_started, on_done);
                }
                Err(e) => on_started(Err(e.into())),
            });
        }
        CaptureArea::All => {
            if outputs.len() > 1 {
                let layout = outputs.clone();
                monitor_selector::select_monitor(&outputs, move |monitor_result| {
                    match monitor_result {
                        Ok(name) => {
                            let target = output_target(&name, &layout);
                            let args = WfRecorderArgs::Monitor { name };
                            launch(args, target, layout, on_started, on_done);
                        }
                        Err(e) => on_started(Err(e.into())),
                    }
                });
            } else {
                let target = outputs
                    .first()
                    .and_then(|o| output_target(&o.name, &outputs));
                launch(WfRecorderArgs::All, target, outputs, on_started, on_done);
            }
        }
    }
}

/// A whole output as the webcam's target area.
fn output_target(name: &str, outputs: &[OutputInfo]) -> Option<webcam::Target> {
    outputs
        .iter()
        .find(|o| o.name == name)
        .map(|o| webcam::Target {
            output: o.name.clone(),
            x: 0,
            y: 0,
            width: o.width,
            height: o.height,
        })
}

/// Start the recorder — after the webcam bubble shows its first frame when
/// one was asked for, so the clip opens with the camera already in place.
/// A camera that can't start never blocks the recording: it records
/// without, and says why.
#[allow(clippy::too_many_arguments)]
fn launch_recording<S, D>(
    backend: RecordBackend,
    audio: Option<String>,
    webcam: webcam::WebcamSize,
    args: WfRecorderArgs,
    target: Option<webcam::Target>,
    outputs: &[OutputInfo],
    on_started: S,
    on_done: D,
) where
    S: FnOnce(anyhow::Result<RecordHandle>) + 'static,
    D: FnOnce(anyhow::Result<RecordResult>) + Send + 'static,
{
    let cam = match target.filter(|_| webcam != webcam::WebcamSize::Off) {
        Some(t) => match webcam::start_bubble(&t, outputs, webcam) {
            Ok(h) => Some(h),
            Err(e) => {
                webcam_unavailable(&e.to_string());
                None
            }
        },
        None => None,
    };
    let Some(cam) = cam else {
        on_started(Ok(start_recording(backend, audio, args, on_done)));
        return;
    };
    let stopper = cam.clone();
    let on_done = move |r: anyhow::Result<RecordResult>| {
        stopper.stop();
        on_done(r);
    };
    let ready = cam.clone();
    webcam::when_ready(&cam, Duration::from_millis(2_500), move || {
        if let Some(why) = ready.failure() {
            webcam_unavailable(&why);
        }
        on_started(Ok(start_recording(backend, audio, args, on_done)));
    });
}

fn webcam_unavailable(why: &str) {
    use mshell_process::desktop::{Notice, notify};
    notify(
        Notice::new(
            "mshell",
            "Recording without webcam",
            format!("The webcam overlay could not start: {why}"),
        )
        .icon("camera-disabled-symbolic"),
    );
}

pub fn take_screenshot<F>(request: ScreenshotRequest, delay: Duration, on_done: F)
where
    F: FnOnce(Result<ScreenshotResult>) + Send + 'static,
{
    match request.area {
        CaptureArea::SelectRegion => {
            let outputs = match query_outputs() {
                Ok(o) => o,
                Err(e) => return on_done(Err(e)),
            };
            let target = request.target.clone();
            select_region(&outputs, move |region_result| match region_result {
                // Ctrl+S / Ctrl+E inside the selector can override
                // the OutputTarget the calling widget originally
                // wanted — that's the whole point of the in-selector
                // shortcuts. `None` keeps the caller's target.
                Ok((region, override_target)) => {
                    store_last_region(region.clone());
                    let final_target = override_target.unwrap_or(target);
                    let pin_at = region_placement(&region);
                    capture_and_finish_async(
                        move || CaptureBackend::new()?.capture_region(&region),
                        final_target,
                        delay,
                        pin_at,
                        on_done,
                    );
                }
                Err(e) => on_done(Err(e)),
            });
        }
        CaptureArea::RepeatRegion => match load_last_region() {
            Some(region) => {
                let pin_at = region_placement(&region);
                capture_and_finish_async(
                    move || CaptureBackend::new()?.capture_region(&region),
                    request.target,
                    delay,
                    pin_at,
                    on_done,
                );
            }
            None => {
                // Nothing captured yet this session — behave like a
                // first-time region capture (which seeds the cache
                // for the next repeat).
                take_screenshot(
                    ScreenshotRequest {
                        area: CaptureArea::SelectRegion,
                        target: request.target,
                    },
                    delay,
                    on_done,
                );
            }
        },
        CaptureArea::SelectMonitor => {
            let outputs = match query_outputs() {
                Ok(o) => o,
                Err(e) => return on_done(Err(e)),
            };
            let target = request.target.clone();
            monitor_selector::select_monitor(
                &outputs,
                move |monitor_result| match monitor_result {
                    Ok(name) => capture_and_finish_async(
                        move || CaptureBackend::new()?.capture_output(&name),
                        target,
                        delay,
                        None,
                        on_done,
                    ),
                    Err(e) => on_done(Err(e)),
                },
            );
        }
        CaptureArea::SelectWindow => {
            let outputs = match query_outputs() {
                Ok(o) => o,
                Err(e) => return on_done(Err(e)),
            };
            let target = request.target.clone();
            window_selector::select_window(&outputs, move |window_result| match window_result {
                Ok(win) => capture_and_finish_async(
                    move || CaptureBackend::new()?.capture_window(&win),
                    target,
                    delay,
                    None,
                    on_done,
                ),
                Err(e) => on_done(Err(e)),
            });
        }
        CaptureArea::All => {
            capture_and_finish_async(
                || CaptureBackend::new()?.capture_all(),
                request.target,
                delay,
                None,
                on_done,
            );
        }
    }
}

/// Where a pin of `region` opens: exactly over the captured area, 1:1.
fn region_placement(region: &RegionSelection) -> Option<pin::PinPlacement> {
    Some(pin::PinPlacement {
        output: Some(region.output.clone()),
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
    })
}

/// Capture on a worker thread, then deliver per `target` and report on the
/// GTK thread. A [`OutputTarget::Pin`] target hands the image to
/// [`pin::show_pin`] there instead (at `pin_at`, or inset on the first
/// output when the capture has no region).
fn capture_and_finish_async<C, F>(
    capture: C,
    target: OutputTarget,
    delay: Duration,
    pin_at: Option<pin::PinPlacement>,
    on_done: F,
) where
    C: FnOnce() -> Result<image::RgbaImage> + Send + 'static,
    F: FnOnce(Result<ScreenshotResult>) + Send + 'static,
{
    std::thread::spawn(move || {
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if matches!(target, OutputTarget::Pin) {
            let captured = capture();
            let outputs = query_outputs().unwrap_or_default();
            glib::idle_add_once(move || match captured {
                Ok(image) => {
                    let placement = pin_at.unwrap_or_else(|| {
                        pin::PinPlacement::inset(outputs.first(), image.width(), image.height())
                    });
                    pin::show_pin(image, placement, &outputs);
                    on_done(Ok(ScreenshotResult {
                        saved_path: None,
                        in_clipboard: false,
                    }));
                }
                Err(e) => on_done(Err(e)),
            });
            return;
        }
        let result = capture().and_then(|image| finish_capture(image, &target));
        glib::idle_add_once(move || {
            on_done(result);
        });
    });
}

fn finish_capture(image: image::RgbaImage, target: &OutputTarget) -> Result<ScreenshotResult> {
    match target {
        OutputTarget::FileAndClipboard => {
            let path = default_screenshot_path();
            save_to_file(&image, &path)?;
            copy_to_clipboard(&image)?;
            Ok(ScreenshotResult {
                saved_path: Some(path),
                in_clipboard: true,
            })
        }
        OutputTarget::File => {
            let path = default_screenshot_path();
            save_to_file(&image, &path)?;
            Ok(ScreenshotResult {
                saved_path: Some(path),
                in_clipboard: false,
            })
        }
        OutputTarget::Clipboard => {
            copy_to_clipboard(&image)?;
            Ok(ScreenshotResult {
                saved_path: None,
                in_clipboard: true,
            })
        }
        OutputTarget::EditAndSave(editor) => finish_with_editor(image, editor.as_deref()),
        // Pins are shown by `capture_and_finish_async` on the GTK thread;
        // nothing to deliver here.
        OutputTarget::Pin => Ok(ScreenshotResult {
            saved_path: None,
            in_clipboard: false,
        }),
    }
}

/// Edit-then-save flow:
///   1. Persist the raw capture to a temp PNG (editor input).
///   2. Pick + launch the editor synchronously, telling it to
///      write its annotated output to the user's Screenshots dir.
///   3. If the editor wrote that output, clipboard it and report
///      the annotated path; otherwise fall back to the raw capture
///      (still saved + clipboarded) so the user is never left with
///      no screenshot at all.
/// If no editor is available, behaves identically to
/// `FileAndClipboard` and warns via the returned path remaining
/// unedited.
fn finish_with_editor(
    image: image::RgbaImage,
    override_editor: Option<&str>,
) -> Result<ScreenshotResult> {
    let raw_path = default_screenshot_path();
    save_to_file(&image, &raw_path)?;
    let Some(editor) = editor::pick_editor_with_override(override_editor) else {
        // No annotation tool installed — degrade to FileAndClipboard.
        copy_to_clipboard(&image)?;
        return Ok(ScreenshotResult {
            saved_path: Some(raw_path),
            in_clipboard: true,
        });
    };

    // Editor writes its annotated output to a sibling path so the
    // raw + annotated coexist on disk. Suffix is added before the
    // extension so the file still ends in .png.
    let annotated_path = annotated_sibling(&raw_path);

    match editor::launch_editor_blocking(&raw_path, &annotated_path, &editor) {
        Ok(true) => {
            // Editor saved → clipboard the annotated PNG.
            if let Ok(bytes) = std::fs::read(&annotated_path) {
                let _ = copy_png_bytes_to_clipboard(&bytes);
            }
            Ok(ScreenshotResult {
                saved_path: Some(annotated_path),
                in_clipboard: true,
            })
        }
        Ok(false) => {
            // Editor closed without saving (or fire-and-forget tool
            // like gimp/krita). Clipboard the raw capture so the
            // user still gets something useful.
            copy_to_clipboard(&image)?;
            Ok(ScreenshotResult {
                saved_path: Some(raw_path),
                in_clipboard: true,
            })
        }
        Err(e) => Err(e),
    }
}

/// Global layout origin of an area-selector region. The selector reports
/// coordinates local to `region.output`; wf-recorder's `--geometry` (like
/// slurp/grim) takes global ones. An unknown output is left untranslated.
fn region_global_origin(region: &RegionSelection, outputs: &[OutputInfo]) -> (i32, i32) {
    let (ox, oy) = outputs
        .iter()
        .find(|o| o.name == region.output)
        .map(|o| (o.x, o.y))
        .unwrap_or((0, 0));
    (region.x + ox, region.y + oy)
}

fn annotated_sibling(path: &Path) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("shot");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("png");
    let mut out = path.to_path_buf();
    out.set_file_name(format!("{stem}-edit.{ext}"));
    out
}

fn copy_png_bytes_to_clipboard(bytes: &[u8]) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg("image/png")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| ScreenshotError::ClipboardFailed(format!("failed to spawn wl-copy: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(bytes)
            .map_err(|e| ScreenshotError::ClipboardFailed(e.to_string()))?;
    }
    let status = child
        .wait()
        .map_err(|e| ScreenshotError::ClipboardFailed(e.to_string()))?;
    if !status.success() {
        return Err(ScreenshotError::ClipboardFailed(
            "wl-copy exited with non-zero status".into(),
        ));
    }
    Ok(())
}

fn save_to_file(image: &image::RgbaImage, path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);

    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        writer,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );

    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| ScreenshotError::EncodingFailed(e.to_string()))
}

fn copy_to_clipboard(image: &image::RgbaImage) -> Result<()> {
    let mut png_data: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut png_data,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e: image::ImageError| ScreenshotError::EncodingFailed(e.to_string()))?;

    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg("image/png")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| ScreenshotError::ClipboardFailed(format!("failed to spawn wl-copy: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(&png_data)
            .map_err(|e| ScreenshotError::ClipboardFailed(e.to_string()))?;
    }

    let status = child
        .wait()
        .map_err(|e| ScreenshotError::ClipboardFailed(e.to_string()))?;

    if !status.success() {
        return Err(ScreenshotError::ClipboardFailed(
            "wl-copy exited with non-zero status".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotated_sibling_keeps_dir_and_extension() {
        assert_eq!(
            annotated_sibling(Path::new("/p/Screenshots/2026_01_02_screenshot.png")),
            PathBuf::from("/p/Screenshots/2026_01_02_screenshot-edit.png")
        );
        assert_eq!(
            annotated_sibling(Path::new("shot.jpeg")),
            PathBuf::from("shot-edit.jpeg")
        );
        assert_eq!(
            annotated_sibling(Path::new("/p/noext")),
            PathBuf::from("/p/noext-edit.png")
        );
    }

    fn output(name: &str, x: i32, y: i32) -> OutputInfo {
        OutputInfo {
            name: name.into(),
            x,
            y,
            width: 1920,
            height: 1080,
            scale: 1.0,
        }
    }

    #[test]
    fn recording_region_is_translated_to_global_coordinates() {
        let outputs = [output("DP-1", 0, 0), output("HDMI-A-1", 1920, -200)];
        let region = |name: &str| RegionSelection {
            output: name.into(),
            x: 100,
            y: 50,
            width: 640,
            height: 480,
        };
        assert_eq!(region_global_origin(&region("DP-1"), &outputs), (100, 50));
        assert_eq!(
            region_global_origin(&region("HDMI-A-1"), &outputs),
            (2020, -150)
        );
        assert_eq!(region_global_origin(&region("gone"), &outputs), (100, 50));
    }

    /// The owner's layout: DP-3 at the origin, eDP-1 below it offset to
    /// x=320. A region the selector reports on eDP-1 must reach grim with
    /// eDP-1's origin added, or grim captures the matching rectangle of
    /// DP-3 instead.
    #[test]
    fn bridge_geometry_is_global() {
        let outputs = [output("DP-3", 0, 0), output("eDP-1", 320, 1440)];
        let on = |name: &str| RegionSelection {
            output: name.into(),
            x: 10,
            y: 20,
            width: 300,
            height: 200,
        };
        assert_eq!(region_geometry(&on("eDP-1"), &outputs), "330,1460 300x200");
        assert_eq!(region_geometry(&on("DP-3"), &outputs), "10,20 300x200");
    }

    #[test]
    fn last_region_round_trips() {
        store_last_region(RegionSelection {
            output: "DP-1".into(),
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        });
        let r = load_last_region().unwrap();
        assert_eq!(
            (r.output.as_str(), r.x, r.y, r.width, r.height),
            ("DP-1", 1, 2, 3, 4)
        );
    }

    #[test]
    fn saved_png_decodes_back_to_the_same_pixels() {
        let dir = std::env::temp_dir().join(format!("mshell-shot-{}", std::process::id()));
        let path = dir.join("nested").join("a.png");
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
        img.put_pixel(1, 0, image::Rgba([4, 5, 6, 128]));
        save_to_file(&img, &path).unwrap();
        let back = image::open(&path).unwrap().to_rgba8();
        assert_eq!(back, img);
        let _ = std::fs::remove_dir_all(dir);
    }
}
