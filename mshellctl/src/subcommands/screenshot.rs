//! `mshellctl screenshot` — the single front door for screenshots.
//!
//! Drives the shell's own screenshot engine (the same one behind the GUI
//! `mshellctl menu screenshot`: in-shell selectors + save / clipboard /
//! editor / notify) headlessly over IPC. One engine, one tool — keybinds,
//! the CLI, and the menu all run the exact same path.
//!
//! ```text
//! mshellctl screenshot region|window|output|full [EDITOR] [--copy|--save|--edit] [--delay N]
//! ```
//! Default delivery is file + clipboard. `--copy` = clipboard only,
//! `--save` = file only, `--edit` = editor → save (no clipboard).
//! A positional editor name (`mshellctl screenshot region satty`) forces
//! that annotation tool and implies `--edit`.

use crate::bus::{bus_command_with_arg, bus_command_with_reply};
use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum ScreenshotCommands {
    /// Region select (in-shell selector) → capture.
    Region(Capture),
    /// Focused window → capture.
    Window(Capture),
    /// Pick an output (monitor) → capture.
    Output(Capture),
    /// Whole layout (every output) → capture.
    Full(Capture),
    /// (internal) Open the in-shell area selector and print the chosen
    /// region as "X,Y WxH" (slurp format). Used by the `mscreenshot` CLI's
    /// region bridge; not meant for direct use.
    SelectRegion,
}

/// Delivery flags shared by every capture area. If several are passed,
/// precedence is edit > save > copy; the default is file + clipboard.
#[derive(Args, Debug, Default)]
pub struct Capture {
    /// Copy to clipboard only (no file, no editor).
    #[arg(long)]
    copy: bool,
    /// Save to file only (no clipboard, no editor).
    #[arg(long)]
    save: bool,
    /// Open the editor (satty / swappy) → save.
    #[arg(long)]
    edit: bool,
    /// Wait N seconds before capturing (catch menus / tooltips).
    #[arg(long, short = 'd')]
    delay: Option<u32>,
    /// Force a specific annotation editor (satty | swappy | gimp |
    /// krita). Implies `--edit`. Omit to use the default chain
    /// (`SCREENSHOT_EDITOR` env, then satty → swappy → gimp → krita).
    #[arg(value_name = "EDITOR")]
    editor: Option<String>,
}

impl Capture {
    fn target(&self) -> &'static str {
        // A named editor implies edit mode.
        if self.edit || self.editor.is_some() {
            "edit"
        } else if self.save {
            "save"
        } else if self.copy {
            "copy"
        } else {
            "default"
        }
    }
}

/// Fire the headless capture: `"<area> <target> <delay> <editor>"` over IPC.
/// `<editor>` is `-` when unset.
async fn capture(area: &str, c: &Capture) -> anyhow::Result<()> {
    let editor = c.editor.as_deref().unwrap_or("-");
    let spec = format!("{area} {} {} {editor}", c.target(), c.delay.unwrap_or(0));
    bus_command_with_arg("ScreenshotCapture", &spec).await?;
    Ok(())
}

pub async fn execute(command: ScreenshotCommands) -> anyhow::Result<()> {
    match command {
        ScreenshotCommands::Region(c) => capture("region", &c).await?,
        ScreenshotCommands::Window(c) => capture("window", &c).await?,
        ScreenshotCommands::Output(c) => capture("output", &c).await?,
        ScreenshotCommands::Full(c) => capture("full", &c).await?,
        ScreenshotCommands::SelectRegion => {
            let geom: String = bus_command_with_reply("SelectRegion").await?;
            println!("{}", geom);
        }
    }
    Ok(())
}
