//! mtm — native tmux session/layout/buffer/clipboard/plugin manager for
//! margo. Rust port of `tm.sh` (speed launcher + config backup/restore
//! modules land in follow-up commits).

mod backup;
mod buffer;
mod clip;
mod config;
mod fzf;
mod kenp;
mod plugin;
mod session;
mod speed;
mod tmux;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "mtm", version, about = "tmux session/layout manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Session management: create/list/kill/attach/layout/term
    #[command(aliases = ["s"], subcommand)]
    Session(session::SessionCommands),
    /// Buffer management: list/show (fzf pick + copy to clipboard)
    #[command(aliases = ["b"])]
    Buffer {
        #[command(subcommand)]
        cmd: Option<buffer::BufferCommands>,
    },
    /// Clipboard history picker (cliphist or clipse)
    #[command(aliases = ["c"])]
    Clip,
    /// TPM plugin management: install/list/all
    #[command(aliases = ["p"], subcommand)]
    Plugin(plugin::PluginCommands),
    /// Category-aware fzf command launcher (pin + recency)
    #[command(aliases = ["cmd"])]
    Speed {
        #[command(subcommand)]
        cmd: Option<speed::SpeedCommands>,
    },
    /// Config backup/restore
    #[command(aliases = ["cfg"], subcommand)]
    Config(backup::ConfigCommands),
    /// Attach to the default session (also what bare `mtm` does)
    #[command(aliases = ["k"])]
    Kenp { name: Option<String> },
    /// Anything else is treated as a session name to create/attach to,
    /// matching `tm.sh`'s fallback ("bilinmeyen komut" → session name).
    #[command(external_subcommand)]
    External(Vec<String>),
}

fn main() {
    let cli = Cli::parse();
    let cfg = config::Config::load();

    let result = match cli.command {
        None => kenp::default_session(None, &cfg),
        Some(Commands::Session(cmd)) => session::run(cmd, &cfg),
        Some(Commands::Buffer { cmd }) => buffer::run(cmd, &cfg),
        Some(Commands::Clip) => clip::run(&cfg),
        Some(Commands::Plugin(cmd)) => plugin::run(cmd),
        Some(Commands::Speed { cmd }) => speed::run(cmd, &cfg),
        Some(Commands::Config(cmd)) => backup::run(cmd),
        Some(Commands::Kenp { name }) => kenp::default_session(name.as_deref(), &cfg),
        Some(Commands::External(args)) => {
            let name = args.first().map(String::as_str);
            match name {
                Some(name) => kenp_fallback_session(name),
                None => kenp::default_session(None, &cfg),
            }
        }
    };

    if let Err(err) = result {
        eprintln!("mtm: {err}");
        std::process::exit(1);
    }
}

/// A bare word that isn't a known subcommand is a session name — create or
/// attach to it directly (not through the KENP/anka coordination path,
/// which is reserved for the actual default session).
fn kenp_fallback_session(name: &str) -> anyhow::Result<()> {
    session::run(
        session::SessionCommands::Create {
            name: Some(name.to_string()),
            layout: None,
        },
        &config::Config::load(),
    )
}
