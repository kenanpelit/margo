//! `mshellctl translate` — the headless "select-or-copy, then press
//! one keybind" trigger. Deliberately does NOT open the GTK menu
//! panel (`mshellctl menu translate` is the separate manual-entry
//! surface) — this is meant to be bound to a margo gesture/keybind
//! and run silently: capture → privacy-screen → translate → copy the
//! result to the clipboard → toast.
//!
//! ```text
//! mshellctl translate selection   # whichever of selection/clipboard is fresher
//! mshellctl translate clipboard   # same, but ignore the primary selection
//! mshellctl translate text "<str>"  # skip capture, translate this string
//! ```

use crate::bus::bus_command_with_arg;
use clap::Subcommand;
use mshell_translate::{capture, config, looks_sensitive};
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub enum TranslateCommands {
    /// Capture whichever of (primary selection, clipboard) was most
    /// recently set, translate it, copy the result, toast it.
    Selection,
    /// Same, but only ever look at the regular clipboard.
    Clipboard,
    /// Skip capture — translate this string directly.
    Text { text: String },
}

pub async fn execute(command: TranslateCommands) -> anyhow::Result<()> {
    let settings = config::load();

    let text = match command {
        TranslateCommands::Text { text } => Some(text),
        TranslateCommands::Selection => capture::capture(
            Duration::from_secs(settings.selection_max_age_secs as u64),
            Duration::from_secs(settings.clipboard_max_age_secs as u64),
            capture::last_copied().as_deref(),
        ),
        TranslateCommands::Clipboard => capture::capture(
            Duration::from_secs(0),
            Duration::from_secs(settings.clipboard_max_age_secs as u64),
            capture::last_copied().as_deref(),
        ),
    };

    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        toast(
            "Translate",
            "Nothing to translate — select or copy some text first.",
            "warn",
        )
        .await;
        return Ok(());
    };

    if let Some(reason) = looks_sensitive(&text) {
        toast("Translate", &format!("Not sent — {reason}."), "warn").await;
        return Ok(());
    }

    // `config::resolved()` reads the DeepL key from the keyring, which on
    // this project's async-secret-service backend blocks by spinning up
    // its own nested tokio runtime — calling it directly from this
    // already-async fn panics ("Cannot start a runtime from within a
    // runtime"). `translate()`'s own blocking network call belongs off
    // the async task for the same reason `mshell-frame`'s menu widget
    // runs it via spawn_blocking; do the same here.
    let result = tokio::task::spawn_blocking(move || {
        let cfg = config::resolved();
        mshell_translate::translate(&cfg, &text)
    })
    .await
    .unwrap_or_else(|_| Err("worker panicked".into()));

    match result {
        Ok(result) => {
            copy_to_clipboard(&result.translated);
            capture::mark_copied(&result.translated);
            let title = match &result.detected_source {
                Some(src) => format!("Translate ({src} → {})", settings.target_lang),
                None => format!("Translate (→ {})", settings.target_lang),
            };
            toast(&title, &result.translated, "calm").await;
        }
        Err(e) => {
            toast("Translate failed", &e, "danger").await;
        }
    }

    Ok(())
}

fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Best-effort toast via the shell's own transient-toast surface
/// (`mshellctl toast`'s own IPC path) — never fails the command if
/// mshell isn't running or the bus call errors.
async fn toast(title: &str, body: &str, severity: &str) {
    let _ = bus_command_with_arg(
        "Toast",
        &(
            title.to_string(),
            body.to_string(),
            String::new(),
            severity.to_string(),
        ),
    )
    .await;
}
