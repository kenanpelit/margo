//! Selection-or-clipboard capture for the "translate what I just
//! selected/copied" trigger. No typed command — select text (primary
//! selection) OR copy it (regular clipboard), press one keybind,
//! whichever is fresher wins. Borrowed from a DMS reference plugin's
//! "capture pipeline, not a typed command" design.
//!
//! Implementation: one long-lived `wl-paste --watch` background
//! process per source, each piping into a small state file under
//! `$XDG_RUNTIME_DIR`. [`capture`] just reads both files' mtimes +
//! content at the moment it's called — no polling loop, and no
//! shared-stdout framing to parse: `wl-paste --watch` re-execs its
//! command fresh on every change, so trying to read one pipe across
//! invocations has no clean message boundary, but a file's mtime
//! gives us that boundary for free.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

fn runtime_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn clipboard_path() -> PathBuf {
    runtime_dir().join("margo-translate-clipboard.txt")
}

fn primary_path() -> PathBuf {
    runtime_dir().join("margo-translate-primary.txt")
}

/// Kept only to hold the child processes alive for mshell's lifetime;
/// never read after spawn (the OS reaps them on process exit).
struct Watchers {
    _clipboard: Child,
    _primary: Child,
}

static WATCHERS: OnceLock<Option<Watchers>> = OnceLock::new();

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn spawn_watch(path: &Path, primary: bool) -> std::io::Result<Child> {
    // Clear any stale content from a previous run so a freshness
    // check right after startup can't see old data as "just copied".
    let _ = std::fs::write(path, "");
    let mut cmd = Command::new("wl-paste");
    cmd.arg("--watch");
    if primary {
        cmd.arg("--primary");
    }
    cmd.arg("--type")
        .arg("text")
        .arg("sh")
        .arg("-c")
        .arg(format!("cat > {}", shell_quote(path)));
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
}

/// Start the two background watchers if they haven't been started
/// yet. Idempotent, and safe to call even when `wl-paste` is missing
/// (capture then just always comes back empty).
fn ensure_started() {
    WATCHERS.get_or_init(|| {
        let clipboard = spawn_watch(&clipboard_path(), false).ok()?;
        let primary = spawn_watch(&primary_path(), true).ok()?;
        Some(Watchers {
            _clipboard: clipboard,
            _primary: primary,
        })
    });
}

/// `(text, last-modified)` for `path`, or `None` if missing/empty/unreadable.
fn read_state(path: &Path) -> Option<(String, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some((text.to_string(), modified))
    }
}

fn within(modified: SystemTime, max_age: Duration) -> bool {
    modified.elapsed().map(|e| e <= max_age).unwrap_or(false)
}

/// Capture whichever of (primary selection, clipboard) is fresher,
/// each within its own max-age window. `suppress` is the text *we*
/// last wrote back to the clipboard ourselves (a just-copied
/// translation) — if the winning candidate matches it exactly, this
/// returns `None` instead, so re-pressing the keybind right after a
/// translation doesn't immediately re-translate our own output.
pub fn capture(
    selection_max_age: Duration,
    clipboard_max_age: Duration,
    suppress: Option<&str>,
) -> Option<String> {
    ensure_started();

    let primary = read_state(&primary_path()).filter(|(_, m)| within(*m, selection_max_age));
    let clipboard = read_state(&clipboard_path()).filter(|(_, m)| within(*m, clipboard_max_age));

    let winner = match (primary, clipboard) {
        (Some((pt, pm)), Some((ct, cm))) => Some(if pm >= cm { pt } else { ct }),
        (Some((pt, _)), None) => Some(pt),
        (None, Some((ct, _))) => Some(ct),
        (None, None) => None,
    };

    winner.filter(|t| Some(t.as_str()) != suppress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_window() {
        let now = SystemTime::now();
        assert!(within(now, Duration::from_secs(60)));
        let old = now - Duration::from_secs(120);
        assert!(!within(old, Duration::from_secs(60)));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        let p = PathBuf::from("/tmp/a'b.txt");
        assert_eq!(shell_quote(&p), "'/tmp/a'\\''b.txt'");
    }
}
