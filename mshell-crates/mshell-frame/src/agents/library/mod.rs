//! Every saved Claude Code session on disk, for the Sessions page's
//! session manager: one row per transcript (`<config>/projects/<folder>/
//! <uuid>.jsonl`) of every account, with whether it can resume as it is
//! ([`classify`]), and the actions on it ([`ops`], [`leftovers`],
//! [`locks`]).
//!
//! The scan reads each transcript once, then only what was appended
//! ([`meta`]); it runs in [`LIBRARY_POLL`] off the GTK thread, only while
//! the Sessions tab is shown. A project folder that is a symlink to
//! another one (an alias Claude Code follows) is not scanned twice.

pub(crate) mod classify;
pub(crate) mod encode;
pub(crate) mod guard;
pub(crate) mod leftovers;
pub(crate) mod locks;
pub(crate) mod meta;
pub(crate) mod ops;
pub(crate) mod view;

use super::accounts::Account;
use super::jsonl::JsonlTail;
use classify::{FirstPass, LibStatus};
use encode::{Found, Walker};
use leftovers::Leftover;
use meta::{TranscriptMeta, is_uuid};
use mshell_utils::shared_poll::SharedPoll;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// One saved session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LibSession {
    pub(crate) id: String,
    /// Its account's config dir and label.
    pub(crate) config_dir: PathBuf,
    pub(crate) account: String,
    /// The account mshell itself runs under (no `CLAUDE_CONFIG_DIR`
    /// needed to resume it).
    pub(crate) primary: bool,
    /// The `projects/` folder its transcript is in.
    pub(crate) project_dir: String,
    /// Its recorded working directory (`""` when none).
    pub(crate) cwd: String,
    pub(crate) title: String,
    /// The `/rename` title, `""` when never renamed.
    pub(crate) custom_title: String,
    pub(crate) first_prompt: String,
    pub(crate) branch: String,
    /// Last write to the transcript, unix ms.
    pub(crate) mtime_ms: i64,
    pub(crate) size: u64,
    /// No prompt was ever sent.
    pub(crate) empty: bool,
    pub(crate) status: LibStatus,
    pub(crate) running: bool,
    pub(crate) locked: bool,
}

