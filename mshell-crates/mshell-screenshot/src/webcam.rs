//! Webcam picture-in-picture for screen recordings.
//!
//! The recorders capture the screen, so the camera is put *on* the screen:
//! a small click-through bubble on an overlay layer, anchored inside the
//! bottom-right corner of the area being recorded. It is encoded into the
//! clip in the same pass (no second encode, no post-processing), and doubles
//! as the live preview while recording.
//!
//! Frames come from `ffmpeg` reading the V4L2 node and writing raw RGBA at
//! the bubble's device-pixel size to a pipe; a reader thread keeps the
//! latest frame and a ~30 Hz GTK tick paints it. The camera mode is chosen
//! from `v4l2-ctl --list-formats-ext` (port of screen-capture's chooser):
//! a mode that really runs at 30 fps wins over a sharper slow one, MJPG over
//! raw YUYV, greyscale (IR) nodes are skipped. Without `v4l2-ctl` the first
//! capture node is opened with ffmpeg's defaults.
//!
//! Every parser / chooser / geometry helper is pure and unit-tested against
//! this machine's real `v4l2-ctl` output; the process + GTK lifecycle is
//! not (it would switch the camera on).

use crate::common::OutputInfo;
use crate::utils::find_gdk_monitor;
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::LayerShell;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Bubble size relative to the recorded area's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WebcamSize {
    #[default]
    Off,
    Small,
    Medium,
    Large,
}

impl WebcamSize {
    pub const ALL: [WebcamSize; 4] = [
        WebcamSize::Off,
        WebcamSize::Small,
        WebcamSize::Medium,
        WebcamSize::Large,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WebcamSize::Off => "No webcam",
            WebcamSize::Small => "Webcam · small",
            WebcamSize::Medium => "Webcam · medium",
            WebcamSize::Large => "Webcam · large",
        }
    }

    /// IPC / CLI token.
    pub fn token(self) -> &'static str {
        match self {
            WebcamSize::Off => "-",
            WebcamSize::Small => "small",
            WebcamSize::Medium => "medium",
            WebcamSize::Large => "large",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "-" | "off" | "none" => Some(WebcamSize::Off),
            "small" | "s" => Some(WebcamSize::Small),
            "medium" | "m" | "on" => Some(WebcamSize::Medium),
            "large" | "l" => Some(WebcamSize::Large),
            _ => None,
        }
    }

    /// Bubble width as a share of the recorded area's width.
    fn fraction(self) -> f64 {
        match self {
            WebcamSize::Off => 0.0,
            WebcamSize::Small => 0.14,
            WebcamSize::Medium => 0.20,
            WebcamSize::Large => 0.28,
        }
    }
}

/// One advertised capture mode.
#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// Modes from `v4l2-ctl --list-formats-ext`.
pub fn parse_formats(out: &str) -> Vec<Mode> {
    let mut modes = Vec::new();
    let mut format = String::new();
    let mut size: Option<(u32, u32)> = None;
    for line in out.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.contains("]:") {
            // `[0]: 'MJPG' (Motion-JPEG, compressed)`
            format = l.split('\'').nth(1).unwrap_or("").to_string();
            size = None;
        } else if let Some(rest) = l.strip_prefix("Size:") {
            size = rest.split_whitespace().find_map(parse_wh);
        } else if let Some(rest) = l.strip_prefix("Interval:") {
            // `Discrete 0.033s (30.000 fps)` — read the fps in parentheses.
            let fps = rest
                .split('(')
                .nth(1)
                .and_then(|p| p.split_whitespace().next())
                .and_then(|n| n.parse::<f64>().ok());
            if let (Some((w, h)), Some(fps)) = (size, fps)
                && !format.is_empty()
                && fps > 0.0
            {
                modes.push(Mode {
                    format: format.clone(),
                    width: w,
                    height: h,
                    fps,
                });
            }
        }
    }
    modes
}

