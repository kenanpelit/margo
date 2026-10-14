//! `session_end reboot|poweroff|logout|cancel` — end the session the way
//! GNOME does instead of pulling the compositor out from under clients.
//!
//! Every window is asked to close (`xdg_toplevel.close` / X11 close), so
//! browsers shut down through their normal path and record a clean exit.
//! Once the last window is gone the action runs. Windows still open after
//! `session_end_grace_ms` (an unsaved-changes dialog, a hung client) get a
//! notification naming them, with a Cancel button, and the action runs when
//! `session_end_countdown_ms` runs out — or as soon as they close.

use std::sync::{Arc, Mutex};

use smithay::reexports::calloop::{self, RegistrationToken};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEndAction {
    Reboot,
    Poweroff,
    Logout,
}

impl SessionEndAction {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "reboot" | "restart" => Some(Self::Reboot),
            "poweroff" | "shutdown" => Some(Self::Poweroff),
            "logout" | "exit" => Some(Self::Logout),
            _ => None,
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Reboot => "Restarting",
            Self::Poweroff => "Shutting down",
            Self::Logout => "Logging out",
        }
    }
}

/// An in-progress session end.
pub struct SessionEnd {
    action: SessionEndAction,
    /// Grace / countdown timer.
    timer: Option<RegistrationToken>,
    /// Receives the notification's Cancel action.
    cancel_source: Option<RegistrationToken>,
    /// Id of the countdown notification, once notify-send printed it.
    notification_id: Arc<Mutex<Option<u32>>>,
}

impl MargoState {
    /// Dispatch entry point: an action name, or `cancel`.
    pub fn session_end(&mut self, arg: &str) {
        if arg.trim().eq_ignore_ascii_case("cancel") {
            self.session_end_cancel();
            return;
        }
        let Some(action) = SessionEndAction::parse(arg) else {
            tracing::warn!(
                arg,
                "session_end: expected reboot | poweroff | logout | cancel"
            );
            return;
        };
        if let Some(current) = &mut self.session_ending {
            // Already ending: just retarget (e.g. reboot → poweroff).
            current.action = action;
            return;
        }
        tracing::info!(
            ?action,
            windows = self.clients.len(),
            "session_end: closing windows"
        );
        for client in &self.clients {
            close_window(&client.window);
        }
        self.session_ending = Some(SessionEnd {
            action,
            timer: None,
            cancel_source: None,
            notification_id: Arc::default(),
        });
        let grace = u64::from(self.config.session_end_grace_ms);
        self.arm_session_end_timer(grace, |state| state.session_end_grace_elapsed());
        self.session_end_check();
    }

    /// A window went away: finish once none are left.
    pub(crate) fn session_end_check(&mut self) {
        if self.session_ending.is_some() && self.clients.is_empty() {
            self.session_end_proceed();
        }
    }

    fn session_end_grace_elapsed(&mut self) {
        if self.clients.is_empty() {
            self.session_end_proceed();
            return;
        }
        let Some(action) = self.session_ending.as_ref().map(|s| s.action) else {
            return;
        };
        let countdown = u64::from(self.config.session_end_countdown_ms);
        let mut names: Vec<String> = self
            .clients
            .iter()
            .map(|c| {
                if c.app_id.is_empty() {
                    c.title.clone()
                } else {
                    c.app_id.clone()
                }
            })
            .collect();
        names.sort();
        names.dedup();
        let count = self.clients.len();
        let body = format!(
            "{count} window{} did not close: {}. Close or save them, or Cancel.",
            if count == 1 { "" } else { "s" },
            names.join(", ")
        );
        let title = format!("{} in {} s", action.verb(), countdown.div_ceil(1000));
        tracing::info!(
            remaining = count,
            "session_end: windows still open, counting down"
        );
        self.notify_session_end(&title, &body, countdown);
        self.arm_session_end_timer(countdown, |state| state.session_end_proceed());
    }

    /// Abort a pending session end (windows already closed stay closed).
    pub fn session_end_cancel(&mut self) {
        let Some(ending) = self.session_ending.take() else {
            return;
        };
        self.drop_session_end_sources(&ending);
        tracing::info!(action = ?ending.action, "session_end: cancelled");
        let replaces = *ending
            .notification_id
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        spawn_notification(
            "Session end cancelled",
            "Nothing was restarted or shut down.",
            3000,
            replaces,
            false,
        );
    }

    fn session_end_proceed(&mut self) {
        let Some(ending) = self.session_ending.take() else {
            return;
        };
        self.drop_session_end_sources(&ending);
        tracing::info!(action = ?ending.action, "session_end: proceeding");
        match ending.action {
            SessionEndAction::Logout => self.should_quit = true,
            SessionEndAction::Reboot => run_systemctl("reboot"),
            SessionEndAction::Poweroff => run_systemctl("poweroff"),
        }
    }