impl LibSession {
    /// Unique per transcript (an id can sit in two project folders).
    pub(crate) fn key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.config_dir.display(),
            self.project_dir,
            self.id
        )
    }

    /// The folder it belongs to: where a moved project is now, else its
    /// recorded folder, else a guess from the project folder name.
    pub(crate) fn folder(&self) -> String {
        match &self.status {
            LibStatus::Moved(to) => to.to_string_lossy().into_owned(),
            _ if !self.cwd.is_empty() => self.cwd.clone(),
            _ => encode::decode_guess(&self.project_dir).unwrap_or_else(|| "Unknown folder".into()),
        }
    }

    /// The folder it resumes from; `None` when it can't resume as it is
    /// (orphan, misplaced, unknown, nothing recorded).
    pub(crate) fn resume_dir(&self) -> Option<String> {
        match &self.status {
            LibStatus::Ok if !self.cwd.is_empty() => Some(self.cwd.clone()),
            LibStatus::Moved(to) => Some(to.to_string_lossy().into_owned()),
            _ => None,
        }
    }

    /// Why a click does not resume it, `None` when it does.
    pub(crate) fn no_resume_reason(&self) -> Option<String> {
        if self.running {
            return Some("Running now — jump to it from the list above".into());
        }
        if self.empty {
            return Some("Empty: nothing was ever sent in it".into());
        }
        match self.resume_dir() {
            Some(_) => None,
            None if self.status == LibStatus::Ok => Some("It never recorded its folder".into()),
            None => Some(self.status.reason(&self.cwd)),
        }
    }

    /// `cd <dir> && claude --resume <id>` (with the account's
    /// `CLAUDE_CONFIG_DIR` when it isn't mshell's own).
    pub(crate) fn resume_line(&self) -> Option<String> {
        let dir = self.resume_dir()?;
        let env = if self.primary {
            String::new()
        } else {
            format!(
                "CLAUDE_CONFIG_DIR={} ",
                view::shell_quote(&self.config_dir.to_string_lossy())
            )
        };
        Some(format!(
            "cd {} && {env}claude --resume {}",
            view::shell_quote(&dir),
            self.id
        ))
    }

    /// Rename is offered: not running, not orphaned.
    pub(crate) fn can_rename(&self) -> bool {
        !self.running && !matches!(self.status, LibStatus::Orphan { .. })
    }

    pub(crate) fn can_delete(&self) -> bool {
        !self.running && !self.locked
    }

    pub(crate) fn can_repair(&self) -> bool {
        !self.running && self.status == LibStatus::Misplaced
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct LibrarySnapshot {
    /// Newest first.
    pub(crate) sessions: Vec<LibSession>,
    /// Several accounts: rows name theirs.
    pub(crate) multi: bool,
}

/// One transcript's incremental reader and what it found.
#[derive(Debug, Default)]
pub(crate) struct FileState {
    tail: JsonlTail,
    meta: TranscriptMeta,
}

/// The scan's memory between polls: per transcript path.
#[derive(Debug, Default)]
pub(crate) struct ScanCache {
    files: HashMap<PathBuf, FileState>,
}

/// How long the search for moved folders may take per scan.
const WALK_TIME: Duration = Duration::from_secs(3);

fn mtime_ms(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// A transcript waiting for its project folder to be searched.
struct Pending {
    session: LibSession,
    first: FirstPass,
    /// Its project folder holds memory files.
    memory: bool,
}

/// Scan every account (primary first). `live`: ids of running sessions;
/// `locked`: whether an id is locked. Blocking.
pub(crate) fn scan(
    accounts: &[Account],
    live: &HashSet<String>,
    locked: &dyn Fn(&str) -> bool,
    cache: &mut ScanCache,
) -> Vec<LibSession> {
    let mut out = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut seen_roots: Vec<PathBuf> = Vec::new();
    let mut seen_files: HashSet<PathBuf> = HashSet::new();
    for a in accounts {
        let Ok(projects) = guard::account_dir(&a.dir, "projects") else {
            continue;
        };
        if seen_roots.contains(&projects) {
            continue;
        }
        seen_roots.push(projects.clone());
        let Ok(rd) = std::fs::read_dir(&projects) else {
            continue;
        };
        let mut folders: Vec<String> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        folders.sort();
        for folder in folders {
            let dir = projects.join(&folder);
            let has_memory =
                std::fs::read_dir(dir.join("memory")).is_ok_and(|mut r| r.next().is_some());
            let Ok(files) = std::fs::read_dir(&dir) else {
                continue;
            };
            for f in files.flatten() {
                let path = f.path();
                let Some(id) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".jsonl"))
                    .filter(|id| is_uuid(id))
                    .map(str::to_string)
                else {
                    continue;
                };
                let Ok(m) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if !m.is_file() {
                    continue;
                }
                let st = cache.files.entry(path.clone()).or_default();
                if st.tail.would_restart(&path) {
                    st.meta = TranscriptMeta::default();
                }
                let meta = &mut st.meta;
                if let Err(e) = st.tail.read(&path, None, |l| meta.add_line(l)) {
                    tracing::debug!(error = %e, path = %path.display(), "agents: transcript not read");
                }
                seen_files.insert(path.clone());
                let meta = st.meta.clone();
                let cwd = meta.effective_cwd().to_string();
                let cwd_is_dir =
                    !cwd.is_empty() && std::fs::metadata(&cwd).is_ok_and(|m| m.is_dir());
                let same_folder = !cwd.is_empty()
                    && std::fs::canonicalize(projects.join(encode::encode_cwd(&cwd)))
                        .is_ok_and(|p| p == dir);
                let session = LibSession {
                    title: meta.title(&id),
                    running: live.contains(&id),
                    locked: locked(&id),
                    id,
                    config_dir: a.dir.clone(),
                    account: a.label.clone(),
                    primary: a.primary,
                    project_dir: folder.clone(),
                    cwd: cwd.clone(),
                    custom_title: meta.custom_title.clone(),
                    first_prompt: meta.first_prompt.clone(),
                    branch: meta.git_branch.clone(),
                    mtime_ms: mtime_ms(&m),
                    size: m.len(),
                    empty: !meta.has_prompt,
                    status: LibStatus::Unknown,
                };
                pending.push(Pending {
                    session,
                    first: classify::first_pass(&cwd, &folder, cwd_is_dir, same_folder),
                    memory: has_memory,
                });
            }
        }
    }
    cache.files.retain(|p, _| seen_files.contains(p));
    // The folder search, once per project folder name.
    let mut walker = Walker::new(encode::list_subdirs, Some(Instant::now() + WALK_TIME));
    let mut found: HashMap<String, Found> = HashMap::new();
    for p in pending {
        let mut s = p.session;
        s.status = match &p.first {
            FirstPass::Settled(st) => st.clone(),
            first => {
                let f = found
                    .entry(s.project_dir.clone())
                    .or_insert_with(|| walker.resolve(&s.project_dir));
                classify::settle(first.clone(), f, p.memory)
            }
        };
        out.push(s);
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.mtime_ms));
    out
}

