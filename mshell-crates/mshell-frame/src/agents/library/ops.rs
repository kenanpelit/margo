//! The Sessions page's changes to saved sessions: rename, delete, repair.
//! Blocking file work; the menu runs it off the GTK thread after the
//! user confirmed. Every path goes through [`super::guard`]; running
//! sessions are refused (the caller passes a fresh check), locked ones
//! are never deleted.

use super::encode::{MAX_NAME, encode_cwd, names_match};
use super::guard::{account_dir, real_dir_in, remove_entry, rename_noreplace, transcript};
use serde::Serialize;
use std::path::Path;

/// Longest title accepted (characters).
pub(crate) const TITLE_MAX: usize = 120;

/// The title as it will be stored: control characters (line breaks)
/// become spaces, outer whitespace goes; empty or over
/// [`TITLE_MAX`] is refused.
pub(crate) fn clean_title(title: &str) -> Result<String, String> {
    let t: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let t = t
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if t.is_empty() {
        return Err("The name is empty".into());
    }
    if t.chars().count() > TITLE_MAX {
        return Err(format!("Names are at most {TITLE_MAX} characters"));
    }
    Ok(t)
}

/// The record `/rename` appends, field order as Claude Code writes it.
#[derive(Serialize)]
struct CustomTitle<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "customTitle")]
    custom_title: &'a str,
    #[serde(rename = "sessionId")]
    session_id: &'a str,
}

/// Rename a saved session the way Claude Code's `/rename` does
/// (2.1.322): append `{"type":"custom-title","customTitle":…,
/// "sessionId":…}` to the transcript (after a newline if its last line
/// lacks one, so the record never glues onto it), then write the same
/// title to the `<id>/custom-title.json` sidecar beside it. The append
/// is one `write(2)` with `O_APPEND`; nothing already in the file is
/// touched.
pub(crate) fn rename(
    config_dir: &Path,
    project_dir: &str,
    id: &str,
    title: &str,
    running: bool,
) -> Result<String, String> {
    use std::io::Write;
    use std::os::unix::fs::{FileExt, OpenOptionsExt};
    if running {
        return Err("Rename it in Claude Code with /rename while it runs".into());
    }
    let title = clean_title(title)?;
    let (_, folder, file) = transcript(config_dir, project_dir, id)?;
    let line = serde_json::to_string(&CustomTitle {
        kind: "custom-title",
        custom_title: &title,
        session_id: id,
    })
    .map_err(|e| e.to_string())?;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&file)
        .map_err(|e| format!("Couldn't open the transcript: {e}"))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let mut buf = Vec::with_capacity(line.len() + 2);
    if len > 0 {
        let mut last = [0u8; 1];
        f.read_exact_at(&mut last, len - 1)
            .map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            buf.push(b'\n');
        }
    }
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    (&f).write_all(&buf)
        .and_then(|()| f.sync_data())
        .map_err(|e| format!("Couldn't write the transcript: {e}"))?;
    if let Err(e) = write_title_sidecar(&folder, id, &title) {
        tracing::debug!(error = %e, "agents: title sidecar not written");
    }
    Ok(format!("Renamed to “{title}”"))
}

/// `<project folder>/<id>/custom-title.json`, as `/rename` writes it
/// (folder 0700, file 0600). Skipped when either is a symlink.
fn write_title_sidecar(folder: &Path, id: &str, title: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let dir = folder.join(id);
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        Ok(_) => return Ok(()),
        Err(_) => std::fs::DirBuilder::new().mode(0o700).create(&dir)?,
    }
    let file = dir.join("custom-title.json");
    if std::fs::symlink_metadata(&file).is_ok_and(|m| !m.is_file()) {
        return Ok(());
    }
    let json = serde_json::json!({ "customTitle": title }).to_string();
    margo_fs::atomic_write::atomic_write(&file, json.as_bytes())?;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
}

/// Whether any project folder of the account still holds a transcript
/// of `id`.
pub(crate) fn has_transcript(projects: &Path, id: &str) -> bool {
    let name = format!("{id}.jsonl");
    std::fs::read_dir(projects).is_ok_and(|rd| {
        rd.flatten()
            .any(|e| std::fs::symlink_metadata(e.path().join(&name)).is_ok())
    })
}

