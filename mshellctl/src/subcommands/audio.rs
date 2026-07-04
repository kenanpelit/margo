use crate::bus::{bus_command, bus_command_with_arg, bus_command_with_reply};
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
            let method = if json {
                "AudioListJson"
            } else {
                "AudioListText"
            };
            let out: String = bus_command_with_reply(method).await?;
            println!("{out}");
        }
        AudioCommands::Status { json } => {
            let method = if json {
                "AudioStatusJson"
            } else {
                "AudioStatusText"
            };
            let out: String = bus_command_with_reply(method).await?;
            println!("{out}");
        }
        AudioCommands::VolumeUp => {
            bus_command("VolumeUp").await?;
        }
        AudioCommands::VolumeDown => {
            bus_command("VolumeDown").await?;
        }
        AudioCommands::Volume { percent } => {
            bus_command_with_arg("AudioVolumeSet", &percent).await?;
        }
        AudioCommands::Mute { state } => {
            bus_command_with_arg("AudioMuteSet", &mute_mode(state)).await?;
        }
        AudioCommands::Output { target } => {
            bus_command_with_arg("AudioOutputSwitch", &target).await?;
        }
        AudioCommands::Input { target } => {
            bus_command_with_arg("AudioInputSwitch", &target).await?;
        }
        AudioCommands::Switch => {
            bus_command_with_arg("AudioOutputSwitch", &"next".to_string()).await?;
        }
        AudioCommands::SwitchMic => {
            bus_command_with_arg("AudioInputSwitch", &"next".to_string()).await?;
        }
        AudioCommands::RouteNext => {
            bus_command("AudioRouteCycle").await?;
        }
        AudioCommands::MicUp => {
            bus_command("AudioMicUp").await?;
        }
        AudioCommands::MicDown => {
            bus_command("AudioMicDown").await?;
        }
        AudioCommands::Mic { percent } => {
            bus_command_with_arg("AudioMicVolumeSet", &percent).await?;
        }
        AudioCommands::MicMute { state } => {
            bus_command_with_arg("AudioMicMuteSet", &mute_mode(state)).await?;
        }
    }
    Ok(())
}