// ── The poll ───────────────────────────────────────────────────────────

/// While the Sessions tab shows, files change slowly; an action asks for a
/// fresh scan at once.
const TICK: Duration = Duration::from_secs(30);

pub(crate) static LIBRARY_POLL: LazyLock<SharedPoll<LibrarySnapshot>> =
    LazyLock::new(|| SharedPoll::new(|| TICK, || Box::pin(probe())));

static CACHE: LazyLock<Mutex<ScanCache>> = LazyLock::new(Mutex::default);

async fn probe() -> LibrarySnapshot {
    tokio::task::spawn_blocking(snapshot_now)
        .await
        .unwrap_or_default()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Running session ids of every account (fresh).
fn live_ids(accounts: &[Account]) -> HashSet<String> {
    accounts
        .iter()
        .flat_map(|a| super::sessions::read_live_sessions(&a.dir, &a.label))
        .map(|s| s.session_id)
        .collect()
}

/// A fresh scan. Blocking.
pub(crate) fn snapshot_now() -> LibrarySnapshot {
    let accounts = super::accounts::accounts(now_ms());
    let live = live_ids(&accounts);
    let locks = locks::load();
    let Ok(mut cache) = CACHE.lock() else {
        return LibrarySnapshot::default();
    };
    LibrarySnapshot {
        sessions: scan(&accounts, &live, &|id| locks.contains(id), &mut cache),
        multi: accounts.len() > 1,
    }
}

/// Which session an action is for. The page sends these back; the
/// action looks the session up again in a fresh scan and acts on what
/// it finds there, never on paths from the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionRef {
    pub(crate) config_dir: PathBuf,
    pub(crate) project_dir: String,
    pub(crate) id: String,
}

impl SessionRef {
    pub(crate) fn of(s: &LibSession) -> Self {
        SessionRef {
            config_dir: s.config_dir.clone(),
            project_dir: s.project_dir.clone(),
            id: s.id.clone(),
        }
    }

