use std::process::Stdio;
use tokio::process::Command;

pub const ACTIONS: [&str; 5] = ["ignore", "poweroff", "suspend", "hibernate", "lock"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogindHandlers {
    pub power_key: String,
    pub lid: String,
    pub lid_external: String,
}

impl Default for LogindHandlers {
    fn default() -> Self {
        Self {
            power_key: "poweroff".into(),
            lid: "suspend".into(),
            lid_external: "suspend".into(),
        }
    }
}

/// Parse the `Handle*` keys from logind config fragments; later fragments
/// (drop-ins) override earlier. Commented (`#`) lines are ignored.
pub(crate) fn parse_handlers(fragments: &[String]) -> LogindHandlers {
    let mut h = LogindHandlers::default();
    for frag in fragments {
        for line in frag.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim() {
                "HandlePowerKey" => h.power_key = v.trim().to_string(),
                "HandleLidSwitch" => h.lid = v.trim().to_string(),
                "HandleLidSwitchExternalPower" => h.lid_external = v.trim().to_string(),
                _ => {}
            }
        }
    }
    h
}

/// The managed drop-in body margo writes to /etc/systemd/logind.conf.d/99-margo.conf.
///
/// `HandleLidSwitchDocked` is written to the same value as `HandleLidSwitch`
/// so the lid action applies even when "docked" — i.e. when an external
/// display is connected. logind defaults `Docked` to `ignore`, which is
/// why a multi-monitor laptop wouldn't suspend on lid close even with
/// `HandleLidSwitch=suspend`.
pub(crate) fn render_dropin(h: &LogindHandlers) -> String {
    format!(
        "# Managed by margo Settings — do not edit by hand.\n[Login]\nHandlePowerKey={}\nHandleLidSwitch={}\nHandleLidSwitchExternalPower={}\nHandleLidSwitchDocked={}\n",
        h.power_key, h.lid, h.lid_external, h.lid
    )
}

const MAIN: &str = "/etc/systemd/logind.conf";
const DROPIN: &str = "/etc/systemd/logind.conf.d/99-margo.conf";

/// Read main conf + every *.conf.d/*.conf (sorted) for effective values.
pub async fn read_handlers() -> LogindHandlers {
    let mut frags = Vec::new();
    if let Ok(s) = tokio::fs::read_to_string(MAIN).await {
        frags.push(s);
    }
    if let Ok(mut rd) = tokio::fs::read_dir("/etc/systemd/logind.conf.d").await {
        let mut names = Vec::new();
        while let Ok(Some(e)) = rd.next_entry().await {
            if e.path().extension().and_then(|x| x.to_str()) == Some("conf") {
                names.push(e.path());
            }
        }
        names.sort();
        for p in names {
            if let Ok(s) = tokio::fs::read_to_string(&p).await {
                frags.push(s);
            }
        }
    }
    parse_handlers(&frags)
}

/// Write the managed drop-in via pkexec (mshell-polkit prompts). Does NOT restart
/// logind — changes apply on next login. Err(stderr) on failure/denial.
pub async fn write_dropin(h: &LogindHandlers) -> Result<(), String> {
    let body = render_dropin(h);
    let script = format!("mkdir -p /etc/systemd/logind.conf.d && cat > {DROPIN}");
    let mut child = Command::new("pkexec")
        .args(["sh", "-c", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn pkexec: {e}"))?;
    use tokio::io::AsyncWriteExt;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_handlers_with_later_override() {
        let main = "[Login]\n#HandlePowerKey=poweroff\nHandleLidSwitch=suspend\n";
        let dropin = "[Login]\nHandlePowerKey=ignore\nHandleLidSwitchExternalPower=lock\n";
        let h = parse_handlers(&[main.to_string(), dropin.to_string()]);
        assert_eq!(h.power_key, "ignore");
        assert_eq!(h.lid, "suspend");
        assert_eq!(h.lid_external, "lock");
    }
    #[test]
    fn defaults_when_unset() {
        let h = parse_handlers(&["[Login]\n".to_string()]);
        assert_eq!(h.power_key, "poweroff");
        assert_eq!(h.lid, "suspend");
        assert_eq!(h.lid_external, "suspend");
    }
    #[test]
    fn serializes_dropin() {
        let h = LogindHandlers {
            power_key: "ignore".into(),
            lid: "lock".into(),
            lid_external: "ignore".into(),
        };
        let s = render_dropin(&h);
        assert!(s.contains("[Login]"));
        assert!(s.contains("HandlePowerKey=ignore"));
        assert!(s.contains("HandleLidSwitch=lock"));
        assert!(s.contains("HandleLidSwitchExternalPower=ignore"));
        // Docked follows the base lid action so docked/multi-monitor
        // laptops honour the lid setting too.
        assert!(s.contains("HandleLidSwitchDocked=lock"));
    }
}
