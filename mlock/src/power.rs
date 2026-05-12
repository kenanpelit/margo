//! Power actions triggered from the lock screen via F-keys.
//!
//! All three execute through `systemctl` — logind permits an active
//! session's owner to poweroff/reboot/suspend without privileges.

use std::process::Command;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Shutdown,
    Reboot,
    Suspend,
}

impl PowerAction {
    pub fn label_tr(self) -> &'static str {
        match self {
            PowerAction::Shutdown => "Kapat",
            PowerAction::Reboot => "Yeniden Başlat",
            PowerAction::Suspend => "Uyut",
        }
    }

    fn systemctl_arg(self) -> &'static str {
        match self {
            PowerAction::Shutdown => "poweroff",
            PowerAction::Reboot => "reboot",
            PowerAction::Suspend => "suspend",
        }
    }
}

/// Fire the action — non-blocking, the screen stays locked while
/// systemd does its thing. If suspend, mlock keeps running; on resume
/// the user finishes auth normally.
pub fn execute(action: PowerAction) {
    let arg = action.systemctl_arg();
    info!(?action, "executing power action: systemctl {arg}");
    match Command::new("systemctl").arg(arg).spawn() {
        Ok(_) => {}
        Err(e) => warn!(?action, "systemctl spawn failed: {e}"),
    }
}
