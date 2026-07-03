//! Remote ICS provider — subscription URLs fetched over HTTP, mirroring
//! dankcalendar's `internal/providers/ical`.
//!
//! Each subscription is one calendar. A successful fetch is cached to disk; if
//! a later fetch fails (offline, server down) the cached copy is used so the
//! calendar degrades to *stale* rather than *empty*. No OAuth — this is the
//! "paste your Google secret iCal URL" path.

use super::{Provider, Window};
use crate::config::Subscription;
use crate::error::McalError;
use crate::ics::parse_ics;
use crate::model::{Calendar, Event};
use crate::recur;
use std::fs;
use std::path::PathBuf;

/// A calendar source backed by remote `.ics` subscription URLs.
pub struct RemoteIcsProvider {
    account_id: String,
    subscriptions: Vec<Subscription>,
    cache_dir: PathBuf,
}

impl RemoteIcsProvider {
    /// Build a provider over `subscriptions`, caching bodies under
    /// `$XDG_CACHE_HOME/margo/mcal`.
    pub fn new(account_id: impl Into<String>, subscriptions: Vec<Subscription>) -> Self {
        let cache_dir = dirs::cache_dir()
            .map(|c| c.join("margo").join("mcal"))
            .unwrap_or_else(|| PathBuf::from("/tmp/margo-mcal"));
        Self {
            account_id: account_id.into(),
            subscriptions,
            cache_dir,
        }
    }

    /// Fetch a subscription body, falling back to the on-disk cache on error.
    fn fetch(&self, sub: &Subscription) -> Result<String, McalError> {
        match ureq::get(&sub.url).call() {
            Ok(response) => {
                let body = response.into_string().map_err(|source| McalError::Io {
                    path: sub.url.clone(),
                    source,
                })?;
                self.write_cache(sub, &body);
                Ok(body)
            }
            Err(err) => match self.read_cache(sub) {
                Some(cached) => {
                    tracing::warn!(url = %sub.url, "mcal: fetch failed, serving cached copy");
                    Ok(cached)
                }
                None => Err(McalError::Fetch {
                    url: sub.url.clone(),
                    source: Box::new(err),
                }),
            },
        }
    }

    fn cache_path(&self, sub: &Subscription) -> PathBuf {
        self.cache_dir.join(format!("{}.ics", cache_key(&sub.url)))
    }

    fn write_cache(&self, sub: &Subscription, body: &str) {
        if let Err(err) = fs::create_dir_all(&self.cache_dir) {
            tracing::warn!(%err, "mcal: cannot create cache dir");
            return;
        }
        if let Err(err) = fs::write(self.cache_path(sub), body) {
            tracing::warn!(url = %sub.url, %err, "mcal: cache write failed");
        }
    }

    fn read_cache(&self, sub: &Subscription) -> Option<String> {
        fs::read_to_string(self.cache_path(sub)).ok()
    }
}

impl Provider for RemoteIcsProvider {
    fn calendars(&self) -> Result<Vec<Calendar>, McalError> {
        Ok(self
            .subscriptions
            .iter()
            .map(|sub| Calendar {
                account_id: self.account_id.clone(),
                remote_id: sub.url.clone(),
                name: sub.name.clone(),
                color: sub.color.clone(),
            })
            .collect())
    }

    fn events(&self, window: Window) -> Result<Vec<Event>, McalError> {
        let mut out = Vec::new();
        for sub in &self.subscriptions {
            let text = match self.fetch(sub) {
                Ok(text) => text,
                Err(err) => {
                    tracing::warn!(url = %sub.url, %err, "mcal: subscription unavailable, skipping");
                    continue;
                }
            };
            match parse_ics(&text, &sub.url) {
                Ok(events) => {
                    for event in &events {
                        out.extend(recur::expand(event, window.0, window.1));
                    }
                }
                Err(err) => {
                    tracing::warn!(url = %sub.url, %err, "mcal: malformed subscription, skipping")
                }
            }
        }
        Ok(out)
    }
}

/// A filesystem-safe cache key derived from a URL (alphanumerics kept, the rest
/// collapsed to `_`). Not cryptographic — just a stable, readable filename.
fn cache_key(url: &str) -> String {
    url.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_filesystem_safe() {
        let key = cache_key("https://cal.example.com/a/b?x=1&y=2");
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        assert!(!key.contains('/'));
        // Stable for the same URL.
        assert_eq!(key, cache_key("https://cal.example.com/a/b?x=1&y=2"));
    }

    #[test]
    fn calendars_map_one_per_subscription() {
        let provider = RemoteIcsProvider::new(
            "remote",
            vec![
                Subscription {
                    name: "Work".into(),
                    url: "https://x/work.ics".into(),
                    color: Some("#4285F4".into()),
                },
                Subscription {
                    name: "Home".into(),
                    url: "https://x/home.ics".into(),
                    color: None,
                },
            ],
        );
        let cals = provider.calendars().unwrap();
        assert_eq!(cals.len(), 2);
        assert_eq!(cals[0].name, "Work");
        assert_eq!(cals[0].color.as_deref(), Some("#4285F4"));
    }
}
