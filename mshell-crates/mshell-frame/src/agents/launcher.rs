//! `cc <query>` in the launcher: search Claude Code sessions by title,
//! name, project path, last prompt or account. Running sessions come
//! first (Enter jumps to the session's terminal), then recent saved ones
//! (Enter resumes it in a new terminal). Lives beside the rest of the
//! agents code so the launcher reads the same session data as the menu.
//!
//! The session list is read off the GTK thread when the launcher opens
//! (and when the query first asks for it); `search` only filters it.

use super::SessionsSnapshot;
use super::sessions::{self, LiveSession, SavedSession};
use mshell_launcher::provider::RefreshNotifier;
use mshell_launcher::{LauncherItem, LauncherPreview, Provider};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const PREFIX: &str = "cc";
const NAME: &str = "Claude Code";

#[derive(Default)]
pub(crate) struct ClaudeSessionsProvider {
    data: Arc<Mutex<Option<SessionsSnapshot>>>,
    loading: Arc<AtomicBool>,
    notifier: Option<RefreshNotifier>,
}

impl ClaudeSessionsProvider {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn reload(&self) {
        if self.loading.swap(true, Ordering::SeqCst) {
            return;
        }
        let (data, loading, notifier) = (
            self.data.clone(),
            self.loading.clone(),
            self.notifier.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("cc-sessions".into())
            .spawn(move || {
                let snap = super::sessions_snapshot(false);
                if let Ok(mut d) = data.lock() {
                    *d = Some(snap);
                }
                loading.store(false, Ordering::SeqCst);
                if let Some(n) = notifier {
                    n();
                }
            });
        if spawned.is_err() {
            self.loading.store(false, Ordering::SeqCst);
        }
    }
}

/// The filter after the `cc` prefix, or `None` when `query` isn't ours.
pub(crate) fn filter_of(query: &str) -> Option<String> {
    let q = query.trim_start();
    let rest = q.strip_prefix(PREFIX)?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim().to_lowercase())
}

/// Every word of `filter` appears in one of `fields` (case-insensitive).
pub(crate) fn matches(filter: &str, fields: &[&str]) -> bool {
    let hay: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    filter
        .split_whitespace()
        .all(|w| hay.iter().any(|h| h.contains(w)))
}

fn live_fields(s: &LiveSession) -> [&str; 5] {
    let prompt = s.detail.as_ref().map_or("", |d| d.title.as_str());
    [&s.name, &s.cwd, &s.account, prompt, &s.waiting_for]
}

fn saved_fields(s: &SavedSession) -> [&str; 3] {
    [&s.last_prompt, &s.project, &s.account]
}

fn item(id: String, name: String, description: String, icon: &str, score: f64) -> LauncherItem {
    LauncherItem {
        id,
        name,
        description,
        icon: icon.into(),
        icon_is_path: false,
        score,
        provider_name: NAME.into(),
        usage_key: None,
        on_activate: Rc::new(|| {}),
    }
}

/// Run `f` off the GTK thread; toast its error.
fn in_background(f: impl FnOnce() -> Result<(), String> + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("cc-action".into())
        .spawn(move || {
            if let Err(e) = f() {
                mshell_launcher::notify::toast(NAME, e);
            }
        });
}

impl Provider for ClaudeSessionsProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        filter_of(query).is_some()
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        self.notifier = Some(notifier);
    }

    fn on_opened(&mut self) {
        self.reload();
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![item(
            "cc:palette".into(),
            PREFIX.into(),
            "Search Claude Code sessions".into(),
            "ai-agents-symbolic",
            0.0,
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(filter) = filter_of(query) else {
            return Vec::new();
        };
        let snap = self.data.lock().ok().and_then(|d| d.clone());
        let Some(snap) = snap else {
            self.reload();
            return vec![item(
                "cc:loading".into(),
                "Reading Claude Code sessions…".into(),
                String::new(),
                "ai-agents-symbolic",
                100.0,
            )];
        };
        let multi = snap.multi_account();
        let tag = |account: &str| {
            if multi {
                format!(" · {account}")
            } else {
                String::new()
            }
        };
        let mut out = Vec::new();
        for (i, s) in snap
            .live
            .iter()
            .filter(|s| matches(&filter, &live_fields(s)))
            .enumerate()
        {
            let pid = s.pid;
            let mut it = item(
                format!("cc:live:{}", s.key()),
                s.title().to_string(),
                format!(
                    "{} · {}{}",
                    s.status.label(),
                    sessions::tilde(&s.cwd),
                    tag(&s.account)
                ),
                "ai-agents-symbolic",
                300.0 - i as f64,
            );
            it.on_activate = Rc::new(move || {
                in_background(move || {
                    sessions::focus_session(pid)
                        .then_some(())
                        .ok_or_else(|| "Couldn't find that session's terminal window".into())
                })
            });
            out.push(it);
        }
        for (i, s) in snap
            .saved
            .iter()
            .filter(|s| matches(&filter, &saved_fields(s)))
            .enumerate()
        {
            let (id, cwd, dir) = (
                s.session_id.clone(),
                s.project.clone(),
                s.config_dir.clone(),
            );
            let title = if s.last_prompt.is_empty() {
                s.session_id.clone()
            } else {
                s.last_prompt.clone()
            };
            let mut it = item(
                format!("cc:saved:{}", s.session_id),
                title,
                format!(
                    "Resume · {}{}",
                    sessions::tilde(&s.project),
                    tag(&s.account)
                ),
                "media-playback-start-symbolic",
                200.0 - i as f64,
            );
            it.on_activate = Rc::new(move || {
                let (id, cwd, dir) = (id.clone(), cwd.clone(), dir.clone());
                in_background(move || {
                    let dir = (!dir.as_os_str().is_empty()).then_some(dir);
                    sessions::resume(&id, &cwd, dir.as_deref())
                })
            });
            out.push(it);
        }
        if out.is_empty() {
            out.push(item(
                "cc:none".into(),
                "No matching Claude Code session".into(),
                String::new(),
                "dialog-information-symbolic",
                100.0,
            ));
        }
        out
    }

    fn preview(&self, item: &LauncherItem) -> Option<LauncherPreview> {
        let key = item.id.strip_prefix("cc:live:")?;
        let snap = self.data.lock().ok()?.clone()?;
        let s = snap.live.iter().find(|s| s.key() == key)?;
        let d = s.detail.as_ref()?;
        let mut body = String::new();
        if !d.tool.is_empty() {
            body.push_str(&format!("{}\n\n", d.tool));
        }
        body.push_str(&d.reply);
        Some(LauncherPreview::text(s.title().to_string(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_and_filter() {
        assert_eq!(filter_of("cc").as_deref(), Some(""));
        assert_eq!(filter_of("  cc Margo  Bar").as_deref(), Some("margo  bar"));
        assert_eq!(filter_of("ccache"), None);
        assert_eq!(filter_of("firefox"), None);
    }

    #[test]
    fn every_word_must_match_some_field() {
        let f = ["linux-meteor-62", "/repo/archive/.kod/linux-meteor", "work"];
        assert!(matches("meteor work", &f));
        assert!(matches("", &f));
        assert!(!matches("meteor personal", &f));
    }
}
