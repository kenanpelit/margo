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
    /// Connect the device assigned this quick-connect number (set from the
    /// Bluetooth menu) — for a stable keybind that doesn't care which slot
    /// in the config's device list this happens to be
    ConnectNumber { number: u8 },
}

pub async fn execute(command: BluetoothCommands) -> anyhow::Result<()> {
    let action = match command {
        BluetoothCommands::Toggle => "toggle".to_string(),
        BluetoothCommands::Connect => "connect".to_string(),
        BluetoothCommands::Disconnect => "disconnect".to_string(),
        BluetoothCommands::ConnectNumber { number } => format!("connect:{number}"),
    };
    bus_command_with_arg("BluetoothCtl", &action).await?;
    Ok(())
}
