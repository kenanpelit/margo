//! What a running Claude Code session is doing, from its transcript
//! (`<config>/projects/<munged cwd>/<session id>.jsonl`): the AI title,
//! model, permission mode, git branch, the tool it is running
//! (`Bash: npm test`), todo progress, context size, Claude's last reply
//! and the tokens it billed (per model, for the cost estimate; subagent
//! transcripts under `<session id>/subagents/` included).
//!
//! Transcripts reach hundreds of MB, so each is read once from the start
//! and then only the appended lines ([`super::jsonl::JsonlTail`]), and a
//! line is only parsed when a cheap substring test says it is one of the
//! few kinds folded here.

use super::jsonl::JsonlTail;
use super::pricing::{BilledTokens, PriceTable};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Longest kept reply, characters.
const MAX_REPLY: usize = 4000;
/// Longest tool argument shown, characters.
const MAX_TOOL_ARG: usize = 120;

/// The transcript of session `sid` started in `cwd` under `config_dir`:
/// Claude Code turns every non-alphanumeric byte of the cwd into `-`.
pub(crate) fn transcript_path(config_dir: &Path, cwd: &str, sid: &str) -> PathBuf {
    let munged: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    config_dir
        .join("projects")
        .join(munged)
        .join(format!("{sid}.jsonl"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Todos {
    pub(crate) done: u32,
    pub(crate) total: u32,
}

/// The folded state of one transcript.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SessionDetail {
    pub(crate) title: String,
    /// `opus-5-5` (the `claude-` prefix dropped).
    pub(crate) model: String,
    /// The full model id, for pricing and the context window.
    pub(crate) model_id: String,
    pub(crate) mode: String,
    pub(crate) branch: String,
    /// `Bash: npm test` while the last assistant message called a tool.
    pub(crate) tool: String,
    pub(crate) todos: Option<Todos>,
    /// The todo in progress (`activeForm`).
    pub(crate) todo_now: String,
    /// Tokens of the last request (the conversation's current size).
    pub(crate) context: u64,
    /// Claude's last text reply.
    pub(crate) reply: String,
    /// Length of the last finished turn.
    pub(crate) turn_ms: Option<i64>,
    /// Billed tokens per model, this session and its subagents.
    pub(crate) billed: BTreeMap<String, BilledTokens>,
}

impl SessionDetail {
    pub(crate) fn cost_usd(&self, prices: &PriceTable) -> Option<f64> {
        prices.cost(self.billed.iter().map(|(m, t)| (m.as_str(), t)))
    }

    /// Context use as a fraction of the model's window, when known.
    pub(crate) fn context_fraction(&self, prices: &PriceTable) -> Option<f64> {
        let window = prices.price(&self.model_id)?.context? as f64;
        (self.context > 0 && window > 0.0).then(|| self.context as f64 / window)
    }
}

/// `Bash` + `{command: "npm test"}` → `Bash: npm test`.
pub(crate) fn tool_text(name: &str, input: Option<&Value>) -> String {
    let arg = input
        .and_then(|i| {
            [
                "command",
                "file_path",
                "pattern",
                "description",
                "url",
                "query",
                "prompt",
            ]
            .iter()
            .find_map(|k| i.get(*k).and_then(Value::as_str))
        })
        .map(one_line)
        .unwrap_or_default();
    if arg.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {arg}")
    }
}

/// Whitespace runs folded to one space, cut to [`MAX_TOOL_ARG`] chars.
fn one_line(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&joined, MAX_TOOL_ARG)
}

fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Folds transcript lines into a [`SessionDetail`].
#[derive(Debug, Default)]
pub(crate) struct DetailFold {
    pub(crate) detail: SessionDetail,
    /// Message ids already billed (a message is one line per content block,
    /// each repeating the usage).
    seen: HashSet<String>,
    /// The message whose text blocks make up `detail.reply`.
    reply_id: String,
}

