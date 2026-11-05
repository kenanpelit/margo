//! Live and saved Claude Code sessions.
//!
//! Claude Code (2.1+) keeps `<config>/sessions/<pid>.json` per running
//! session, with its status (`busy` / `waiting` / `idle`), cwd, name and
//! process start time. Only `<pid>.json` is read: the `<pid>.<hash>.key`
//! files beside them hold a peer token and are never opened.
//!
//! Jumping to a session's terminal: inside tmux the session's pane is
//! selected and the most recently active tmux client stands in for the
//! session; then the process tree is walked up to the first process that
//! owns a margo window (margo publishes each window's pid in state.json),
//! which is focused with `focuswindowid`. kitty windows are additionally
//! focused through kitty remote control when it is enabled.
//!
//! Saved sessions (to resume) come from `<config>/history.jsonl`: every
//! prompt line names its session and project. [`SavedIndex`] reads only
//! the lines appended since its last look.

use super::details::SessionDetail;
use super::jsonl::JsonlTail;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SessionStatus {
    /// Waiting for you: a permission prompt or a question.
    Waiting,
    /// Working on a turn.
    Busy,
    Idle,
}

impl SessionStatus {
    fn parse(s: &str) -> Self {
        match s {
            "waiting" => SessionStatus::Waiting,
            "busy" => SessionStatus::Busy,
            _ => SessionStatus::Idle,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            SessionStatus::Waiting => "Needs you",
            SessionStatus::Busy => "Working",
            SessionStatus::Idle => "Idle",
        }
    }

