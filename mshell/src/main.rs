use clap::Parser;
use std::error::Error;

#[derive(Parser)]
#[command(
    name = "mshell",
    version,
    about = "MShell desktop shell",
    styles = mshell_cli_style::get_styles(),
)]
struct Args {}

fn main() -> Result<(), Box<dyn Error>> {
    let _args = Args::parse();

    mshell_logging::init("mshell");

    mshell_core::run()?;

    Ok(())
}
