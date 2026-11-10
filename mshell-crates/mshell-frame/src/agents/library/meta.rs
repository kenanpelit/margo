//! What the Sessions page needs from one transcript, read incrementally
//! (a transcript reaches hundreds of MB; after the first read only the
//! appended lines are read, see [`super::super::jsonl::JsonlTail`]).
//!
//! Only lines that can matter are parsed: the first one with a `cwd`, the
//! title records (`custom-title`, written by `/rename`; `ai-title`), the
//! `relocated` record Claude Code writes when a session moves to another
//! folder, the first `last-prompt`, and user / assistant turns until the
//! first real prompt is seen.

use serde_json::Value;

/// Longest first prompt kept (the row shows one line).
const PROMPT_CHARS: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TranscriptMeta {
    /// The working directory of the first line that records one.
    pub(crate) cwd: String,
    /// The last `relocated` record's folder (the session moved there).
    pub(crate) relocated: String,
    pub(crate) git_branch: String,
    /// The last `/rename` title.
    pub(crate) custom_title: String,
    pub(crate) ai_title: String,
    /// The opening prompt, one line.
    pub(crate) first_prompt: String,
    /// A real prompt (not a slash command or a system note) or an
    /// assistant turn exists: the session is not empty.
    pub(crate) has_prompt: bool,
}

