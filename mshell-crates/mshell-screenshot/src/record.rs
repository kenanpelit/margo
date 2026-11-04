//! Screen recording: one recorder process per clip, two backends.
//!
//! * **wf-recorder** (default, today's behaviour) — CPU x264 via ffmpeg.
//!   It takes a single PulseAudio/PipeWire source, so "desktop +
//!   microphone" is mixed *before* it: a private null sink is created with
//!   `pactl`, each source is looped into it, and wf-recorder records the
//!   sink's monitor. The modules are unloaded when the clip ends (and any
//!   left over by a crash are swept at the next start).
//! * **gpu-screen-recorder** — hardware encoding (VAAPI on the Intel iGPU,
//!   NVENC on NVIDIA). It mixes `|`-joined audio sources itself.
//!
//! The audio spec both take is the string the menu / `mshellctl
//! screenrecord --audio` passes: a source name, `default_output` (desktop
//! audio), `default_input` (microphone), `both`, or several of those joined
//! with `|` (gpu-screen-recorder's own syntax).
//!
//! Every argv / plan below is a pure function so it is unit-tested; what is
//! left untested is the process lifecycle itself (spawn, SIGINT, wait).

use crate::utils::default_recording_path;
use anyhow::{Result, anyhow};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone)]
pub struct RecordResult {
    pub saved_path: Option<PathBuf>,
}

/// Returned to the caller immediately after recording starts.
/// Call `.stop()` to terminate the recorder and trigger `on_done`.
#[derive(Debug, Clone)]
pub struct RecordHandle {
    stopped: Arc<AtomicBool>,
}

impl RecordHandle {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

/// Which recorder binary encodes the clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecordBackend {
    /// wf-recorder + libx264 (CPU). Today's behaviour, the default.
    #[default]
    WfRecorder,
    /// gpu-screen-recorder (VAAPI / NVENC hardware encoding).
    GpuScreenRecorder,
}

impl RecordBackend {
    pub const ALL: [RecordBackend; 2] =
        [RecordBackend::WfRecorder, RecordBackend::GpuScreenRecorder];

    pub fn binary(self) -> &'static str {
        match self {
            RecordBackend::WfRecorder => "wf-recorder",
            RecordBackend::GpuScreenRecorder => "gpu-screen-recorder",
        }
    }

    /// Short user-facing name.
    pub fn label(self) -> &'static str {
        match self {
            RecordBackend::WfRecorder => "wf-recorder (CPU)",
            RecordBackend::GpuScreenRecorder => "gpu-screen-recorder (GPU)",
        }
    }

    /// Token used on the IPC spec / `mshellctl screenrecord --backend`.
    pub fn token(self) -> &'static str {
        match self {
            RecordBackend::WfRecorder => "wf",
            RecordBackend::GpuScreenRecorder => "gsr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "wf" | "wf-recorder" => Some(RecordBackend::WfRecorder),
            "gsr" | "gpu" | "gpu-screen-recorder" => Some(RecordBackend::GpuScreenRecorder),
            _ => None,
        }
    }
}

/// Desktop audio (the default sink's monitor).
pub const AUDIO_DESKTOP: &str = "default_output";
/// Microphone (the default source).
pub const AUDIO_MIC: &str = "default_input";
/// Desktop + microphone mixed into one track.
pub const AUDIO_DESKTOP_AND_MIC: &str = "default_output|default_input";

/// Audio sources named by an audio spec, deduplicated, in order. `None`,
/// empty and `-` mean silent; `both` = desktop + microphone.
pub fn parse_audio(spec: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in spec.unwrap_or("").split('|').map(str::trim) {
        let expanded: &[&str] = match tok {
            "" | "-" => &[],
            "both" => &[AUDIO_DESKTOP, AUDIO_MIC],
            "desktop" => &[AUDIO_DESKTOP],
            "mic" => &[AUDIO_MIC],
            other => {
                if !out.iter().any(|o| o == other) {
                    out.push(other.to_string());
                }
                continue;
            }
        };
        for t in expanded {
            if !out.iter().any(|o| o == t) {
                out.push((*t).to_string());
            }
        }
    }
    out
}

/// A concrete PulseAudio source name for a spec token, given the server's
/// default sink and source (`pactl get-default-sink` / `-source`). Device
/// names pass through.
pub fn resolve_source(token: &str, default_sink: &str, default_source: &str) -> String {
    match token {
        AUDIO_DESKTOP => format!("{default_sink}.monitor"),
        AUDIO_MIC => default_source.to_string(),
        other => other.to_string(),
    }
}

