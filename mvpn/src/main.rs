//! mvpn — native Mullvad VPN control for margo.
//!
//! One binary: a full CLI (`mvpn connect`, `mvpn de`, `mvpn fastest`, …) and a
//! GTK4 layer-shell control panel (`mvpn menu`). The bar pill is a declarative
//! custom widget that polls `mvpn status --pill`.

mod engine;

use clap::{Parser, Subcommand};
use engine::{actions, relays, status};

#[derive(Parser, Debug)]
#[command(name = "mvpn", version, about = "Native Mullvad VPN control for margo")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Show connection status (`--pill` for the bar feed, `-v`/`--json` for detail).
    Status {
        /// Emit the one-line bar-pill feed (`#active` + label when connected).
        #[arg(long)]
        pill: bool,
        /// Verbose multi-line status.
        #[arg(short, long)]
        verbose: bool,
        /// Machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Connect using the current relay settings.
    Connect,
    /// Disconnect the tunnel.
    Disconnect,
    /// Toggle the connection on/off.
    Toggle,
    /// Reconnect (re-establish the tunnel).
    Reconnect,
    /// Connect to a random relay (optionally in a country).
    Random { country: Option<String> },
    /// Connect to a Mullvad-owned relay (optionally in a country).
    Owned { country: Option<String> },
    /// Connect to a rented relay (optionally in a country).
    Rented { country: Option<String> },
    /// Toggle the tunnel protocol (WireGuard ↔ OpenVPN).
    Protocol,
    /// Set lockdown mode (block traffic when the VPN drops).
    Lockdown {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Set auto-connect on daemon start.
    #[command(name = "auto-connect")]
    AutoConnect {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Open the GTK control panel.
    Menu,
    /// Anything else is treated as `<country> [city]` (e.g. `mvpn de`, `mvpn us nyc`).
    #[command(external_subcommand)]
    Location(Vec<String>),
}

fn main() {
    let cli = Cli::parse();
    // No subcommand → show status (like `osc-mullvad` with no args).
    let cmd = cli.cmd.unwrap_or(Cmd::Status {
        pill: false,
        verbose: false,
        json: false,
    });

    let ok = match cmd {
        Cmd::Status {
            pill,
            verbose,
            json,
        } => {
            print_status(pill, verbose, json);
            true
        }
        Cmd::Connect => actions::connect(),
        Cmd::Disconnect => actions::disconnect(),
        Cmd::Toggle => actions::toggle(),
        Cmd::Reconnect => actions::reconnect(),
        Cmd::Random { country } => {
            actions::random(country.as_deref().unwrap_or(""), "", relays::Ownership::Any)
        }
        Cmd::Owned { country } => actions::random(
            country.as_deref().unwrap_or(""),
            "",
            relays::Ownership::Owned,
        ),
        Cmd::Rented { country } => actions::random(
            country.as_deref().unwrap_or(""),
            "",
            relays::Ownership::Rented,
        ),
        Cmd::Protocol => actions::toggle_protocol(),
        Cmd::Lockdown { state } => actions::set_lockdown(state == "on"),
        Cmd::AutoConnect { state } => actions::set_autoconnect(state == "on"),
        Cmd::Menu => {
            eprintln!("mvpn menu: the GTK panel is not wired up yet (UI phase).");
            false
        }
        Cmd::Location(args) => {
            // `mvpn de` / `mvpn us nyc` → pick a random relay there + connect.
            let country = args.first().map(String::as_str).unwrap_or("");
            let city = args.get(1).map(String::as_str).unwrap_or("");
            if country.is_empty() {
                eprintln!("mvpn: unknown command");
                false
            } else {
                actions::random(country, city, relays::Ownership::Any)
            }
        }
    };

    if !ok {
        std::process::exit(1);
    }
}

fn print_status(pill: bool, verbose: bool, json: bool) {
    let st = status::query();
    if pill {
        // First line `#active` → mshell turns it into the `.active` CSS class
        // (accent tint). Label = the connected country code, else "off".
        if st.connected {
            let cc = st.relay.split('-').next().unwrap_or("").to_uppercase();
            println!("#active");
            println!("{}", if cc.is_empty() { "VPN".into() } else { cc });
        } else {
            println!("off");
        }
        return;
    }
    if json {
        println!(
            "{}",
            serde_json::to_string(&st).unwrap_or_else(|_| "{}".into())
        );
        return;
    }
    if verbose {
        print!("{}", engine::sys::mullvad(&["status", "-v"]));
        println!();
        return;
    }
    if st.connected {
        let where_ = if st.city.is_empty() {
            st.country.clone()
        } else {
            format!("{}, {}", st.city, st.country)
        };
        println!("● Connected · {} · {}", st.relay, where_);
    } else {
        println!("○ {}", st.state);
    }
}
