use crate::utils::default_recording_path;
use anyhow::Result;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone)]
pub struct RecordResult {
    pub saved_path: Option<PathBuf>,
}

/// Returned to the caller immediately after recording starts.
/// Call `.stop()` to terminate wf-recorder and trigger `on_done`.
#[derive(Debug, Clone)]
pub struct RecordHandle {
    stopped: Arc<AtomicBool>,
}

impl RecordHandle {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

pub(crate) fn start_recording(
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
        // ensure dir exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut cmd = Command::new("wf-recorder");
        cmd.args(wf_recorder_argv(&args, &path, audio.as_deref()));

        let mut child: Child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return on_done(Err(e.into())),
        };

        // Poll the stop flag, then kill the child
        loop {
            if stopped.load(Ordering::SeqCst) {
                let pid = Pid::from_raw(child.id() as i32);
                let _ = kill(pid, Signal::SIGINT);
                let _ = child.wait(); // wait for it to finish cleanly
                break;
            }
            if let Ok(Some(_)) = child.try_wait() {
                // process exited on its own
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        on_done(Ok(RecordResult {
            saved_path: Some(path),
        }));
    });

    handle
}

/// wf-recorder's argv (without the program name) for a recording of
/// `args` into `path`.
fn wf_recorder_argv(
    args: &WfRecorderArgs,
    path: &std::path::Path,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::Path;

    fn argv(args: WfRecorderArgs, audio: Option<&str>) -> Vec<String> {
        wf_recorder_argv(&args, Path::new("/v/rec.mp4"), audio)
            .into_iter()
            .map(|a: OsString| a.into_string().unwrap())
            .collect()
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
}
