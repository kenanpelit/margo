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

/// The privileged argv writing the drop-in: the root helper's fixed
/// `logind-dropin` verb (fixed path, body on stdin, re-validated as
/// root), never a shell.
pub(crate) fn dropin_argv() -> Vec<String> {
    mshell_privileged::lossy(&mshell_privileged::Request::LogindDropin.argv())
}

/// The helper's own check, run before the prompt: a value read back
/// from a hand-edited logind.conf that root would refuse fails here.
pub(crate) fn check_dropin(body: &str) -> Result<(), String> {
    mshell_privileged::validate::logind_dropin(body.as_bytes()).map_err(|e| e.message().to_string())
}

/// Write the managed drop-in
/// ([`mshell_privileged::validate::LOGIND_DROPIN`]) as root through
/// `mshell-privileged logind-dropin` (silent `sudo -n` when available,
/// else the polkit agent — see [`crate::sys::privileged`]). Does NOT
/// restart logind — changes apply on next login. Err on failure/denial.
pub async fn write_dropin(h: &LogindHandlers) -> Result<(), String> {
    let body = render_dropin(h);
    check_dropin(&body)?;
    if !mshell_privileged::installed() {
        return Err(mshell_privileged::MISSING.to_string());
    }
    let argv = dropin_argv();
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    crate::sys::privileged::run_with_stdin(&args, body.as_bytes()).await
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
    fn the_dropin_goes_through_the_helper_and_passes_its_check() {
        assert_eq!(dropin_argv(), [mshell_privileged::HELPER, "logind-dropin"]);
        for power_key in ACTIONS {
            for lid in ACTIONS {
                for lid_external in ACTIONS {
                    let h = LogindHandlers {
                        power_key: power_key.into(),
                        lid: lid.into(),
                        lid_external: lid_external.into(),
                    };
                    let body = render_dropin(&h);
                    assert_eq!(
                        mshell_privileged::validate::logind_dropin(body.as_bytes()),
                        Ok(()),
                        "{body}"
                    );
                }
            }
        }
        // A value read back from a hand-edited logind.conf that root
        // would refuse is refused before the prompt.
        let h = LogindHandlers {
            power_key: "ignore\nKillUserProcesses=no".into(),
            ..LogindHandlers::default()
        };
        assert!(check_dropin(&render_dropin(&h)).is_err());
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
