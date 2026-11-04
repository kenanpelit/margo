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

impl std::str::FromStr for ToastSeverity {
    type Err = String;

    /// Parse a CLI / D-Bus severity name (`calm`, `warn`, `danger`,
    /// `positive`, plus a few synonyms). An empty string is `calm`; anything
    /// else unknown is an error rather than a silent fallback.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "calm" | "info" => Ok(ToastSeverity::Calm),
            "warn" | "warning" => Ok(ToastSeverity::Warn),
            "danger" | "error" | "critical" => Ok(ToastSeverity::Danger),
            "positive" | "success" | "ok" => Ok(ToastSeverity::Positive),
            other => Err(format!(
                "unknown toast severity '{other}' (calm | warn | danger | positive)"
            )),
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

/// Paths whose failed save was already toasted (cleared by a good save).
static SIDE_SAVE_TOASTED: std::sync::Mutex<Vec<std::path::PathBuf>> =
    std::sync::Mutex::new(Vec::new());

/// Report the outcome of a side-file save (`ai.json`, `translate.json`,
/// `mtune.toml`, …): a failure is logged every time and toasted once per
/// file until a save to it succeeds again, so a slider drag over a broken
/// file doesn't stack toasts.
pub fn report_side_save(path: &std::path::Path, res: Result<(), margo_fs::side_file::SaveError>) {
    let mut toasted = SIDE_SAVE_TOASTED.lock().unwrap_or_else(|e| e.into_inner());
    match res {
        Ok(()) => toasted.retain(|p| p != path),
        Err(e) => {
            tracing::warn!(error = %e, "settings not saved");
            if toasted.iter().any(|p| p == path) {
                return;
            }
            let delivered = try_push_toast(ToastEvent {
                icon: "dialog-warning-symbolic".to_string(),
                title: "Settings not saved".to_string(),
                body: Some(e.to_string()),
                severity: ToastSeverity::Warn,
            });
            if delivered {
                toasted.push(path.to_path_buf());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_save_failure_toasts_once_per_file_until_it_works() {
        let mut rx = subscribe();
        let path = std::path::Path::new("/nonexistent/side-save-test.json");
        let fail = || {
            Err(margo_fs::side_file::SaveError::Unparseable {
                path: path.to_path_buf(),
                error: "oops".into(),
            })
        };
        report_side_save(path, fail());
        report_side_save(path, fail());
        let mut got = 0;
        while let Ok(ev) = rx.try_recv() {
            if ev
                .body
                .as_deref()
                .is_some_and(|b| b.contains("side-save-test"))
            {
                got += 1;
            }
        }
        assert_eq!(got, 1, "one toast for repeated failures");
        report_side_save(path, Ok(()));
        report_side_save(path, fail());
        assert!(
            rx.try_recv().is_ok(),
            "a new failure after a good save toasts again"
        );
    }

    #[test]
    fn severity_parses_known_names_and_rejects_typos() {
        assert_eq!("".parse(), Ok(ToastSeverity::Calm));
        assert_eq!("Warning".parse(), Ok(ToastSeverity::Warn));
        assert_eq!("error".parse(), Ok(ToastSeverity::Danger));
        assert_eq!("success".parse(), Ok(ToastSeverity::Positive));
        assert!("wran".parse::<ToastSeverity>().is_err());
    }

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
