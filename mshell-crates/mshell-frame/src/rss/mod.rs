//! RSS / Atom notifier — a headless service: every configured feed is
//! polled on its interval, new items pop a desktop notification whose
//! *Open* action opens the item in the browser.
//!
//! * Feeds, interval and limits come from `rss.*` (Settings →
//!   Notifications → RSS feeds). No feeds = the worker just waits for the
//!   config to change: no network, no timers.
//! * Fetching runs on blocking threads of the shared tokio runtime (`ureq`,
//!   20 s timeout, 4 MiB cap) with a conditional GET (`If-None-Match` /
//!   `If-Modified-Since` from the last answer), so an unchanged feed costs
//!   one 304.
//! * Seen item ids + validators persist in `$XDG_STATE_HOME/margo/
//!   rss-seen.json` ([`seen::SeenStore`]); the first fetch of a feed only
//!   marks its items seen.
//! * Notifications go straight to `org.freedesktop.Notifications` over
//!   zbus from one task that also listens for `ActionInvoked`, so an
//!   unanswered notification never parks a thread. Only `http(s)` links
//!   are opened.
//!
//! Feed URLs can carry private tokens: logs name the host only.

mod parse;
mod seen;

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, RssConfig};
use parse::{Feed, decode_body, origin, parse_feed, valid_feed_url};
use reactive_graph::prelude::{Get, GetUntracked};
use seen::{Notice, SeenStore, display_name, plan_notices};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

const USER_AGENT: &str = concat!("mshell-rss/", env!("CARGO_PKG_VERSION"), " (margo)");
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY: u64 = 4 * 1024 * 1024;
const ICON: &str = "application-rss+xml-symbolic";

fn seen_path() -> PathBuf {
    margo_fs::xdg::state_home().join("margo/rss-seen.json")
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A host for logs (never the full URL: it may hold a token).
fn host(url: &str) -> &str {
    origin(url).unwrap_or("feed")
}

// ── Store I/O (blocking) ───────────────────────────────────────────────────

fn load_store(path: &Path) -> SeenStore {
    match margo_fs::side_file::load(path, SeenStore::from_json) {
        margo_fs::side_file::SideLoad::Loaded(s) => s,
        margo_fs::side_file::SideLoad::Missing => SeenStore::default(),
        margo_fs::side_file::SideLoad::Broken { error, backup } => {
            // Only a cache: start over (every feed re-primes silently).
            warn!(%error, ?backup, "rss: seen-items file unreadable, starting fresh");
            SeenStore::default()
        }
    }
}

fn save_store(path: &Path, store: &SeenStore) {
    if let Err(e) = margo_fs::atomic_write::atomic_write(path, store.to_json().as_bytes()) {
        warn!(error = %e, "rss: could not save the seen items");
    }
}

// ── Fetching (blocking) ────────────────────────────────────────────────────

#[derive(Debug)]
enum Fetched {
    NotModified,
    Feed {
        feed: Feed,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

/// `charset=` of a Content-Type header.
fn charset(content_type: &str) -> Option<&str> {
    content_type.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k.trim().eq_ignore_ascii_case("charset")).then(|| v.trim().trim_matches('"'))
    })
}

/// One conditional GET + parse. Blocking: call on a blocking thread.
fn fetch(url: &str, etag: Option<&str>, last_modified: Option<&str>) -> Result<Fetched, String> {
    let mut req = ureq::get(url)
        .timeout(TIMEOUT)
        .set("User-Agent", USER_AGENT)
        .set(
            "Accept",
            "application/rss+xml, application/atom+xml, application/rdf+xml;q=0.9, application/xml;q=0.8, text/xml;q=0.8, */*;q=0.5",
        );
    if let Some(e) = etag {
        req = req.set("If-None-Match", e);
    }
    if let Some(m) = last_modified {
        req = req.set("If-Modified-Since", m);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(format!("HTTP {code}")),
        Err(ureq::Error::Transport(t)) => return Err(t.kind().to_string()),
    };
    if resp.status() == 304 {
        return Ok(Fetched::NotModified);
    }
    let etag = resp.header("ETag").map(str::to_string);
    let last_modified = resp.header("Last-Modified").map(str::to_string);
    let charset = charset(resp.content_type()).map(str::to_string);
    let mut body = Vec::new();
    resp.into_reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    if body.len() as u64 > MAX_BODY {
        return Err("feed larger than 4 MiB".to_string());
    }
    let feed = parse_feed(&decode_body(&body, charset.as_deref()))?;
    Ok(Fetched::Feed {
        feed,
        etag,
        last_modified,
    })
}

