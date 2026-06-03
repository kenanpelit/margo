//! `mshellctl screenshot` — the single front door for screenshots.
//!
//! Every screenshot keybind and the screenshot menu route through here, so
//! capture behaviour is identical everywhere. The capture engine is
//! `mscreenshot` (grim + editor + clipboard + notify); region capture goes
//! through margo's in-compositor frozen-overlay selector
//! (`mctl dispatch screenshot-region-ui <mode>`), which then hands the
//! geometry to `mscreenshot`.
//!
//! ```text
//! mshellctl screenshot region|window|output|full [--copy|--save|--edit] [--delay N]
//! ```
//! Default delivery is edit → save + clipboard. `--copy` = clipboard only,
//! `--save` = file only, `--edit` = edit → save (no clipboard).

use crate::bus::bus_command_with_reply;
use clap::{Args, Subcommand};
use std::process::Command;

#[derive(Subcommand, Debug)]
pub enum ScreenshotCommands {
    /// Region select (frozen-overlay) → capture.
    Region(Capture),
    /// Focused window → capture.
    Window(Capture),
    /// Focused output (current monitor) → capture.
    Output(Capture),
    /// Whole layout (every output) → capture.
    Full(Capture),
    /// (internal) Open the in-shell area selector and print the chosen
    /// region as "X,Y WxH" (slurp format). `mscreenshot` polls this before
    /// falling back to slurp; not meant for direct use.
    SelectRegion,
}

/// Delivery flags shared by every capture area. If several are passed,
/// precedence is edit > save > copy; the default is edit → save + clipboard.
#[derive(Args, Debug, Default)]
pub struct Capture {
    /// Copy to clipboard only (no file, no editor).
    #[arg(long)]
    copy: bool,
    /// Save to file only (no clipboard, no editor).
    #[arg(long)]
    save: bool,
    /// Open the editor (satty / swappy) → save (no clipboard).
    #[arg(long)]
    edit: bool,
    /// Wait N seconds before capturing (catch menus / tooltips).
    #[arg(long, short = 'd')]
    delay: Option<u32>,
}

impl Capture {
    /// Pick the mscreenshot delivery subcommand for this area. The four
    /// args are the `(default, copy, save, edit)` mscreenshot subcommands.
    fn pick(
        &self,
        default: &'static str,
        copy: &'static str,
        save: &'static str,
        edit: &'static str,
    ) -> &'static str {
        if self.edit {
            edit
        } else if self.save {
            save
        } else if self.copy {
            copy
        } else {
            default
        }
    }
}

fn run_mscreenshot(mode: &str, delay: Option<u32>) {
    let mut cmd = Command::new("mscreenshot");
    cmd.arg(mode);
    if let Some(d) = delay {
        cmd.args(["-d", &d.to_string()]);
    }
    if let Err(e) = cmd.spawn() {
        eprintln!("mshellctl: failed to spawn mscreenshot: {e}");
    }
}

pub async fn execute(command: ScreenshotCommands) -> anyhow::Result<()> {
    match command {
        ScreenshotCommands::Region(c) => {
            // Region runs through margo's in-compositor selector, which
            // spawns mscreenshot with the chosen delivery mode.
            let m = c.pick("rec", "rc", "rf", "ri");
            if let Err(e) = Command::new("mctl")
                .args(["dispatch", "screenshot-region-ui", m])
                .spawn()
            {
                eprintln!("mshellctl: failed to spawn mctl dispatch: {e}");
            }
        }
        ScreenshotCommands::Window(c) => {
            run_mscreenshot(c.pick("window", "wc", "wf", "wi"), c.delay);
        }
        ScreenshotCommands::Output(c) => {
            run_mscreenshot(c.pick("screen", "sc", "sf", "si"), c.delay);
        }
        ScreenshotCommands::Full(c) => {
            // No clipboard-less "edit" alias for all-outputs; edit falls back
            // to the editing default.
            run_mscreenshot(c.pick("all", "ac", "af", "all"), c.delay);
        }
        ScreenshotCommands::SelectRegion => {
            let geom: String = bus_command_with_reply("SelectRegion").await?;
            println!("{}", geom);
        }
    }
    Ok(())
}
