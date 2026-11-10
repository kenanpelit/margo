//! `timer <text>` — set a one-shot timer or reminder from the launcher.
//!
//! Typing `timer 10m tea` (or `t 10m tea`, `t 14:30 meeting`, `timer
//! yarın 9:00 kahve`) shows the parsed result live — "Rings at 14:35 (in
//! 10 min) — tea" — before Enter starts it; a line that doesn't parse shows
//! why instead, and Enter does nothing. Bare `timer` lists the pending
//! timers; Enter on one cancels it. The parser and the store are
//! [`crate::timers`], shared with the Alarm Clock menu, the bar pill and
//! `mshellctl timer`.
//!
//! Lives in `mshell-frame` (not `mshell-launcher`) because it drives the
//! shell's timer store — the same split as the Tags / Windows providers.

use crate::timers::{self, format};
use mshell_launcher::{LauncherItem, Provider};
use std::rc::Rc;

pub struct TimersProvider;

impl TimersProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TimersProvider {
    fn default() -> Self {
        Self::new()
    }
}

const NAME: &str = "Timers";
const ICON: &str = "timer-symbolic";

/// The text after the prefix, or `None` when `query` isn't a timer query.
/// `timer …` always is; the short `t …` only when a digit follows, so
/// `t` + space while searching apps isn't taken over.
fn timer_text(query: &str) -> Option<&str> {
    let q = query.trim_start();
    for prefix in ["timer", "remind"] {
        if let Some(rest) = q.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return Some(rest.trim());
        }
    }
    let rest = q.strip_prefix("t ")?.trim_start();
    rest.starts_with(|c: char| c.is_ascii_digit())
        .then_some(rest.trim())
}

fn item(id: String, name: String, description: String, on_activate: Rc<dyn Fn()>) -> LauncherItem {
    LauncherItem {
        id,
        name,
        description,
        icon: ICON.into(),
        icon_is_path: false,
        score: 200.0,
        provider_name: NAME.into(),
        usage_key: None,
        on_activate,
    }
}

impl Provider for TimersProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        timer_text(query).is_some()
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![item(
            "timers:palette".into(),
            "timer <text>".into(),
            "Quick timer or reminder: 10m tea · 14:30 meeting · tomorrow 9am".into(),
            Rc::new(|| {}),
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(text) = timer_text(query) else {
            return Vec::new();
        };
        if text.is_empty() {
            return pending_rows();
        }
        match timers::preview(text) {
            Ok(preview) => {
                let line = text.to_string();
                vec![item(
                    "timers:new".into(),
                    preview,
                    "Enter starts it · rings with Stop / Snooze".into(),
                    Rc::new(move || {
                        if let Err(e) = timers::add(&line) {
                            tracing::warn!(error = %e, "launcher: timer not added");
                        }
                    }),
                )]
            }
            Err(e) => vec![item(
                "timers:error".into(),
                e,
                "Keep typing — e.g. 10m tea · 1h 30m · 14:30 · yarın 9:00".into(),
                Rc::new(|| {}),
            )],
        }
    }

    /// Enter on an unparsed line does nothing: keep the launcher open so
    /// the user can fix it.
    fn keeps_open(&self, item: &LauncherItem, _alt: bool) -> bool {
        item.id == "timers:error" || item.id == "timers:none"
    }
}

/// Bare `timer`: the pending timers (Enter cancels one), or a hint.
fn pending_rows() -> Vec<LauncherItem> {
    let now = timers::now_unix();
    let pending = timers::list();
    if pending.is_empty() {
        return vec![item(
            "timers:none".into(),
            "No timers — type one: timer 10m tea".into(),
            "Also 14:30 meeting · tomorrow 9am · 10 dk sonra çay".into(),
            Rc::new(|| {}),
        )];
    }
    pending
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let rings = format::rings_phrase(&timers::due_clock(&t, now));
            let name = match t.label.as_str() {
                "" => format!("{} left", format::pill_text(t.remaining(now))),
                label => format!("{label} — {} left", format::pill_text(t.remaining(now))),
            };
            let id = t.id;
            let mut row = item(
                format!("timers:{id}"),
                name,
                format!("{} · Enter cancels it", capitalize_first(&rings)),
                Rc::new(move || {
                    timers::cancel(id);
                }),
            );
            row.score = 200.0 - i as f64;
            row
        })
        .collect()
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefixes_are_timer_remind_and_t_with_a_digit() {
        assert_eq!(timer_text("timer 10m tea"), Some("10m tea"));
        assert_eq!(timer_text("  timer"), Some(""));
        assert_eq!(timer_text("remind 14:30 call"), Some("14:30 call"));
        assert_eq!(timer_text("t 10m"), Some("10m"));
        assert_eq!(timer_text("t  25 Pomodoro"), Some("25 Pomodoro"));
        // Not ours: app names and a bare `t`.
        assert_eq!(timer_text("timeshift"), None);
        assert_eq!(timer_text("reminders"), None);
        assert_eq!(timer_text("t thunderbird"), None);
        assert_eq!(timer_text("t"), None);
        assert_eq!(timer_text("firefox"), None);
    }

    #[test]
    fn a_line_shows_its_parsed_result_before_enter() {
        let p = TimersProvider::new();
        assert!(p.handles_command("timer 10m tea"));
        let rows = p.search("timer 10m tea");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "timers:new");
        assert!(rows[0].name.starts_with("Rings at "), "{}", rows[0].name);
        assert!(
            rows[0].name.ends_with("(in 10 min) — tea"),
            "{}",
            rows[0].name
        );
        assert!(!p.keeps_open(&rows[0], false));
    }

    #[test]
    fn garbage_shows_the_reason_and_keeps_the_launcher_open() {
        let p = TimersProvider::new();
        let rows = p.search("timer tea");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "timers:error");
        assert!(rows[0].name.contains("No time found"), "{}", rows[0].name);
        assert!(p.keeps_open(&rows[0], false));
    }

    #[test]
    fn it_never_answers_a_plain_search() {
        let p = TimersProvider::new();
        assert!(!p.handles_search());
        assert!(p.search("firefox").is_empty());
    }
}
