//! Typing + clipboard options for the `pass` provider, and the autotype
//! login sequence.
//!
//! Split from `pass.rs` so the sequencing — the part that decides which
//! keys reach the focused window — is a pure, tested function
//! ([`autotype_steps`]) and the side-effecting runner ([`run_steps`]) stays
//! small.
//!
//! Secrets never reach argv or a log: text steps are piped to
//! `wtype -` on stdin, only key names (`Tab`, `Return`) are passed as
//! arguments, and [`TypeStep`]'s `Debug` redacts the text.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

/// User-tunable behaviour (Settings → Launcher → Password store). The
/// defaults are the provider's behaviour before these became settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassOptions {
    /// Seconds a copied value stays on the clipboard.
    pub clip_timeout_secs: u32,
    /// Wait after the launcher closes before typing.
    pub type_delay_ms: u32,
    /// Delay between keystrokes (`wtype -d`); 0 = none.
    pub key_delay_ms: u32,
    /// Separator key between username and password.
    pub separator: Separator,
    /// Press Enter after the last value.
    pub submit: bool,
}

impl Default for PassOptions {
    fn default() -> Self {
        Self {
            clip_timeout_secs: 45,
            type_delay_ms: 250,
            key_delay_ms: 0,
            separator: Separator::Tab,
            submit: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    Tab,
    Enter,
}

impl Separator {
    fn key(self) -> &'static str {
        match self {
            Separator::Tab => "Tab",
            Separator::Enter => "Return",
        }
    }
}

/// One thing to send to the focused window.
#[derive(Clone, PartialEq, Eq)]
pub enum TypeStep {
    /// Text, typed via stdin.
    Text(String),
    /// A named key (`Tab`, `Return`).
    Key(&'static str),
}

impl std::fmt::Debug for TypeStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeStep::Text(t) => write!(f, "Text(<{} chars>)", t.chars().count()),
            TypeStep::Key(k) => write!(f, "Key({k})"),
        }
    }
}

/// The autotype login: username, separator, password, then — for an
/// OTP entry — Enter + the current code; finally Enter to submit when
/// `submit` is on. (The Enter before the OTP always fires: the code
/// field only appears once the password is submitted.)
pub fn autotype_steps(
    username: &str,
    password: &str,
    otp: Option<&str>,
    opts: &PassOptions,
) -> Vec<TypeStep> {
    let mut steps = vec![
        TypeStep::Text(username.to_string()),
        TypeStep::Key(opts.separator.key()),
        TypeStep::Text(password.to_string()),
    ];
    if let Some(code) = otp {
        steps.push(TypeStep::Key("Return"));
        steps.push(TypeStep::Text(code.to_string()));
    }
    if opts.submit {
        steps.push(TypeStep::Key("Return"));
    }
    steps
}

/// `wtype` argv (without the binary) for one step. Text is read from
/// stdin (`-`), never placed in argv.
pub fn wtype_args(step: &TypeStep, key_delay_ms: u32) -> Vec<String> {
    match step {
        TypeStep::Text(_) => {
            let mut args = Vec::new();
            if key_delay_ms > 0 {
                args.push("-d".into());
                args.push(key_delay_ms.to_string());
            }
            args.push("-".into());
            args
        }
        TypeStep::Key(k) => vec!["-k".into(), (*k).into()],
    }
}

/// Send `steps` to the focused window, after the type delay. Blocking —
/// worker threads only.
pub fn run_steps(steps: &[TypeStep], opts: &PassOptions) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(u64::from(opts.type_delay_ms)));
    for step in steps {
        let mut child = Command::new("wtype")
            .args(wtype_args(step, opts.key_delay_ms))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("wtype: {e}"))?;
        if let (TypeStep::Text(text), Some(mut stdin)) = (step, child.stdin.take()) {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| format!("wtype: {e}"))?;
        }
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return Err("wtype failed".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_sequence_tab_then_submit() {
        let steps = autotype_steps("me@x", "pw", None, &PassOptions::default());
        assert_eq!(
            steps,
            vec![
                TypeStep::Text("me@x".into()),
                TypeStep::Key("Tab"),
                TypeStep::Text("pw".into()),
                TypeStep::Key("Return"),
            ]
        );
    }

    #[test]
    fn otp_follows_an_enter_and_submit_can_be_off() {
        let opts = PassOptions {
            separator: Separator::Enter,
            submit: false,
            ..PassOptions::default()
        };
        let steps = autotype_steps("u", "p", Some("123456"), &opts);
        assert_eq!(
            steps,
            vec![
                TypeStep::Text("u".into()),
                TypeStep::Key("Return"),
                TypeStep::Text("p".into()),
                TypeStep::Key("Return"),
                TypeStep::Text("123456".into()),
            ]
        );
    }

    #[test]
    fn secrets_go_through_stdin_not_argv() {
        let text = TypeStep::Text("s3cr3t".into());
        assert_eq!(wtype_args(&text, 0), ["-"]);
        assert_eq!(wtype_args(&text, 12), ["-d", "12", "-"]);
        assert_eq!(wtype_args(&TypeStep::Key("Tab"), 12), ["-k", "Tab"]);
        let all = [wtype_args(&text, 0), wtype_args(&text, 5)].concat();
        assert!(!all.iter().any(|a| a.contains("s3cr3t")));
    }

    #[test]
    fn debug_never_prints_the_text() {
        let steps = autotype_steps("user", "hunter2", Some("987654"), &PassOptions::default());
        let dbg = format!("{steps:?}");
        assert!(!dbg.contains("hunter2") && !dbg.contains("987654") && !dbg.contains("user"));
        assert!(dbg.contains("Key(Tab)"));
    }
}
