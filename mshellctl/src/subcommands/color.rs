//! `mshellctl color …` — pick a screen colour, proxied to the native
//! `mpicker` (wlr-screencopy → frozen overlay + zoom lens).
//!
//! With no flags it prints the picked colour to stdout (script-friendly).

use crate::subcommands::proxy;
use clap::{Args, ValueEnum};

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
    Cmyk,
}

impl ColorFormat {
    fn as_str(self) -> &'static str {
        match self {
            ColorFormat::Hex => "hex",
            ColorFormat::Rgb => "rgb",
            ColorFormat::Hsl => "hsl",
            ColorFormat::Cmyk => "cmyk",
        }
    }
}

#[derive(Args, Debug)]
pub struct ColorArgs {
    /// Copy the picked colour to the clipboard.
    #[arg(long)]
    copy: bool,
    /// Show a notification with the picked colour.
    #[arg(long)]
    notify: bool,
    /// Output format (default: hex).
    #[arg(long, value_enum, default_value = "hex")]
    format: ColorFormat,
    /// Lower-case the hex output.
    #[arg(long)]
    lowercase: bool,
    /// Disable the zoom lens.
    #[arg(long)]
    no_zoom: bool,
    /// Suppress the printed result (pair with `--copy`/`--notify`).
    #[arg(long)]
    quiet: bool,
}

pub async fn execute(args: ColorArgs) -> anyhow::Result<()> {
    let mut argv: Vec<String> = vec!["--format".into(), args.format.as_str().into()];
    if args.copy {
        argv.push("--autocopy".into());
    }
    if args.notify {
        argv.push("--notify".into());
    }
    if args.lowercase {
        argv.push("--lowercase-hex".into());
    }
    if args.no_zoom {
        argv.push("--no-zoom".into());
    }
    if args.quiet {
        argv.push("--quiet".into());
    }
    proxy::run("mpicker", &argv)
}