fn parse_wh(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// The mode to open: one that reaches 30 fps first (preferring 1280×720,
/// then 640×360, then 640×480), MJPG before YUYV; otherwise the fastest.
/// Greyscale-only nodes (IR cameras) return `None`.
pub fn choose_mode(modes: &[Mode]) -> Option<Mode> {
    let res_rank = |m: &Mode| match (m.width, m.height) {
        (1280, 720) => 0,
        (640, 360) => 1,
        (640, 480) => 2,
        _ => 3,
    };
    let kind_rank = |m: &Mode| match m.format.as_str() {
        "MJPG" => 0,
        "YUYV" => 1,
        _ => 2,
    };
    modes
        .iter()
        .filter(|m| m.format != "GREY" && m.format != "Y8" && m.format != "Y16")
        .min_by(|a, b| {
            let fast = |m: &Mode| m.fps >= 29.5;
            match (fast(a), fast(b)) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                // Both reach 30 fps: preferred size, then MJPG.
                (true, true) => (res_rank(a), kind_rank(a))
                    .cmp(&(res_rank(b), kind_rank(b)))
                    .then(b.fps.total_cmp(&a.fps)),
                // Neither does: the fastest wins, then size / kind.
                (false, false) => b
                    .fps
                    .total_cmp(&a.fps)
                    .then((res_rank(a), kind_rank(a)).cmp(&(res_rank(b), kind_rank(b)))),
            }
            .then((b.width * b.height).cmp(&(a.width * a.height)))
        })
        .cloned()
}

/// `/dev/videoN` nodes from `v4l2-ctl --list-devices`, in order.
pub fn parse_list_devices(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("/dev/video"))
        .map(str::to_string)
        .collect()
}

/// Whether a `v4l2-ctl --info` dump is a capture node: its *Device Caps*
/// (not the whole-device Capabilities) list "Video Capture".
pub fn is_capture_node(info: &str) -> bool {
    let mut in_caps = false;
    for line in info.lines() {
        let t = line.trim();
        if t.starts_with("Device Caps") {
            in_caps = true;
            continue;
        }
        if in_caps {
            // Caps are indented one level deeper than their header.
            if !line.starts_with("\t\t") {
                return false;
            }
            if t == "Video Capture" {
                return true;
            }
        }
    }
    false
}

/// The camera to use: the first capture node with a colour mode, and that
/// mode (`None` = let ffmpeg choose). Blocking (`v4l2-ctl` runs) — call off
/// the GTK thread or accept a few ms.
pub fn find_camera() -> Option<(String, Option<Mode>)> {
    let run = |args: &[&str]| -> Option<String> {
        let out = Command::new("v4l2-ctl")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    if !margo_fs::path::on_path("v4l2-ctl") {
        // No way to inspect modes: the first node is the capture node on
        // every UVC camera.
        return std::path::Path::new("/dev/video0")
            .exists()
            .then(|| ("/dev/video0".to_string(), None));
    }
    for dev in parse_list_devices(&run(&["--list-devices"])?) {
        if !run(&["-d", &dev, "--info"]).is_some_and(|i| is_capture_node(&i)) {
            continue;
        }
        let modes = parse_formats(&run(&["-d", &dev, "--list-formats-ext"]).unwrap_or_default());
        if let Some(mode) = choose_mode(&modes) {
            return Some((dev, Some(mode)));
        }
    }
    None
}

/// `ffmpeg` argv: open `device` (at `mode` when known), crop to an 8:9
/// portrait bubble, scale to `w × h` device pixels, raw RGBA to stdout.
pub fn ffmpeg_argv(device: &str, mode: Option<&Mode>, w: u32, h: u32) -> Vec<String> {
    let mut a: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-f",
        "v4l2",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(m) = mode {
        match m.format.as_str() {
            "MJPG" => a.extend(["-input_format".into(), "mjpeg".into()]),
            "YUYV" => a.extend(["-input_format".into(), "yuyv422".into()]),
            _ => {}
        }
        a.extend([
            "-framerate".into(),
            format!("{}", m.fps.round() as u32),
            "-video_size".into(),
            format!("{}x{}", m.width, m.height),
        ]);
    }
    a.extend([
        "-i".into(),
        device.into(),
        "-vf".into(),
        format!("crop=ih*8/9:ih,scale={w}:{h},format=rgba"),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "-".into(),
    ]);
    a
}

/// The area being recorded, output-local logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Where the bubble goes: logical size and its margins from the output's
/// right / bottom edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bubble {
    pub width: i32,
    pub height: i32,
    pub right: i32,
    pub bottom: i32,
}

