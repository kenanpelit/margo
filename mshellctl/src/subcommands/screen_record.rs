//! `mshellctl screenrecord` — start / stop / toggle screen recording.
//!
//! Drives the shell's own recording engine (the same one behind the
//! screenshot menu's "Screen recording" section + the recording-indicator
//! pill) headlessly over IPC. One engine, one tool.
//!
//! ```text
//! mshellctl screenrecord start|toggle [region|window|output|full] [--audio SRC] [--backend wf|gsr]
//!                                     [--webcam off|small|medium|large]
//! mshellctl screenrecord stop
//! ```

use crate::bus::shell;
use clap::{Args, Subcommand, ValueEnum};

#[derive(Subcommand, Debug)]
pub enum ScreenRecordCommands {
    /// Start recording (no-op if one is already running).
    Start(RecordArgs),
    /// Stop the active recording.
    Stop,
    /// Toggle: start if idle, stop if recording.
    Toggle(RecordArgs),
}

#[derive(Args, Debug)]
pub struct RecordArgs {
    /// What to record. Region prompts the in-shell selector; output prompts
    /// a monitor pick; full grabs the whole layout.
    #[arg(value_enum, default_value_t = Area::Full)]
    area: Area,
    /// Audio: a PipeWire source name, `desktop`, `mic`, `both` (desktop +
    /// microphone mixed), or several sources joined with `|`. Omit for
    /// silent.
    #[arg(long)]
    audio: Option<String>,
    /// Recorder: `wf` (wf-recorder, CPU x264) or `gsr`
    /// (gpu-screen-recorder, VAAPI/NVENC). Omit to use the menu's choice.
    #[arg(long, value_enum)]
    backend: Option<Backend>,
    /// Webcam picture-in-picture: a camera bubble in the recorded area's
    /// bottom-right corner, baked into the clip (needs ffmpeg; v4l2-ctl
    /// picks the camera mode). `--webcam` alone = medium. Omit to use the
    /// menu's choice.
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "medium")]
    webcam: Option<Webcam>,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Webcam {
    Off,
    Small,
    Medium,
    Large,
}

impl Webcam {
    fn token(self) -> &'static str {
        match self {
            Webcam::Off => "off",
            Webcam::Small => "small",
            Webcam::Medium => "medium",
            Webcam::Large => "large",
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Backend {
    Wf,
    Gsr,
}

#[derive(ValueEnum, Clone, Copy, Debug, Default)]
enum Area {
    Region,
    Window,
    Output,
    #[default]
    Full,
}

impl Area {
    fn as_str(self) -> &'static str {
        match self {
            Area::Region => "region",
            Area::Window => "window",
            Area::Output => "output",
            Area::Full => "full",
        }
    }
}

async fn send(action: &str, args: &RecordArgs) -> anyhow::Result<()> {
    let spec = format!(
        "{action} {} {} {} {}",
        args.area.as_str(),
        args.audio
            .as_deref()
            .filter(|a| !a.is_empty())
            .unwrap_or("-"),
        match args.backend {
            Some(Backend::Wf) => "wf",
            Some(Backend::Gsr) => "gsr",
            None => "-",
        },
        args.webcam.map(Webcam::token).unwrap_or("-"),
    );
    shell().await?.screen_record(&spec).await?;
    Ok(())
}

pub async fn execute(command: ScreenRecordCommands) -> anyhow::Result<()> {
    match command {
        ScreenRecordCommands::Start(a) => send("start", &a).await?,
        ScreenRecordCommands::Toggle(a) => send("toggle", &a).await?,
        ScreenRecordCommands::Stop => {
            shell().await?.screen_record("stop").await?;
        }
    }
    Ok(())
}
