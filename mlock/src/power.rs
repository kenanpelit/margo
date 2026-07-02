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
    pub fn label(self) -> &'static str {
        match self {
            PowerAction::Shutdown => "Shut down",
            PowerAction::Reboot => "Restart",
            PowerAction::Suspend => "Suspend",
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

/// Whether a second F-key press confirms a pending power action.
///
/// A confirmation is valid only when the pending action matches `action`
/// **and** its deadline is still in the future (`deadline > now`). Split out
/// from `MlockState::power_request` so the double-press-within-3 s grace
/// window is unit-testable without a live Wayland state.
pub(crate) fn is_confirmed(
    pending: Option<(PowerAction, std::time::Instant)>,
    action: PowerAction,
    now: std::time::Instant,
) -> bool {
    matches!(pending, Some((p, deadline)) if p == action && deadline > now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn labels_are_user_facing_and_distinct() {
        assert_eq!(PowerAction::Shutdown.label(), "Shut down");
        assert_eq!(PowerAction::Reboot.label(), "Restart");
        assert_eq!(PowerAction::Suspend.label(), "Suspend");
    }

    #[test]
    fn systemctl_args_map_to_the_right_verb() {
        // A wrong mapping here would suspend when the user asked to reboot,
        // etc. — lock these down.
        assert_eq!(PowerAction::Shutdown.systemctl_arg(), "poweroff");
        assert_eq!(PowerAction::Reboot.systemctl_arg(), "reboot");
        assert_eq!(PowerAction::Suspend.systemctl_arg(), "suspend");
    }

    #[test]
    fn no_pending_action_is_never_confirmed() {
        assert!(!is_confirmed(None, PowerAction::Shutdown, Instant::now()));
    }

    #[test]
    fn matching_action_within_window_confirms() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(3);
        assert!(is_confirmed(
            Some((PowerAction::Reboot, deadline)),
            PowerAction::Reboot,
            now
        ));
    }

    #[test]
    fn mismatched_action_does_not_confirm() {
        // Pressing F1 (shutdown) then F2 (reboot) must NOT trigger — each
        // action needs its own double-press.
        let now = Instant::now();
        let deadline = now + Duration::from_secs(3);
        assert!(!is_confirmed(
            Some((PowerAction::Shutdown, deadline)),
            PowerAction::Reboot,
            now
        ));
    }

    #[test]
    fn expired_window_does_not_confirm() {
        // Deadline already passed (>3 s elapsed) → the second press starts a
        // fresh window instead of firing.
        let now = Instant::now();
        let deadline = now - Duration::from_millis(1);
        assert!(!is_confirmed(
            Some((PowerAction::Suspend, deadline)),
            PowerAction::Suspend,
            now
        ));
    }
}
