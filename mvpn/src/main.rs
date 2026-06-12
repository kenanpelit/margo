//! mvpn — native Mullvad VPN control for margo.
//!
//! One binary: a full CLI (`mvpn connect`, `mvpn de`, `mvpn fastest`, …) and a
//! GTK4 layer-shell control panel (`mvpn menu`). The bar pill is a declarative
//! custom widget that polls `mvpn status --pill`.

mod engine;
mod ui;

use clap::{Parser, Subcommand};
use engine::{actions, blocky, diag, favorites, notify, obf, relays, slot, status, timer};

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
    Toggle {
        /// Also reconcile the blocky DNS guard to the new VPN state
        /// (VPN up → blocky off; VPN down → blocky on), like
        /// `osc-mullvad toggle --with-blocky`.
        #[arg(long)]
        with_blocky: bool,
    },
    /// Reconnect (re-establish the tunnel).
    Reconnect,
    /// Connect to a random relay (optionally in a country).
    Random { country: Option<String> },
    /// Connect to a Mullvad-owned relay (optionally in a country).
    Owned { country: Option<String> },
    /// Connect to a rented relay (optionally in a country).
    Rented { country: Option<String> },
    /// Toggle WireGuard quantum-resistant key exchange (`protocol` = alias).
    #[command(alias = "protocol")]
    Quantum,
    /// Ping every relay (in a country, or a global sample), connect to the
    /// genuinely fastest. Does NOT touch favorites.
    Fastest { country: Option<String> },
    /// Like `fastest`, but also record the winner in favorites.
    #[command(name = "fastest-fav")]
    FastestFav { country: Option<String> },
    /// Seed favorites with the fastest relay across a country group
    /// (europe|americas|asia|africa|other|all).
    #[command(name = "fastest-fav-sweep")]
    FastestFavSweep {
        group: String,
        /// Relays to ping per country (default 6).
        count: Option<usize>,
    },
    /// Manage favorite relays.
    Fav {
        #[command(subcommand)]
        action: FavCmd,
    },
    /// List Mullvad countries as `code<TAB>name<TAB>relay-count` (for the menu).
    Countries,
    /// Anti-censorship / obfuscation: bare = show; `cycle`, `hunt443`, or a mode
    /// (auto|off|udp2tcp|shadowsocks|quic).
    Obf { arg: Option<String> },
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
    /// Device-slot management (multi-machine 5-device limit).
    Slot {
        #[command(subcommand)]
        action: SlotCmd,
    },
    /// Auto-switch to a random relay every N minutes.
    Timer {
        #[command(subcommand)]
        action: TimerCmd,
    },
    /// Leak test: confirm traffic exits through Mullvad.
    Test,
    /// Show processes excluded from the tunnel (split-tunnel).
    Split,
    /// Fail-safe: drive the blocky DNS guard from the current VPN state.
    Ensure,
    /// Print the bar-pill config snippet to add to your mshell profile.
    InstallPill,
    /// Print toggle states as key=value lines (for the Settings → VPN page).
    Toggles,
    /// Open the GTK control panel.
    Menu,
    /// Internal: the detached timer loop (used by `timer start`).
    #[command(name = "__timer-run", hide = true)]
    TimerRun { minutes: u64 },
    /// Anything else is treated as `<country> [city]` (e.g. `mvpn de`, `mvpn us nyc`).
    #[command(external_subcommand)]
    Location(Vec<String>),
}

#[derive(Subcommand, Debug)]
enum FavCmd {
    /// Add the currently-connected relay (measures its ping).
    Add,
    /// Remove a relay from favorites.
    Remove { relay: String },
    /// List favorites, fastest-first, with their pick numbers.
    List,
    /// Connect to a favorite: by list number (`fav connect 2`), by relay
    /// name, or with no argument an interactive pick (fastest when piped).
    Connect { relay: Option<String> },
    /// Re-ping favorites (optionally in a country), drop dead ones, re-sort.
    Refresh { country: Option<String> },
}

#[derive(Subcommand, Debug)]
enum SlotCmd {
    /// Revoke other machines' devices → log in → connect → record self.
    Recycle {
        /// Only report what would be revoked.
        #[arg(long)]
        dry_run: bool,
        /// Run inside a held kitty window so the output stays visible (for
        /// keybind / menu triggers). Falls back to inline if kitty is missing.
        /// Mirrors `osc-mullvad slot --hold recycle`.
        #[arg(long)]
        hold: bool,
    },
    /// Show slot state + current device.
    Status,
    /// Print the current Mullvad device name.
    Whoami,
    /// List devices on the account.
    List,
    /// Revoke a device by name (refuses the current device).
    Revoke { device: String },
    /// Disconnect the VPN.
    Disconnect,
}

