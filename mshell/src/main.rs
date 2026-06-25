use clap::Parser;
use std::error::Error;

// Route every Rust-side allocation through mimalloc instead of glibc malloc.
// The shell is long-lived and many-threaded; mimalloc fragments far less and
// returns freed memory to the OS, so RSS doesn't ratchet up over a session.
// GTK's C-side allocations still use glibc malloc — `MALLOC_ARENA_MAX=2` in
// mshell.service bounds those arenas — so the two changes are complementary.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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

    // File logging is brought up inside `mshell_core::run()` once the config
    // (and thus the user's `[logging]` knobs) is loaded.
    mshell_core::run()?;

    Ok(())
}