pub(crate) enum WfRecorderArgs {
    Region {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    Monitor {
        name: String,
    },
    Window {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    All,
}

/// wf-recorder's argv (without the program name) for a recording of
/// `args` into `path`. `audio` is one concrete source (already resolved /
/// mixed).
fn wf_recorder_argv(
    args: &WfRecorderArgs,
    path: &Path,
    audio: Option<&str>,
) -> Vec<std::ffi::OsString> {
    let mut argv: Vec<std::ffi::OsString> = Vec::new();
    match args {
        WfRecorderArgs::Region {
            x,
            y,
            width,
            height,
        }
        | WfRecorderArgs::Window {
            x,
            y,
            width,
            height,
        } => {
            argv.push("--geometry".into());
            argv.push(format!("{x},{y} {width}x{height}").into());
        }
        WfRecorderArgs::Monitor { name } => {
            argv.push("-o".into());
            argv.push(name.into());
        }
        WfRecorderArgs::All => {}
    }
    argv.push("-f".into());
    argv.push(path.into());
    for a in [
        "-c",
        "libx264",
        "-p",
        "crf=23",
        "-p",
        "preset=fast",
        "-x",
        "yuv420p",
    ] {
        argv.push(a.into());
    }
    if let Some(audio) = audio {
        argv.push(format!("--audio={audio}").into());
    }
    argv
}

/// gpu-screen-recorder's argv (without the program name). `audio` are
/// spec tokens / device names; gsr resolves `default_output` /
/// `default_input` itself and mixes `|`-joined sources into one track.
///
/// * Region / window → `-w region -region WxH+X+Y` (logical layout
///   coordinates, the same space as slurp / wf-recorder `--geometry`).
/// * `All` is only reached with a single output (see `record_screen`), so
///   `-w screen` (gsr: the first monitor) is that output.
/// * `-k auto` + `-encoder gpu` picks VAAPI H.264/HEVC on Intel/AMD;
///   `-fallback-cpu-encoding yes` keeps a machine without a usable
///   hardware encoder recording instead of failing.
/// * Opus, not AAC: gsr 6.1's AAC path emits non-monotonic dts into the
///   mp4 muxer.
fn gsr_argv(args: &WfRecorderArgs, path: &Path, audio: &[String]) -> Vec<std::ffi::OsString> {
    let mut argv: Vec<std::ffi::OsString> = Vec::new();
    match args {
        WfRecorderArgs::Region {
            x,
            y,
            width,
            height,
        }
        | WfRecorderArgs::Window {
            x,
            y,
            width,
            height,
        } => {
            argv.extend(["-w".into(), "region".into(), "-region".into()]);
            argv.push(format!("{width}x{height}+{x}+{y}").into());
        }
        WfRecorderArgs::Monitor { name } => {
            argv.push("-w".into());
            argv.push(name.into());
        }
        WfRecorderArgs::All => {
            argv.extend(["-w".into(), "screen".into()]);
        }
    }
    for a in [
        "-c",
        "mp4",
        "-k",
        "auto",
        "-encoder",
        "gpu",
        "-fallback-cpu-encoding",
        "yes",
        "-q",
        "very_high",
        "-f",
        "60",
        "-cursor",
        "yes",
    ] {
        argv.push(a.into());
    }
    if !audio.is_empty() {
        argv.push("-a".into());
        argv.push(audio.join("|").into());
        argv.push("-ac".into());
        argv.push("opus".into());
    }
    argv.push("-o".into());
    argv.push(path.into());
    argv
}

/// Prefix of the private null sink wf-recorder records a mix from.
const MIX_SINK_PREFIX: &str = "margo_rec_mix";

/// `pactl` argv lists that build the mix for wf-recorder: one null sink
/// named `sink`, then one loopback per (resolved) source into it. Record
/// from `{sink}.monitor`.
fn mix_plan(sources: &[String], sink: &str) -> Vec<Vec<String>> {
    let mut plan = vec![vec![
        "load-module".to_string(),
        "module-null-sink".to_string(),
        format!("sink_name={sink}"),
        "sink_properties=device.description=margo-recording-mix".to_string(),
    ]];
    for src in sources {
        plan.push(vec![
            "load-module".to_string(),
            "module-loopback".to_string(),
            format!("source={src}"),
            format!("sink={sink}"),
            "latency_msec=20".to_string(),
            "source_dont_move=true".to_string(),
            "sink_dont_move=true".to_string(),
        ]);
    }
    plan
}

/// Module index `pactl load-module` prints on success.
fn parse_module_id(stdout: &str) -> Option<u32> {
    stdout.trim().parse().ok()
}

/// Indices of mix modules (null sink + loopbacks) left behind by a
/// recording that never cleaned up (mshell killed mid-clip), from
/// `pactl list short modules` (`ID\tNAME\tARGS…`).
fn stale_mix_modules(list: &str) -> Vec<u32> {
    list.lines()
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let id = cols.next()?.trim().parse::<u32>().ok()?;
            let name = cols.next()?;
            let args = cols.next().unwrap_or("");
            let ours = matches!(name, "module-null-sink" | "module-loopback")
                && (args.contains(&format!("sink_name={MIX_SINK_PREFIX}"))
                    || args.contains(&format!("sink={MIX_SINK_PREFIX}")));
            ours.then_some(id)
        })
        .collect()
}

