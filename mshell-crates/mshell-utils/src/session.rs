//! Session actions — the power menu's Lock / Logout / Suspend /
//! Reboot / Shutdown. Each action runs the command configured in
//! the `[session]` config block, or a built-in default when that
//! field is left empty.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, SessionStoreFields};
use reactive_graph::traits::GetUntracked;
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    Lock,
    Logout,
    Suspend,
    Reboot,
    Shutdown,
}

impl SessionAction {
    /// Every action, in menu display order.
    pub const ALL: [SessionAction; 5] = [
        SessionAction::Lock,
        SessionAction::Logout,
        SessionAction::Suspend,
        SessionAction::Reboot,
        SessionAction::Shutdown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SessionAction::Lock => "Lock",
            SessionAction::Logout => "Logout",
            SessionAction::Suspend => "Suspend",
            SessionAction::Reboot => "Reboot",
            SessionAction::Shutdown => "Shutdown",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SessionAction::Lock => "system-lock-screen-symbolic",
            SessionAction::Logout => "system-log-out-symbolic",
            SessionAction::Suspend => "weather-clear-night-symbolic",
            SessionAction::Reboot => "system-reboot-symbolic",
            SessionAction::Shutdown => "system-shutdown-symbolic",
        }
    }

    /// CSS state class for the menu button.
    pub fn css_class(self) -> &'static str {
        match self {
            SessionAction::Lock => "session-lock",
            SessionAction::Logout => "session-logout",
            SessionAction::Suspend => "session-suspend",
            SessionAction::Reboot => "session-reboot",
            SessionAction::Shutdown => "session-shutdown",
        }
    }

    /// Parse the lowercase CLI / IPC token.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "lock" => Some(SessionAction::Lock),
            "logout" => Some(SessionAction::Logout),
            "suspend" => Some(SessionAction::Suspend),
            "reboot" => Some(SessionAction::Reboot),
            "shutdown" => Some(SessionAction::Shutdown),
            _ => None,
        }
    }

    /// The `[session]` override command for this action — empty
    /// string means "use the built-in".
    fn configured_command(self) -> String {
        let session = config_manager().config().session();
        match self {
            SessionAction::Lock => session.lock_command().get_untracked(),
            SessionAction::Logout => session.logout_command().get_untracked(),
            SessionAction::Suspend => session.suspend_command().get_untracked(),
            SessionAction::Reboot => session.reboot_command().get_untracked(),
            SessionAction::Shutdown => session.shutdown_command().get_untracked(),
        }
    }
}

/// Run `action`: the configured override command if `[session]`
/// sets one, otherwise the built-in default.
///
/// Must be called from the GTK main thread — the built-in `Lock`
/// path touches the thread-local session-lock instance.
pub fn run_session_action(action: SessionAction) {
    let cmd = action.configured_command();
    let cmd = cmd.trim();
    if !cmd.is_empty() {
        spawn_sh(cmd.to_string());
        return;
    }
    match action {
        SessionAction::Lock => {
            mshell_session::session_lock::lock_session();
        }
        SessionAction::Suspend => spawn(&["systemctl", "suspend"]),
        // margo closes every window first so apps (browsers) exit cleanly;
        // outside margo (mctl unreachable) fall back to systemd directly.
        SessionAction::Logout => {
            session_end("logout", &["systemctl", "--user", "exit"]);
        }
        SessionAction::Reboot => session_end("reboot", &["systemctl", "reboot"]),
        SessionAction::Shutdown => session_end("poweroff", &["systemctl", "poweroff"]),
    }
}

/// `mctl dispatch session_end <verb>`, or `fallback` when margo can't be
/// reached.
fn session_end(verb: &'static str, fallback: &[&str]) {
    let fallback: Vec<String> = fallback.iter().map(|s| s.to_string()).collect();
    tokio::spawn(async move {
        let via_margo = Command::new("mctl")
            .args(["dispatch", "session_end", verb])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success());
        if via_margo {
            return;
        }
        tracing::warn!(
            verb,
            "session_end via margo failed; using systemctl directly"
        );
        let argv: Vec<&str> = fallback.iter().map(String::as_str).collect();
        spawn(&argv);
    });
}

fn spawn(argv: &[&str]) {
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    tokio::spawn(async move {
        let Some((cmd, args)) = argv.split_first() else {
            return;
        };
        match Command::new(cmd)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
        {
            Ok(out) if out.status.success() => {}
            Ok(out) => tracing::error!(
                status = %out.status,
                cmd = %cmd,
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "session action failed",
            ),
            Err(e) => tracing::error!(error = %e, cmd = %cmd, "session action spawn failed"),
        }
    });
}

fn spawn_sh(command: String) {
    tokio::spawn(async move {
        match Command::new("sh")
            .args(["-c", &command])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
        {
            Ok(out) if out.status.success() => {}
            Ok(out) => tracing::error!(
                status = %out.status,
                command = %command,
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "session command failed",
            ),
            Err(e) => {
                tracing::error!(error = %e, command = %command, "session command spawn failed")
            }
        }
    });
}