    fn is(&self, s: &LibSession) -> bool {
        s.config_dir == self.config_dir && s.project_dir == self.project_dir && s.id == self.id
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LibAction {
    Rename(SessionRef, String),
    Delete(SessionRef),
    /// Every empty session that isn't running or locked.
    DeleteEmpty,
    Repair(SessionRef),
    Lock(SessionRef, bool),
    /// Remove these leftovers (as listed), each re-checked.
    RemoveLeftovers(Vec<Leftover>),
}

/// The session `r` points at, in a fresh scan.
fn fresh(r: &SessionRef) -> Result<LibSession, String> {
    snapshot_now()
        .sessions
        .into_iter()
        .find(|s| r.is(s))
        .ok_or_else(|| "That session is gone".to_string())
}

/// Run `action`; `Ok` is a one-line result for the page. Blocking.
pub(crate) fn run(action: LibAction) -> Result<String, String> {
    let result = match action {
        LibAction::Rename(r, title) => {
            let s = fresh(&r)?;
            if !s.can_rename() {
                return Err("This session can't be renamed".into());
            }
            ops::rename(&s.config_dir, &s.project_dir, &s.id, &title, s.running)
        }
        LibAction::Delete(r) => {
            let s = fresh(&r)?;
            ops::delete(&s.config_dir, &s.project_dir, &s.id, s.running, s.locked)
        }
        LibAction::DeleteEmpty => {
            let snap = snapshot_now();
            let mut n = 0;
            let mut failed = 0;
            // A session written to in the last minute may be starting up
            // before its live-session file exists: left alone.
            let now = now_ms();
            for s in snap
                .sessions
                .iter()
                .filter(|s| s.empty && !s.running && !s.locked && now - s.mtime_ms > 60_000)
            {
                match ops::delete(&s.config_dir, &s.project_dir, &s.id, s.running, s.locked) {
                    Ok(_) => n += 1,
                    Err(e) => {
                        failed += 1;
                        tracing::debug!(error = %e, "agents: empty session not deleted");
                    }
                }
            }
            if failed > 0 {
                Err(format!("Deleted {n} empty sessions; {failed} failed"))
            } else {
                Ok(format!(
                    "Deleted {n} empty session{}",
                    if n == 1 { "" } else { "s" }
                ))
            }
        }
        LibAction::Repair(r) => {
            let s = fresh(&r)?;
            if !s.can_repair() {
                return Err("Only a misplaced session that isn't running can be repaired".into());
            }
            ops::repair(&s.config_dir, &s.project_dir, &s.id, &s.cwd, s.running)
        }
        LibAction::Lock(r, on) => locks::set(&r.id, on).map(|()| {
            if on {
                "Locked".into()
            } else {
                "Unlocked".into()
            }
        }),
        LibAction::RemoveLeftovers(mut items) => {
            // Only the accounts mshell knows; nothing the page made up.
            let accounts = super::accounts::accounts(now_ms());
            items.retain(|l| accounts.iter().any(|a| a.dir == l.config_dir));
            let (_, keep) = leftover_scan();
            leftovers::remove(&items, &keep, now_ms())
                .map(|n| format!("Removed {n} leftover{}", if n == 1 { "" } else { "s" }))
        }
    };
    LIBRARY_POLL.refresh_now();
    result
}

/// Leftovers of every account, and the ids that must stay (every id with
/// a transcript in any account, and every running one). Blocking.
fn leftover_scan() -> (Vec<Leftover>, HashSet<String>) {
    let accounts = super::accounts::accounts(now_ms());
    let mut keep = live_ids(&accounts);
    keep.extend(snapshot_now().sessions.into_iter().map(|s| s.id));
    // Subagent-only ids and transcripts of folders the scan skipped
    // (unreadable) are kept too: any `<id>.jsonl` anywhere below a
    // project folder.
    for a in &accounts {
        if let Ok(projects) = guard::account_dir(&a.dir, "projects") {
            keep.extend(all_transcript_ids(&projects));
        }
    }
    let now = now_ms();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut items = Vec::new();
    for a in &accounts {
        let real = std::fs::canonicalize(&a.dir).unwrap_or_else(|_| a.dir.clone());
        if seen.contains(&real) {
            continue;
        }
        seen.push(real);
        items.extend(leftovers::find(&a.dir, &keep, now));
    }
    (items, keep)
}

/// Ids of every `<uuid>.jsonl` directly in a project folder.
fn all_transcript_ids(projects: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(rd) = std::fs::read_dir(projects) else {
        return out;
    };
    for d in rd.flatten() {
        let Ok(files) = std::fs::read_dir(d.path()) else {
            continue;
        };
        for f in files.flatten() {
            if let Some(id) = f
                .file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".jsonl"))
                .filter(|id| is_uuid(id))
            {
                out.insert(id.to_string());
            }
        }
    }
    out
}

/// The leftovers to show before a cleanup. Blocking.
pub(crate) fn find_leftovers() -> Vec<Leftover> {
    leftover_scan().0
}

#[cfg(test)]
#[path = "mod_tests.rs"]
pub(crate) mod tests;