fn top_type(v: &Value) -> &str {
    v.get("type").and_then(Value::as_str).unwrap_or("")
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Wrappers Claude Code puts around what is not a prompt typed to the
/// model (a local slash command, its output, a system note).
fn is_wrapper(s: &str) -> bool {
    let s = s.trim_start();
    [
        "<command-name>",
        "<command-message>",
        "<local-command-",
        "<system-reminder>",
    ]
    .iter()
    .any(|p| s.starts_with(p))
}

/// The prompt text of a user turn, `None` when it isn't one.
fn prompt_of(v: &Value) -> Option<String> {
    if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    match v.pointer("/message/content")? {
        Value::String(s) if !s.trim().is_empty() && !is_wrapper(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let mut any = false;
            for b in blocks {
                match top_type(b) {
                    "text" => {
                        let t = text(b, "text");
                        if !t.trim().is_empty() && !is_wrapper(t) {
                            return Some(t.to_string());
                        }
                    }
                    // A tool result or an image is part of a real exchange.
                    "" => {}
                    _ => any = true,
                }
            }
            any.then(String::new)
        }
        _ => None,
    }
}

fn one_line(s: &str) -> String {
    let line = s.trim().lines().next().unwrap_or("").trim();
    if line.chars().count() > PROMPT_CHARS {
        let cut: String = line.chars().take(PROMPT_CHARS).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

impl TranscriptMeta {
    /// Fold one transcript line in.
    pub(crate) fn add_line(&mut self, line: &str) {
        let want_cwd = self.cwd.is_empty() && line.contains("\"cwd\":\"");
        // Claude Code writes these records as `{type:…, …}` object
        // literals, so `"type"` is their first key: only the line's start
        // is looked at (most lines are long tool output).
        let titled = line.starts_with(r#"{"type":"custom-title""#)
            || line.starts_with(r#"{"type":"ai-title""#)
            || line.starts_with(r#"{"type":"relocated""#)
            || (self.first_prompt.is_empty() && line.starts_with(r#"{"type":"last-prompt""#));
        let turn = !self.has_prompt
            && (line.contains("\"type\":\"user\"") || line.contains("\"type\":\"assistant\""));
        if !(want_cwd || titled || turn) {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if want_cwd {
            let cwd = text(&v, "cwd");
            if !cwd.is_empty() {
                self.cwd = cwd.to_string();
                self.git_branch = text(&v, "gitBranch").to_string();
            }
        }
        match top_type(&v) {
            "custom-title" => self.custom_title = one_line(text(&v, "customTitle")),
            "ai-title" => self.ai_title = one_line(text(&v, "aiTitle")),
            "relocated" => {
                let to = text(&v, "relocatedCwd");
                if !to.is_empty() {
                    self.relocated = to.to_string();
                }
            }
            "last-prompt" if self.first_prompt.is_empty() => {
                self.first_prompt = one_line(text(&v, "lastPrompt"));
            }
            "assistant" => self.has_prompt = true,
            "user" => {
                if let Some(p) = prompt_of(&v) {
                    self.has_prompt = true;
                    let p = one_line(&p);
                    if !p.is_empty() {
                        // The typed prompt beats a `last-prompt` record.
                        self.first_prompt = p;
                    }
                }
            }
            _ => {}
        }
    }

    /// The session's working directory: where it moved, else where it
    /// started.
    pub(crate) fn effective_cwd(&self) -> &str {
        if self.relocated.is_empty() {
            &self.cwd
        } else {
            &self.relocated
        }
    }

    /// What the row is called: the `/rename` title, the AI title, the
    /// opening prompt, else `Untitled · <id>`.
    pub(crate) fn title(&self, id: &str) -> String {
        [&self.custom_title, &self.ai_title, &self.first_prompt]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Untitled · {}", id.get(..8).unwrap_or(id)))
    }
}

/// A session id: always a UUID. Anything else is never built into a path.
pub(crate) fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Lines as Claude Code 2.1.322 writes them on this machine (ids and
    /// prompt text replaced).
    pub(crate) const MODE: &str =
        r#"{"type":"mode","mode":"normal","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64"}"#;
    pub(crate) const CAVEAT: &str = r#"{"parentUuid":"2340da12","isSidechain":false,"type":"user","message":{"role":"user","content":"<local-command-caveat>The command below was run directly in Claude Code, not sent to you as a request.</local-command-caveat>"},"isMeta":true,"uuid":"a1","timestamp":"2026-11-01T10:00:00.000Z","cwd":"/repo/archive/.eng/android","sessionId":"cae70d77-c84d-45b9-b9ef-3a39caf81fcb","gitBranch":"main"}"#;
    pub(crate) const CLEAR: &str = r#"{"parentUuid":"6e120d3e","isSidechain":false,"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>\n            <command-message>clear</command-message>\n            <command-args></command-args>"},"uuid":"c6","timestamp":"2026-11-01T10:00:01.000Z","cwd":"/repo/archive/.eng/android","sessionId":"cae70d77-c84d-45b9-b9ef-3a39caf81fcb"}"#;
    pub(crate) const PROMPT: &str = r#"{"parentUuid":null,"isSidechain":false,"promptId":"0bca","type":"user","message":{"role":"user","content":"Write a short essay on rivers.\nSecond line"},"uuid":"e2","timestamp":"2026-11-05T21:51:24.903Z","permissionMode":"dontAsk","cwd":"/home/kenan","sessionId":"cb2df89f-7319-48d3-9080-266afcf02cd0","gitBranch":""}"#;
    pub(crate) const LAST_PROMPT: &str = r#"{"type":"last-prompt","lastPrompt":"which key combo kept the region?","leafUuid":"7b0d","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64"}"#;
    pub(crate) const AI_TITLE: &str = r#"{"type":"ai-title","aiTitle":"Screenshot key combo","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64"}"#;
    /// The record `/rename` appends (2.1.322: `{type:"custom-title",
    /// customTitle, sessionId}`).
    pub(crate) const CUSTOM_TITLE: &str = r#"{"type":"custom-title","customTitle":"My name","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64"}"#;
    /// The record a `cd` relocation appends (2.1.322: `{type:"relocated",
    /// sessionId, relocatedCwd}`).
    pub(crate) const RELOCATED: &str = r#"{"type":"relocated","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64","relocatedCwd":"/r/new"}"#;

    fn meta(lines: &[&str]) -> TranscriptMeta {
        let mut m = TranscriptMeta::default();
        for l in lines {
            m.add_line(l);
        }
        m
    }

    #[test]
    fn slash_commands_and_notes_are_not_prompts() {
        let m = meta(&[MODE, CAVEAT, CLEAR]);
        assert!(!m.has_prompt, "a /clear-only session is empty");
        assert_eq!(m.cwd, "/repo/archive/.eng/android");
        assert_eq!(m.git_branch, "main");
        assert_eq!(
            m.title("cae70d77-c84d-45b9-b9ef-3a39caf81fcb"),
            "Untitled · cae70d77"
        );
        let m = meta(&[MODE, PROMPT]);
        assert!(m.has_prompt, "a prompt with no reply yet is not empty");
        assert_eq!(m.first_prompt, "Write a short essay on rivers.");
        let assistant = r#"{"type":"assistant","message":{"content":[]},"cwd":"/x"}"#;
        assert!(meta(&[CLEAR, assistant]).has_prompt);
        // A nested "type":"user" inside another record is not a turn.
        let nested =
            r#"{"type":"progress","data":{"message":{"type":"user","message":{"content":"hi"}}}}"#;
        assert!(!meta(&[nested]).has_prompt);
    }

    #[test]
    fn titles_rename_wins_and_the_last_one_counts() {
        let m = meta(&[LAST_PROMPT, AI_TITLE]);
        assert_eq!(m.title("x"), "Screenshot key combo");
        let renamed = CUSTOM_TITLE.replace("My name", "Second");
        let m = meta(&[LAST_PROMPT, AI_TITLE, CUSTOM_TITLE, &renamed]);
        assert_eq!(m.title("x"), "Second");
        assert_eq!(
            meta(&[LAST_PROMPT]).title("x"),
            "which key combo kept the region?"
        );
    }

    #[test]
    fn a_relocation_moves_the_working_directory() {
        let m = meta(&[CAVEAT, RELOCATED]);
        assert_eq!(m.cwd, "/repo/archive/.eng/android");
        assert_eq!(m.effective_cwd(), "/r/new");
        assert_eq!(
            meta(&[CAVEAT]).effective_cwd(),
            "/repo/archive/.eng/android"
        );
    }

    #[test]
    fn uuids_only() {
        assert!(is_uuid("cae70d77-c84d-45b9-b9ef-3a39caf81fcb"));
        assert!(!is_uuid("cae70d77-c84d-45b9-b9ef-3a39caf81fc"));
        assert!(!is_uuid("../../../etc/passwd-aaaa-bbbb-cccc-dd"));
        assert!(!is_uuid("cae70d77xc84d-45b9-b9ef-3a39caf81fcb"));
        assert!(!is_uuid(""));
    }
}
