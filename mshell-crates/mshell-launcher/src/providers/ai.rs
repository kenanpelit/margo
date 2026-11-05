//! `ai <question>` — ask Claude Code from the launcher and read the
//! answer in place (port of the `ask-claude` Noctalia plugin).
//!
//! Typing never runs anything: Enter on the **Ask Claude** row starts
//! `claude -p` (the launcher stays open), the row then streams the answer
//! (the preview pane shows all of it), and Enter on the finished answer
//! continues the same Claude Code session (`claude --resume <id>`) in a
//! new terminal or a new window of the most recently used tmux session.
//!
//! The ask runs locked down: `--safe-mode` (no CLAUDE.md, memory, skills,
//! hooks or plugins), `--strict-mcp-config` + `--disallowedTools mcp__*`,
//! `--permission-mode dontAsk --permission-prompts none`, and no tools at
//! all unless web lookups are allowed (then only WebSearch / WebFetch).
//! The question goes in on stdin, never in argv, and is capped at
//! [`MAX_QUESTION`] characters; the ask is killed after the configured
//! timeout, and a new ask kills the one still running. A session that
//! used the web resumes with `--permission-mode default`, so every tool
//! call after a possibly prompt-injected page needs your approval.

use crate::item::LauncherItem;
use crate::notify::toast;
use crate::preview::LauncherPreview;
use crate::provider::{Provider, RefreshNotifier};
use mshell_process::SpawnReaped;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PREFIX: &str = "ai";
const NAME: &str = "Ask Claude";
pub const MAX_QUESTION: usize = 8000;
/// Longest answer kept, characters.
const MAX_ANSWER: usize = 30_000;
/// Most source URLs listed.
const MAX_SOURCES: usize = 5;
/// The row is repainted at most this often while the answer streams.
const REPAINT: Duration = Duration::from_millis(150);

const NOTE: &str = "Your answer is shown as plain text in a small desktop launcher window. \
Reply in the language of the question. Lead with the direct answer; add detail only if the \
question needs it. No Markdown: no headings, bold, tables or code fences. Plain lines and \
simple '-' lists only.";

/// Where Enter on a finished answer continues the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AskTarget {
    #[default]
    Terminal,
    /// A new window in the most recently active tmux session (a new
    /// terminal when no tmux server runs).
    Tmux,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSettings {
    /// `sonnet`, `haiku`, `opus`, or empty for Claude Code's default.
    pub model: String,
    /// Allow WebSearch / WebFetch (otherwise no tools at all).
    pub web: bool,
    pub timeout_s: u64,
    pub target: AskTarget,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            model: "sonnet".into(),
            web: false,
            timeout_s: 120,
            target: AskTarget::Terminal,
        }
    }
}

/// `claude -p` arguments for one ask (the question goes on stdin).
pub fn ask_args(s: &AiSettings) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--safe-mode",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--disallowedTools",
        "mcp__*",
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--append-system-prompt",
        NOTE,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if matches!(s.model.as_str(), "sonnet" | "haiku" | "opus") {
        a.extend(["--model".into(), s.model.clone()]);
    }
    if s.web {
        a.extend([
            "--tools".into(),
            "WebSearch,WebFetch".into(),
            "--allowedTools".into(),
            "WebSearch,WebFetch".into(),
        ]);
    } else {
        a.extend(["--tools".into(), String::new()]);
    }
    a
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskStatus {
    Thinking,
    Done,
    Failed,
}

/// One ask as the stream reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub question: String,
    pub status: AskStatus,
    pub text: String,
    /// The tool running right now (`WebSearch`).
    pub tool: Option<String>,
    pub session_id: Option<String>,
    /// It used the web (resumes with approval required).
    pub web: bool,
    pub sources: Vec<String>,
    pub error: Option<String>,
}

impl Ask {
    pub fn new(question: &str) -> Self {
        Ask {
            question: question.to_string(),
            status: AskStatus::Thinking,
            text: String::new(),
            tool: None,
            session_id: None,
            web: false,
            sources: Vec::new(),
            error: None,
        }
    }