const BUBBLE_MARGIN: i32 = 24;
const BUBBLE_MIN_W: i32 = 96;
const BUBBLE_MAX_W: i32 = 480;

/// Bubble geometry inside the bottom-right of `t` on an `out_w × out_h`
/// output. Width = the size's share of the area (clamped, even), 8:9.
pub fn bubble_geometry(t: &Target, out_w: i32, out_h: i32, size: WebcamSize) -> Bubble {
    let raw = (f64::from(t.width) * size.fraction()).round() as i32;
    let width = (raw.clamp(BUBBLE_MIN_W, BUBBLE_MAX_W) / 2) * 2;
    let height = ((width * 9 / 8) / 2) * 2;
    let right = (out_w - (t.x + t.width) + BUBBLE_MARGIN).max(0);
    let bottom = (out_h - (t.y + t.height) + BUBBLE_MARGIN).max(0);
    Bubble {
        width,
        height,
        right,
        bottom,
    }
}

/// Controls a running bubble from any thread.
#[derive(Clone, Debug)]
pub struct WebcamHandle {
    stop: Arc<AtomicBool>,
    first_frame: Arc<AtomicBool>,
    /// Why the camera could not start (no camera, ffmpeg missing, …).
    failed: Arc<Mutex<Option<String>>>,
}

impl WebcamHandle {
    /// Close the bubble and release the camera (on the next GTK tick).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// The camera delivered its first frame (the bubble shows a picture).
    pub fn has_frame(&self) -> bool {
        self.first_frame.load(Ordering::SeqCst)
    }

    /// The reason the camera did not start, once known.
    pub fn failure(&self) -> Option<String> {
        self.failed.lock().ok().and_then(|f| f.clone())
    }
}

/// Paint cadence; the camera itself runs at ≤ 30 fps.
const TICK: Duration = Duration::from_millis(33);

