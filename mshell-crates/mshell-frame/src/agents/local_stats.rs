//! Local Claude Code statistics for today: prompts typed (from
//! `<config>/history.jsonl`) and tokens used (from the session transcripts
//! under `<config>/projects/`).
//!
//! `~/.claude/stats-cache.json` (what the old model-usage plugin read) is
//! only rewritten when Claude Code's `/stats` runs, so it is days stale;
//! the transcripts are the live record. They can be tens of MB, so they
//! are read incrementally: [`StatsCache`] remembers each file's offset and
//! only parses appended lines, and starts over at local midnight.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::limits::parse_iso_ms;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TokenTotals {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
}

impl TokenTotals {
    pub(crate) fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LocalStats {
    pub(crate) prompts: u32,
    pub(crate) sessions: u32,
    pub(crate) tokens: TokenTotals,
    /// Total tokens per model, largest first.
    pub(crate) by_model: Vec<(String, u64)>,
}

/// Prompts typed since `since_ms` in `history.jsonl` (one JSON object per
/// line, appended in time order). Slash commands (`/usage`, `/clear`) are
/// not prompts. Returns `(prompts, distinct sessions)`.
pub(crate) fn count_prompts_since(history: &str, since_ms: i64) -> (u32, u32) {
    let mut prompts = 0;
    let mut sessions = HashSet::new();
    for line in history.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(ts) = v.get("timestamp").and_then(Value::as_i64) else {
            continue;
        };
        if ts < since_ms {
            break;
        }
        let display = v
            .get("display")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if display.is_empty() || is_slash_command(display) {
            continue;
        }
        prompts += 1;
        if let Some(s) = v.get("sessionId").and_then(Value::as_str) {
            sessions.insert(s.to_owned());
        }
    }
    (prompts, sessions.len() as u32)
}

/// `/usage`, `/clear foo` are commands; `/home/x/file fix this` is a prompt
/// that starts with a path.
fn is_slash_command(display: &str) -> bool {
    display
        .strip_prefix('/')
        .and_then(|rest| rest.split_whitespace().next())
        .is_some_and(|cmd| !cmd.is_empty() && !cmd.contains('/'))
}

/// Sums assistant-message token usage from transcript lines. Claude Code
/// writes one line per content block of a message, each repeating the
/// message's `usage`, so lines are de-duplicated on message id + request id.
#[derive(Debug, Default)]
pub(crate) struct UsageAccumulator {
    seen: HashSet<String>,
    totals: TokenTotals,
    by_model: BTreeMap<String, u64>,
}

impl UsageAccumulator {
    pub(crate) fn add_line(&mut self, line: &str, since_ms: i64) {
        // Cheap pre-filter: most lines (user turns, tool results) have no usage.
        if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(ts) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms)
        else {
            return;
        };
        if ts < since_ms {
            return;
        }
        let Some(msg) = v.get("message") else { return };
        let Some(usage) = msg.get("usage") else {
            return;
        };
        let id = msg.get("id").and_then(Value::as_str).unwrap_or("");
        let req = v.get("requestId").and_then(Value::as_str).unwrap_or("");
        if (!id.is_empty() || !req.is_empty()) && !self.seen.insert(format!("{id}:{req}")) {
            return;
        }
        let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
        let t = TokenTotals {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
        };
        self.totals.input += t.input;
        self.totals.output += t.output;
        self.totals.cache_read += t.cache_read;
        self.totals.cache_write += t.cache_write;
        let model = msg
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        // Claude Code marks its own non-model lines "<synthetic>".
        if !model.starts_with('<') {
            *self.by_model.entry(model.to_owned()).or_default() += t.total();
        }
    }

    pub(crate) fn totals(&self) -> TokenTotals {
        self.totals
    }

    pub(crate) fn by_model(&self) -> Vec<(String, u64)> {
        let mut v: Vec<_> = self.by_model.iter().map(|(k, n)| (k.clone(), *n)).collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }
}

/// Incremental reader over every transcript touched today.
#[derive(Debug, Default)]
pub(crate) struct StatsCache {
    day_start: i64,
    /// Bytes consumed per file (always at a line boundary).
    offsets: HashMap<PathBuf, u64>,
    acc: UsageAccumulator,
}

impl StatsCache {
    /// Bring the totals up to date for the day starting at `day_start`
    /// (unix ms, local midnight) and return today's stats.
    pub(crate) fn refresh(&mut self, config_dir: &Path, day_start: i64) -> LocalStats {
        if day_start != self.day_start {
            *self = StatsCache {
                day_start,
                ..StatsCache::default()
            };
        }
        for path in transcripts_modified_since(&config_dir.join("projects"), day_start) {
            let offset = self.offsets.get(&path).copied().unwrap_or(0);
            if let Some(next) =
                read_new_lines(&path, offset, |line| self.acc.add_line(line, day_start))
            {
                self.offsets.insert(path, next);
            }
        }
        let history = std::fs::read_to_string(config_dir.join("history.jsonl")).unwrap_or_default();
        let (prompts, sessions) = count_prompts_since(&history, day_start);
        LocalStats {
            prompts,
            sessions,
            tokens: self.acc.totals(),
            by_model: self.acc.by_model(),
        }
    }
}

/// Feed every complete line after `offset` to `f`; returns the new offset
/// (end of the last complete line). A shrunk file (rewritten) starts over.
fn read_new_lines(path: &Path, offset: u64, mut f: impl FnMut(&str)) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = if len < offset { 0 } else { offset };
    if len == start {
        return Some(start);
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.take(len - start).read_to_end(&mut buf).ok()?;
    let complete = buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    for line in String::from_utf8_lossy(&buf[..complete]).lines() {
        f(line);
    }
    Some(start + complete as u64)
}

