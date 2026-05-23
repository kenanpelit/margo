//! mwizard — opens margo's setup wizard.
//!
//! The wizard is no longer a standalone GTK window. It is an in-shell
//! **layer-shell menu** owned by mshell (the same surface class as every
//! other bar-adjacent menu), so it can sit above the shell's other
//! layer-shell surfaces and actually take input. This binary is kept only
//! as a thin compatibility shim: it asks the running shell to open that
//! menu over D-Bus (`com.mshell.Shell` → `OpenWizard`), exactly like
//! `mshellctl wizard`. It never spawns a window of its own.
//!
//! Legacy flags such as `--force` are accepted and ignored — there is no
//! longer a separate "force re-run" path; opening the menu always works.

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about = "Open margo's in-shell setup wizard", long_about = None)]
struct Cli {
    /// Accepted for backwards compatibility; the wizard menu always opens.
    #[arg(long, hide = true)]
    force: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let _ = Cli::parse();

    let connection = zbus::connection::Builder::session()
        .context("connect to the session bus")?
        .build()
        .await
        .context("connect to the session bus")?;

    connection
        .call_method(
            Some("com.mshell.Shell"),
            "/com/mshell/Shell",
            Some("com.mshell.Shell"),
            "OpenWizard",
            &(),
        )
        .await
        .context("ask mshell to open the setup wizard menu (is the shell running?)")?;

    Ok(())
}
