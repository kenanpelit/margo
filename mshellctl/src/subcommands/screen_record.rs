//! `mshellctl screen-record` — start / stop / toggle screen recording.
//!
//! Drives the shell's own recording engine (the same one behind the
//! screenshot menu's "Screen recording" section + the recording-indicator
//! pill) headlessly over IPC. One engine, one tool.
//!
//! ```text
//! mshellctl screen-record start|toggle [region|window|output|full] [--audio SRC]
//! mshellctl screen-record stop
//! ```

use crate::bus::bus_command_with_arg;
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
    /// Mix in an audio source (PipeWire source name). Omit for silent.
    #[arg(long)]
    audio: Option<String>,
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
        "{action} {} {}",
        args.area.as_str(),
        args.audio.as_deref().unwrap_or("-")
    );
    bus_command_with_arg("ScreenRecord", &spec).await?;
    Ok(())
}

pub async fn execute(command: ScreenRecordCommands) -> anyhow::Result<()> {
    match command {
        ScreenRecordCommands::Start(a) => send("start", &a).await?,
        ScreenRecordCommands::Toggle(a) => send("toggle", &a).await?,
        ScreenRecordCommands::Stop => {
            bus_command_with_arg("ScreenRecord", &"stop".to_string()).await?;
        }
    }
    Ok(())
}
