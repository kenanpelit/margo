//! Where Claude Code usage goes over time: billed tokens per local day,
//! model, hour of day and project, from every transcript under each
//! account's `projects/` (subagent transcripts included).
//!
//! The transcripts are 1.5 GB on a busy machine, so the totals are kept
//! per file in `$XDG_CACHE_HOME/mshell/agents-usage.json` with the offset
//! each file was read to: the first build reads everything once, later
//! refreshes only what was appended. A file Claude Code deletes keeps its
//! totals, so the all-time numbers grow from the first build on.

use super::jsonl;
use super::limits::parse_iso_ms;
use super::pricing::{BilledTokens, PriceTable};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

/// Bumped when the cached shape or the counting changes: an old cache is
/// rebuilt rather than misread.
const VERSION: u32 = 1;
/// Message ids remembered per file to skip the repeated usage of a
/// message written as one line per content block.
const RECENT_IDS: usize = 32;

/// Billed tokens per model and assistant messages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Totals {
    pub(crate) billed: BTreeMap<String, BilledTokens>,
    pub(crate) messages: u32,
}

impl Totals {
    pub(crate) fn add(&mut self, o: &Totals) {
        for (m, t) in &o.billed {
            self.billed.entry(m.clone()).or_default().add(t);
        }
        self.messages += o.messages;
    }

    pub(crate) fn tokens(&self) -> u64 {
        self.billed.values().map(BilledTokens::total).sum()
    }