#[derive(Subcommand, Debug)]
enum TimerCmd {
    /// Start switching relays every N minutes.
    Start { minutes: u64 },
    /// Stop the auto-switch timer.
    Stop,
    /// Show whether the timer is running.
    Status,
}

// Ping sampling defaults (made configurable via mvpn.toml in a later phase).
/// Worldwide `fastest` sample cap (osc-mullvad's `max_relays`). A specific
/// country tests *all* its relays — see `favorites::fastest`.
const FASTEST_SAMPLE: usize = 10;
const PING_COUNT: u32 = 3;
const PING_TIMEOUT: u32 = 2;
const PASS_ENTRY: &str = "mullvad/account";

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
        Cmd::Connect => notify_after(actions::connect()),
        Cmd::Disconnect => notify_after(actions::disconnect()),
        Cmd::Toggle { with_blocky } => {
            let r = actions::toggle();
            if with_blocky {
                // Drive blocky to match the new VPN state (best-effort).
                let _ = blocky::ensure();
            }
            notify_after(r)
        }
        Cmd::Reconnect => notify_after(actions::reconnect()),
        Cmd::Random { country } => notify_after(actions::random(
            country.as_deref().unwrap_or(""),
            "",
            relays::Ownership::Any,
        )),
        Cmd::Owned { country } => notify_after(actions::random(
            country.as_deref().unwrap_or(""),
            "",
            relays::Ownership::Owned,
        )),
        Cmd::Rented { country } => notify_after(actions::random(
            country.as_deref().unwrap_or(""),
            "",
            relays::Ownership::Rented,
        )),
        Cmd::Quantum => actions::toggle_quantum(),
        // `fastest` leaves favorites untouched; `fastest-fav` records the
        // winner — same sweep otherwise (osc-mullvad's `add_to_favorites`).
        Cmd::Fastest { country } => run_fastest(country.as_deref().unwrap_or(""), false),
        Cmd::FastestFav { country } => run_fastest(country.as_deref().unwrap_or(""), true),
        Cmd::FastestFavSweep { group, count } => match relays::group_codes(&group) {
            Some(codes) => {
                let n = favorites::sweep(&codes, count.unwrap_or(6), PING_COUNT, PING_TIMEOUT);
                println!("seeded {n} favorite(s) across {group}");
                true
            }
            None => {
                eprintln!("mvpn: unknown group '{group}' (europe|americas|asia|africa|other|all)");
                false
            }
        },
        Cmd::Fav { action } => run_fav(action),
        Cmd::Countries => {
            for c in relays::countries() {
                println!("{}\t{}\t{}", c.code, c.name, c.relays);
            }
            true
        }
        Cmd::Obf { arg } => match arg.as_deref() {
            None => {
                let m = obf::current();
                println!("obfuscation: {}", if m.is_empty() { "unknown" } else { &m });
                true
            }
            Some("cycle") => obf::cycle().inspect(|m| println!("→ {m}")).is_some(),
            Some("hunt443") => obf::hunt443(),
            Some(mode) => obf::set(mode),
        },
        Cmd::Lockdown { state } => actions::set_lockdown(state == "on"),
        Cmd::AutoConnect { state } => actions::set_autoconnect(state == "on"),
        Cmd::Slot { action } => run_slot(action),
        Cmd::Timer { action } => match action {
            TimerCmd::Start { minutes } => match timer::start(minutes) {
                Ok(()) => {
                    println!("timer: switching every {minutes} min");
                    true
                }
                Err(e) => {
                    eprintln!("mvpn timer: {e}");
                    false
                }
            },
            TimerCmd::Stop => {
                println!(
                    "timer: {}",
                    if timer::stop() {
                        "stopped"
                    } else {
                        "not running"
                    }
                );
                true
            }
            TimerCmd::Status => {
                println!(
                    "timer: {}",
                    if timer::is_running() {
                        "running"
                    } else {
                        "stopped"
                    }
                );
                true
            }
        },
        Cmd::TimerRun { minutes } => timer::run(minutes),
        Cmd::Test => {
            let r = diag::leak_test();
            if !r.connected {
                println!("○ Not connected — exit IP {}", r.exit_ip);
            } else if r.mullvad_exit {
                println!(
                    "✔ Secure · exiting via Mullvad ({}) · {}",
                    r.exit_ip, r.relay
                );
            } else {
                println!("✘ LEAK · not exiting via Mullvad ({})", r.exit_ip);
            }
            r.mullvad_exit || !r.connected
        }
        Cmd::Split => {
            print!("{}", diag::split_tunnel());
            println!();
            true
        }
        Cmd::Ensure => {
            println!("blocky: {}", blocky::ensure());
            true
        }
        Cmd::InstallPill => {
            print_pill_snippet();
            true
        }
        Cmd::Toggles => {
            let on = |b: bool| if b { "on" } else { "off" };
            println!("lockdown={}", on(status::setting_on("lockdown-mode")));
            println!("autoconnect={}", on(status::setting_on("auto-connect")));
            println!("quantum={}", on(actions::quantum_on()));
            let m = obf::current();
            println!("obf={}", if m.is_empty() { "auto" } else { &m });
            println!("expiry={}", status::account_expiry());
            true
        }
        Cmd::Menu => ui::run(),
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

