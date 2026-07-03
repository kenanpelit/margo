//! `mshellctl calendar …` — a thin proxy to the `mcal` calendar CLI, so the
//! shell's control surface can reach calendar data + account management without
//! remembering a separate binary.
//!
//! Shells out to `mcal` (must be on `$PATH`); stdio is inherited so
//! `account setup google` can drive the browser OAuth flow and agenda output
//! renders normally. mcal's exit code is propagated.

use crate::subcommands::proxy;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum CalendarCommands {
    /// Events happening today.
    Today,
    /// Events over the next DAYS days (default 7).
    Agenda {
        /// Number of days (default 7).
        days: Option<u32>,
    },
    /// Events on a specific date (YYYY-MM-DD).
    On {
        /// The date, e.g. 2026-07-04.
        date: String,
    },
    /// Manage connected accounts — `list`, `setup google`, `remove <id>`.
    Account {
        /// Arguments passed straight to `mcal account …`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

pub async fn execute(command: CalendarCommands) -> anyhow::Result<()> {
    let mut args: Vec<String> = Vec::new();
    match command {
        CalendarCommands::Today => args.push("today".into()),
        CalendarCommands::Agenda { days } => {
            args.push("agenda".into());
            if let Some(days) = days {
                args.push(days.to_string());
            }
        }
        CalendarCommands::On { date } => {
            args.push("on".into());
            args.push(date);
        }
        CalendarCommands::Account { args: rest } => {
            args.push("account".into());
            args.extend(rest);
        }
    }
    proxy::run("mcal", &args)
}
