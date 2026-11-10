use crate::bus::shell;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum TimerCommands {
    /// Add a timer or reminder from one line of text — "10m tea",
    /// "1h 30m", "14:30 meeting", "11am standup", "tomorrow 09:00 coffee",
    /// "in 10m tea", "after 5m - stretch", "10 dk sonra çay",
    /// "yarın 09:00 kahve". Prints when it rings.
    Add {
        /// The line (quote it, or pass it as several words).
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// List pending timers, soonest first: id, due time, time left, label
    /// (tab-separated).
    List,
    /// Cancel the timer with this id (from `list`).
    Cancel {
        /// Timer id.
        id: u32,
    },
    /// Cancel every pending timer.
    Clear,
}

pub async fn execute(command: Option<TimerCommands>) -> anyhow::Result<()> {
    let (verb, arg) = match command {
        // Bare `mshellctl timer` lists.
        None | Some(TimerCommands::List) => ("list", String::new()),
        Some(TimerCommands::Add { text }) => ("add", text.join(" ")),
        Some(TimerCommands::Cancel { id }) => ("cancel", id.to_string()),
        Some(TimerCommands::Clear) => ("clear", String::new()),
    };
    let out: String = shell().await?.timer(verb, &arg).await?;
    if out.is_empty() {
        if verb == "list" {
            eprintln!("no timers");
        }
    } else {
        println!("{out}");
    }
    Ok(())
}
