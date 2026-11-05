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

use super::jsonl::JsonlTail;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

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

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Every running session under `config_dir`.
pub(crate) fn read_live_sessions(config_dir: &Path) -> Vec<LiveSession> {
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

/// Terminals tried for resuming, in order.
const TERMINALS: &[&str] = &[
    "kitty",
    "foot",
    "alacritty",
    "wezterm",
    "ghostty",
    "konsole",
    "xterm",
];

/// argv that opens `term` in `cwd` running `claude --resume <id>`. The
/// directory and id travel as positional arguments to `sh -c`, never
/// spliced into the script.
pub(crate) fn resume_argv(term: &str, claude: &str, cwd: &str, session_id: &str) -> Vec<String> {
    let script = "cd \"$1\" 2>/dev/null; exec \"$2\" --resume \"$3\"";
    let mut argv: Vec<String> = match term {
        "kitty" | "ghostty" => vec!["-e".into()],
        "foot" => vec![],
        "wezterm" => vec!["start".into(), "--".into()],
        _ => vec!["-e".into()],
    };
    argv.extend(
        ["sh", "-c", script, "sh", cwd, claude, session_id]
            .iter()
            .map(|s| s.to_string()),
    );
    argv
}

/// `claude` on PATH, else the installer's `~/.local/bin/claude` (mshell
/// runs under systemd and may not see the login shell's PATH).
fn claude_binary() -> Option<String> {
    margo_fs::path::find_in_path("claude")
        .or_else(|| {
            let p = margo_fs::xdg::home()?.join(".local/bin/claude");
            p.is_file().then_some(p)
        })
        .map(|p| p.to_string_lossy().into_owned())
}

/// Open a terminal resuming `session_id` in `cwd`. Errors are a short
/// user-facing reason.
pub(crate) fn resume(session_id: &str, cwd: &str) -> Result<(), String> {
    use mshell_process::SpawnReaped;
    let claude = claude_binary().ok_or("claude is not installed")?;
    let term = TERMINALS
        .iter()
        .find(|t| margo_fs::path::on_path(t))
        .ok_or("no terminal emulator found")?;
    // In its own scope: the resumed session must not die with an mshell
    // restart (a plain child is killed with mshell.service).
    mshell_process::outliving_shell(term, resume_argv(term, &claude, cwd, session_id), "claude")
        .spawn_reaped()
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
    fn resume_argv_keeps_paths_out_of_the_script() {
        let argv = resume_argv("kitty", "/usr/bin/claude", "/tmp/it's here", "abc");
        assert_eq!(argv[0], "-e");
        assert_eq!(
            &argv[argv.len() - 3..],
            ["/tmp/it's here", "/usr/bin/claude", "abc"]
        );
        assert!(!argv[3].contains("it's"));
        assert_eq!(resume_argv("foot", "c", "/", "x")[0], "sh");
        assert_eq!(&resume_argv("wezterm", "c", "/", "x")[..2], ["start", "--"]);
    }
}
