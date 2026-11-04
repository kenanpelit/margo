//! Structured config diagnostics.
//!
//! The parser proper (`parser.rs`) silently defaults on malformed
//! values to keep the compositor up even when the user has a broken
//! config. The validator (`validator.rs`) re-walks the same file
//! and collects structured diagnostics so the user can be told
//! exactly what is wrong and where.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
}

/// A single config issue with span info so the renderer can point
/// at the bad token niri-style:
///
/// ```text
/// error: trailing comma in `bind` value
///   --> ~/.config/margo/config.conf:689:43
///     |
/// 689 | bind = alt,Tab,overview_focus_next,
///     |                                   ^ remove the trailing comma
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiagnostic {
    pub path: PathBuf,
    /// 1-indexed line number.
    pub line: usize,
    /// 1-indexed start column of the bad token (the caret target).
    /// `0` ⇒ point at the whole line.
    pub col: usize,
    /// 1-indexed exclusive end column (used to draw the caret span).
    /// `0` ⇒ render a single `^` at `col`.
    pub end_col: usize,
    pub severity: Severity,
    /// Short stable code so machine-readable consumers can switch on
    /// it (`E001` = empty bind field, `W001` = unknown key, etc.; the full
    /// list is in `validator.rs`'s module docs). An `E…` code means margo
    /// drops or misreads the line; a `W…` code means it loads it.
    /// Owned `String` rather than `&'static str` so the struct can
    /// round-trip through serde for IPC.
    pub code: String,
    pub message: String,
    /// Raw line text — used by the formatter to render the caret
    /// underline and a snippet of the offending line.
    pub line_text: String,
}

impl ConfigDiagnostic {
    pub fn is_error(&self) -> bool {
        matches!(self.severity, Severity::Error)
    }

    /// Render as a niri-style human-readable diagnostic. Includes ANSI
    /// colour codes when `colored = true` so terminals can highlight
    /// the severity and caret.
    pub fn render(&self, colored: bool) -> String {
        let mut out = String::new();
        let (sev_label, sev_color) = match self.severity {
            Severity::Error => ("error", "\x1b[1;31m"),
            Severity::Warning => ("warning", "\x1b[1;33m"),
        };
        let bold = "\x1b[1m";
        let dim = "\x1b[2m";
        let reset = "\x1b[0m";
        let blue = "\x1b[1;34m";

        if colored {
            out.push_str(&format!(
                "{sev_color}{sev_label}[{code}]{reset}{bold}: {msg}{reset}\n",
                sev_color = sev_color,
                sev_label = sev_label,
                code = self.code,
                msg = self.message,
                bold = bold,
                reset = reset,
            ));
            out.push_str(&format!(
                "  {blue}-->{reset} {path}:{line}:{col}\n",
                blue = blue,
                reset = reset,
                path = self.path.display(),
                line = self.line,
                col = self.col.max(1),
            ));
        } else {
            out.push_str(&format!("{}[{}]: {}\n", sev_label, self.code, self.message));
            out.push_str(&format!(
                "  --> {}:{}:{}\n",
                self.path.display(),
                self.line,
                self.col.max(1)
            ));
        }

        let gutter = format!("{:>4}", self.line);
        let gutter_blank = " ".repeat(gutter.len());
        let pipe = if colored {
            format!("{blue}|{reset}", blue = blue, reset = reset)
        } else {
            "|".to_string()
        };

        out.push_str(&format!("{gutter_blank} {pipe}\n"));
        out.push_str(&format!(
            "{} {pipe} {}\n",
            if colored {
                format!(
                    "{blue}{gutter}{reset}",
                    blue = blue,
                    reset = reset,
                    gutter = gutter
                )
            } else {
                gutter.clone()
            },
            self.line_text
        ));

        // Caret line. Build using the actual byte/char position so
        // wide chars (tabs etc.) don't visually skew the caret. We
        // align in *bytes* here; the user's config is ASCII in
        // practice and this avoids pulling unicode-width as a dep.
        if self.col > 0 {
            let pad = " ".repeat(self.col.saturating_sub(1));
            let span = self.end_col.saturating_sub(self.col).max(1);
            let caret = "^".repeat(span);
            if colored {
                out.push_str(&format!(
                    "{gutter_blank} {pipe} {pad}{sev_color}{caret}{reset}\n",
                    sev_color = sev_color,
                    reset = reset,
                ));
            } else {
                out.push_str(&format!(
                    "{gutter_blank} {pipe} {pad}{caret}\n",
                    pipe = pipe,
                ));
            }
        }

        if colored {
            out.push_str(dim);
        }

        out
    }
}

/// Aggregate result returned by the validator. Convenient when
/// callers want exit-code semantics (`has_errors()` → 1).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub diagnostics: Vec<ConfigDiagnostic>,
}