// ── The worker ─────────────────────────────────────────────────────────────

/// A configured feed worth polling.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Watched {
    url: String,
    name: String,
}

/// Valid `http(s)` feeds, trimmed, first of each URL.
fn watched(cfg: &RssConfig) -> Vec<Watched> {
    let mut out: Vec<Watched> = Vec::new();
    for f in &cfg.feeds {
        let url = f.url.trim();
        if valid_feed_url(url) && !out.iter().any(|w| w.url == url) {
            out.push(Watched {
                url: url.to_string(),
                name: f.name.trim().to_string(),
            });
        }
    }
    out
}

fn interval_secs(cfg: &RssConfig) -> i64 {
    i64::from(cfg.interval_minutes.clamp(5, 1440)) * 60
}

type FetchFn = fn(&str, Option<&str>, Option<&str>) -> Result<Fetched, String>;

/// Poll until the config channel closes. `fetch` and `notify` are
/// parameters so the loop is tested without network or a bus.
async fn worker(
    mut cfg_rx: watch::Receiver<RssConfig>,
    store_path: PathBuf,
    fetch: FetchFn,
    notify: mpsc::UnboundedSender<Notice>,
) {
    let path = store_path.clone();
    let mut store = tokio::task::spawn_blocking(move || load_store(&path))
        .await
        .unwrap_or_default();
    loop {
        let cfg = cfg_rx.borrow_and_update().clone();
        let feeds = watched(&cfg);
        let urls: Vec<&str> = feeds.iter().map(|f| f.url.as_str()).collect();
        let mut dirty = store.retain_feeds(&urls);
        let interval = interval_secs(&cfg);
        let mut new = Vec::new();
        for f in &feeds {
            let now = now_secs();
            if !store.due(&f.url, now, interval) {
                continue;
            }
            let (etag, lm) = store
                .feeds
                .get(&f.url)
                .map(|s| (s.etag.clone(), s.last_modified.clone()))
                .unwrap_or_default();
            let url = f.url.clone();
            let res =
                tokio::task::spawn_blocking(move || fetch(&url, etag.as_deref(), lm.as_deref()))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
            match res {
                Ok(Fetched::NotModified) => {
                    debug!(host = host(&f.url), "rss: not modified");
                    store.checked(&f.url, now);
                }
                Ok(Fetched::Feed {
                    feed,
                    etag,
                    last_modified,
                }) => {
                    let items = store.observe(&f.url, &feed, (etag, last_modified), now);
                    debug!(host = host(&f.url), new = items.len(), "rss: fetched");
                    if !items.is_empty() {
                        let title = store.feeds.get(&f.url).map(|s| s.title.clone());
                        let name = display_name(&f.name, &title.unwrap_or_default(), &f.url);
                        new.push((name, f.url.clone(), items));
                    }
                }
                Err(e) => {
                    warn!(host = host(&f.url), error = %e, "rss: check failed");
                    store.checked(&f.url, now);
                }
            }
            dirty = true;
        }
        if dirty {
            let (path, snapshot) = (store_path.clone(), store.clone());
            let _ = tokio::task::spawn_blocking(move || save_store(&path, &snapshot)).await;
        }
        if cfg.notify {
            for n in plan_notices(&new, cfg.max_per_poll.clamp(1, 50) as usize) {
                let _ = notify.send(n);
            }
        }
        if feeds.is_empty() {
            // Nothing to watch: no timer at all until the config changes.
            if cfg_rx.changed().await.is_err() {
                return;
            }
            continue;
        }
        let wait = store.next_due_in(&urls, now_secs(), interval).max(30) as u64;
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(wait)) => {}
            r = cfg_rx.changed() => if r.is_err() { return },
        }
    }
}

