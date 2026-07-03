//! `mshellctl power …` — power-profile control, proxied to the `mpower`
//! daemon (CPU + AC/battery aware profile manager).
//!
//! This *sets/queries* the profile. To open the shell's Power Profile menu,
//! use `mshellctl menu power`.

use crate::subcommands::proxy;
use clap::{Subcommand, ValueEnum};

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum PowerProfile {
    Performance,
    Balanced,
    #[value(name = "power-saver")]
    PowerSaver,
}

impl PowerProfile {
    fn as_str(self) -> &'static str {
        match self {
            PowerProfile::Performance => "performance",
            PowerProfile::Balanced => "balanced",
            PowerProfile::PowerSaver => "power-saver",
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum PowerCommands {
    /// Print the active profile + governor state.
    Status,
    /// Cycle to the next profile.
    Cycle,
    /// Set a specific profile.
    Set {
        /// performance | balanced | power-saver
        #[arg(value_enum)]
        profile: PowerProfile,
    },
    /// Pause automatic management (hold the current profile).
    Pause,
    /// Resume automatic management.
    Resume,
    /// Resume automatic management (alias for `resume`).
    Auto,
}

pub async fn execute(command: PowerCommands) -> anyhow::Result<()> {
    match command {
        PowerCommands::Status => proxy::run("mpower", ["status"]),
        PowerCommands::Cycle => proxy::run("mpower", ["cycle"]),
        PowerCommands::Set { profile } => proxy::run("mpower", ["set", profile.as_str()]),
        PowerCommands::Pause => proxy::run("mpower", ["pause"]),
        PowerCommands::Resume => proxy::run("mpower", ["resume"]),
        PowerCommands::Auto => proxy::run("mpower", ["auto"]),
    }
}