impl DetailFold {
    pub(crate) fn add_line(&mut self, line: &str) {
        let interesting = line.contains("\"type\":\"assistant\"")
            || line.contains("\"type\":\"ai-title\"")
            || line.contains("\"type\":\"permission-mode\"")
            || line.contains("\"subtype\":\"turn_duration\"");
        if !interesting {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let d = &mut self.detail;
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
        match text("type") {
            "ai-title" if !text("aiTitle").is_empty() => d.title = one_line(text("aiTitle")),
            "permission-mode" if !text("permissionMode").is_empty() => {
                d.mode = text("permissionMode").to_string();
            }
            "system" if text("subtype") == "turn_duration" => {
                d.turn_ms = v.get("durationMs").and_then(Value::as_i64);
            }
            "assistant" => self.add_assistant(&v),
            _ => {}
        }
    }

    fn add_assistant(&mut self, v: &Value) {
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let Some(msg) = v.get("message") else { return };
        let d = &mut self.detail;
        if let Some(b) = v
            .get("gitBranch")
            .and_then(Value::as_str)
            .filter(|b| !b.is_empty())
        {
            d.branch = b.to_string();
        }
        let id = msg.get("id").and_then(Value::as_str).unwrap_or("");
        let model = msg.get("model").and_then(Value::as_str).unwrap_or("");
        if let Some(usage) = msg.get("usage")
            && !model.starts_with('<')
        {
            let t = BilledTokens::from_usage(usage);
            d.context = t.total();
            if !model.is_empty() {
                d.model_id = model.to_string();
                d.model = model.strip_prefix("claude-").unwrap_or(model).to_string();
            }
            let req = v.get("requestId").and_then(Value::as_str).unwrap_or("");
            if (id.is_empty() && req.is_empty()) || self.seen.insert(format!("{id}:{req}")) {
                d.billed.entry(model.to_string()).or_default().add(&t);
            }
        }
        let Some(blocks) = msg.get("content").and_then(Value::as_array) else {
            return;
        };
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let input = b.get("input");
                    if name == "TodoWrite"
                        && let Some(todos) =
                            input.and_then(|i| i.get("todos")).and_then(Value::as_array)
                    {
                        let status = |t: &Value| {
                            t.get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string()
                        };
                        let done = todos.iter().filter(|t| status(t) == "completed").count();
                        d.todos = (!todos.is_empty()).then_some(Todos {
                            done: done as u32,
                            total: todos.len() as u32,
                        });
                        d.todo_now = todos
                            .iter()
                            .find(|t| status(t) == "in_progress")
                            .and_then(|t| {
                                t.get("activeForm")
                                    .or_else(|| t.get("content"))
                                    .and_then(Value::as_str)
                            })
                            .map(one_line)
                            .unwrap_or_default();
                    }
                    d.tool = tool_text(name, input);
                }
                Some("text") => {
                    let t = b.get("text").and_then(Value::as_str).unwrap_or("").trim();
                    if t.is_empty() {
                        continue;
                    }
                    if self.reply_id != id || id.is_empty() {
                        self.reply_id = id.to_string();
                        d.reply.clear();
                    } else if !d.reply.is_empty() {
                        d.reply.push_str("\n\n");
                    }
                    d.reply.push_str(t);
                    d.reply = cut(&d.reply, MAX_REPLY);
                    // A reply ends the tool call before it.
                    d.tool.clear();
                }
                _ => {}
            }
        }
    }
}

/// Bills only (subagent transcripts add to the session's cost).
#[derive(Debug, Default)]
struct BillFold {
    seen: HashSet<String>,
    billed: BTreeMap<String, BilledTokens>,
}

