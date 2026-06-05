mod cli;
mod config;
mod ipc;
mod layout;
mod native;
mod service;
mod ui;

use clap::Parser;
use cli::{Cli, Cmd};
use ipc::Ipc;
use service::client::MessageService;
use service::host::AppService;

fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let cmd = cli.cmd.unwrap_or(Cmd::Toggle);

    let ipc = Ipc::init();
    if ipc.is_single_instance() {
        // Nothing is running yet — this process becomes the keyboard.
        match cmd {
            // Hide with nothing running is a no-op.
            Cmd::Hide => Ipc::clean_up(),
            Cmd::Show | Cmd::Toggle => {
                ctrlc::set_handler(|| {
                    Ipc::clean_up();
                    std::process::exit(0);
                })
                .ok();
                AppService::new(ipc).run();
            }
        }
    } else {
        // An instance is already running (keyboard visible).
        match cmd {
            // Already visible — nothing to do.
            Cmd::Show => {}
            // Tell the running instance to quit → keyboard disappears.
            Cmd::Hide | Cmd::Toggle => MessageService::new(ipc).send(b"quit"),
        }
    }
}
