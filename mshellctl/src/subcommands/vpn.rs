//! `mshellctl vpn …` — Mullvad VPN control, proxied to the standalone `mvpn`
//! binary (which owns the CLI + the GTK layer-shell panel).
//!
//! This is the *control* surface (connect / disconnect / pick a relay).
//! To merely open the shell's DNS/VPN menu, use `vpn menu` (or
//! `mshellctl menu vpn`).

use crate::bus::bus_command;
use crate::subcommands::proxy;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum VpnCommands {
    /// Connection status (relay, IP, protocol).
    Status,
    /// Connect using the current relay selection.
    Connect,
    /// Disconnect.
    Disconnect,
    /// Toggle the connection on/off.
    Toggle,
    /// Reconnect (disconnect + connect).
    Reconnect,
    /// Connect to a random relay.
    Random,
    /// Connect to the fastest relay (latency-ranked).
    Fastest,
    /// Open the shell's DNS/VPN menu (D-Bus, not `mvpn`).
    Menu,
    /// Any other `mvpn` subcommand passes straight through — e.g.
    /// `vpn countries`, `vpn owned`, `vpn obf`, `vpn lockdown`, `vpn split`.
    #[command(external_subcommand)]
    Exec(Vec<String>),
}

pub async fn execute(command: VpnCommands) -> anyhow::Result<()> {
    let verb = match command {
        VpnCommands::Status => "status",
        VpnCommands::Connect => "connect",
        VpnCommands::Disconnect => "disconnect",
        VpnCommands::Toggle => "toggle",
        VpnCommands::Reconnect => "reconnect",
        VpnCommands::Random => "random",
        VpnCommands::Fastest => "fastest",
        VpnCommands::Menu => {
            bus_command("Vpn").await?;
            return Ok(());
        }
        VpnCommands::Exec(args) => return proxy::run("mvpn", &args),
    };
    proxy::run("mvpn", [verb])
}