fn print_pill_snippet() {
    println!(
        r#"# Add an mvpn pill to your mshell bar — paste under bars.widgets.custom_widgets
# in your mshell profile, then reference its key in a bar slot.
#
# It polls `mvpn status --pill` (emits `#active` when connected → the
# `.custom-bar-widget.active` accent tint), left-click opens the panel,
# right-click toggles the tunnel.

[bars.widgets.custom_widgets.mvpn]
icon            = "network-vpn-symbolic"
exec            = "mvpn status --pill"
template        = "{{output}}"
interval        = 5
on_click        = "mvpn menu"
on_click_right  = "mvpn toggle"
tooltip         = "Mullvad VPN — click for the panel, right-click to toggle"

# Then add "mvpn" to a bar slot (e.g. bars.top.right) in the same profile."#
    );
}

/// Relaunch `mvpn slot recycle` inside a held kitty window (output stays
/// visible after the command exits). Returns true if kitty was spawned; false
/// when kitty is missing so the caller can run the recycle inline instead.
fn spawn_held_recycle(dry_run: bool) -> bool {
    let exe = std::env::current_exe().unwrap_or_else(|_| "mvpn".into());
    let mut cmd = std::process::Command::new("kitty");
    cmd.args(["--hold", "--class", "mvpn", "-T", "mvpn"])
        .arg(exe)
        .args(["slot", "recycle"]);
    if dry_run {
        cmd.arg("--dry-run");
    }
    cmd.spawn().is_ok()
}

fn run_slot(action: SlotCmd) -> bool {
    match action {
        SlotCmd::Whoami => {
            let d = slot::current_device();
            println!("{}", if d.is_empty() { "(not logged in)" } else { &d });
            !d.is_empty()
        }
        SlotCmd::List => {
            for d in slot::list_devices() {
                println!("{d}");
            }
            true
        }
        SlotCmd::Status => {
            println!("os-id:   {}", slot::os_id());
            println!("key:     {}", slot::state_key());
            println!("device:  {}", slot::current_device());
            true
        }
        SlotCmd::Revoke { device } => match slot::revoke(&device) {
            Ok(()) => {
                println!("revoked: {device}");
                true
            }
            Err(e) => {
                eprintln!("mvpn slot revoke: {e}");
                false
            }
        },
        SlotCmd::Disconnect => actions::disconnect(),
        SlotCmd::Recycle { dry_run, hold } => {
            // `--hold`: relaunch the recycle inside a held kitty window so the
            // output stays readable from a keybind / menu. If kitty spawns we're
            // done; otherwise fall through and run inline.
            if hold && spawn_held_recycle(dry_run) {
                return true;
            }
            // Honour OSC_MULLVAD_REVOKE_OTHERS (default true), like osc-mullvad.
            let revoke_others = std::env::var("OSC_MULLVAD_REVOKE_OTHERS")
                .map(|v| v != "false")
                .unwrap_or(true);
            match slot::recycle(revoke_others, PASS_ENTRY, dry_run) {
                Ok(dev) => {
                    println!("slot: {dev}");
                    true
                }
                Err(e) => {
                    eprintln!("mvpn slot recycle: {e}");
                    false
                }
            }
        }
    }
}

/// Print favorites with their 1-based pick numbers — the same ordering
/// `fav connect <n>` indexes into.
fn print_fav_list(favs: &[favorites::Fav]) {
    for (i, f) in favs.iter().enumerate() {
        match f.ping {
            Some(p) => println!("{:>3}) {:>5.0} ms  {}", i + 1, p, f.relay),
            None => println!("{:>3})   N/A    {}", i + 1, f.relay),
        }
    }
}