/// `projects/<project>/<session>.jsonl` plus subagent transcripts
/// (`projects/<project>/<session>/subagents/*.jsonl`) modified since `since_ms`.
fn transcripts_modified_since(projects: &Path, since_ms: i64) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let fresh = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .is_some_and(|d| d.as_millis() as i64 >= since_ms)
    };
    let Ok(projects) = std::fs::read_dir(projects) else {
        return out;
    };
    for project in projects.flatten() {
        let pdir = project.path();
        // A directory's mtime moves when a file is created in it, not when
        // one is appended to, so it can't prune the walk.
        let Ok(entries) = std::fs::read_dir(&pdir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                if fresh(&p) {
                    out.push(p);
                }
            } else if p.is_dir()
                && let Ok(subs) = std::fs::read_dir(p.join("subagents"))
            {
                out.extend(
                    subs.flatten()
                        .map(|s| s.path())
                        .filter(|s| s.extension().is_some_and(|x| x == "jsonl") && fresh(s)),
                );
            }
        }
    }
    out
}

/// Local midnight as unix ms.
pub(crate) fn local_day_start_ms() -> i64 {
    use chrono::{Local, TimeZone};
    let today = Local::now().date_naive();
    today
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).earliest())
        .map_or(0, |d| d.timestamp_millis())
}

/// `1234` → `"1.2k"`, `3_400_000` → `"3.4M"`.
pub(crate) fn compact_count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real lines from ~/.claude/history.jsonl (prompt text shortened).
    const HISTORY: &str = concat!(
        r#"{"display":"devam et bakalım","pastedContents":{},"timestamp":1793701564241,"project":"/others/tor/dizi/11.22.63","sessionId":"5cd8742e-3267-44e1-ac49-2453da758bbe"}"#,
        "\n",
        r#"{"display":"/others/tor/film buradaki çeviri","pastedContents":{},"timestamp":1793702743582,"project":"/others/tor/dizi/11.22.63","sessionId":"5cd8742e-3267-44e1-ac49-2453da758bbe"}"#,
        "\n",
        r#"{"display":"/usage","pastedContents":{},"timestamp":1793774337285,"project":"/home/kenan/.cachy","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64"}"#,
        "\n",
        r#"{"display":"fix the bar","pastedContents":{},"timestamp":1793774400000,"project":"/home/kenan/.cachy","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64"}"#,
        "\n",
    );

    #[test]
    fn prompts_since_skip_slash_commands_and_older_lines() {
        // "/usage" is a command; "/others/tor/film …" is a prompt that
        // starts with a path.
        assert_eq!(count_prompts_since(HISTORY, 1793774000000), (1, 1));
        assert_eq!(count_prompts_since(HISTORY, 0), (3, 2));
        assert_eq!(count_prompts_since("", 0), (0, 0));
    }

    // A real assistant transcript line (2026-10-30, trimmed of content).
    const ASSISTANT: &str = r#"{"parentUuid":"x","isSidechain":false,"type":"assistant","timestamp":"2026-10-30T21:21:58.119Z","sessionId":"9f9ccc81-8d2d-461e-ad48-637aa098ba64","requestId":"req_011CgZ86FNrzkHnRtAPeJZtP","message":{"id":"msg_011CgZ86FczLmznvncpz5AqA","type":"message","role":"assistant","model":"claude-sonnet-5","content":[],"usage":{"input_tokens":2,"cache_creation_input_tokens":206,"cache_read_input_tokens":377011,"output_tokens":275,"service_tier":"standard"}}}"#;

    #[test]
    fn usage_is_summed_once_per_message() {
        let since = parse_iso_ms("2026-10-30T00:00:00Z").unwrap();
        let mut acc = UsageAccumulator::default();
        acc.add_line(ASSISTANT, since);
        acc.add_line(ASSISTANT, since); // second content block of the same message
        acc.add_line(
            r#"{"type":"user","timestamp":"2026-10-30T21:21:42.590Z","message":{"role":"user"}}"#,
            since,
        );
        acc.add_line("not json", since);
        let t = acc.totals();
        assert_eq!(
            t,
            TokenTotals {
                input: 2,
                output: 275,
                cache_read: 377011,
                cache_write: 206
            }
        );
        assert_eq!(
            acc.by_model(),
            vec![("claude-sonnet-5".to_string(), t.total())]
        );
        // Before the day starts → not counted.
        let mut acc = UsageAccumulator::default();
        acc.add_line(ASSISTANT, since + 86_400_000);
        assert_eq!(acc.totals().total(), 0);
    }

    #[test]
    fn cache_reads_only_appended_complete_lines() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects/-home-x");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("s.jsonl");
        std::fs::write(&file, format!("{ASSISTANT}\n")).unwrap();
        let since = parse_iso_ms("2026-10-30T00:00:00Z").unwrap();
        let mut cache = StatsCache::default();
        let s = cache.refresh(dir.path(), since);
        assert_eq!(s.tokens.output, 275);
        // A second message plus a half-written line.
        let second = ASSISTANT.replace("msg_011CgZ86FczLmznvncpz5AqA", "msg_2");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap();
        use std::io::Write;
        write!(f, "{second}\n{{\"type\":\"assis").unwrap();
        assert_eq!(cache.refresh(dir.path(), since).tokens.output, 550);
        // Nothing new → unchanged (the partial line is not counted twice).
        assert_eq!(cache.refresh(dir.path(), since).tokens.output, 550);
        // A new day starts from zero.
        assert_eq!(
            cache.refresh(dir.path(), since + 86_400_000).tokens.output,
            0
        );
    }

    #[test]
    fn compact_counts() {
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1234), "1.2k");
        assert_eq!(compact_count(3_400_000), "3.4M");
    }
}