/// Show the bubble for `target` and start the camera. GTK main thread only;
/// the camera lookup (`v4l2-ctl`) and `ffmpeg` run on a worker thread, so
/// this returns at once. A camera that can't start closes the bubble and
/// reports through [`WebcamHandle::failure`].
pub fn start_bubble(
    target: &Target,
    outputs: &[OutputInfo],
    size: WebcamSize,
) -> anyhow::Result<WebcamHandle> {
    if size == WebcamSize::Off {
        anyhow::bail!("webcam is off");
    }
    if !margo_fs::path::on_path("ffmpeg") {
        anyhow::bail!("the webcam overlay needs ffmpeg");
    }
    let out = outputs
        .iter()
        .find(|o| o.name == target.output)
        .ok_or_else(|| anyhow::anyhow!("output {} not found", target.output))?;
    let bubble = bubble_geometry(target, out.width, out.height, size);
    let scale = if out.scale > 0.0 { out.scale } else { 1.0 };
    let px_w = ((f64::from(bubble.width) * scale).round() as u32 / 2) * 2;
    let px_h = ((f64::from(bubble.height) * scale).round() as u32 / 2) * 2;

    let handle = WebcamHandle {
        stop: Arc::new(AtomicBool::new(false)),
        first_frame: Arc::new(AtomicBool::new(false)),
        failed: Arc::new(Mutex::new(None)),
    };
    let latest: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let child: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));

    // Worker: find the camera, run ffmpeg, keep the newest frame.
    {
        let latest = latest.clone();
        let child = child.clone();
        let handle = handle.clone();
        std::thread::spawn(move || {
            let fail = |why: String| {
                if let Ok(mut f) = handle.failed.lock() {
                    *f = Some(why);
                }
            };
            let Some((device, mode)) = find_camera() else {
                return fail("no webcam found".into());
            };
            let spawned = Command::new("ffmpeg")
                .args(ffmpeg_argv(&device, mode.as_ref(), px_w, px_h))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let mut proc = match spawned {
                Ok(p) => p,
                Err(e) => return fail(format!("spawn ffmpeg: {e}")),
            };
            let Some(mut stdout) = proc.stdout.take() else {
                return fail("ffmpeg has no stdout".into());
            };
            if let Ok(mut c) = child.lock() {
                *c = Some(proc);
            }
            // Stopped while the camera was being looked up.
            if handle.stop.load(Ordering::SeqCst) {
                return;
            }
            let mut buf = vec![0u8; px_w as usize * px_h as usize * 4];
            let mut frames = 0u64;
            while !handle.stop.load(Ordering::SeqCst) {
                if stdout.read_exact(&mut buf).is_err() {
                    break; // ffmpeg ended (killed, camera busy / unplugged)
                }
                frames += 1;
                if let Ok(mut slot) = latest.lock() {
                    *slot = Some(buf.clone());
                }
            }
            if frames == 0 && !handle.stop.load(Ordering::SeqCst) {
                fail(format!("{device} delivered no picture (busy?)"));
            }
        });
    }

    let window = gtk4::Window::new();
    window.set_decorated(false);
    window.add_css_class("screen-record-webcam-window");
    window.init_layer_shell();
    window.set_namespace(Some("mshell-webcam"));
    window.set_layer(gtk4_layer_shell::Layer::Overlay);
    window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
    window.set_anchor(gtk4_layer_shell::Edge::Right, true);
    window.set_margin(gtk4_layer_shell::Edge::Right, bubble.right);
    window.set_margin(gtk4_layer_shell::Edge::Bottom, bubble.bottom);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
    if let Some(display) = gdk::Display::default()
        && let Some(monitor) = find_gdk_monitor(&display.monitors(), out)
    {
        window.set_monitor(Some(&monitor));
    }
    let picture = gtk4::Picture::new();
    picture.set_size_request(bubble.width, bubble.height);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk4::ContentFit::Cover);
    picture.add_css_class("screen-record-webcam");
    picture.set_overflow(gtk4::Overflow::Hidden);
    picture.set_can_target(false);
    window.set_child(Some(&picture));
    // Click-through: an empty input region, so the bubble never steals a
    // click from what is being recorded.
    window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(Some(&gtk4::cairo::Region::create()));
        }
    });
    // Stay unmapped until the first frame: a blank card must not be the
    // first thing the clip shows.
    window.set_visible(false);

    {
        let handle = handle.clone();
        glib::timeout_add_local(TICK, move || {
            if handle.stop.load(Ordering::SeqCst) || handle.failure().is_some() {
                if let Ok(mut c) = child.lock()
                    && let Some(mut proc) = c.take()
                {
                    let _ = proc.kill();
                    let _ = proc.wait();
                }
                window.close();
                return glib::ControlFlow::Break;
            }
            let frame = latest.lock().ok().and_then(|mut s| s.take());
            if let Some(frame) = frame {
                let bytes = glib::Bytes::from_owned(frame);
                let tex = gdk::MemoryTexture::new(
                    px_w as i32,
                    px_h as i32,
                    gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    px_w as usize * 4,
                );
                picture.set_paintable(Some(&tex));
                if !handle.first_frame.swap(true, Ordering::SeqCst) {
                    window.present();
                }
            }
            glib::ControlFlow::Continue
        });
    }
    Ok(handle)
}