impl DiagnosticReport {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(ConfigDiagnostic::is_error)
    }

    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d.severity, Severity::Warning))
    }

    pub fn errors(&self) -> impl Iterator<Item = &ConfigDiagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &ConfigDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| matches!(d.severity, Severity::Warning))
    }

    pub fn push(&mut self, d: ConfigDiagnostic) {
        self.diagnostics.push(d);
    }

    pub fn extend<I: IntoIterator<Item = ConfigDiagnostic>>(&mut self, iter: I) {
        self.diagnostics.extend(iter);
    }
}

/// What a config reload does with the validator's report on the new file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadGate {
    /// Swap in the new config.
    Apply {
        /// Record it as a known-good generation (the boot fallback's
        /// source). Only for a config without errors: a forced reload of a
        /// config with errors must not become the fallback.
        save_generation: bool,
        /// Show the config-error banner (the config still has errors).
        banner: bool,
    },
    /// Keep the running config; the message says why.
    Refuse(String),
}

/// Decide a reload. A config with errors is refused unless `force`, in
/// which case it is applied (margo's permissive parser loads what it can)
/// but neither saved as last-good nor left unflagged. A file that can't be
/// read or parsed at all is refused by the parse step regardless of `force`.
pub fn reload_gate(report: &DiagnosticReport, force: bool) -> ReloadGate {
    let errors = report.errors().count();
    if errors == 0 {
        ReloadGate::Apply {
            save_generation: true,
            banner: false,
        }
    } else if force {
        ReloadGate::Apply {
            save_generation: false,
            banner: true,
        }
    } else {
        ReloadGate::Refuse(format!(
            "config has {errors} error(s) — run `mctl check-config` for details, \
             or `mctl reload --force` to apply it anyway"
        ))
    }
}

/// Whether a `reload` dispatch argument asks for a forced reload
/// (`mctl reload --force` sends `reload_config force`; a bind can too).
pub fn reload_force_requested(arg: Option<&str>) -> bool {
    arg.is_some_and(|a| a.trim().eq_ignore_ascii_case("force"))
}

/// The desktop notification for a set of diagnostics — `(summary, body)`,
/// or `None` when there is nothing to report. Used after boot, where the
/// errors and warnings would otherwise only be found by running
/// `mctl config-errors` on a hunch.
pub fn problem_notification(diagnostics: &[ConfigDiagnostic]) -> Option<(String, String)> {
    let errors = diagnostics.iter().filter(|d| d.is_error()).count();
    let warnings = diagnostics.len() - errors;
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    match (errors, warnings) {
        (0, 0) => None,
        (0, w) => Some((
            "Margo: config has warnings".to_string(),
            format!("{w} warning{} — run `mctl config-errors`", plural(w)),
        )),
        (e, w) => Some((
            "Margo: config has errors".to_string(),
            format!(
                "{e} error{}, {w} warning{} — run `mctl config-errors`",
                plural(e),
                plural(w)
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(severity: Severity) -> ConfigDiagnostic {
        ConfigDiagnostic {
            path: PathBuf::from("/c.conf"),
            line: 1,
            col: 1,
            end_col: 2,
            severity,
            code: "X".into(),
            message: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn clean_or_warning_only_config_applies_and_is_saved() {
        let mut r = DiagnosticReport::default();
        let ok = ReloadGate::Apply {
            save_generation: true,
            banner: false,
        };
        assert_eq!(reload_gate(&r, false), ok);
        r.push(diag(Severity::Warning));
        assert_eq!(reload_gate(&r, false), ok);
        assert_eq!(reload_gate(&r, true), ok);
    }

    #[test]
    fn errors_refuse_unless_forced_and_forced_is_never_last_good() {
        let mut r = DiagnosticReport::default();
        r.push(diag(Severity::Error));
        assert!(matches!(reload_gate(&r, false), ReloadGate::Refuse(m) if m.contains("--force")));
        assert_eq!(
            reload_gate(&r, true),
            ReloadGate::Apply {
                save_generation: false,
                banner: true,
            }
        );
    }

    #[test]
    fn force_argument() {
        assert!(reload_force_requested(Some("force")));
        assert!(reload_force_requested(Some(" FORCE ")));
        assert!(!reload_force_requested(None));
        assert!(!reload_force_requested(Some("")));
        assert!(!reload_force_requested(Some("1")));
    }

    #[test]
    fn notification_counts_errors_and_warnings() {
        assert_eq!(problem_notification(&[]), None);
        let (title, body) = problem_notification(&[diag(Severity::Warning)]).unwrap();
        assert!(title.contains("warnings"), "{title}");
        assert_eq!(body, "1 warning — run `mctl config-errors`");
        let (title, body) = problem_notification(&[
            diag(Severity::Error),
            diag(Severity::Warning),
            diag(Severity::Warning),
        ])
        .unwrap();
        assert!(title.contains("errors"), "{title}");
        assert_eq!(body, "1 error, 2 warnings — run `mctl config-errors`");
    }
}
