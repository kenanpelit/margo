//! The Mullvad system service (`mullvad-daemon.service`): is it there,
//! running, enabled at boot — and start / enable / restart it.
//!
//! Starting a system unit needs root. mvpn asks systemd itself:
//! `systemctl start mullvad-daemon.service` from the user's session asks
//! polkit (`org.freedesktop.systemd1.manage-units`), whose agent (mshell's
//! own) shows a password prompt that names the unit. That is how the system
//! already authorises unit control, so mvpn adds no root code, no shell
//! and no free-form argument: the three verbs below are the whole surface.

use super::sys;

/// The unit the Mullvad package installs.
pub const UNIT: &str = "mullvad-daemon.service";

/// What `mvpn service <verb>` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    /// Start now and at every boot (`enable --now`).
    Enable,
    Restart,
}

impl Action {
    pub fn parse(verb: &str) -> Option<Self> {
        match verb {
            "start" => Some(Self::Start),
            "enable" => Some(Self::Enable),
            "restart" => Some(Self::Restart),
            _ => None,
        }
    }

    /// The fixed `systemctl` argv.
    pub fn argv(self) -> &'static [&'static str] {
        match self {
            Self::Start => &["start", UNIT],
            Self::Enable => &["enable", "--now", UNIT],
            Self::Restart => &["restart", UNIT],
        }
    }
}

/// Run the action through `systemctl` (polkit asks for the password).
pub fn run(action: Action) -> Result<(), String> {
    run_with("systemctl", action)
}

pub(crate) fn run_with(program: &str, action: Action) -> Result<(), String> {
    let out = sys::run(program, action.argv(), None);
    if out.success {
        return Ok(());
    }
    if !out.spawned {
        return Err("systemctl is not available".into());
    }
    Err(failure(&out.stderr))
}

/// `systemctl`'s refusal → a plain sentence.
pub fn failure(stderr: &str) -> String {
    if stderr.contains("Interactive authentication required") {
        "no password prompt is available (is a polkit agent running?)".into()
    } else if stderr.contains("Access denied") || stderr.contains("dismissed") {
        "the password prompt was cancelled or refused".into()
    } else if stderr.contains("not found") || stderr.contains("not loaded") {
        format!("{UNIT} is not installed")
    } else {
        stderr
            .lines()
            .next()
            .filter(|l| !l.trim().is_empty())
            .unwrap_or("systemctl failed")
            .to_string()
    }
}

/// `systemctl is-enabled` → `Some(false)` only for a plainly disabled
/// unit; `None` when unknown (static, masked, no systemctl, no unit).
pub fn parse_is_enabled(stdout: &str) -> Option<bool> {
    match stdout.trim() {
        "enabled" | "enabled-runtime" | "alias" => Some(true),
        "disabled" => Some(false),
        _ => None,
    }
}

pub fn boot_enabled() -> Option<bool> {
    // `is-enabled` exits 1 for "disabled": read stdout, not the status.
    parse_is_enabled(&sys::run("systemctl", &["is-enabled", UNIT], None).stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testutil::Fake;

    #[test]
    fn verbs_map_to_fixed_argv() {
        assert_eq!(
            Action::parse("start").map(Action::argv),
            Some(&["start", UNIT][..])
        );
        assert_eq!(
            Action::parse("enable").map(Action::argv),
            Some(&["enable", "--now", UNIT][..])
        );
        assert_eq!(
            Action::parse("restart").map(Action::argv),
            Some(&["restart", UNIT][..])
        );
        assert_eq!(Action::parse("stop"), None);
        assert_eq!(Action::parse("start; rm"), None);
    }

    #[test]
    fn runs_systemctl_and_explains_a_refusal() {
        let ok = Fake::new("svc-ok", 0, "", "");
        assert_eq!(run_with(ok.program(), Action::Enable), Ok(()));
        assert_eq!(ok.argv(), "enable\n--now\nmullvad-daemon.service\n");
        drop(ok);
        let denied = Fake::new(
            "svc-denied",
            1,
            "",
            "Failed to start mullvad-daemon.service: Access denied\n",
        );
        assert_eq!(
            run_with(denied.program(), Action::Start),
            Err("the password prompt was cancelled or refused".into())
        );
        drop(denied);
        assert_eq!(
            failure("Failed to restart x: Interactive authentication required."),
            "no password prompt is available (is a polkit agent running?)"
        );
        assert_eq!(
            failure(
                "Failed to start mullvad-daemon.service: Unit mullvad-daemon.service not found."
            ),
            "mullvad-daemon.service is not installed"
        );
        assert_eq!(
            run_with("/nonexistent/systemctl", Action::Start),
            Err("systemctl is not available".into())
        );
    }

    #[test]
    fn is_enabled_answers() {
        assert_eq!(parse_is_enabled("enabled\n"), Some(true));
        assert_eq!(parse_is_enabled("disabled\n"), Some(false));
        assert_eq!(parse_is_enabled("masked"), None);
        assert_eq!(parse_is_enabled("static"), None);
        assert_eq!(parse_is_enabled(""), None);
    }
}