    fn fail(&mut self, msg: &str) {
        self.status = AskStatus::Failed;
        let msg = clean(msg.trim());
        self.error = Some(if is_auth_error(&msg) {
            "Claude Code isn't signed in — run `claude` and log in".into()
        } else if msg.is_empty() {
            "Claude Code gave no answer".into()
        } else {
            cut(&msg, 400)
        });
    }

    /// Fold one `stream-json` line in.
    pub fn apply_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        if let Some(sid) = text("session_id").filter(|s| is_session_id(s)) {
            self.session_id = Some(sid.to_string());
        }
        match text("type") {
            Some("stream_event") if self.status == AskStatus::Thinking => {
                let Some(ev) = v.get("event") else { return };
                match ev.get("type").and_then(Value::as_str) {
                    Some("content_block_start") => {
                        let b = ev.get("content_block");
                        let kind = b.and_then(|b| b.get("type")).and_then(Value::as_str);
                        match kind {
                            Some("tool_use" | "server_tool_use") => {
                                let name = b
                                    .and_then(|b| b.get("name"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("tool");
                                self.web |= is_web_tool(name);
                                self.tool = Some(clean(name));
                            }
                            Some("text") => {
                                self.tool = None;
                                if !self.text.is_empty() {
                                    self.text.push_str("\n\n");
                                }
                            }
                            _ => {}
                        }
                    }
                    Some("content_block_delta") => {
                        if let Some(t) = ev
                            .get("delta")
                            .filter(|d| d.get("type").and_then(Value::as_str) == Some("text_delta"))
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            && self.text.chars().count() < MAX_ANSWER
                        {
                            self.text.push_str(t);
                        }
                    }
                    _ => {}
                }
            }
            Some("assistant") => {
                let blocks = v
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for b in blocks {
                    let kind = b.get("type").and_then(Value::as_str);
                    let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                    if matches!(kind, Some("tool_use" | "server_tool_use")) && is_web_tool(name) {
                        self.web = true;
                        if let Some(url) = b.pointer("/input/url").and_then(Value::as_str)
                            && self.sources.len() < MAX_SOURCES
                            && !self.sources.iter().any(|s| s == url)
                        {
                            self.sources.push(clean(url));
                        }
                    }
                }
            }
            Some("result") => {
                let failed = v.get("is_error").and_then(Value::as_bool) == Some(true)
                    || text("subtype").is_some_and(|s| s != "success");
                if failed {
                    self.fail(text("result").unwrap_or(""));
                } else {
                    if let Some(r) = text("result").filter(|r| !r.is_empty()) {
                        self.text = r.to_string();
                    }
                    self.status = AskStatus::Done;
                    self.tool = None;
                }
            }
            _ => {}
        }
    }

    /// The process ended (`code`; `None` = killed by the timeout or a
    /// newer ask). A stream that never sent its result is an error.
    pub fn finish(&mut self, code: Option<i32>, timed_out: bool, stderr: &str, timeout_s: u64) {
        if self.status != AskStatus::Thinking {
            return;
        }
        if timed_out {
            self.fail(&format!("No answer within {timeout_s} s"));
        } else if code == Some(0) && !self.text.is_empty() {
            self.status = AskStatus::Done;
        } else if !stderr.trim().is_empty() {
            self.fail(stderr);
        } else {
            self.fail(&format!(
                "claude exited with {}",
                code.map_or("a signal".into(), |c| c.to_string())
            ));
        }
    }

    /// The answer as shown: Markdown stripped, control characters
    /// dropped, sources not already quoted listed under it.
    pub fn body(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        let mut body = strip_markdown(&clean(&self.text));
        if body.is_empty() {
            body = match &self.tool {
                Some(t) => format!("Using {t}…"),
                None => "Thinking…".into(),
            };
        }
        let extra: Vec<&String> = self
            .sources
            .iter()
            .filter(|s| !self.text.contains(s.as_str()))
            .collect();
        if !extra.is_empty() {
            body.push_str("\n\nSources:");
            for s in extra {
                body.push_str(&format!("\n- {s}"));
            }
        }
        body
    }
}

fn is_web_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "websearch" | "webfetch" | "web_search" | "web_fetch"
    )
}

