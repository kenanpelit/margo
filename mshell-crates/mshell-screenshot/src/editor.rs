//! Annotation-editor launcher (satty / swappy / gimp / krita).
//!
//! Shared between widget pipelines (the in-shell screenshot menu)
//! and any future CLI invocations. Keeps the editor probe + spawn
//! logic in one place so swappy↔satty arg quirks aren't duplicated.

use std::path::Path;
use std::process::Command;

use super::{Result, ScreenshotError};

/// Decide which editor to invoke. Honours `SCREENSHOT_EDITOR` env
/// override (only if the named binary is actually on `$PATH`), then
/// falls back to the standard preference chain. Returns `None` when
/// no editor is available — callers should treat that as "skip the
/// edit step" rather than an error.
pub fn pick_editor() -> Option<String> {
    if let Ok(forced) = std::env::var("SCREENSHOT_EDITOR")
        && !forced.is_empty()
        && which(&forced)
    {
        return Some(forced);
    }
    for cand in ["satty", "swappy", "gimp", "krita"] {
        if which(cand) {
            return Some(cand.to_string());
        }
    }
    None
}

/// True if any editor is on `$PATH` — UI uses this to grey out the
/// "Edit" option when no annotation tool is installed.
pub fn editor_available() -> bool {
    pick_editor().is_some()
}

/// Launch the named editor on `input`. swappy/satty support a
/// `--output-filename` style switch and exit when the user saves;
/// for those we wait synchronously and return whether the editor
/// actually wrote `output`. gimp/krita are fire-and-forget (the
/// user owns their own save dialog) so we spawn detached and
/// report "edited path = input" so the caller can still clipboard
/// the unedited capture.
///
/// Returns Ok(true) when the editor wrote `output` to disk, Ok(false)
/// when the editor exited without saving (user closed the window
/// without confirming) — callers usually fall back to the original
/// capture in that case.
pub fn launch_editor_blocking(input: &Path, output: &Path, editor: &str) -> Result<bool> {
    match editor {
        "satty" => {
            let status = Command::new("satty")
                .arg("--filename")
                .arg(input)
                .arg("--output-filename")
                .arg(output)
                .status()
                .map_err(|e| ScreenshotError::CaptureFailed(format!("spawn satty: {e}")))?;
            if !status.success() {
                return Err(ScreenshotError::CaptureFailed(format!(
                    "satty exited with status {status}"
                )));
            }
            Ok(output.exists() && output.metadata().map(|m| m.len()).unwrap_or(0) > 0)
        }
        "swappy" => {
            let status = Command::new("swappy")
                .arg("-f")
                .arg(input)
                .arg("-o")
                .arg(output)
                .status()
                .map_err(|e| ScreenshotError::CaptureFailed(format!("spawn swappy: {e}")))?;
            if !status.success() {
                return Err(ScreenshotError::CaptureFailed(format!(
                    "swappy exited with status {status}"
                )));
            }
            Ok(output.exists() && output.metadata().map(|m| m.len()).unwrap_or(0) > 0)
        }
        "gimp" | "krita" => {
            // Fire-and-forget — the editor owns its own save flow.
            // We don't get a callback when the user saves; treat
            // the input file as the canonical capture and let the
            // user save manually from inside the editor.
            Command::new(editor)
                .arg(input)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| ScreenshotError::CaptureFailed(format!("spawn {editor}: {e}")))?;
            Ok(false)
        }
        other => Err(ScreenshotError::CaptureFailed(format!(
            "unknown editor {other:?}"
        ))),
    }
}

fn which(cmd: &str) -> bool {
    let path = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(cmd);
        if candidate.is_file() {
            // good enough — caller will error if exec fails
            return true;
        }
    }
    false
}
