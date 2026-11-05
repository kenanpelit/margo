//! Local Claude Code statistics for today: prompts typed (from
//! `<config>/history.jsonl`) and tokens used (from the session transcripts
//! under `<config>/projects/`).
//!
//! `~/.claude/stats-cache.json` (what the old model-usage plugin read) is
//! only rewritten when Claude Code's `/stats` runs, so it is days stale;
//! the transcripts are the live record. They reach hundreds of MB, so they
//! are read incrementally ([`super::jsonl`]): a file's first visit seeks
//! back from its end to the day's first line, later visits only read the
//! appended lines, and local midnight resets the totals but not the
//! offsets, so yesterday's part is never read again. `history.jsonl` is
//! read the same way.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::jsonl::JsonlTail;
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
#[cfg(test)]
pub(crate) fn count_prompts_since(history: &str, since_ms: i64) -> (u32, u32) {
    let mut c = PromptCounter::default();
    for line in history.lines() {
        c.add_line(line, since_ms);
    }
    c.counts()
}

/// Prompts typed since a day start, fed `history.jsonl` lines in order.
#[derive(Debug, Default)]
pub(crate) struct PromptCounter {
    prompts: u32,
    sessions: HashSet<String>,
}

impl PromptCounter {
    pub(crate) fn add_line(&mut self, line: &str, since_ms: i64) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let Some(ts) = v.get("timestamp").and_then(Value::as_i64) else {
            return;
        };
        if ts < since_ms {
            return;
        }
        let display = v
            .get("display")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if display.is_empty() || is_slash_command(display) {
            return;
        }
        self.prompts += 1;
        if let Some(s) = v.get("sessionId").and_then(Value::as_str)
            && !self.sessions.contains(s)
        {
            self.sessions.insert(s.to_owned());
        }
    }

    /// `(prompts, distinct sessions)`.
    pub(crate) fn counts(&self) -> (u32, u32) {
        (self.prompts, self.sessions.len() as u32)
    }
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
    /// Where each transcript touched today was read up to.
    tails: HashMap<PathBuf, JsonlTail>,
    acc: UsageAccumulator,
    history: JsonlTail,
    prompts: PromptCounter,
    /// Bytes read from disk so far (the tests check what a refresh costs).
    pub(crate) bytes_read: u64,
}

impl StatsCache {
    /// Bring the totals up to date for the day starting at `day_start`
    /// (unix ms, local midnight) and return today's stats.
    pub(crate) fn refresh(&mut self, config_dir: &Path, day_start: i64) -> LocalStats {
        if day_start != self.day_start {
            // A new day: the totals start over, but the offsets stay. Every
            // line before them was read before this tick, so it is from an
            // earlier day; reading on from them skips yesterday's part.
            self.day_start = day_start;
            self.acc = UsageAccumulator::default();
            self.prompts = PromptCounter::default();
        }
        let fresh = transcripts_modified_since(&config_dir.join("projects"), day_start);
        // Files not touched today drop out; a later touch scans from the end.
        self.tails.retain(|p, _| fresh.contains(p));
        for path in fresh {
            let tail = self.tails.entry(path.clone()).or_default();
            if let Ok(out) = tail.read(&path, Some(day_start), |line| {
                self.acc.add_line(line, day_start)
            }) {
                // A restarted transcript is re-read from today's first
                // line; the accumulator's message-id set keeps it from
                // counting twice.
                self.bytes_read += out.bytes;
            }
        }
        let mut fresh_count = PromptCounter::default();
        let mut restarted = false;
        let history = config_dir.join("history.jsonl");
        let read = self.history.read(&history, Some(day_start), |line| {
            fresh_count.add_line(line, day_start);
        });
        if let Ok(out) = read {
            self.bytes_read += out.bytes;
            restarted = out.restarted;
        }
        if restarted {
            self.prompts = fresh_count;
        } else {
            self.prompts.prompts += fresh_count.prompts;
            self.prompts.sessions.extend(fresh_count.sessions);
        }
        let (prompts, sessions) = self.prompts.counts();
        LocalStats {
            prompts,
            sessions,
            tokens: self.acc.totals(),
            by_model: self.acc.by_model(),
        }
    }
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

    /// `ASSISTANT` re-stamped at `ts` with its own message id.
    fn assistant_at(ts: &str, id: &str) -> String {
        ASSISTANT
            .replace("2026-10-30T21:21:58.119Z", ts)
            .replace("msg_011CgZ86FczLmznvncpz5AqA", id)
    }

    #[test]
    fn a_big_transcript_is_read_from_todays_first_line_and_midnight_skips_yesterday() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects/-home-x");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("s.jsonl");
        // ~8 MB from the 29th (a resumed old session), then one line today.
        let mut data = String::new();
        let filler = format!(
            r#"{{"type":"user","timestamp":"2026-10-29T10:00:00.000Z","message":{{"content":"{}"}}}}"#,
            "y".repeat(800)
        );
        while data.len() < 8_000_000 {
            data.push_str(&filler);
            data.push('\n');
        }
        let old_len = data.len() as u64;
        data.push_str(&assistant_at("2026-10-30T09:00:00.000Z", "today1"));
        data.push('\n');
        std::fs::write(&file, &data).unwrap();
        let day = parse_iso_ms("2026-10-30T00:00:00Z").unwrap();
        let mut cache = StatsCache::default();
        assert_eq!(cache.refresh(dir.path(), day).tokens.output, 275);
        assert!(
            cache.bytes_read < 256 * 1024,
            "first visit read {} of {old_len} bytes",
            cache.bytes_read
        );

        // Late on the 30th one more line, then the next day starts: only
        // the new line is read and the totals start over.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap();
        use std::io::Write;
        writeln!(f, "{}", assistant_at("2026-10-30T23:59:00.000Z", "late")).unwrap();
        writeln!(f, "{}", assistant_at("2026-10-31T00:01:00.000Z", "next")).unwrap();
        drop(f);
        let before = cache.bytes_read;
        let next_day = cache.refresh(dir.path(), day + 86_400_000);
        assert_eq!(next_day.tokens.output, 275);
        assert!(
            cache.bytes_read - before < 4096,
            "rollover read {} bytes",
            cache.bytes_read - before
        );
    }

    #[test]
    fn history_prompts_are_counted_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let hist = dir.path().join("history.jsonl");
        std::fs::write(&hist, HISTORY).unwrap();
        let mut cache = StatsCache::default();
        assert_eq!(cache.refresh(dir.path(), 1793774000000).prompts, 1);
        let before = cache.bytes_read;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&hist)
            .unwrap();
        use std::io::Write;
        let line = r#"{"display":"one more","timestamp":1793774500000,"sessionId":"other"}"#;
        writeln!(f, "{line}").unwrap();
        drop(f);
        let s = cache.refresh(dir.path(), 1793774000000);
        assert_eq!((s.prompts, s.sessions), (2, 2));
        // Only the appended line was read, not the file again.
        assert_eq!(cache.bytes_read - before, line.len() as u64 + 1);
        // Nothing new → nothing read, same counts.
        let before = cache.bytes_read;
        assert_eq!(cache.refresh(dir.path(), 1793774000000).prompts, 2);
        assert_eq!(cache.bytes_read, before);
    }

    #[test]
    fn compact_counts() {
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1234), "1.2k");
        assert_eq!(compact_count(3_400_000), "3.4M");
    }
}
