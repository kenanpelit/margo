//! Settings -> Guide -> Tools: live `--help` output for margo's companion
//! binaries. None of these expose a structured catalogue the way
//! `mctl::actions` does, and several aren't confirmed to use `clap`, so
//! this runs the real binary and shows its output verbatim rather than
//! trying to parse a shape that isn't guaranteed consistent.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The companion binaries a user would actually run themselves --
/// excludes `margo`/`mshell` (not standalone CLI tools), `margo-portal`/
/// `mshellshare` (backend daemons, no user-facing CLI), `start-margo` (a
/// supervisor process), `mgreet` (a graphical greeter with no desktop
/// session to run it from), and `mvisual` (an internal debugging helper).
pub const TOOLS: &[&str] = &[
    "mctl",
    "mshellctl",
    "mlock",
    "mlogind",
    "mlayout",
    "mscreenshot",
    "mkeys",
    "mvpn",
    "mcal",
    "mtune",
    "mpicker",
    "mdots",
    "mpower",
    "mwizard",
];

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub enum HelpResult {
    Output(String),
    NotFound,
    TimedOut,
}

/// Runs `<binary> --help` on a background thread and calls `on_done` with
/// the result. `on_done` is called from that background thread, NOT the
/// GTK main thread -- callers must marshal back via
/// `ComponentSender::input` (or equivalent) themselves, same as
/// `about_settings.rs`'s `spawn_gpu`/`GpuLoaded`.
pub fn spawn_help<F>(binary: &'static str, on_done: F)
where
    F: FnOnce(HelpResult) + Send + 'static,
{
    std::thread::spawn(move || {
        on_done(run_help(binary));
    });
}

fn run_help(binary: &str) -> HelpResult {
    let mut child = match Command::new(binary)
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return HelpResult::NotFound,
    };

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                let Ok(output) = child.wait_with_output() else {
                    return HelpResult::NotFound;
                };
                // Some tools print --help to stderr instead of stdout;
                // show whichever stream actually has content, preferring
                // stdout.
                let stdout = String::from_utf8_lossy(&output.stdout);
                let text = if stdout.trim().is_empty() {
                    String::from_utf8_lossy(&output.stderr).into_owned()
                } else {
                    stdout.into_owned()
                };
                return HelpResult::Output(text);
            }
            Ok(None) => {
                if start.elapsed() >= TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return HelpResult::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return HelpResult::NotFound,
        }
    }
}