    pub(crate) fn css_class(self) -> &'static str {
        match self {
            SessionStatus::Waiting => "waiting",
            SessionStatus::Busy => "busy",
            SessionStatus::Idle => "idle",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LiveSession {
    pub(crate) pid: u32,
    pub(crate) session_id: String,
    pub(crate) name: String,
    pub(crate) cwd: String,
    pub(crate) status: SessionStatus,
    /// What a waiting session waits for, when Claude Code says.
    pub(crate) waiting_for: String,
    /// When the status last changed, unix ms.
    pub(crate) since_ms: i64,
    /// `/proc/<pid>/stat` start time Claude Code recorded; guards against
    /// a reused pid.
    proc_start: Option<u64>,
    /// The claude.ai/code session id when Remote Control is on.
    pub(crate) bridge: Option<String>,
    /// The account (config dir) it runs under, and its label.
    pub(crate) config_dir: PathBuf,
    pub(crate) account: String,
    /// What its transcript says it is doing, once read.
    pub(crate) detail: Option<SessionDetail>,
    /// Its estimated cost so far, USD.
    pub(crate) cost_usd: Option<f64>,
    /// Its context use as a share of the model's window.
    pub(crate) context_fraction: Option<f64>,
}

impl LiveSession {
    /// Unique across accounts.
    pub(crate) fn key(&self) -> String {
        format!("{}|{}", self.config_dir.display(), self.session_id)
    }

    /// Its page on claude.ai when Remote Control is on.
    pub(crate) fn remote_url(&self) -> Option<String> {
        self.bridge
            .as_deref()
            .map(|b| format!("https://claude.ai/code/{b}"))
    }

    /// The AI title when the transcript has one, else the session name.
    pub(crate) fn title(&self) -> &str {
        self.detail
            .as_ref()
            .map(|d| d.title.as_str())
            .filter(|t| !t.is_empty())
            .unwrap_or(&self.name)
    }
}

/// Parse one `sessions/<pid>.json`. Agent SDK runs (hooks, scripts) are
/// not interactive sessions and are skipped.
pub(crate) fn parse_session(raw: &str) -> Option<LiveSession> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let pid = u32::try_from(v.get("pid")?.as_u64()?).ok()?;
    if let Some(kind) = v.get("kind").and_then(Value::as_str)
        && kind != "interactive"
    {
        return None;
    }
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let int = |k: &str| v.get(k).and_then(Value::as_i64);
    Some(LiveSession {
        pid,
        session_id: text("sessionId"),
        name: Some(text("name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| pid.to_string()),
        cwd: text("cwd"),
        status: SessionStatus::parse(v.get("status").and_then(Value::as_str).unwrap_or("")),
        waiting_for: text("waitingFor"),
        since_ms: int("statusUpdatedAt")
            .or_else(|| int("updatedAt"))
            .or_else(|| int("startedAt"))
            .unwrap_or(0),
        proc_start: v.get("procStart").and_then(|p| {
            p.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| p.as_u64())
        }),
        bridge: Some(text("bridgeSessionId")).filter(|b| {
            !b.is_empty()
                && b.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        }),
        config_dir: PathBuf::new(),
        account: String::new(),
        detail: None,
        cost_usd: None,
        context_fraction: None,
    })
}

#[cfg(test)]
pub(crate) fn test_session(status: SessionStatus, since_ms: i64) -> LiveSession {
    LiveSession {
        pid: 1,
        session_id: String::new(),
        name: String::new(),
        cwd: String::new(),
        status,
        waiting_for: String::new(),
        since_ms,
        proc_start: None,
        bridge: None,
        config_dir: PathBuf::new(),
        account: String::new(),
        detail: None,
        cost_usd: None,
        context_fraction: None,
    }
}

/// Field 22 (starttime) of a `/proc/<pid>/stat` line. `comm` (field 2)
/// may contain spaces and parens, so count from the last `)`.
pub(crate) fn stat_start_time(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Parent pid (field 4) of a `/proc/<pid>/stat` line.
pub(crate) fn stat_ppid(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

fn proc_stat(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
}

/// The session's process still runs (and is the same process).
fn alive(s: &LiveSession) -> bool {
    match (proc_stat(s.pid), s.proc_start) {
        (Some(stat), Some(start)) => stat_start_time(&stat) == Some(start),
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Order for the list and the pill: needs-you first, then working, then
/// idle; most recent change first within a group.
pub(crate) fn sort_sessions(v: &mut [LiveSession]) {
    v.sort_by(|a, b| a.status.cmp(&b.status).then(b.since_ms.cmp(&a.since_ms)));
}

/// Every running session under `config_dir` (tagged with `account`).
pub(crate) fn read_live_sessions(config_dir: &Path, account: &str) -> Vec<LiveSession> {
    let Ok(entries) = std::fs::read_dir(config_dir.join("sessions")) else {
        return Vec::new();
    };
    let mut out: Vec<LiveSession> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_session_file(p))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|raw| parse_session(&raw))
        .filter(alive)
        .map(|mut s| {
            s.config_dir = config_dir.to_path_buf();
            s.account = account.to_string();
            s
        })
        .collect();
    sort_sessions(&mut out);
    out
}

/// `<digits>.json` only — never the `.key` peer-token files.
pub(crate) fn is_session_file(p: &Path) -> bool {
    p.extension().is_some_and(|x| x == "json")
        && p.file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

// ── Saved sessions ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedSession {
    pub(crate) session_id: String,
    pub(crate) project: String,
    /// The last prompt typed in it.
    pub(crate) last_prompt: String,
    pub(crate) last_ms: i64,
    pub(crate) prompts: u32,
    /// The account it belongs to (set by the caller; empty in the index).
    pub(crate) config_dir: PathBuf,
    pub(crate) account: String,
}

/// Sessions grouped from `history.jsonl`, kept up to date by reading only
/// the lines appended since the last [`SavedIndex::refresh`]. The file is
/// megabytes and changes on every prompt; it used to be re-read whole each
/// time.
#[derive(Debug, Default)]
pub(crate) struct SavedIndex {
    tail: JsonlTail,
    by_id: HashMap<String, SavedSession>,
    /// Bytes read from disk so far (the tests check what a refresh costs).
    pub(crate) bytes_read: u64,
}

impl SavedIndex {
    /// Fold one history line in.
    fn add_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let Some(id) = v
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let ts = v.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
        let display = v
            .get("display")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let project = v.get("project").and_then(Value::as_str).unwrap_or("");
        self.fold(SavedSession {
            session_id: id.to_string(),
            project: project.to_string(),
            last_prompt: display.lines().next().unwrap_or("").to_string(),
            last_ms: ts,
            prompts: 1,
            config_dir: PathBuf::new(),
            account: String::new(),
        });
    }

    /// Merge `s` (one line, or a group of newer lines) into its session.
    fn fold(&mut self, s: SavedSession) {
        let e = self
            .by_id
            .entry(s.session_id.clone())
            .or_insert_with(|| SavedSession {
                session_id: s.session_id.clone(),
                project: s.project.clone(),
                last_prompt: String::new(),
                last_ms: 0,
                prompts: 0,
                config_dir: PathBuf::new(),
                account: String::new(),
            });
        e.prompts += s.prompts;
        if s.last_ms >= e.last_ms {
            e.last_ms = s.last_ms;
            if !s.last_prompt.is_empty() {
                e.last_prompt = s.last_prompt;
            }
            if !s.project.is_empty() {
                e.project = s.project;
            }
        }
    }

    /// Read what was appended to `path` since the last call (all of it the
    /// first time, or after the file was replaced). Returns whether
    /// anything changed.
    pub(crate) fn refresh(&mut self, path: &Path) -> bool {
        let mut fresh = SavedIndex::default();
        let Ok(out) = self.tail.read(path, None, |line| fresh.add_line(line)) else {
            return false;
        };
        self.bytes_read += out.bytes;
        if out.restarted {
            self.by_id = fresh.by_id;
            return true;
        }
        let changed = !fresh.by_id.is_empty();
        for (_, s) in fresh.by_id {
            self.fold(s);
        }
        changed
    }

    /// Every project with a session, with its newest prompt time.
    pub(crate) fn projects(&self) -> Vec<(i64, String)> {
        let mut by: HashMap<&str, i64> = HashMap::new();
        for s in self.by_id.values().filter(|s| !s.project.is_empty()) {
            let e = by.entry(s.project.as_str()).or_insert(s.last_ms);
            *e = (*e).max(s.last_ms);
        }
        by.into_iter().map(|(p, ms)| (ms, p.to_string())).collect()
    }

    /// The `limit` most recent sessions, newest first, leaving out
    /// `exclude` (the running ones).
    pub(crate) fn top(&self, exclude: &[&str], limit: usize) -> Vec<SavedSession> {
        let mut v: Vec<SavedSession> = self
            .by_id
            .values()
            .filter(|s| !exclude.contains(&s.session_id.as_str()))
            .cloned()
            .collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.last_ms));
        v.truncate(limit);
        v
    }
}

/// The `limit` most recent sessions in `history` (the text of
/// `history.jsonl`), newest first, leaving out `exclude`.
#[cfg(test)]
pub(crate) fn saved_sessions(history: &str, exclude: &[&str], limit: usize) -> Vec<SavedSession> {
    let mut index = SavedIndex::default();
    for line in history.lines() {
        index.add_line(line);
    }
    index.top(exclude, limit)
}

/// Sessions with a prompt holding every word of `query`
/// (case-insensitive), read from a `history.jsonl` stream. Each session
/// shows its newest matching prompt; `prompts` counts the matches.
/// Newest first, at most `limit`. Slash commands (`/clear`) are not
/// prompts and never match.
pub(crate) fn search_history(
    history: impl std::io::BufRead,
    query: &str,
    limit: usize,
) -> Vec<SavedSession> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut by_id: HashMap<String, SavedSession> = HashMap::new();
    for line in history.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
        let (id, display, project) = (text("sessionId"), text("display").trim(), text("project"));
        if id.is_empty() || display.is_empty() || super::local_stats::is_slash_command(display) {
            continue;
        }
        let hay = display.to_lowercase();
        if !words.iter().all(|w| hay.contains(w.as_str())) {
            continue;
        }
        let ts = v.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
        let e = by_id.entry(id.to_string()).or_insert_with(|| SavedSession {
            session_id: id.to_string(),
            project: project.to_string(),
            last_prompt: String::new(),
            last_ms: 0,
            prompts: 0,
            config_dir: PathBuf::new(),
            account: String::new(),
        });
        e.prompts += 1;
        if ts >= e.last_ms {
            e.last_ms = ts;
            e.last_prompt = display.lines().next().unwrap_or("").to_string();
        }
    }
    let mut v: Vec<SavedSession> = by_id.into_values().collect();
    v.sort_by_key(|s| std::cmp::Reverse(s.last_ms));
    v.truncate(limit);
    v
}