/// Run `then` once the bubble shows its first frame (so the clip opens with
/// the camera in it), when the camera failed, or after `timeout`.
pub fn when_ready(handle: &WebcamHandle, timeout: Duration, then: impl FnOnce() + 'static) {
    let handle = handle.clone();
    let started = std::time::Instant::now();
    let mut then = Some(then);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if handle.has_frame() || handle.failure().is_some() || started.elapsed() >= timeout {
            if let Some(f) = then.take() {
                f();
            }
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMATS: &str = include_str!("../tests/fixtures/v4l2_formats_video0.txt");
    const FORMATS_IR: &str = include_str!("../tests/fixtures/v4l2_formats_video2.txt");
    const INFO_CAPTURE: &str = include_str!("../tests/fixtures/v4l2_info_video0.txt");
    const INFO_META: &str = include_str!("../tests/fixtures/v4l2_info_video1.txt");
    const LIST: &str = include_str!("../tests/fixtures/v4l2_list_devices.txt");

    #[test]
    fn parses_real_formats() {
        let modes = parse_formats(FORMATS);
        assert_eq!(modes.len(), 14);
        assert_eq!(
            modes[0],
            Mode {
                format: "MJPG".into(),
                width: 1280,
                height: 720,
                fps: 30.0
            }
        );
        let yuyv_720 = modes
            .iter()
            .find(|m| m.format == "YUYV" && m.width == 1280)
            .unwrap();
        assert_eq!(yuyv_720.fps, 10.0);
    }

    #[test]
    fn picks_30fps_mjpg_720p_over_slow_raw() {
        let m = choose_mode(&parse_formats(FORMATS)).unwrap();
        assert_eq!((m.format.as_str(), m.width, m.height), ("MJPG", 1280, 720));
    }

    #[test]
    fn falls_back_to_the_fastest_when_nothing_reaches_30() {
        let modes = vec![
            Mode {
                format: "YUYV".into(),
                width: 1280,
                height: 720,
                fps: 10.0,
            },
            Mode {
                format: "YUYV".into(),
                width: 640,
                height: 480,
                fps: 15.0,
            },
        ];
        assert_eq!(choose_mode(&modes).unwrap().width, 640);
    }

    #[test]
    fn ir_greyscale_node_is_skipped() {
        assert!(choose_mode(&parse_formats(FORMATS_IR)).is_none());
    }

    #[test]
    fn capture_node_detection_reads_device_caps_only() {
        assert!(is_capture_node(INFO_CAPTURE));
        // video1's *Capabilities* mention Video Capture (the whole device),
        // its Device Caps don't: a metadata node.
        assert!(!is_capture_node(INFO_META));
    }

    #[test]
    fn lists_video_nodes_only() {
        assert_eq!(
            parse_list_devices(LIST),
            vec!["/dev/video0", "/dev/video1", "/dev/video2", "/dev/video3"]
        );
    }

    #[test]
    fn ffmpeg_argv_shape() {
        let m = choose_mode(&parse_formats(FORMATS)).unwrap();
        let a = ffmpeg_argv("/dev/video0", Some(&m), 256, 288);
        let s = a.join(" ");
        assert!(s.contains("-f v4l2 -input_format mjpeg -framerate 30 -video_size 1280x720"));
        assert!(s.contains("-i /dev/video0"));
        assert!(s.contains("crop=ih*8/9:ih,scale=256:288,format=rgba"));
        assert!(s.ends_with("-f rawvideo -pix_fmt rgba -"));
        let bare = ffmpeg_argv("/dev/video0", None, 96, 108).join(" ");
        assert!(!bare.contains("-video_size"));
    }

    #[test]
    fn bubble_sits_inside_the_targets_bottom_right() {
        let full = Target {
            output: "DP-3".into(),
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let b = bubble_geometry(&full, 2560, 1440, WebcamSize::Medium);
        assert_eq!(
            (b.width, b.height),
            (BUBBLE_MAX_W, 540),
            "clamped to the max"
        );
        assert_eq!((b.right, b.bottom), (BUBBLE_MARGIN, BUBBLE_MARGIN));
        // A region in the middle: margins measured from the region's edge.
        let region = Target {
            output: "DP-3".into(),
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        };
        let b = bubble_geometry(&region, 2560, 1440, WebcamSize::Small);
        assert_eq!((b.width, b.height), (112, 126));
        assert_eq!(b.right, 2560 - 900 + BUBBLE_MARGIN);
        assert_eq!(b.bottom, 1440 - 700 + BUBBLE_MARGIN);
        // Tiny region: never below the minimum.
        let tiny = Target {
            width: 100,
            height: 100,
            ..region
        };
        assert_eq!(
            bubble_geometry(&tiny, 2560, 1440, WebcamSize::Large).width,
            BUBBLE_MIN_W
        );
    }

    #[test]
    fn size_tokens_round_trip() {
        for s in WebcamSize::ALL {
            assert_eq!(WebcamSize::parse(s.token()), Some(s));
        }
        assert_eq!(WebcamSize::parse("on"), Some(WebcamSize::Medium));
        assert_eq!(WebcamSize::parse("huge"), None);
    }
}