fn is_auth_error(s: &str) -> bool {
    s.contains("Please run /login")
        || s.contains("API Error: 401")
        || s.contains("authentication_error")
}

/// A UUID, the only shape a session id passed to `--resume` may have.
pub fn is_session_id(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Drop ANSI escapes and control characters (newlines and tabs stay).
pub fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c.is_control() && c != '\n' && c != '\t' {
            continue;
        }
        out.push(c);
    }
    out
}

/// The Markdown a plain-text pane shouldn't show: fences, heading marks,
/// bold, inline code ticks; `[t](url)` becomes `t (url)`.
pub fn strip_markdown(s: &str) -> String {
    let mut lines = Vec::new();
    for line in s.lines() {
        if line.trim_start().starts_with("```") {
            continue;
        }
        let l = line.trim_start_matches('#').trim_start_matches(' ');
        let l = if line.starts_with('#') { l } else { line };
        lines.push(l.replace("**", "").replace("__", "").replace('`', ""));
    }
    let joined = lines.join("\n");
    let re = regex::Regex::new(r"\[([^\]\n]+)\]\((https?://[^)\s]+)\)").ok();
    match re {
        Some(re) => re.replace_all(&joined, "$1 ($2)").into_owned(),
        None => joined,
    }
}

fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// The question after the `ai` prefix, or `None` when it isn't ours.
pub fn question_of(query: &str) -> Option<String> {
    let q = query.trim_start();
    let rest = q.strip_prefix(PREFIX)?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim().to_string())
}

/// `claude` on PATH, else the installer's `~/.local/bin/claude` (a
/// graphical session's PATH often lacks it).
fn claude_binary() -> Option<PathBuf> {
    margo_fs::path::find_in_path("claude").or_else(|| {
        let p = margo_fs::xdg::home()?.join(".local/bin/claude");
        p.is_file().then_some(p)
    })
}

