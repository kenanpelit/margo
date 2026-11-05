//! Desktop side effects the shell fires and forgets: a notification
//! ([`notify`]) and "open this URL / file / folder in the user's default
//! app" ([`open`]). One path for each, so every notification carries an
//! app name, icon and urgency the same way and nothing forks
//! `notify-send` (which may not even be installed).

use std::ffi::OsStr;
use std::io;

use crate::SpawnReaped;

/// Notification urgency (freedesktop spec levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// One desktop notification. Build with [`Notice::new`] and the setters,
/// then hand it to [`notify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub app: String,
    pub summary: String,
    pub body: String,
    pub icon: Option<String>,
    pub urgency: Urgency,
    /// Expire after this many ms; `None` = the server's default.
    pub timeout_ms: Option<u32>,
    /// Replace the previous notification with the same tag instead of
    /// stacking (`x-canonical-private-synchronous`).
    pub replace_tag: Option<String>,
    pub category: Option<String>,
}

impl Notice {
    pub fn new(
        app: impl Into<String>,
        summary: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            app: app.into(),
            summary: summary.into(),
            body: body.into(),
            icon: None,
            urgency: Urgency::Normal,
            timeout_ms: None,
            replace_tag: None,
            category: None,
        }
    }

    /// An icon name or an image path.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }

    pub fn timeout_ms(mut self, ms: u32) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    pub fn replace(mut self, tag: impl Into<String>) -> Self {
        self.replace_tag = Some(tag.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// The D-Bus notification this notice sends.
    pub fn to_notification(&self) -> notify_rust::Notification {
        let mut n = notify_rust::Notification::new();
        n.appname(&self.app)
            .summary(&self.summary)
            .body(&self.body)
            .urgency(match self.urgency {
                Urgency::Low => notify_rust::Urgency::Low,
                Urgency::Normal => notify_rust::Urgency::Normal,
                Urgency::Critical => notify_rust::Urgency::Critical,
            });
        if let Some(icon) = &self.icon {
            n.icon(icon);
        }
        if let Some(ms) = self.timeout_ms {
            n.timeout(notify_rust::Timeout::Milliseconds(ms));
        }
        if let Some(tag) = &self.replace_tag {
            n.hint(notify_rust::Hint::Custom(
                "x-canonical-private-synchronous".into(),
                tag.clone(),
            ));
        }
        if let Some(category) = &self.category {
            n.hint(notify_rust::Hint::Category(category.clone()));
        }
        n
    }
}

/// Show `notice`. The D-Bus call blocks, so it runs on a short-lived
/// thread: safe to call from the GTK main thread or an async task. A
/// missing or broken notification daemon is logged, never fatal.
pub fn notify(notice: Notice) {
    // Unit tests exercise code that notifies; don't pop real
    // notifications in the developer's session.
    if cfg!(test) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("notify".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            if let Err(err) = notice.to_notification().show() {
                tracing::warn!(?err, app = %notice.app, summary = %notice.summary, "notification failed");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(%err, "notification: could not start thread");
    }
}

/// Open `target` (a URL, file or folder) in the user's default app via
/// `xdg-open`, in its own scope ([`crate::outliving_shell`]) so a browser
/// or file manager it starts survives an mshell restart. Returns once
/// `xdg-open` is spawned (it is reaped in the background).
pub fn open(target: impl AsRef<OsStr>) -> io::Result<u32> {
    crate::outliving_shell("xdg-open", [target.as_ref()], "open").spawn_reaped()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_carries_app_icon_urgency_and_hints() {
        let n = Notice::new("System Updates", "Done", "12 packages")
            .icon("software-update-available-symbolic")
            .urgency(Urgency::Critical)
            .timeout_ms(3000)
            .replace("mshell-media")
            .category("im.received")
            .to_notification();
        assert_eq!(n.appname, "System Updates");
        assert_eq!(n.summary, "Done");
        assert_eq!(n.body, "12 packages");
        assert_eq!(n.icon, "software-update-available-symbolic");
        assert_eq!(n.timeout, notify_rust::Timeout::Milliseconds(3000));
        assert!(
            n.hints
                .contains(&notify_rust::Hint::Urgency(notify_rust::Urgency::Critical))
        );
        // (`Hint::Custom` is stored outside the public `hints` set.)
        assert!(
            n.hints
                .contains(&notify_rust::Hint::Category("im.received".into()))
        );
    }

    #[test]
    fn defaults_are_normal_and_server_timed() {
        let n = Notice::new("mshell", "Hi", "").to_notification();
        assert_eq!(n.timeout, notify_rust::Timeout::Default);
        assert!(
            n.hints
                .contains(&notify_rust::Hint::Urgency(notify_rust::Urgency::Normal))
        );
    }
}
