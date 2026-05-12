//! Daemon ↔ CLI IPC over a Unix socket at
//! `$XDG_RUNTIME_DIR/midle.sock`. The wire format is line-delimited
//! JSON — one request, one response.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    Info,
    Pause { duration: Option<String> },
    Resume,
    ToggleInhibit,
    Reload,
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Response {
    Ok { info: Option<DaemonInfo> },
    Err { message: String },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DaemonInfo {
    pub running: bool,
    pub inhibit: bool,
    /// Granular breakdown of *why* idle is currently suppressed.
    /// Helpful when diagnosing "midle is not firing": shows which
    /// inhibitor source — manual toggle, app scan match, audio
    /// sink RUNNING, D-Bus inhibitor — is responsible.
    pub inhibitors: InhibitorBreakdown,
    pub pause: String,
    pub steps: Vec<StepInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InhibitorBreakdown {
    pub manual: bool,
    pub app: Option<String>,
    pub media: bool,
    pub dbus: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepInfo {
    pub name: String,
    pub timeout_seconds: u64,
    pub fired: bool,
}

pub fn socket_path() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("midle.sock");
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/midle-{uid}.sock"))
}

/// CLI subcommand → daemon request. Synchronous (blocking) — the
/// CLI doesn't run inside the tokio runtime.
pub fn run_client(cmd: crate::Command) -> Result<()> {
    let req = match cmd {
        crate::Command::Info => Request::Info,
        crate::Command::Pause { duration } => Request::Pause { duration },
        crate::Command::Resume => Request::Resume,
        crate::Command::ToggleInhibit => Request::ToggleInhibit,
        crate::Command::Reload => Request::Reload,
        crate::Command::Stop => Request::Stop,
    };

    let path = socket_path();
    let body = serde_json::to_string(&req).context("encode request")?;

    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(&path).with_context(|| {
        format!(
            "connect to midle daemon at {} — is the daemon running?",
            path.display()
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .context("set timeout")?;
    stream.write_all(body.as_bytes()).context("send request")?;
    stream.write_all(b"\n").context("send request")?;
    stream.shutdown(std::net::Shutdown::Write).ok();

    let mut buf = String::new();
    stream.read_to_string(&mut buf).context("read response")?;
    let response: Response = serde_json::from_str(buf.trim())
        .with_context(|| format!("decode response: {buf:?}"))?;

    match response {
        Response::Ok { info } => {
            if let Some(info) = info {
                println!("{}", serde_json::to_string_pretty(&info)?);
            }
            Ok(())
        }
        Response::Err { message } => Err(anyhow!(message)),
    }
}