    fn drop_session_end_sources(&mut self, ending: &SessionEnd) {
        for token in [ending.timer, ending.cancel_source].into_iter().flatten() {
            self.loop_handle.remove(token);
        }
    }

    fn arm_session_end_timer(&mut self, ms: u64, then: fn(&mut MargoState)) {
        let Some(ending) = self.session_ending.as_mut() else {
            return;
        };
        if let Some(token) = ending.timer.take() {
            self.loop_handle.remove(token);
        }
        let timer = calloop::timer::Timer::from_duration(std::time::Duration::from_millis(ms));
        match self
            .loop_handle
            .insert_source(timer, move |_, _, state: &mut MargoState| {
                // Clear first: the source is dropped on return.
                if let Some(ending) = state.session_ending.as_mut() {
                    ending.timer = None;
                }
                then(state);
                calloop::timer::TimeoutAction::Drop
            }) {
            Ok(token) => {
                if let Some(ending) = self.session_ending.as_mut() {
                    ending.timer = Some(token);
                }
            }
            Err(e) => {
                tracing::warn!("session_end timer: {e}; proceeding now");
                self.session_end_proceed();
            }
        }
    }

    /// Countdown notification with a Cancel action; clicking it cancels
    /// through a calloop channel on the compositor thread.
    fn notify_session_end(&mut self, title: &str, body: &str, timeout_ms: u64) {
        let (tx, rx) = calloop::channel::channel::<()>();
        let source = self
            .loop_handle
            .insert_source(rx, |event, _, state: &mut MargoState| {
                if let calloop::channel::Event::Msg(()) = event {
                    state.session_end_cancel();
                }
            });
        let Some(ending) = self.session_ending.as_mut() else {
            return;
        };
        match source {
            Ok(token) => ending.cancel_source = Some(token),
            Err(e) => tracing::warn!("session_end cancel source: {e}"),
        }
        let id_slot = ending.notification_id.clone();
        let mut cmd = notify_command(title, body, timeout_ms, None, true);
        std::thread::spawn(move || {
            let Ok(child) = cmd.spawn() else {
                tracing::warn!("session_end: notify-send unavailable");
                return;
            };
            let Ok(out) = child.wait_with_output() else {
                return;
            };
            // `--print-id` puts the id first, the invoked action after it.
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut lines = stdout.lines();
            if let Some(id) = lines.next().and_then(|l| l.trim().parse().ok()) {
                *id_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
            }
            if lines.any(|l| l.trim() == "cancel") {
                let _ = tx.send(());
            }
        });
    }
}

/// Ask a window to close the way its close button would.
fn close_window(window: &Window) {
    match window.underlying_surface() {
        WindowSurface::Wayland(toplevel) => toplevel.send_close(),
        WindowSurface::X11(x11) => {
            let _ = x11.close();
        }
    }
}

fn notify_command(
    title: &str,
    body: &str,
    timeout_ms: u64,
    replaces: Option<u32>,
    with_cancel: bool,
) -> std::process::Command {
    let mut cmd = std::process::Command::new("notify-send");
    crate::utils::apply_child_env(&mut cmd);
    cmd.args(["--app-name=margo", "--urgency=critical", "--print-id"])
        .arg(format!("--expire-time={timeout_ms}"));
    if let Some(id) = replaces {
        cmd.arg(format!("--replace-id={id}"));
    }
    if with_cancel {
        cmd.arg("--action=cancel=Cancel");
    }
    cmd.arg(title)
        .arg(body)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    cmd
}

fn spawn_notification(
    title: &str,
    body: &str,
    timeout_ms: u64,
    replaces: Option<u32>,
    cancel: bool,
) {
    // Unit tests must not pop real desktop notifications.
    if cfg!(test) {
        return;
    }
    let mut cmd = notify_command(title, body, timeout_ms, replaces, cancel);
    std::thread::spawn(move || {
        if let Ok(child) = cmd.spawn() {
            let _ = child.wait_with_output();
        }
    });
}

/// `systemctl reboot|poweroff`, retried ignoring inhibitors (`-i`) if a
/// lingering inhibitor lock refuses it; a final failure is reported.
fn run_systemctl(verb: &'static str) {
    std::thread::spawn(move || {
        let run = |extra: &[&str]| {
            let mut cmd = std::process::Command::new("systemctl");
            crate::utils::apply_child_env(&mut cmd);
            cmd.arg(verb)
                .args(extra)
                .stdin(std::process::Stdio::null())
                .output()
        };
        let first = run(&[]);
        if first.as_ref().is_ok_and(|o| o.status.success()) {
            return;
        }
        if run(&["-i"]).as_ref().is_ok_and(|o| o.status.success()) {
            return;
        }
        let reason = match first {
            Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) => e.to_string(),
        };
        tracing::error!(verb, %reason, "session_end: systemctl failed");
        spawn_notification(
            &format!("systemctl {verb} failed"),
            &reason,
            10_000,
            None,
            false,
        );
    });
}