/// Delete a saved session: its transcript, its sidecar folder
/// (`<id>/`: subagent transcripts, tool results, title), and — when no
/// other project folder still has a transcript of the same id — its
/// `session-env/<id>` and `file-history/<id>` folders.
pub(crate) fn delete(
    config_dir: &Path,
    project_dir: &str,
    id: &str,
    running: bool,
    locked: bool,
) -> Result<String, String> {
    if running {
        return Err("It is running — end it first".into());
    }
    if locked {
        return Err("It is locked — unlock it first".into());
    }
    let (projects, folder, file) = transcript(config_dir, project_dir, id)?;
    std::fs::remove_file(&file).map_err(|e| format!("Couldn't delete it: {e}"))?;
    let mut note = String::new();
    if let Err(e) = remove_entry(&folder.join(id)) {
        note = format!(" (its {id} folder stayed: {e})");
    }
    if !has_transcript(&projects, id) {
        for kind in ["session-env", "file-history"] {
            let Ok(base) = account_dir(config_dir, kind) else {
                continue;
            };
            if let Err(e) = remove_entry(&base.join(id)) {
                note = format!(" ({kind}/{id} stayed: {e})");
            }
        }
    }
    Ok(format!("Deleted the session{note}"))
}

/// Why `cwd` can't take a misplaced transcript, or its project folder
/// name. Refuses what Claude Code may encode differently or that isn't
/// a plain absolute path.
pub(crate) fn repair_target(cwd: &str, project_dir: &str) -> Result<String, String> {
    let unsafe_path = || Err("Refused: the recorded folder is not a plain path".to_string());
    if !cwd.starts_with('/') || cwd == "/" || cwd.ends_with('/') || cwd.contains("//") {
        return unsafe_path();
    }
    if cwd.chars().any(char::is_control) {
        return unsafe_path();
    }
    if cwd.split('/').skip(1).any(|c| c == "." || c == "..") {
        return unsafe_path();
    }
    let name = encode_cwd(cwd);
    if name.len() > MAX_NAME {
        return Err("Refused: the folder name is too long to rebuild".into());
    }
    if names_match(cwd, project_dir) {
        return Err("It is already in its folder".into());
    }
    Ok(name)
}

/// Repair a misplaced session: move its transcript (and sidecar folder)
/// into the project folder of its recorded working directory, which must
/// exist. The working directory is never created; Claude Code's project
/// folder for it is created (one level, inside `projects/`) when it
/// doesn't exist yet. Nothing is ever overwritten.
pub(crate) fn repair(
    config_dir: &Path,
    project_dir: &str,
    id: &str,
    cwd: &str,
    running: bool,
) -> Result<String, String> {
    use std::os::unix::fs::DirBuilderExt;
    if running {
        return Err("It is running — end it first".into());
    }
    let name = repair_target(cwd, project_dir)?;
    if !std::fs::metadata(cwd).is_ok_and(|m| m.is_dir()) {
        return Err(format!("{cwd} is gone"));
    }
    let (projects, folder, file) = transcript(config_dir, project_dir, id)?;
    let target = projects.join(&name);
    let mut created = false;
    let target = match std::fs::symlink_metadata(&target) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => target,
        Ok(m) if m.file_type().is_symlink() => {
            // An alias of another project folder: follow it only inside
            // projects/.
            let real = std::fs::canonicalize(&target).map_err(|e| e.to_string())?;
            if real.parent() != Some(projects.as_path()) || !real.is_dir() {
                return Err("Refused: its project folder points outside projects/".into());
            }
            real
        }
        Ok(_) => return Err("Its project folder is not a folder".into()),
        Err(_) => {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&target)
                .map_err(|e| format!("Couldn't create its project folder: {e}"))?;
            created = true;
            target
        }
    };
    let dest = target.join(format!("{id}.jsonl"));
    let side = folder.join(id);
    let side_dest = target.join(id);
    let has_side = real_dir_in(&folder, id).is_ok();
    let exists = |p: &Path| std::fs::symlink_metadata(p).is_ok();
    if exists(&dest) || (has_side && exists(&side_dest)) {
        if created {
            let _ = std::fs::remove_dir(&target);
        }
        return Err("A session with this id is already there".into());
    }
    if let Err(e) = rename_noreplace(&file, &dest) {
        if created {
            let _ = std::fs::remove_dir(&target);
        }
        return Err(format!("Couldn't move it: {e}"));
    }
    if has_side && let Err(e) = rename_noreplace(&side, &side_dest) {
        return Ok(format!(
            "Moved the transcript; its {id} folder stayed ({e})"
        ));
    }
    Ok(format!("Moved to {}", super::super::sessions::tilde(cwd)))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