/// The command (run in a terminal) that continues `sid` in `cwd`; `web`
/// asks for approval of every tool call. Arguments travel positionally.
pub fn resume_command(claude: &str, cwd: &str, sid: &str, web: bool) -> Vec<String> {
    let script = "cd \"$1\" 2>/dev/null || cd; \
                  if [ \"$4\" = 1 ]; then exec \"$2\" --resume \"$3\" --permission-mode default; \
                  else exec \"$2\" --resume \"$3\"; fi";
    let web = if web { "1" } else { "0" };
    ["sh", "-c", script, "sh", cwd, claude, sid, web]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// The tmux session to open a window in: the one of the most recently
/// active client (`list-clients -F '#{client_activity} #{session_name}'`).
pub fn most_active_session(list_clients: &str) -> Option<String> {
    list_clients
        .lines()
        .filter_map(|l| {
            let (act, name) = l.trim().split_once(' ')?;
            Some((act.parse::<u64>().ok()?, name.to_string()))
        })
        .max_by_key(|(act, _)| *act)
        .map(|(_, name)| name)
}

/// Continue `ask`'s session where `target` says. Blocking; errors are a
/// short user-facing reason.
fn continue_session(ask: &Ask, target: AskTarget) -> Result<(), String> {
    let sid = ask
        .session_id
        .as_deref()
        .filter(|s| is_session_id(s))
        .ok_or("This answer has no session to continue")?;
    let claude = claude_binary().ok_or("claude is not installed")?;
    let claude = claude.to_string_lossy();
    let home = margo_fs::xdg::home().unwrap_or_default();
    let cwd = home.to_string_lossy();
    let argv = resume_command(&claude, &cwd, sid, ask.web);
    if target == AskTarget::Tmux && margo_fs::path::on_path("tmux") {
        let clients = Command::new("tmux")
            .args(["list-clients", "-F", "#{client_activity} #{session_name}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        if let Some(sess) = clients.as_deref().and_then(most_active_session) {
            let mut cmd = Command::new("tmux");
            cmd.args(["new-window", "-t", &format!("{sess}:"), "-n", "AI", "--"])
                .args(&argv);
            if cmd.status().is_ok_and(|s| s.success()) {
                return Ok(());
            }
        }
    }
    let (term, mut cmd) =
        mshell_process::terminal::command(&argv, "claude").map_err(|e| e.to_string())?;
    cmd.spawn_reaped()
        .map(|_| ())
        .map_err(|e| format!("{term}: {e}"))
}

// ── The provider ───────────────────────────────────────────────────────

pub type AiSettingsResolver = Box<dyn Fn() -> AiSettings>;

#[derive(Default)]
struct Shared {
    ask: Option<Ask>,
    /// The running `claude`, to kill on a newer ask or the timeout.
    child: Option<Child>,
}

pub struct AiProvider {
    shared: Arc<Mutex<Shared>>,
    generation: Arc<AtomicU64>,
    notifier: Option<RefreshNotifier>,
    settings: AiSettingsResolver,
}

impl Default for AiProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AiProvider {
    pub fn new() -> Self {
        Self {
            shared: Arc::default(),
            generation: Arc::default(),
            notifier: None,
            settings: Box::new(AiSettings::default),
        }
    }

    pub fn with_settings_resolver(mut self, f: AiSettingsResolver) -> Self {
        self.settings = f;
        self
    }

    fn current(&self) -> Option<Ask> {
        self.shared.lock().ok().and_then(|s| s.ask.clone())
    }
}

fn kill_running(shared: &Mutex<Shared>) {
    if let Ok(mut s) = shared.lock()
        && let Some(mut c) = s.child.take()
    {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Start asking `question` (cancelling any ask still running).
fn start(
    shared: Arc<Mutex<Shared>>,
    generation: Arc<AtomicU64>,
    notifier: Option<RefreshNotifier>,
    question: String,
    settings: AiSettings,
) {
    let gen_now = generation.fetch_add(1, Ordering::SeqCst) + 1;
    kill_running(&shared);
    let mut ask = Ask::new(&question);
    let set = |ask: Ask| {
        if let Ok(mut s) = shared.lock() {
            s.ask = Some(ask);
        }
    };
    if question.chars().count() > MAX_QUESTION {
        ask.fail(&format!(
            "The question is longer than {MAX_QUESTION} characters"
        ));
        set(ask);
        return;
    }
    let Some(claude) = claude_binary() else {
        ask.fail("claude is not installed (Claude Code)");
        set(ask);
        return;
    };
    let home = margo_fs::xdg::home().unwrap_or_else(|| PathBuf::from("/"));
    let spawned = Command::new(&claude)
        .args(ask_args(&settings))
        .current_dir(&home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            ask.fail(&format!("Couldn't start claude: {e}"));
            set(ask);
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(question.as_bytes());
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    set(ask);
    if let Ok(mut s) = shared.lock() {
        s.child = Some(child);
    }
    let _ = std::thread::Builder::new()
        .name("ai-ask".into())
        .spawn(move || {
            let current = || generation.load(Ordering::SeqCst) == gen_now;
            let limit = settings.timeout_s.max(5);
            let deadline = Instant::now() + Duration::from_secs(limit);
            // stderr is drained on its own thread so a chatty claude can't block.
            let err_buf = Arc::new(Mutex::new(String::new()));
            if let Some(mut e) = stderr {
                let buf = err_buf.clone();
                let _ = std::thread::Builder::new()
                    .name("ai-ask-err".into())
                    .spawn(move || {
                        let mut s = String::new();
                        let _ = std::io::Read::read_to_string(&mut e, &mut s);
                        if let Ok(mut b) = buf.lock() {
                            *b = cut(&s, 2000);
                        }
                    });
            }
            // A watchdog kills claude at the deadline (or once superseded).
            let (sh, gen2) = (shared.clone(), generation.clone());
            let _ = std::thread::Builder::new()
                .name("ai-ask-timer".into())
                .spawn(move || {
                    while Instant::now() < deadline && gen2.load(Ordering::SeqCst) == gen_now {
                        std::thread::sleep(Duration::from_millis(200));
                        let done = sh.lock().map(|s| s.child.is_none()).unwrap_or(true);
                        if done {
                            return;
                        }
                    }
                    if gen2.load(Ordering::SeqCst) == gen_now {
                        kill_running(&sh);
                    }
                });
            let mut last_paint = Instant::now();
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    if !current() {
                        return;
                    }
                    if let Ok(mut s) = shared.lock()
                        && let Some(a) = s.ask.as_mut()
                    {
                        a.apply_line(&line);
                    }
                    if last_paint.elapsed() >= REPAINT {
                        last_paint = Instant::now();
                        if let Some(n) = &notifier {
                            n();
                        }
                    }
                }
            }
            if !current() {
                return;
            }
            let timed_out = Instant::now() >= deadline;
            let code = shared
                .lock()
                .ok()
                .and_then(|mut s| s.child.take())
                .and_then(|mut c| c.wait().ok())
                .and_then(|st| st.code());
            // Give the stderr reader a moment to finish.
            std::thread::sleep(Duration::from_millis(50));
            let err = err_buf.lock().map(|b| b.clone()).unwrap_or_default();
            if let Ok(mut s) = shared.lock()
                && let Some(a) = s.ask.as_mut()
            {
                a.finish(code, timed_out, &err, limit);
            }
            if let Some(n) = &notifier {
                n();
            }
        });
}

fn row(id: &str, name: String, description: String, icon: &str) -> LauncherItem {
    LauncherItem {
        id: id.into(),
        name,
        description,
        icon: icon.into(),
        icon_is_path: false,
        score: 400.0,
        provider_name: NAME.into(),
        usage_key: None,
        on_activate: Rc::new(|| {}),
    }
}

/// The first line of `s`, cut for a row subtitle.
fn first_line(s: &str) -> String {
    cut(
        s.lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim(),
        90,
    )
}

impl Provider for AiProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        question_of(query).is_some()
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        self.notifier = Some(notifier);
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![row(
            "ai:palette",
            PREFIX.into(),
            "Ask Claude Code a question".into(),
            "ai-agents-symbolic",
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(q) = question_of(query) else {
            return Vec::new();
        };
        let current = self.current();
        if let Some(a) = current.as_ref().filter(|a| a.question == q || q.is_empty()) {
            let (id, name, icon) = match a.status {
                AskStatus::Thinking => (
                    "ai:state",
                    "Claude is answering…",
                    "content-loading-symbolic",
                ),
                AskStatus::Failed => (
                    "ai:state",
                    "Claude couldn't answer",
                    "dialog-error-symbolic",
                ),
                AskStatus::Done if a.session_id.is_some() => {
                    ("ai:answer", "Claude's answer", "ai-agents-symbolic")
                }
                AskStatus::Done => ("ai:state", "Claude's answer", "ai-agents-symbolic"),
            };
            let mut sub = first_line(&a.body());
            if a.status == AskStatus::Done && a.session_id.is_some() {
                sub = format!("Enter continues in Claude Code · {sub}");
            }
            let mut it = row(id, name.into(), sub, icon);
            if id == "ai:answer" {
                let ask = a.clone();
                let target = (self.settings)().target;
                it.on_activate = Rc::new(move || {
                    let ask = ask.clone();
                    let _ = std::thread::Builder::new()
                        .name("ai-resume".into())
                        .spawn(move || {
                            if let Err(e) = continue_session(&ask, target) {
                                toast(NAME, e);
                            }
                        });
                });
            }
            let mut rows = vec![it];
            if q.is_empty() {
                rows.insert(0, hint());
            }
            return rows;
        }
        if q.is_empty() {
            return vec![hint()];
        }
        let s = (self.settings)();
        let mut it = row(
            "ai:ask",
            format!("Ask Claude: {}", cut(&q, 80)),
            format!(
                "Enter asks {} · {}",
                if s.model.is_empty() {
                    "Claude Code"
                } else {
                    s.model.as_str()
                },
                if s.web {
                    "web lookups allowed"
                } else {
                    "no tools"
                }
            ),
            "ai-agents-symbolic",
        );
        let (shared, generation, notifier) = (
            self.shared.clone(),
            self.generation.clone(),
            self.notifier.clone(),
        );
        it.on_activate = Rc::new(move || {
            start(
                shared.clone(),
                generation.clone(),
                notifier.clone(),
                q.clone(),
                s.clone(),
            );
            if let Some(n) = &notifier {
                n();
            }
        });
        vec![it]
    }

    fn preview(&self, item: &LauncherItem) -> Option<LauncherPreview> {
        match item.id.as_str() {
            "ai:state" | "ai:answer" => {
                let a = self.current()?;
                Some(LauncherPreview::text(a.question.clone(), a.body()))
            }
            _ => Some(LauncherPreview::text(
                item.name.clone(),
                item.description.clone(),
            )),
        }
    }

    fn on_closed(&mut self) {
        // An answer still streaming has nowhere to go.
        let running = self
            .current()
            .is_some_and(|a| a.status == AskStatus::Thinking);
        if running {
            self.generation.fetch_add(1, Ordering::SeqCst);
            kill_running(&self.shared);
            if let Ok(mut s) = self.shared.lock() {
                s.ask = None;
            }
        }
    }
}

fn hint() -> LauncherItem {
    row(
        "ai:hint",
        "ai <question>".into(),
        "Ask Claude Code; Enter on the answer continues the session".into(),
        "dialog-information-symbolic",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `claude -p --output-format stream-json --include-partial-messages`
    /// output (Claude Code 2.1.318, haiku, 2026-11-05), long fields cut.
    const STREAM: &[&str] = &[
        r#"{"type":"system","subtype":"init","cwd":"/tmp","session_id":"27a56d4a-60ee-474a-96f9-15cb4170bd56","tools":[],"mcp_servers":[],"model":"claude-haiku-5-5","permissionMode":"dontAsk"}"#,
        r#"{"type":"system","subtype":"status","status":"requesting","session_id":"27a56d4a-60ee-474a-96f9-15cb4170bd56"}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"session_id":"27a56d4a-60ee-474a-96f9-15cb4170bd56","parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Blue."}},"session_id":"27a56d4a-60ee-474a-96f9-15cb4170bd56","parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"model":"claude-haiku-5-5","id":"msg_011CgkSpPhUd78DeV7PHf3Fg","type":"message","role":"assistant","content":[{"type":"text","text":"Blue."}]},"session_id":"27a56d4a-60ee-474a-96f9-15cb4170bd56"}"#,
        r#"{"type":"stream_event","event":{"type":"message_stop"},"session_id":"27a56d4a-60ee-474a-96f9-15cb4170bd56"}"#,
        r#"{"duration_api_ms":875,"stop_reason":"end_turn","session_id":"27a56d4a-60ee-474a-96f9-15cb4170bd56","total_cost_usd":0.00046258,"is_error":false,"num_turns":1,"subtype":"success","result":"Blue.","type":"result"}"#,
    ];

    #[test]
    fn a_real_stream_ends_in_an_answer_with_a_session() {
        let mut a = Ask::new("sky?");
        for (i, l) in STREAM.iter().enumerate() {
            a.apply_line(l);
            if i == 3 {
                assert_eq!((a.status, a.text.as_str()), (AskStatus::Thinking, "Blue."));
            }
        }
        assert_eq!(a.status, AskStatus::Done);
        assert_eq!(a.body(), "Blue.");
        assert_eq!(
            a.session_id.as_deref(),
            Some("27a56d4a-60ee-474a-96f9-15cb4170bd56")
        );
        assert!(!a.web);
    }

    #[test]
    fn web_tools_mark_the_ask_and_list_sources() {
        let mut a = Ask::new("q");
        a.apply_line(r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"server_tool_use","name":"web_search"}}}"#);
        assert_eq!(a.tool.as_deref(), Some("web_search"));
        assert_eq!(a.body(), "Using web_search…");
        a.apply_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebFetch","input":{"url":"https://kernel.org/x"}}]}}"#);
        a.apply_line(r#"{"type":"result","subtype":"success","is_error":false,"result":"Use **uname -r**."}"#);
        assert!(a.web);
        assert_eq!(
            a.body(),
            "Use uname -r.\n\nSources:\n- https://kernel.org/x"
        );
    }

    #[test]
    fn errors_timeouts_and_login() {
        let mut a = Ask::new("q");
        a.apply_line(r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"API Error: 401 authentication_error"}"#);
        assert_eq!(a.status, AskStatus::Failed);
        assert!(a.body().contains("log in"));
        let mut a = Ask::new("q");
        a.finish(None, true, "", 120);
        assert_eq!(a.body(), "No answer within 120 s");
        let mut a = Ask::new("q");
        a.finish(Some(1), false, "\u{1b}[31mboom\u{1b}[0m\n", 120);
        assert_eq!(a.body(), "boom");
        // A finished answer isn't overwritten by the exit.
        let mut a = Ask::new("q");
        a.apply_line(STREAM[6]);
        a.finish(Some(0), false, "", 120);
        assert_eq!(a.status, AskStatus::Done);
    }

    #[test]
    fn args_are_locked_down_and_the_question_is_not_among_them() {
        let s = AiSettings::default();
        let a = ask_args(&s);
        for flag in [
            "--safe-mode",
            "--strict-mcp-config",
            "--disallowedTools",
            "dontAsk",
            "--permission-prompts",
        ] {
            assert!(a.iter().any(|x| x == flag), "{flag}");
        }
        let tools = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[tools + 1], "");
        assert!(a.windows(2).any(|w| w == ["--model", "sonnet"]));
        let web = ask_args(&AiSettings {
            web: true,
            model: String::new(),
            ..AiSettings::default()
        });
        assert!(
            web.windows(2)
                .any(|w| w == ["--tools", "WebSearch,WebFetch"])
        );
        assert!(!web.iter().any(|x| x == "--model"));
    }

    #[test]
    fn prefix_ids_markdown_and_tmux_sessions() {
        assert_eq!(question_of("ai  how big? ").as_deref(), Some("how big?"));
        assert_eq!(question_of("ai").as_deref(), Some(""));
        assert_eq!(question_of("air"), None);
        assert!(is_session_id("27a56d4a-60ee-474a-96f9-15cb4170bd56"));
        assert!(!is_session_id("27a56d4a; rm -rf ~"));
        assert_eq!(
            strip_markdown("## Title\n```sh\nuname -r\n```\nSee [docs](https://x.org/a)."),
            "Title\nuname -r\nSee docs (https://x.org/a)."
        );
        assert_eq!(
            most_active_session("1793808300 work\n1793808999 kenp\n").as_deref(),
            Some("kenp")
        );
        let argv = resume_command("/bin/claude", "/home/u", "id", true);
        assert_eq!(
            &argv[argv.len() - 4..],
            ["/home/u", "/bin/claude", "id", "1"]
        );
    }

    #[test]
    fn resume_command_runs_in_sh() {
        let dir = std::env::temp_dir().join(format!("mshell-ai-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("claude");
        std::fs::write(&fake, "#!/bin/sh\necho \"$PWD|$*\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = |web| {
            let argv = resume_command(fake.to_str().unwrap(), "/tmp", "abc", web);
            let out = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(run(false), "/tmp|--resume abc");
        assert_eq!(run(true), "/tmp|--resume abc --permission-mode default");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
