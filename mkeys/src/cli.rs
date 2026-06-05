use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "mkeys", version, about = "margo on-screen keyboard")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug, Clone, Copy)]
pub enum Cmd {
    /// Show the keyboard (start it if not already running).
    Show,
    /// Hide the keyboard (quit the running instance).
    Hide,
    /// Toggle the keyboard — the default when run with no subcommand.
    Toggle,
}
