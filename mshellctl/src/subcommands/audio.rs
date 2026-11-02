use crate::bus::shell;
use clap::{Subcommand, ValueEnum};

/// Force a mute state; absence of the argument means "toggle".
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum MuteArg {
    On,
    Off,
}

#[derive(Subcommand, Debug)]
pub enum AudioCommands {
    /// List output + input devices (friendly names, volume, active marker)
    List {
        /// Emit JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Show the current default output / input device
    Status {
        /// Emit JSON instead of a line
        #[arg(long)]
        json: bool,
    },
    /// Increase the output volume by 5 percent
    VolumeUp,
    /// Decrease the output volume by 5 percent
    VolumeDown,
    /// Set the output volume to an absolute PERCENT (0–150)
    Volume { percent: f64 },
    /// Toggle output mute, or force `on` / `off`
    Mute { state: Option<MuteArg> },
    /// Switch the default output — next | prev | INDEX | NAME fragment
    Output { target: String },
    /// Switch the default input — next | prev | INDEX | NAME fragment
    Input { target: String },
    /// Cycle the default output to the next device (alias for `output next`)
    Switch,
    /// Cycle the default input to the next device (alias for `input next`)
    SwitchMic,
    /// Cycle the default output to the next routable sink: always skips HDMI/DP
    /// and, when `audio.route_switch_microphone` is on, moves the mic across the
    /// headset boundary too (unlike the plain `switch`)
    RouteNext,
    /// Increase the microphone volume by 5 percent
    MicUp,
    /// Decrease the microphone volume by 5 percent
    MicDown,
    /// Set the microphone volume to an absolute PERCENT (0–150)
    Mic { percent: f64 },
    /// Toggle microphone mute, or force `on` / `off`
    MicMute { state: Option<MuteArg> },
}

/// 0 = unmute, 1 = mute, 2 = toggle — the wire encoding the daemon expects.
fn mute_mode(state: Option<MuteArg>) -> i32 {
    match state {
        Some(MuteArg::Off) => 0,
        Some(MuteArg::On) => 1,
        None => 2,
    }
}

pub async fn execute(command: AudioCommands) -> anyhow::Result<()> {
    match command {
        AudioCommands::List { json } => {
            let shell = shell().await?;
            let out = if json {
                shell.audio_list_json().await?
            } else {
                shell.audio_list_text().await?
            };
            println!("{out}");
        }
        AudioCommands::Status { json } => {
            let shell = shell().await?;
            let out = if json {
                shell.audio_status_json().await?
            } else {
                shell.audio_status_text().await?
            };
            println!("{out}");
        }
        AudioCommands::VolumeUp => {
            shell().await?.volume_up().await?;
        }
        AudioCommands::VolumeDown => {
            shell().await?.volume_down().await?;
        }
        AudioCommands::Volume { percent } => {
            shell().await?.audio_volume_set(percent).await?;
        }
        AudioCommands::Mute { state } => {
            shell().await?.audio_mute_set(mute_mode(state)).await?;
        }
        AudioCommands::Output { target } => {
            shell().await?.audio_output_switch(&target).await?;
        }
        AudioCommands::Input { target } => {
            shell().await?.audio_input_switch(&target).await?;
        }
        AudioCommands::Switch => {
            shell().await?.audio_output_switch("next").await?;
        }
        AudioCommands::SwitchMic => {
            shell().await?.audio_input_switch("next").await?;
        }
        AudioCommands::RouteNext => {
            shell().await?.audio_route_cycle().await?;
        }
        AudioCommands::MicUp => {
            shell().await?.audio_mic_up().await?;
        }
        AudioCommands::MicDown => {
            shell().await?.audio_mic_down().await?;
        }
        AudioCommands::Mic { percent } => {
            shell().await?.audio_mic_volume_set(percent).await?;
        }
        AudioCommands::MicMute { state } => {
            shell().await?.audio_mic_mute_set(mute_mode(state)).await?;
        }
    }
    Ok(())
}
