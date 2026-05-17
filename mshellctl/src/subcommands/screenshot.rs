//! `mshellctl screenshot` — bridge subcommand for the mscreenshot
//! CLI. Currently only exposes `select-region`, which opens the
//! in-shell area selector via D-Bus and prints the resulting
//! geometry to stdout in slurp's `"X,Y WxH"` format. mscreenshot
//! polls this before falling back to spawning `slurp` directly.

use crate::bus::bus_command_with_reply;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ScreenshotCommands {
    /// Open the in-shell area selector and print the user-chosen
    /// region to stdout as "X,Y WxH" (slurp format). Empty stdout
    /// on cancel; exit code is always 0 as long as the IPC call
    /// itself succeeds — callers detect cancel by checking for an
    /// empty line, the same convention slurp uses.
    SelectRegion,
}

pub async fn execute(command: ScreenshotCommands) -> anyhow::Result<()> {
    match command {
        ScreenshotCommands::SelectRegion => {
            let geom: String = bus_command_with_reply("SelectRegion").await?;
            println!("{}", geom);
        }
    }
    Ok(())
}