    pub(crate) fn cost(&self, prices: &PriceTable) -> Option<f64> {
        prices.cost(self.billed.iter().map(|(m, t)| (m.as_str(), t)))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DayUsage {
    pub(crate) totals: Totals,
    /// Tokens per local hour of the day.
    pub(crate) hours: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct FileUsage {
    /// `(dev, ino)` and the offset read to.
    id: (u64, u64),
    offset: u64,
    recent: VecDeque<String>,
    /// The session's working directory.
    project: String,
    /// A subagent transcript (not a session of its own).
    subagent: bool,
    /// `YYYY-MM-DD` (local) → usage.
    days: BTreeMap<String, DayUsage>,
}

/// Local `(YYYY-MM-DD, hour)` of a unix-ms instant.
pub(crate) type DayOf = dyn Fn(i64) -> Option<(String, usize)>;

pub(crate) fn local_day(ms: i64) -> Option<(String, usize)> {
    use chrono::{Local, TimeZone, Timelike};
    let t = Local.timestamp_millis_opt(ms).single()?;
    Some((t.format("%Y-%m-%d").to_string(), t.hour() as usize))
}

impl FileUsage {
    fn add_line(&mut self, line: &str, day_of: &DayOf) {
        if self.project.is_empty()
            && line.contains("\"cwd\"")
            && let Ok(v) = serde_json::from_str::<Value>(line)
            && let Some(cwd) = v.get("cwd").and_then(Value::as_str)
        {
            self.project = cwd.to_string();
        }
        if !line.contains("\"usage\"") || !line.contains("\"type\":\"assistant\"") {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(msg) = v.get("message") else { return };
        let (Some(usage), Some(model)) =
            (msg.get("usage"), msg.get("model").and_then(Value::as_str))
        else {
            return;
        };
        if model.starts_with('<') {
            return;
        }
        let Some((day, hour)) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms)
            .and_then(day_of)
        else {
            return;
        };
        let id = format!(
            "{}:{}",
            msg.get("id").and_then(Value::as_str).unwrap_or(""),
            v.get("requestId").and_then(Value::as_str).unwrap_or("")
        );
        if id != ":" {
            if self.recent.contains(&id) {
                return;
            }
            self.recent.push_back(id);
            if self.recent.len() > RECENT_IDS {
                self.recent.pop_front();
            }
        }
        let t = BilledTokens::from_usage(usage);
        let d = self.days.entry(day).or_default();
        d.totals
            .billed
            .entry(model.to_string())
            .or_default()
            .add(&t);
        d.totals.messages += 1;
        if d.hours.len() != 24 {
            d.hours = vec![0; 24];
        }
        d.hours[hour.min(23)] += t.total();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UsageHistory {
    version: u32,
    files: BTreeMap<PathBuf, FileUsage>,
    /// Bytes read from disk by the last refresh (not persisted meaning).
    #[serde(skip)]
    pub(crate) bytes_read: u64,
}

/// Every transcript under `projects`: `<project>/<session>.jsonl` and
/// `<project>/<session>/subagents/*.jsonl` (the flag says which).
fn transcripts(projects: &Path) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(projects) else {
        return out;
    };
    for d in dirs.flatten() {
        let Ok(entries) = std::fs::read_dir(d.path()) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                out.push((p, false));
            } else if let Ok(subs) = std::fs::read_dir(p.join("subagents")) {
                out.extend(
                    subs.flatten()
                        .map(|s| s.path())
                        .filter(|s| s.extension().is_some_and(|x| x == "jsonl"))
                        .map(|s| (s, true)),
                );
            }
        }
    }
    out
}

impl UsageHistory {
    /// Read what was appended to every transcript under the `projects`
    /// dirs. Returns whether anything changed. Blocking.
    pub(crate) fn refresh(&mut self, projects: &[PathBuf], day_of: &DayOf) -> bool {
        use std::os::unix::fs::MetadataExt;
        if self.version != VERSION {
            *self = UsageHistory {
                version: VERSION,
                ..UsageHistory::default()
            };
        }
        self.bytes_read = 0;
        let mut changed = false;
        for dir in projects {
            for (path, subagent) in transcripts(dir) {
                let Ok(mut file) = std::fs::File::open(&path) else {
                    continue;
                };
                let Ok(meta) = file.metadata() else { continue };
                let id = (meta.dev(), meta.ino());
                let entry = self.files.entry(path).or_default();
                if entry.id == id && entry.offset == meta.len() {
                    continue;
                }
                if entry.id != id || entry.offset > meta.len() {
                    *entry = FileUsage {
                        id,
                        subagent,
                        ..FileUsage::default()
                    };
                }
                let mut counted = jsonl::Counting::new(&mut file);
                let read =
                    jsonl::read_from(&mut counted, meta.len(), Some(entry.offset), None, |line| {
                        entry.add_line(line, day_of)
                    });
                self.bytes_read += counted.read;
                if let Ok(next) = read
                    && next != entry.offset
                {
                    entry.offset = next;
                    changed = true;
                }
            }
        }
        changed
    }

    /// Usage per day over every file, `YYYY-MM-DD` → usage.
    pub(crate) fn by_day(&self) -> BTreeMap<&str, DayUsage> {
        let mut out: BTreeMap<&str, DayUsage> = BTreeMap::new();
        for f in self.files.values() {
            for (day, u) in &f.days {
                let d = out.entry(day.as_str()).or_default();
                d.totals.add(&u.totals);
                if d.hours.len() != 24 {
                    d.hours = vec![0; 24];
                }
                for (h, n) in u.hours.iter().enumerate().take(24) {
                    d.hours[h] += n;
                }
            }
        }
        out
    }

    /// Sessions with any usage (subagent transcripts are part of theirs).
    pub(crate) fn sessions(&self) -> u32 {
        self.files
            .values()
            .filter(|f| !f.subagent && !f.days.is_empty())
            .count() as u32
    }

    /// Per project since `from_day` (inclusive), most tokens first. A
    /// Claude Code worktree (`<repo>/.claude/worktrees/<name>`) counts
    /// toward its repo.
    pub(crate) fn by_project(&self, from_day: &str) -> Vec<(String, Totals)> {
        let mut m: BTreeMap<String, Totals> = BTreeMap::new();
        for f in self.files.values() {
            let mut t = Totals::default();
            for (_, u) in f.days.range(from_day.to_string()..) {
                t.add(&u.totals);
            }
            if t.messages > 0 {
                let key = if f.project.is_empty() {
                    "?".to_string()
                } else {
                    project_root(&f.project).to_string()
                };
                m.entry(key).or_default().add(&t);
            }
        }
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by_key(|(_, t)| std::cmp::Reverse(t.tokens()));
        v
    }
}

/// `/r/app/.claude/worktrees/agent-1/sub` → `/r/app`.
pub(crate) fn project_root(cwd: &str) -> &str {
    match cwd.find("/.claude/worktrees/") {
        Some(i) if i > 0 => &cwd[..i],
        _ => cwd,
    }
}

fn cache_path() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("mshell")
        .join("agents-usage.json")
}

pub(crate) fn load() -> UsageHistory {
    std::fs::read(cache_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) fn save(h: &UsageHistory) {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_vec(h) {
        Ok(json) => {
            if let Err(e) = margo_fs::atomic_write::atomic_write(&path, &json) {
                tracing::debug!(error = %e, "agents: usage cache not written");
            }
        }
        Err(e) => tracing::debug!(error = %e, "agents: usage cache not serialised"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A real assistant line (claude-sonnet-5, 2026-10-30), content cut.
    pub(crate) const LINE: &str = r#"{"parentUuid":"x","isSidechain":false,"type":"assistant","timestamp":"2026-10-30T21:21:58.119Z","cwd":"/home/kenan/.cachy","sessionId":"9f9c","requestId":"req_011CgZ86FNrzkHnRtAPeJZtP","message":{"id":"msg_011CgZ86FczLmznvncpz5AqA","type":"message","role":"assistant","model":"claude-sonnet-5","content":[],"usage":{"input_tokens":2,"cache_creation_input_tokens":206,"cache_read_input_tokens":377011,"output_tokens":275,"service_tier":"standard"}}}"#;

    /// UTC days and hours, so the tests don't depend on the local zone.
    pub(crate) fn utc(ms: i64) -> Option<(String, usize)> {
        use chrono::{TimeZone, Timelike, Utc};
        let t = Utc.timestamp_millis_opt(ms).single()?;
        Some((t.format("%Y-%m-%d").to_string(), t.hour() as usize))
    }

    pub(crate) fn at(ts: &str, id: &str) -> String {
        LINE.replace("2026-10-30T21:21:58.119Z", ts)
            .replace("msg_011CgZ86FczLmznvncpz5AqA", id)
    }

    #[test]
    fn counts_each_message_once_by_day_hour_and_project() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("projects/-home-kenan--cachy");
        std::fs::create_dir_all(p.join("s1/subagents")).unwrap();
        std::fs::write(
            p.join("s1.jsonl"),
            format!("{LINE}\n{LINE}\n{}\n", at("2026-10-31T08:00:00.000Z", "m2")),
        )
        .unwrap();
        std::fs::write(
            p.join("s1/subagents/agent-a.jsonl"),
            format!("{}\n", at("2026-10-31T08:30:00.000Z", "m3")),
        )
        .unwrap();
        let mut h = UsageHistory::default();
        assert!(h.refresh(&[dir.path().join("projects")], &utc));
        let days = h.by_day();
        let d30 = &days["2026-10-30"];
        assert_eq!(d30.totals.messages, 1, "the repeated line counts once");
        assert_eq!(d30.hours[21], 2 + 206 + 377011 + 275);
        assert_eq!(days["2026-10-31"].totals.messages, 2, "subagent included");
        let proj = h.by_project("2026-10-31");
        assert_eq!(proj.len(), 1);
        assert_eq!(
            (proj[0].0.as_str(), proj[0].1.messages),
            ("/home/kenan/.cachy", 2)
        );
        // Nothing appended: nothing read.
        assert!(!h.refresh(&[dir.path().join("projects")], &utc));
        assert_eq!(h.bytes_read, 0);
    }

    #[test]
    fn worktrees_count_toward_their_repo() {
        assert_eq!(
            project_root("/r/app/.claude/worktrees/agent-a2eb"),
            "/r/app"
        );
        assert_eq!(project_root("/r/app"), "/r/app");
        assert_eq!(project_root("/.claude/worktrees/x"), "/.claude/worktrees/x");
    }

    #[test]
    fn appends_are_read_alone_and_the_cache_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("projects/-w");
        std::fs::create_dir_all(&p).unwrap();
        let f = p.join("s.jsonl");
        std::fs::write(&f, format!("{LINE}\n")).unwrap();
        let mut h = UsageHistory::default();
        h.refresh(&[dir.path().join("projects")], &utc);
        let json = serde_json::to_vec(&h).unwrap();
        let mut back: UsageHistory = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.by_day(), h.by_day());
        use std::io::Write;
        let line = at("2026-10-30T22:00:00.000Z", "m9");
        let mut w = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        writeln!(w, "{line}").unwrap();
        drop(w);
        assert!(back.refresh(&[dir.path().join("projects")], &utc));
        assert_eq!(back.bytes_read, line.len() as u64 + 1);
        assert_eq!(back.by_day()["2026-10-30"].totals.messages, 2);
        // A deleted transcript keeps its totals.
        std::fs::remove_file(&f).unwrap();
        back.refresh(&[dir.path().join("projects")], &utc);
        assert_eq!(back.by_day()["2026-10-30"].totals.messages, 2);
    }
}