/// Last non-empty line of a recorder's stderr, for the failure message.
fn last_line(stderr: &str) -> &str {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
}

fn pactl(args: &[String]) -> Result<String> {
    let out = Command::new("pactl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| anyhow!("spawn pactl: {e}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "pactl {} failed: {}",
            args.first().map(String::as_str).unwrap_or(""),
            last_line(&String::from_utf8_lossy(&out.stderr))
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Loaded mix modules; unloads them (newest first) on drop so every exit
/// path of the recording thread tears the mix down.
struct AudioMix {
    modules: Vec<u32>,
}

impl Drop for AudioMix {
    fn drop(&mut self) {
        for id in self.modules.iter().rev() {
            let _ = pactl(&["unload-module".to_string(), id.to_string()]);
        }
    }
}

/// Resolve `sources` and, for more than one, build the null-sink mix.
/// Returns the single source wf-recorder should record plus the guard
/// owning the mix modules.
fn prepare_wf_audio(sources: &[String]) -> Result<(Option<String>, Option<AudioMix>)> {
    if sources.is_empty() {
        return Ok((None, None));
    }
    let needs_defaults = sources.iter().any(|s| s == AUDIO_DESKTOP || s == AUDIO_MIC);
    let (sink, source) = if needs_defaults {
        (
            pactl(&["get-default-sink".to_string()])?.trim().to_string(),
            pactl(&["get-default-source".to_string()])?
                .trim()
                .to_string(),
        )
    } else {
        (String::new(), String::new())
    };
    let resolved: Vec<String> = sources
        .iter()
        .map(|s| resolve_source(s, &sink, &source))
        .collect();
    if let [one] = resolved.as_slice() {
        return Ok((Some(one.clone()), None));
    }
    // Sweep modules a crashed run left behind before adding ours.
    if let Ok(list) = pactl(&[
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ]) {
        for id in stale_mix_modules(&list) {
            let _ = pactl(&["unload-module".to_string(), id.to_string()]);
        }
    }
    let sink_name = format!("{MIX_SINK_PREFIX}_{}", std::process::id());
    let mut mix = AudioMix {
        modules: Vec::new(),
    };
    for step in mix_plan(&resolved, &sink_name) {
        let out = pactl(&step)?; // `mix` drops → partial mix unloaded
        if let Some(id) = parse_module_id(&out) {
            mix.modules.push(id);
        }
    }
    Ok((Some(format!("{sink_name}.monitor")), Some(mix)))
}

pub(crate) fn start_recording(
    backend: RecordBackend,
    audio: Option<String>,
    args: WfRecorderArgs,
    on_done: impl FnOnce(Result<RecordResult>) + Send + 'static,
) -> RecordHandle {
    let stopped = Arc::new(AtomicBool::new(false));
    let handle = RecordHandle {
        stopped: stopped.clone(),
    };

    std::thread::spawn(move || {
        let path = default_recording_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let sources = parse_audio(audio.as_deref());

        // `_mix` lives until this thread returns: the loopbacks must
        // outlast the recorder.
        let (argv, _mix) = match backend {
            RecordBackend::WfRecorder => match prepare_wf_audio(&sources) {
                Ok((src, mix)) => (wf_recorder_argv(&args, &path, src.as_deref()), mix),
                Err(e) => return on_done(Err(e.context("set up recording audio"))),
            },
            RecordBackend::GpuScreenRecorder => (gsr_argv(&args, &path, &sources), None),
        };

        let mut child: Child = match Command::new(backend.binary())
            .args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return on_done(Err(anyhow!("spawn {}: {e}", backend.binary()))),
        };

        // Drain stderr on its own thread (a chatty recorder would block on
        // a full pipe otherwise); keep only the tail for error reports.
        let tail = Arc::new(Mutex::new(String::new()));
        let drain = child.stderr.take().map(|mut err| {
            let tail = tail.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = err.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                    t.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if t.len() > 8192 {
                        let cut = t.len() - 4096;
                        let cut = (cut..t.len()).find(|i| t.is_char_boundary(*i)).unwrap_or(0);
                        t.drain(..cut);
                    }
                }
            })
        });

        // Poll the stop flag, then SIGINT the child (both recorders
        // finalise the file on SIGINT).
        let mut stopped_by_user = false;
        let status = loop {
            if stopped.load(Ordering::SeqCst) {
                stopped_by_user = true;
                let pid = Pid::from_raw(child.id() as i32);
                let _ = kill(pid, Signal::SIGINT);
                break child.wait().ok();
            }
            if let Ok(Some(status)) = child.try_wait() {
                break Some(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        if let Some(d) = drain {
            let _ = d.join();
        }
        let stderr = tail.lock().unwrap_or_else(|p| p.into_inner()).clone();

        let written = std::fs::metadata(&path).is_ok_and(|m| m.len() > 0);
        let failed_early = !stopped_by_user && !status.is_some_and(|s| s.success());
        if failed_early || !written {
            let status = status.map_or_else(|| "?".to_string(), |s| s.to_string());
            return on_done(Err(anyhow!(
                "{} exited {status}: {}",
                backend.binary(),
                last_line(&stderr)
            )));
        }
        on_done(Ok(RecordResult {
            saved_path: Some(path),
        }));
    });

    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn strings(v: Vec<OsString>) -> Vec<String> {
        v.into_iter().map(|a| a.into_string().unwrap()).collect()
    }

    fn argv(args: WfRecorderArgs, audio: Option<&str>) -> Vec<String> {
        strings(wf_recorder_argv(&args, Path::new("/v/rec.mp4"), audio))
    }

    fn gsr(args: WfRecorderArgs, audio: &[&str]) -> Vec<String> {
        let audio: Vec<String> = audio.iter().map(|s| s.to_string()).collect();
        strings(gsr_argv(&args, Path::new("/v/rec.mp4"), &audio))
    }

    const TAIL: [&str; 10] = [
        "-f",
        "/v/rec.mp4",
        "-c",
        "libx264",
        "-p",
        "crf=23",
        "-p",
        "preset=fast",
        "-x",
        "yuv420p",
    ];

    #[test]
    fn region_and_window_use_slurp_style_geometry() {
        let region = WfRecorderArgs::Region {
            x: 10,
            y: -20,
            width: 300,
            height: 200,
        };
        let a = argv(region, None);
        assert_eq!(&a[..2], ["--geometry", "10,-20 300x200"]);
        assert_eq!(&a[2..], TAIL);

        let window = WfRecorderArgs::Window {
            x: 1920,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(&argv(window, None)[..2], ["--geometry", "1920,0 800x600"]);
    }

    #[test]
    fn monitor_and_all() {
        let a = argv(
            WfRecorderArgs::Monitor {
                name: "DP-2".into(),
            },
            None,
        );
        assert_eq!(&a[..2], ["-o", "DP-2"]);
        assert_eq!(argv(WfRecorderArgs::All, None), TAIL);
    }

    #[test]
    fn audio_device_goes_last_as_one_argument() {
        let a = argv(WfRecorderArgs::All, Some("alsa_output.pci monitor"));
        assert_eq!(a.last().unwrap(), "--audio=alsa_output.pci monitor");
        assert_eq!(a.len(), TAIL.len() + 1);
    }

    #[test]
    fn audio_spec_parsing() {
        assert!(parse_audio(None).is_empty());
        assert!(parse_audio(Some("")).is_empty());
        assert!(parse_audio(Some("-")).is_empty());
        assert_eq!(
            parse_audio(Some("alsa_input.usb-mic")),
            ["alsa_input.usb-mic"]
        );
        assert_eq!(
            parse_audio(Some(AUDIO_DESKTOP_AND_MIC)),
            [AUDIO_DESKTOP, AUDIO_MIC]
        );
        assert_eq!(parse_audio(Some("both")), [AUDIO_DESKTOP, AUDIO_MIC]);
        assert_eq!(
            parse_audio(Some("desktop| mic |default_input")),
            [AUDIO_DESKTOP, AUDIO_MIC]
        );
        assert_eq!(
            parse_audio(Some("a.monitor|alsa_input.x")),
            ["a.monitor", "alsa_input.x"]
        );
    }

    #[test]
    fn default_tokens_resolve_to_pulse_names() {
        let sink = "alsa_output.pci-0000_00_1f.3.analog-stereo";
        let src = "alsa_input.pci-0000_00_1f.3.analog-stereo";
        assert_eq!(
            resolve_source(AUDIO_DESKTOP, sink, src),
            format!("{sink}.monitor")
        );
        assert_eq!(resolve_source(AUDIO_MIC, sink, src), src);
        assert_eq!(resolve_source("x.monitor", sink, src), "x.monitor");
    }

    #[test]
    fn mix_plan_is_one_null_sink_then_a_loopback_per_source() {
        let plan = mix_plan(&["d.monitor".into(), "mic".into()], "margo_rec_mix_7");
        assert_eq!(plan.len(), 3);
        assert_eq!(
            plan[0],
            [
                "load-module",
                "module-null-sink",
                "sink_name=margo_rec_mix_7",
                "sink_properties=device.description=margo-recording-mix"
            ]
        );
        assert_eq!(
            &plan[1][..4],
            [
                "load-module",
                "module-loopback",
                "source=d.monitor",
                "sink=margo_rec_mix_7"
            ]
        );
        assert_eq!(plan[2][2], "source=mic");
        assert_eq!(parse_module_id("536870913\n"), Some(536_870_913));
        assert_eq!(parse_module_id("Failure: x"), None);
    }

    #[test]
    fn stale_sweep_only_touches_our_modules() {
        // `pactl list short modules` shape (pipewire-pulse).
        // Captured shape: pipewire's own modules print multi-line args.
        let list = "1\tlibpipewire-module-rt\t{\n            nice.level    = -11\n            rt.prio       = 88\n\
                    536870912\tmodule-always-sink\t\n\
                    536870913\tmodule-null-sink\tsink_name=margo_rec_mix_42 sink_properties=device.description=margo-recording-mix\n\
                    536870914\tmodule-loopback\tsource=x.monitor sink=margo_rec_mix_42 latency_msec=20\n\
                    536870915\tmodule-loopback\tsource=y sink=speakers\n\
                    536870916\tmodule-null-sink\tsink_name=other\n";
        assert_eq!(stale_mix_modules(list), [536_870_913, 536_870_914]);
        assert!(stale_mix_modules("").is_empty());
    }

    #[test]
    fn gsr_region_monitor_and_all() {
        let a = gsr(
            WfRecorderArgs::Region {
                x: 330,
                y: 1460,
                width: 300,
                height: 200,
            },
            &[],
        );
        assert_eq!(&a[..4], ["-w", "region", "-region", "300x200+330+1460"]);
        assert_eq!(&a[a.len() - 2..], ["-o", "/v/rec.mp4"]);
        assert!(!a.contains(&"-a".to_string()), "silent: no -a");
        for pair in [["-k", "auto"], ["-encoder", "gpu"], ["-c", "mp4"]] {
            assert!(a.windows(2).any(|w| w == pair), "{pair:?} in {a:?}");
        }

        let a = gsr(
            WfRecorderArgs::Monitor {
                name: "DP-3".into(),
            },
            &[],
        );
        assert_eq!(&a[..2], ["-w", "DP-3"]);
        let a = gsr(WfRecorderArgs::All, &[]);
        assert_eq!(&a[..2], ["-w", "screen"]);
    }

    #[test]
    fn gsr_mixes_audio_itself() {
        let a = gsr(WfRecorderArgs::All, &[AUDIO_DESKTOP, AUDIO_MIC]);
        let i = a.iter().position(|x| x == "-a").unwrap();
        assert_eq!(a[i + 1], "default_output|default_input");
        assert_eq!(&a[i + 2..i + 4], ["-ac", "opus"]);
    }

    #[test]
    fn backend_tokens_round_trip() {
        for b in RecordBackend::ALL {
            assert_eq!(RecordBackend::parse(b.token()), Some(b));
            assert_eq!(RecordBackend::parse(b.binary()), Some(b));
        }
        assert_eq!(RecordBackend::parse("ffmpeg"), None);
        assert_eq!(RecordBackend::default(), RecordBackend::WfRecorder);
    }

    #[test]
    fn last_stderr_line() {
        assert_eq!(
            last_line("a\nfailed to open device\n\n"),
            "failed to open device"
        );
        assert_eq!(last_line(""), "");
    }
}