/// `/home/kenan/x` → `~/x`.
pub(crate) fn tilde(path: &str) -> String {
    match margo_fs::xdg::home() {
        Some(home) => match Path::new(path).strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.to_string(),
        },
        None => path.to_string(),
    }
}

// ── Jump to the terminal ───────────────────────────────────────────────

fn environ(pid: u32) -> HashMap<String, String> {
    std::fs::read(format!("/proc/{pid}/environ"))
        .map(|b| parse_environ(&b))
        .unwrap_or_default()
}

/// NUL-separated `KEY=value` pairs.
pub(crate) fn parse_environ(bytes: &[u8]) -> HashMap<String, String> {
    bytes
        .split(|b| *b == 0)
        .filter_map(|kv| {
            let s = std::str::from_utf8(kv).ok()?;
            let (k, v) = s.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// The first of `start` and its ancestors that owns a window in
/// `windows` (`(pid, window id)` pairs).
pub(crate) fn window_for_process(
    start: u32,
    windows: &[(u32, u64)],
    parent: impl Fn(u32) -> Option<u32>,
) -> Option<u64> {
    let mut pid = start;
    for _ in 0..64 {
        if pid <= 1 {
            return None;
        }
        if let Some((_, id)) = windows.iter().find(|(p, _)| *p == pid) {
            return Some(*id);
        }
        pid = parent(pid)?;
    }
    None
}

/// The tmux client to stand in for a pane: the most recently active one
/// in `list-clients -F '#{client_activity} #{client_pid}'` output.
pub(crate) fn most_active_client(list_clients: &str) -> Option<u32> {
    list_clients
        .lines()
        .filter_map(|l| {
            let (act, pid) = l.trim().split_once(' ')?;
            Some((act.parse::<u64>().ok()?, pid.trim().parse::<u32>().ok()?))
        })
        .max_by_key(|(act, _)| *act)
        .map(|(_, pid)| pid)
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Bring the terminal running session `pid` to the front. Blocking (runs
/// `tmux` / `kitty @`); call off the GTK thread. Returns whether a window
/// was focused.
pub(crate) fn focus_session(pid: u32) -> bool {
    let env = environ(pid);
    let mut anchor = pid;
    if let (Some(tmux), Some(pane)) = (env.get("TMUX"), env.get("TMUX_PANE")) {
        let sock = tmux.split(',').next().unwrap_or("");
        let t = |args: &[&str]| {
            let mut a = vec!["-S", sock];
            a.extend_from_slice(args);
            run("tmux", &a)
        };
        let Some(session) = t(&["display-message", "-p", "-t", pane, "#{session_name}"]) else {
            return false;
        };
        let _ = t(&["select-window", "-t", pane]);
        let _ = t(&["select-pane", "-t", pane]);
        match t(&[
            "list-clients",
            "-t",
            session.trim(),
            "-F",
            "#{client_activity} #{client_pid}",
        ])
        .as_deref()
        .and_then(most_active_client)
        {
            Some(client) => anchor = client,
            None => return false,
        }
    }
    // kitty: land on the exact tab/split when remote control is on.
    let anchor_env = if anchor == pid { env } else { environ(anchor) };
    if let (Some(kpid), Some(wid)) = (
        anchor_env.get("KITTY_PID"),
        anchor_env.get("KITTY_WINDOW_ID"),
    ) {
        let _ = run(
            "kitty",
            &[
                "@",
                "--to",
                &format!("unix:@kitty-{kpid}"),
                "focus-window",
                "--match",
                &format!("id:{wid}"),
            ],
        );
    }
    let Some(state) = mshell_margo_client::read_state_json() else {
        return false;
    };
    let windows: Vec<(u32, u64)> = state
        .clients
        .iter()
        .filter_map(|c| {
            u32::try_from(c.pid)
                .ok()
                .filter(|p| *p > 0)
                .map(|p| (p, c.id))
        })
        .collect();
    let parent = |p: u32| proc_stat(p).as_deref().and_then(stat_ppid);
    match window_for_process(anchor, &windows, parent) {
        Some(id) => mshell_margo_client::ctl::dispatch("focuswindowid", &[&id.to_string()]).is_ok(),
        None => false,
    }
}

// ── Resume ─────────────────────────────────────────────────────────────

/// The command (run inside the user's terminal) that resumes `session_id`
/// in `cwd` — or starts a new session there when `session_id` is empty —
/// under `config_dir` (`CLAUDE_CONFIG_DIR`; empty = the default). The
/// directory, id and config dir travel as positional arguments to
/// `sh -c`, never spliced into the script.
pub(crate) fn resume_command(
    claude: &str,
    cwd: &str,
    session_id: &str,
    config_dir: &str,
) -> Vec<String> {
    let script = "cd \"$1\" 2>/dev/null; [ -n \"$4\" ] && export CLAUDE_CONFIG_DIR=\"$4\"; \
                  if [ -n \"$3\" ]; then exec \"$2\" --resume \"$3\"; else exec \"$2\"; fi";
    [
        "sh", "-c", script, "sh", cwd, claude, session_id, config_dir,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `claude` on PATH, else the installer's `~/.local/bin/claude` (mshell
/// runs under systemd and may not see the login shell's PATH).
pub(crate) fn claude_binary() -> Option<String> {
    margo_fs::path::find_in_path("claude")
        .or_else(|| {
            let p = margo_fs::xdg::home()?.join(".local/bin/claude");
            p.is_file().then_some(p)
        })
        .map(|p| p.to_string_lossy().into_owned())
}

/// Open a terminal resuming `session_id` in `cwd` (a new session when it
/// is empty) under `config_dir` (`None` = the default account). Errors
/// are a short user-facing reason.
pub(crate) fn resume(session_id: &str, cwd: &str, config_dir: Option<&Path>) -> Result<(), String> {
    use mshell_process::SpawnReaped;
    let claude = claude_binary().ok_or("claude is not installed")?;
    let dir = config_dir
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    // In its own scope: the resumed session must not die with an mshell
    // restart (a plain child is killed with mshell.service).
    let (term, mut cmd) = mshell_process::terminal::command(
        &resume_command(&claude, cwd, session_id, &dir),
        "claude",
    )
    .map_err(|e| e.to_string())?;
    cmd.spawn_reaped()
        .map(|_| ())
        .map_err(|e| format!("{term}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `~/.claude/sessions/10844.json` (Claude Code 2.1.318).
    const SESSION: &str = r#"{"pid":10844,"sessionId":"f8c35107-1195-4697-9687-27f171d17c69","cwd":"/repo/archive/.kod/linux-meteor","startedAt":1793808296853,"procStart":"3065","version":"2.1.318","peerProtocol":1,"peerFeatures":["notify_idle","reply_across_default_dirs","artifact_yield"],"kind":"interactive","entrypoint":"cli","pidDomain":"linux:f0ca762a05d74d3090dea8e841890ce7:pid:[4026531836]","asksForConvertedSkills":false,"tmux":"Meteor:@15.%17","messagingSocketPath":"/run/user/1000/cc-socks/10844.sock","name":"linux-meteor-62","nameSource":"derived","nameSince":1793808296854,"updatedAt":1793808300449,"status":"idle","statusUpdatedAt":1793808300449,"bridgeSessionId":"session_0199mJTFiXH171ULbbkGauz9"}"#;

    #[test]
    fn parses_a_real_session_file() {
        let s = parse_session(SESSION).unwrap();
        assert_eq!(s.pid, 10844);
        assert_eq!(s.name, "linux-meteor-62");
        assert_eq!(s.cwd, "/repo/archive/.kod/linux-meteor");
        assert_eq!(s.status, SessionStatus::Idle);
        assert_eq!(s.since_ms, 1793808300449);
        assert_eq!(s.proc_start, Some(3065));
        assert_eq!(
            s.remote_url().as_deref(),
            Some("https://claude.ai/code/session_0199mJTFiXH171ULbbkGauz9")
        );
        let odd = SESSION.replace("session_0199mJTFiXH171ULbbkGauz9", "x/../y");
        assert_eq!(parse_session(&odd).unwrap().bridge, None);
    }

    #[test]
    fn status_and_kind() {
        let busy = SESSION.replace("\"status\":\"idle\"", "\"status\":\"busy\"");
        assert_eq!(parse_session(&busy).unwrap().status, SessionStatus::Busy);
        let waiting = SESSION.replace(
            "\"status\":\"idle\"",
            "\"status\":\"waiting\",\"waitingFor\":\"approve Bash\"",
        );
        let w = parse_session(&waiting).unwrap();
        assert_eq!(
            (w.status, w.waiting_for.as_str()),
            (SessionStatus::Waiting, "approve Bash")
        );
        let sdk = SESSION.replace("\"kind\":\"interactive\"", "\"kind\":\"sdk\"");
        assert_eq!(parse_session(&sdk), None);
        assert_eq!(parse_session("{}"), None);
    }

    #[test]
    fn only_pid_json_files_are_sessions() {
        assert!(is_session_file(Path::new("/x/sessions/10844.json")));
        assert!(!is_session_file(Path::new(
            "/x/sessions/10844.d06cb9f820e546b318559275eacbd346f1423ea37ce4b0760bb1fd2943eb1bc8.key"
        )));
        assert!(!is_session_file(Path::new("/x/sessions/notes.json")));
    }

    #[test]
    fn sorts_needs_you_first() {
        let mk = test_session;
        let mut v = vec![
            mk(SessionStatus::Idle, 9),
            mk(SessionStatus::Busy, 1),
            mk(SessionStatus::Waiting, 2),
            mk(SessionStatus::Busy, 5),
        ];
        sort_sessions(&mut v);
        let got: Vec<_> = v.iter().map(|s| (s.status, s.since_ms)).collect();
        assert_eq!(
            got,
            vec![
                (SessionStatus::Waiting, 2),
                (SessionStatus::Busy, 5),
                (SessionStatus::Busy, 1),
                (SessionStatus::Idle, 9)
            ]
        );
    }

    #[test]
    fn proc_stat_fields_survive_spaces_in_comm() {
        // Real /proc/<pid>/stat head of a claude process, comm altered to
        // carry a space and a paren.
        let stat = "10844 (claude (x) y) S 10800 10844 10800 34817 10844 4194560 1 2 3 4 5 6 7 8 20 0 12 0 3065 1000 99";
        assert_eq!(stat_ppid(stat), Some(10800));
        assert_eq!(stat_start_time(stat), Some(3065));
    }

    #[test]
    fn walks_up_to_the_window_owner() {
        let parents: HashMap<u32, u32> = [(500, 400), (400, 300), (300, 1)].into();
        let windows = [(300, 7u64), (999, 8)];
        let parent = |p: u32| parents.get(&p).copied();
        assert_eq!(window_for_process(500, &windows, parent), Some(7));
        assert_eq!(window_for_process(400, &[(999, 8)], parent), None);
    }

    #[test]
    fn picks_the_most_recent_tmux_client() {
        assert_eq!(
            most_active_client("1793808300 4242\n1793808999 5151\n"),
            Some(5151)
        );
        assert_eq!(most_active_client(""), None);
    }

    #[test]
    fn environ_pairs() {
        let env = parse_environ(b"TMUX=/tmp/tmux-1000/default,123,0\0TMUX_PANE=%17\0BAD\0");
        assert_eq!(env.get("TMUX_PANE").map(String::as_str), Some("%17"));
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn saved_sessions_group_by_id_newest_first() {
        let history = concat!(
            r#"{"display":"first","timestamp":10,"project":"/a","sessionId":"s1"}"#,
            "\n",
            r#"{"display":"other","timestamp":20,"project":"/b","sessionId":"s2"}"#,
            "\n",
            r#"{"display":"second\nline two","timestamp":30,"project":"/a","sessionId":"s1"}"#,
            "\n",
            r#"{"display":"live","timestamp":40,"project":"/c","sessionId":"live"}"#,
            "\n",
        );
        let v = saved_sessions(history, &["live"], 10);
        assert_eq!(v.len(), 2);
        assert_eq!(
            (
                v[0].session_id.as_str(),
                v[0].last_prompt.as_str(),
                v[0].prompts
            ),
            ("s1", "second", 2)
        );
        assert_eq!(v[1].session_id, "s2");
        assert_eq!(saved_sessions(history, &[], 1).len(), 1);
    }

    #[test]
    fn saved_index_reads_only_appended_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let first = concat!(
            r#"{"display":"first","timestamp":10,"project":"/a","sessionId":"s1"}"#,
            "\n",
            r#"{"display":"other","timestamp":20,"project":"/b","sessionId":"s2"}"#,
            "\n",
        );
        std::fs::write(&path, first).unwrap();
        let mut index = SavedIndex::default();
        assert!(index.refresh(&path));
        assert_eq!(index.top(&[], 10)[0].session_id, "s2");
        // Nothing appended: nothing read, nothing changed.
        let before = index.bytes_read;
        assert!(!index.refresh(&path));
        assert_eq!(index.bytes_read, before);
        // One prompt appended: only that line is read, and s1 is newest.
        let line = r#"{"display":"again","timestamp":30,"project":"/a","sessionId":"s1"}"#;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        use std::io::Write;
        writeln!(f, "{line}").unwrap();
        drop(f);
        assert!(index.refresh(&path));
        assert_eq!(index.bytes_read - before, line.len() as u64 + 1);
        let top = index.top(&[], 10);
        assert_eq!(
            (
                top[0].session_id.as_str(),
                top[0].last_prompt.as_str(),
                top[0].prompts
            ),
            ("s1", "again", 2)
        );
        // The same as grouping the whole file at once.
        let whole = std::fs::read_to_string(&path).unwrap();
        assert_eq!(top, saved_sessions(&whole, &[], 10));
        assert_eq!(index.top(&["s1"], 10).len(), 1);
    }

    #[test]
    fn resume_command_keeps_paths_out_of_the_script() {
        let argv = resume_command("/usr/bin/claude", "/tmp/it's here", "abc", "/h/.claude-w");
        assert_eq!(argv[..2], ["sh", "-c"]);
        assert_eq!(
            &argv[argv.len() - 4..],
            ["/tmp/it's here", "/usr/bin/claude", "abc", "/h/.claude-w"]
        );
        assert!(!argv[2].contains("it's"));
        assert!(!argv[2].contains("claude-w"));
    }

    #[test]
    fn full_text_search_over_every_prompt() {
        let history = concat!(
            r#"{"display":"fix the margo bar pill","timestamp":10,"project":"/r/margo","sessionId":"s1"}"#,
            "\n",
            r#"{"display":"unrelated","timestamp":20,"project":"/r/margo","sessionId":"s1"}"#,
            "\n",
            r#"{"display":"Bar colours\nsecond line","timestamp":30,"project":"/r/site","sessionId":"s2"}"#,
            "\n",
            r#"{"display":"the BAR again","timestamp":40,"project":"/r/margo","sessionId":"s1"}"#,
            "\n",
            r#"{"display":"/clear","timestamp":50,"project":"/r/bar","sessionId":"s3"}"#,
            "\n",
            "not json\n",
        );
        let v = search_history(history.as_bytes(), "bar", 10);
        let got: Vec<_> = v
            .iter()
            .map(|s| (s.session_id.as_str(), s.last_prompt.as_str(), s.prompts))
            .collect();
        assert_eq!(got, [("s1", "the BAR again", 2), ("s2", "Bar colours", 1)]);
        // Every word must match, in the prompt itself.
        let v = search_history(history.as_bytes(), "bar colours", 10);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].session_id, "s2");
        assert!(search_history(history.as_bytes(), "bar site", 10).is_empty());
        assert!(search_history(history.as_bytes(), "  ", 10).is_empty());
        assert_eq!(search_history(history.as_bytes(), "bar", 1).len(), 1);
    }

    #[test]
    fn projects_with_their_newest_prompt() {
        let history = concat!(
            r#"{"display":"a","timestamp":10,"project":"/a","sessionId":"s1"}"#,
            "\n",
            r#"{"display":"b","timestamp":50,"project":"/a","sessionId":"s2"}"#,
            "\n",
            r#"{"display":"c","timestamp":20,"project":"/b","sessionId":"s3"}"#,
            "\n",
        );
        let mut index = SavedIndex::default();
        for line in history.lines() {
            index.add_line(line);
        }
        let mut p = index.projects();
        p.sort();
        assert_eq!(p, [(20, "/b".to_string()), (50, "/a".to_string())]);
    }

    #[test]
    fn resume_command_runs_in_sh() {
        // The script really resumes / starts and exports the account dir.
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho \"$PWD|${CLAUDE_CONFIG_DIR:-none}|$*\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = |sid: &str, cfg: &str| {
            let argv = resume_command(fake.to_str().unwrap(), "/tmp", sid, cfg);
            let out = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(run("abc", ""), "/tmp|none|--resume abc");
        assert_eq!(run("", "/h/.claude-w"), "/tmp|/h/.claude-w|");
    }
}