fn run_fav(action: FavCmd) -> bool {
    match action {
        FavCmd::Add => favorites::add_current(),
        FavCmd::Remove { relay } => {
            favorites::remove(&relay);
            true
        }
        FavCmd::List => {
            print_fav_list(&favorites::load());
            true
        }
        // With an arg → a 1-based index into the `fav list` ordering, or a
        // relay name. Without → interactive pick on a TTY, fastest favorite
        // otherwise (keybind/script-safe: the original behaviour).
        FavCmd::Connect { relay: Some(r) } => {
            let target = match r.parse::<usize>() {
                Ok(n) if n >= 1 => {
                    let favs = favorites::load();
                    match favs.get(n - 1) {
                        Some(f) => f.relay.clone(),
                        None => {
                            eprintln!(
                                "mvpn fav connect: index {n} out of range (favorites: 1-{})",
                                favs.len()
                            );
                            return false;
                        }
                    }
                }
                _ => r,
            };
            if actions::set_relay(&target) {
                println!("→ {target}");
                true
            } else {
                eprintln!("mvpn fav connect: failed to connect to {target}");
                false
            }
        }
        FavCmd::Connect { relay: None } => {
            use std::io::IsTerminal;
            // Non-interactive caller (keybind, script, pipe): keep the
            // original "connect to the fastest favorite" contract.
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return match favorites::connect_fastest() {
                    Some(r) => {
                        println!("→ {r}");
                        true
                    }
                    None => {
                        eprintln!("mvpn fav connect: no favorites (add one with `mvpn fav add`)");
                        false
                    }
                };
            }
            let favs = favorites::load();
            if favs.is_empty() {
                eprintln!("mvpn fav connect: no favorites (add one with `mvpn fav add`)");
                return false;
            }
            print_fav_list(&favs);
            eprint!("connect [1-{}] (Enter = fastest, q = cancel): ", favs.len());
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                return false;
            }
            let pick = line.trim();
            let target = match pick {
                "" => favs[0].relay.clone(),
                "q" | "Q" => return true,
                _ => match pick.parse::<usize>() {
                    Ok(n) if (1..=favs.len()).contains(&n) => favs[n - 1].relay.clone(),
                    _ => {
                        eprintln!(
                            "mvpn fav connect: pick a number between 1 and {}",
                            favs.len()
                        );
                        return false;
                    }
                },
            };
            if actions::set_relay(&target) {
                println!("→ {target}");
                true
            } else {
                eprintln!("mvpn fav connect: failed to connect to {target}");
                false
            }
        }
        FavCmd::Refresh { country } => {
            let favs =
                favorites::refresh(country.as_deref().unwrap_or(""), PING_COUNT, PING_TIMEOUT);
            print_fav_list(&favs);
            true
        }
    }
}

/// Run a connection action, then fire a desktop notification reflecting the
/// resulting status (connected relay + location, or disconnected). Returns the
/// action's own success flag unchanged.
/// Poll status until the tunnel settles (no longer "Connecting"), bounded to
/// ~4s. `mullvad connect` returns *before* the tunnel is actually up, so a
/// bare post-action `status::query()` can read the transient state and notify
/// the wrong direction ("Tunnel is down" right after a successful connect).
fn settle() -> status::Status {
    for _ in 0..20 {
        let st = status::query();
        if !st.connecting {
            return st;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    status::query()
}

fn notify_after(ok: bool) -> bool {
    let st = settle();
    if st.connected {
        let loc = if st.location.is_empty() {
            String::new()
        } else {
            format!(" · {}", st.location)
        };
        notify::send(
            "Mullvad connected",
            &format!("{}{loc}", st.relay),
            notify::icon_for(true),
        );
    } else {
        notify::send(
            "Mullvad disconnected",
            "Tunnel is down",
            notify::icon_for(false),
        );
    }
    ok
}

/// `fastest` / `fastest-fav`: sweep relays, print each tested relay's ping
/// (fastest-first, osc-mullvad style), connect to the genuinely fastest, and
/// notify. `add_to_fav` records the winner in favorites.
fn run_fastest(country: &str, add_to_fav: bool) -> bool {
    let where_ = if country.is_empty() {
        "globally".to_string()
    } else {
        format!("in {}", country.to_uppercase())
    };
    notify::send(
        "Mullvad",
        &format!("Finding fastest relay {where_}…"),
        "network-vpn-acquiring-symbolic",
    );
    println!("Finding fastest relay {where_}…");

    match favorites::fastest(
        country,
        FASTEST_SAMPLE,
        PING_COUNT,
        PING_TIMEOUT,
        add_to_fav,
    ) {
        Some(res) => {
            for (relay, ms) in &res.measured {
                let mark = if *relay == res.relay { "→" } else { " " };
                println!("  {mark} {relay}  {ms:.0} ms");
            }
            println!("\nConnected: {} · {:.0} ms", res.relay, res.avg);
            let st = status::query();
            let loc = if st.location.is_empty() {
                String::new()
            } else {
                format!(" · {}", st.location)
            };
            notify::send(
                "Mullvad connected",
                &format!("{} · {:.0} ms{loc}", res.relay, res.avg),
                notify::icon_for(true),
            );
            true
        }
        None => {
            eprintln!("mvpn fastest: no responsive relay found");
            notify::send(
                "Mullvad",
                "No responsive relay found",
                notify::icon_for(false),
            );
            false
        }
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