impl BillFold {
    fn add_line(&mut self, line: &str) {
        if !line.contains("\"usage\"") || !line.contains("\"type\":\"assistant\"") {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let Some(msg) = v.get("message") else { return };
        let (Some(usage), Some(model)) =
            (msg.get("usage"), msg.get("model").and_then(Value::as_str))
        else {
            return;
        };
        if model.starts_with('<') {
            return;
        }
        let id = msg.get("id").and_then(Value::as_str).unwrap_or("");
        let req = v.get("requestId").and_then(Value::as_str).unwrap_or("");
        if (id.is_empty() && req.is_empty()) || self.seen.insert(format!("{id}:{req}")) {
            self.billed
                .entry(model.to_string())
                .or_default()
                .add(&BilledTokens::from_usage(usage));
        }
    }
}

/// One session's transcript (and its subagents'), read incrementally.
#[derive(Debug, Default)]
pub(crate) struct DetailTracker {
    path: PathBuf,
    tail: JsonlTail,
    fold: DetailFold,
    subagents: HashMap<PathBuf, (JsonlTail, BillFold)>,
    stat: Option<(u64, std::time::SystemTime)>,
}

impl DetailTracker {
    /// Bring the detail up to date with `path`; returns it.
    pub(crate) fn refresh(&mut self, path: &Path) -> SessionDetail {
        if self.path != path {
            *self = DetailTracker {
                path: path.to_path_buf(),
                ..DetailTracker::default()
            };
        }
        let stat = std::fs::metadata(path)
            .ok()
            .map(|m| (m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
        if stat.is_some() && stat != self.stat {
            self.stat = stat;
            if self.tail.would_restart(path) {
                // A first read (or a replaced file) starts from nothing.
                self.fold = DetailFold::default();
            }
            let fold = &mut self.fold;
            let _ = self.tail.read(path, None, |line| fold.add_line(line));
        }
        let dir = path.with_extension("").join("subagents");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for p in entries.flatten().map(|e| e.path()) {
                if p.extension().is_some_and(|x| x == "jsonl") {
                    let (tail, fold) = self.subagents.entry(p.clone()).or_default();
                    let _ = tail.read(&p, None, |line| fold.add_line(line));
                }
            }
        }
        let mut d = self.fold.detail.clone();
        for (_, f) in self.subagents.values() {
            for (m, t) in &f.billed {
                d.billed.entry(m.clone()).or_default().add(t);
            }
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::pricing::test_table;

    // Real lines from a claude-opus-5-5 transcript (2026-11-05), content
    // and ids shortened; the TodoWrite line follows the tool's input
    // schema (no transcript on this machine has one).
    const TITLE: &str = r#"{"type":"ai-title","aiTitle":"Tuş bileşimi   ekran görüntüsü sorunu","sessionId":"9f9c"}"#;
    const MODE: &str =
        r#"{"type":"permission-mode","permissionMode":"bypassPermissions","sessionId":"9f9c"}"#;
    const TOOL: &str = r#"{"parentUuid":"a","isSidechain":false,"message":{"id":"msg_1","model":"claude-opus-5-5","role":"assistant","type":"message","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"git push 2>&1","description":"Push all new commits"}}],"usage":{"input_tokens":2,"cache_creation_input_tokens":322,"cache_read_input_tokens":376689,"output_tokens":123,"cache_creation":{"ephemeral_1h_input_tokens":322,"ephemeral_5m_input_tokens":0}}},"requestId":"req_1","type":"assistant","timestamp":"2026-11-05T20:16:00.000Z","gitBranch":"margo"}"#;
    const TEXT: &str = r#"{"parentUuid":"b","isSidechain":false,"message":{"id":"msg_2","model":"claude-opus-5-5","role":"assistant","type":"message","content":[{"type":"text","text":"Pushed both commits."}],"stop_reason":"end_turn","usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":377000,"output_tokens":20}},"requestId":"req_2","type":"assistant","timestamp":"2026-11-05T20:16:10.000Z","gitBranch":"margo"}"#;
    const TURN: &str = r#"{"isSidechain":false,"type":"system","subtype":"turn_duration","durationMs":206236,"messageCount":169,"timestamp":"2026-10-01T11:58:37.197Z"}"#;
    const TODO: &str = r#"{"isSidechain":false,"message":{"id":"msg_3","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"Read code","status":"completed","activeForm":"Reading code"},{"content":"Fix bug","status":"in_progress","activeForm":"Fixing the bug"},{"content":"Test","status":"pending","activeForm":"Testing"}]}}]},"type":"assistant"}"#;

    fn fold(lines: &[&str]) -> SessionDetail {
        let mut f = DetailFold::default();
        for l in lines {
            f.add_line(l);
        }
        f.detail
    }

    #[test]
    fn transcript_path_munges_the_cwd() {
        assert_eq!(
            transcript_path(Path::new("/h/.claude"), "/home/kenan/.cachy", "abc"),
            Path::new("/h/.claude/projects/-home-kenan--cachy/abc.jsonl")
        );
    }

    #[test]
    fn folds_title_mode_model_tool_and_context() {
        let d = fold(&[TITLE, MODE, TOOL]);
        assert_eq!(d.title, "Tuş bileşimi ekran görüntüsü sorunu");
        assert_eq!(d.mode, "bypassPermissions");
        assert_eq!(d.model, "opus-5-5");
        assert_eq!(d.branch, "margo");
        assert_eq!(d.tool, "Bash: git push 2>&1");
        assert_eq!(d.context, 2 + 322 + 376689 + 123);
        // The reply that follows ends the tool call.
        let d = fold(&[TITLE, MODE, TOOL, TEXT, TURN]);
        assert_eq!(d.tool, "");
        assert_eq!(d.reply, "Pushed both commits.");
        assert_eq!(d.turn_ms, Some(206236));
    }

    #[test]
    fn bills_each_message_once_and_prices_it() {
        // The same message repeated (one line per content block).
        let d = fold(&[TOOL, TOOL, TEXT]);
        let t = d.billed["claude-opus-5-5"];
        assert_eq!(t.output, 123 + 20);
        assert_eq!(t.cache_write_1h, 322);
        let cost = d.cost_usd(&test_table()).unwrap();
        assert!(cost > 0.15 && cost < 0.16, "{cost}");
        let frac = d.context_fraction(&test_table()).unwrap();
        assert!((frac - 0.377021).abs() < 1e-6);
    }

    #[test]
    fn todo_progress_and_the_current_todo() {
        let d = fold(&[TODO]);
        assert_eq!(d.todos, Some(Todos { done: 1, total: 3 }));
        assert_eq!(d.todo_now, "Fixing the bug");
        assert_eq!(d.tool, "TodoWrite");
    }

    #[test]
    fn a_reply_spanning_lines_of_one_message_is_joined() {
        let second = TEXT.replace("Pushed both commits.", "Done.");
        let d = fold(&[TEXT, &second]);
        assert_eq!(d.reply, "Pushed both commits.\n\nDone.");
        let next = TEXT
            .replace("msg_2", "msg_9")
            .replace("Pushed both commits.", "New turn.");
        assert_eq!(fold(&[TEXT, &next]).reply, "New turn.");
    }

    #[test]
    fn tool_text_picks_the_telling_argument() {
        let i: Value = serde_json::from_str(r#"{"file_path":"/src/app.ts","old":"x"}"#).unwrap();
        assert_eq!(tool_text("Edit", Some(&i)), "Edit: /src/app.ts");
        assert_eq!(tool_text("Task", None), "Task");
        let long = format!(r#"{{"command":"{}"}}"#, "a ".repeat(200));
        let i: Value = serde_json::from_str(&long).unwrap();
        assert!(tool_text("Bash", Some(&i)).ends_with('…'));
    }

    #[test]
    fn tracker_reads_appends_and_subagents() {
        let dir = tempfile::tempdir().unwrap();
        let path = transcript_path(dir.path(), "/w", "s1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{TITLE}\n{TOOL}\n")).unwrap();
        let mut t = DetailTracker::default();
        let d = t.refresh(&path);
        assert_eq!(d.tool, "Bash: git push 2>&1");
        assert_eq!(d.billed["claude-opus-5-5"].output, 123);
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{TEXT}").unwrap();
        drop(f);
        let d = t.refresh(&path);
        assert_eq!(
            (d.tool.as_str(), d.reply.as_str()),
            ("", "Pushed both commits.")
        );
        assert_eq!(d.billed["claude-opus-5-5"].output, 143);
        // A subagent's bill adds to the session's.
        let sub = path.with_extension("").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let agent = TOOL.replace("msg_1", "msg_a").replace("req_1", "req_a");
        std::fs::write(sub.join("agent-1.jsonl"), format!("{agent}\n")).unwrap();
        assert_eq!(t.refresh(&path).billed["claude-opus-5-5"].output, 266);
        // Unchanged → the same detail.
        assert_eq!(t.refresh(&path).billed["claude-opus-5-5"].output, 266);
    }
}
