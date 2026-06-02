use crate::bus::bus_command_with_arg;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum BluetoothCommands {
    /// Smart toggle: power on + connect, or disconnect if already connected
    /// (replaces the old F10 bluetooth_toggle script)
    Toggle,
    /// Connect the configured device(s), trying each in order
    Connect,
    /// Disconnect any connected configured device
    Disconnect,
}

pub async fn execute(command: BluetoothCommands) -> anyhow::Result<()> {
    let action = match command {
        BluetoothCommands::Toggle => "toggle",
        BluetoothCommands::Connect => "connect",
        BluetoothCommands::Disconnect => "disconnect",
    };
    bus_command_with_arg("BluetoothCtl", &action.to_string()).await?;
    Ok(())
}