// ── Notifications ──────────────────────────────────────────────────────────

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// Links of notifications still open, by notification id.
const MAX_TRACKED: usize = 64;

/// Body text is markup for many servers: escape it.
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Show notices and open a link when its notification is clicked.
async fn notifier(mut rx: mpsc::UnboundedReceiver<Notice>) {
    use futures::StreamExt;
    // Connect only once there is something to show.
    let Some(first) = rx.recv().await else {
        return;
    };
    let proxy = async {
        let conn = zbus::Connection::session().await?;
        NotificationsProxy::new(&conn).await
    };
    let proxy = match proxy.await {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "rss: no notification service");
            return;
        }
    };
    let (Ok(mut invoked), Ok(mut closed)) = (
        proxy.receive_action_invoked().await,
        proxy.receive_notification_closed().await,
    ) else {
        warn!("rss: could not listen for notification actions");
        return;
    };
    let mut links: VecDeque<(u32, String)> = VecDeque::new();
    let mut pending = Some(first);
    loop {
        let notice = match pending.take() {
            Some(n) => n,
            None => tokio::select! {
                n = rx.recv() => match n { Some(n) => n, None => return },
                Some(sig) = invoked.next() => {
                    if let Ok(a) = sig.args()
                        && matches!(a.action_key.as_str(), "default" | "open")
                        && let Some(pos) = links.iter().position(|(id, _)| *id == a.id)
                        && let Some((_, link)) = links.remove(pos)
                        && let Err(e) = mshell_process::desktop::open(&link)
                    {
                        warn!(error = %e, "rss: could not open the item");
                    }
                    continue;
                }
                Some(sig) = closed.next() => {
                    if let Ok(a) = sig.args() {
                        links.retain(|(id, _)| *id != a.id);
                    }
                    continue;
                }
            },
        };
        let actions: &[&str] = if notice.link.is_some() {
            &["default", "Open", "open", "Open"]
        } else {
            &[]
        };
        let hints = HashMap::new();
        match proxy
            .notify(
                "RSS",
                0,
                ICON,
                &notice.title,
                &escape_markup(&notice.feed),
                actions,
                hints,
                -1,
            )
            .await
        {
            Ok(id) => {
                if let Some(link) = notice.link {
                    links.push_back((id, link));
                    while links.len() > MAX_TRACKED {
                        links.pop_front();
                    }
                }
            }
            Err(e) => warn!(error = %e, "rss: notification failed"),
        }
    }
}

// ── Start ──────────────────────────────────────────────────────────────────

thread_local! {
    /// Keeps the config effect alive for the shell's lifetime.
    static EFFECTS: std::cell::RefCell<Option<EffectScope>> = const { std::cell::RefCell::new(None) };
}

/// Start the notifier (GTK thread, once at shell start; later calls are
/// no-ops). It idles until a feed is configured.
pub fn start() {
    if EFFECTS.with(|e| e.borrow().is_some()) {
        return;
    }
    let initial = config_manager().config().rss().get_untracked();
    let (cfg_tx, cfg_rx) = watch::channel(initial);
    let mut effects = EffectScope::new();
    effects.push(move |_| {
        let cfg = config_manager().config().rss().get();
        cfg_tx.send_if_modified(|cur| {
            let changed = *cur != cfg;
            if changed {
                *cur = cfg;
            }
            changed
        });
    });
    EFFECTS.with(|e| *e.borrow_mut() = Some(effects));
    let (notice_tx, notice_rx) = mpsc::unbounded_channel();
    mshell_services::tokio_rt_spawn(notifier(notice_rx));
    mshell_services::tokio_rt_spawn(worker(cfg_rx, seen_path(), fetch, notice_tx));
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
