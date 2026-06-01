//! MPD backend via the `mpc` CLI.

use super::status::{Command, Status};
use std::process::Command as Proc;

/// Extract the playback state from `mpc status` output. The state line
/// carries a `[playing]` / `[paused]` bracket; absence means stopped.
pub fn parse_mpc_status(out: &str) -> Status {
    for line in out.lines() {
        if let Some(start) = line.find('[')
            && let Some(end) = line[start + 1..].find(']')
        {
            return Status::normalize(&line[start + 1..start + 1 + end]);
        }
    }
    Status::Stopped
}

pub fn available() -> bool {
    Proc::new("mpc")
        .arg("status")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn status() -> Status {
    match Proc::new("mpc").arg("status").output() {
        Ok(o) if o.status.success() => parse_mpc_status(&String::from_utf8_lossy(&o.stdout)),
        _ => Status::Unknown,
    }
}

pub fn current(fmt: &str) -> Option<String> {
    let o = Proc::new("mpc")
        .args(["current", "-f", fmt])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub fn control(cmd: Command) -> bool {
    let verb = match cmd {
        Command::Toggle => "toggle",
        Command::Play => "play",
        Command::Pause => "pause",
        Command::Stop => "stop",
        Command::Next => "next",
        Command::Prev => "prev",
        Command::Status => return true,
    };
    Proc::new("mpc")
        .arg(verb)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state_bracket() {
        let playing = "Artist - Title\n[playing] #1/10   0:12/3:45 (5%)\nvolume: 50%";
        assert_eq!(parse_mpc_status(playing), Status::Playing);
        let paused = "Artist - Title\n[paused]  #1/10   0:12/3:45 (5%)\n";
        assert_eq!(parse_mpc_status(paused), Status::Paused);
        // Stopped MPD prints no bracketed state line.
        assert_eq!(parse_mpc_status("volume: 50%\n"), Status::Stopped);
    }
}
