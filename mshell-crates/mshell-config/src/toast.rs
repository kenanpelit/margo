//! The in-process toast bus.
//!
//! A toast announces a state change in a small corner card (rendered by
//! `mshell-osd`'s per-output `ToastSurfaceModel`). The bus lives here, in the
//! lowest shell crate, so every layer can raise one in-process — the config
//! manager, `mshell-services`, the frame, the D-Bus service — without depending
//! on the GTK surface crate or shelling out to `mshellctl toast`.

use std::sync::OnceLock;
use tokio::sync::broadcast;

/// Severity tint for a toast — maps to the DESIGN.md §2 calm/warn/danger
/// ladder (plus the stable `positive` green) via a CSS class on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastSeverity {
    Calm,
    Warn,
    Danger,
    Positive,
}

impl ToastSeverity {
    /// The CSS class the toast card carries for this severity.
    pub fn class(self) -> &'static str {
        match self {
            ToastSeverity::Calm => "calm",
            ToastSeverity::Warn => "warn",
            ToastSeverity::Danger => "danger",
            ToastSeverity::Positive => "positive",
        }
    }
}

/// One toast: a symbolic icon, a title, an optional body line, and a severity.
#[derive(Debug, Clone)]
pub struct ToastEvent {
    pub icon: String,
    pub title: String,
    pub body: Option<String>,
    pub severity: ToastSeverity,
}

static TOAST_BUS: OnceLock<broadcast::Sender<ToastEvent>> = OnceLock::new();

fn bus() -> &'static broadcast::Sender<ToastEvent> {
    TOAST_BUS.get_or_init(|| broadcast::channel(16).0)
}

/// Broadcast a toast to every per-output toast surface. A no-op before the
/// surfaces exist or after they're all gone.
pub fn push_toast(event: ToastEvent) {
    try_push_toast(event);
}

/// [`push_toast`], reporting delivery: `false` when nobody is listening and the
/// toast was dropped — a caller that must be seen once can retry later.
pub fn try_push_toast(event: ToastEvent) -> bool {
    bus().send(event).is_ok()
}

/// Subscribe a toast surface to the bus.
pub fn subscribe() -> broadcast::Receiver<ToastEvent> {
    bus().subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_reports_delivery_and_reaches_subscribers() {
        let ev = || ToastEvent {
            icon: "dialog-warning-symbolic".into(),
            title: "t".into(),
            body: None,
            severity: ToastSeverity::Warn,
        };
        let mut rx = subscribe();
        assert!(try_push_toast(ev()), "a live subscriber receives the toast");
        let got = rx.try_recv().expect("delivered");
        assert_eq!(got.severity, ToastSeverity::Warn);
    }
}
