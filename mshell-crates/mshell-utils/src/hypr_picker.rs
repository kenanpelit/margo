use std::process::Stdio;
use tokio::process::Command;
use tokio::time::sleep;
use tracing::error;

fn extract_hex_color(s: &str) -> Option<String> {
    // minimal + fast: find first token starting with '#'
    // tighten if needed (6 or 8 hex digits)
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' {
            let rest = &s[i..];
            let token = rest.split_whitespace().next().unwrap_or(rest);

            let hex = token.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '#');

            // accept #RRGGBB or #RRGGBBAA
            if (hex.len() == 7 || hex.len() == 9) && hex[1..].chars().all(|c| c.is_ascii_hexdigit())
            {
                return Some(hex.to_string());
            }
        }
    }
    None
}

/// Spawn the picker binary and capture its stdout hex. Prefers
/// margo's native `mpicker` (frozen screencap + zoom lens, ships
/// in this workspace); falls back to `hyprpicker` for users who
/// haven't installed mpicker yet. Either tool prints the picked
/// colour as its first stdout line; we don't pass `-a` because
/// the caller (`spawn_color_picker`) copies + notifies itself
/// once it has the hex token in hand.
async fn run_picker() -> anyhow::Result<String> {
    let tool = if which("mpicker") {
        "mpicker"
    } else {
        "hyprpicker"
    };
    let out = Command::new(tool)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn which(cmd: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        if dir.join(cmd).is_file() {
            return true;
        }
    }
    false
}

async fn wl_copy(text: &str) -> anyhow::Result<()> {
    // wl-copy reads from stdin; don't pass as an argument.
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    {
        use tokio::io::AsyncWriteExt;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("wl-copy stdin unavailable"))?;
        stdin.write_all(text.as_bytes()).await?;
        // close stdin so wl-copy commits
        stdin.shutdown().await?;
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("wl-copy failed with status {status}");
    }

    Ok(())
}

async fn notify(color: &str) {
    if let Err(e) = Command::new("notify-send")
        .arg(format!("Copied color {color} to clipboard"))
        .arg("--app-name")
        .arg("Color Picker")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        error!("notify-send failed: {e}");
    }
}

pub fn spawn_color_picker(delay_millis: u64) {
    tokio::spawn(async move {
        sleep(core::time::Duration::from_millis(delay_millis)).await;
        let stdout = match run_picker().await {
            Ok(s) => s,
            Err(e) => {
                error!("color picker failed: {e}");
                return;
            }
        };

        let Some(color) = extract_hex_color(&stdout) else {
            error!("hyprpicker output did not contain a hex color: {stdout:?}");
            return;
        };

        if let Err(e) = wl_copy(&color).await {
            error!("wl-copy failed: {e}");
            return;
        }

        notify(&color).await;
    });
}
