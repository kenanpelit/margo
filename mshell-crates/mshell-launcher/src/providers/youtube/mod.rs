//! `yt <query>` — YouTube explorer: search, channels, Watch later,
//! favourite-channel feed, history and resume, all played in the mpv
//! companion window (`mshellctl play`).
//!
//! Native port of the `youtube-explorer` community plugin (which
//! subsumes `youtube-search`). Searching and channel pages are flat
//! yt-dlp listings fetched off the GTK thread ([`fetch`]); thumbnails are
//! cached off-thread in `~/.cache/margo/youtube-thumbs` ([`thumbs`]);
//! Watch later + favourite channels live in
//! `~/.local/state/margo/youtube/library.json` and history + resume
//! positions in `progress.json` next to it ([`store`]) — the latter kept
//! by `mshellctl`'s tracker while mpv plays.
//!
//! | Query | Rows |
//! |---|---|
//! | `yt` | continue watching, then Watch later / feed / channels / history |
//! | `yt <text>` | search: videos, channels, playlists |
//! | `yt <url>` | play / queue / save a pasted link |
//! | `yt @<channel-id> [text]` | the channel's latest uploads (or a search in it) + ★ favourite |
//! | `yt :later` · `:feed` · `:channels` · `:history` `[filter]` | the saved lists |
//! | `yt !<video-id>` | everything you can do with one video |
//!
//! | Key | On a video |
//! |---|---|
//! | Enter | play now (resumes where you stopped) |
//! | Ctrl+Enter | the video's actions: queue, Watch later, channel, copy / open URL, play from start |
//! | Ctrl+Delete | remove from Watch later / history (in those lists) |
//!
//! Playback always goes through `mshellctl play play|queue`, so videos
//! share one mpv window, the queue is mpv's playlist, and resume +
//! history are recorded no matter where a video was started from.

mod fetch;
pub mod listing;
pub mod store;
pub mod thumbs;

use self::fetch::{Fetch, Fetcher};
use self::listing::{Entry, Kind, Request, format_count, format_duration};
use self::store::{Channel, Library, LibraryFile, Progress, ProgressEntry, Video};
use self::thumbs::Thumbs;
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use mshell_process::SpawnReaped;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const PROVIDER: &str = "YouTube";
const PREFIX: &str = "yt";
const ICON: &str = "video-x-generic-symbolic";
/// Rows per search / channel page.
const PAGE: usize = 20;
/// Latest uploads per favourite channel in the feed.
const FEED_PER_CHANNEL: usize = 4;
/// Continue-watching rows on the home view.
const HOME_CONTINUE: usize = 3;
/// History rows listed (the store keeps up to 500).
const HISTORY_ROWS: usize = 100;
/// Shortest search term sent to YouTube.
const MIN_QUERY: usize = 2;

/// A video as the rows need it, from any source (listing, library,
/// history).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoView {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub channel_id: String,
    pub duration: Option<f64>,
    pub live: bool,
}

impl VideoView {
    fn from_entry(e: &Entry) -> Self {
        Self {
            id: e.id.clone(),
            title: e.title.clone(),
            channel: e.channel.clone(),
            channel_id: e.channel_id.clone(),
            duration: e.duration,
            live: e.live,
        }
    }

    fn from_saved(v: &Video) -> Self {
        Self {
            id: v.id.clone(),
            title: v.title.clone(),
            channel: v.channel.clone(),
            channel_id: v.channel_id.clone(),
            duration: v.duration,
            live: false,
        }
    }

    fn from_progress(id: &str, p: &ProgressEntry) -> Self {
        Self {
            id: id.to_string(),
            title: if p.title.is_empty() {
                format!("YouTube · {id}")
            } else {
                p.title.clone()
            },
            channel: p.channel.clone(),
            channel_id: p.channel_id.clone(),
            duration: p.duration,
            live: false,
        }
    }

    fn to_saved(&self) -> Video {
        Video {
            id: self.id.clone(),
            title: self.title.clone(),
            channel: self.channel.clone(),
            channel_id: self.channel_id.clone(),
            duration: self.duration,
        }
    }

    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }
}

/// What a `yt …` query asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Home,
    Search(String),
    Url(String),
    Channel {
        id: String,
        filter: String,
    },
    Later(String),
    Feed(String),
    Channels(String),
    History(String),
    ClearHistory,
    Video(String),
    /// `:something` that isn't a view, or a too-short search.
    Hint(String),
}

/// Parse the part after `yt`.
pub fn route(rest: &str) -> Route {
    let q = rest.trim();
    if q.is_empty() {
        return Route::Home;
    }
    if let Some(view) = q.strip_prefix(':') {
        let (name, filter) = view.split_once(' ').unwrap_or((view, ""));
        let filter = filter.trim().to_string();
        return match name {
            "later" | "wl" => Route::Later(filter),
            "feed" => Route::Feed(filter),
            "channels" | "fav" => Route::Channels(filter),
            "history" | "h" => Route::History(filter),
            "clear-history" => Route::ClearHistory,
            _ => Route::Hint(format!(
                "Unknown view :{name} — try :later, :feed, :channels, :history"
            )),
        };
    }
    if let Some(rest) = q.strip_prefix('@') {
        let (id, filter) = rest.split_once(' ').unwrap_or((rest, ""));
        if listing::valid_channel_id(id) {
            return Route::Channel {
                id: id.to_string(),
                filter: filter.trim().to_string(),
            };
        }
        return Route::Hint("A channel id starts with UC (24 characters)".into());
    }
    if let Some(id) = q.strip_prefix('!') {
        let id = id.trim();
        if listing::valid_video_id(id) {
            return Route::Video(id.to_string());
        }
        return Route::Hint("A video id has 11 characters".into());
    }
    if q.starts_with("https://") || q.starts_with("http://") {
        return Route::Url(q.to_string());
    }
    if q.chars().count() < MIN_QUERY {
        return Route::Hint("Keep typing to search YouTube".into());
    }
    Route::Search(q.to_string())
}

/// `true` when `needle` (lowercase) is in the title or channel.
fn matches(v: &VideoView, needle: &str) -> bool {
    needle.is_empty()
        || v.title.to_lowercase().contains(needle)
        || v.channel.to_lowercase().contains(needle)
}

/// Spawn `mshellctl play <verb> <url> …` outside mshell's cgroup (mpv
/// must survive a shell restart). Falls back to the browser when
/// mshellctl is missing.
fn mshellctl_play(verb: &str, url: &str, video: Option<&VideoView>) {
    if !margo_fs::path::on_path("mshellctl") {
        open_url(url);
        return;
    }
    let mut args: Vec<String> = vec!["play".into(), verb.into(), url.into()];
    if let Some(v) = video {
        for (flag, value) in [
            ("--title", v.title.as_str()),
            ("--channel", v.channel.as_str()),
            ("--channel-id", v.channel_id.as_str()),
        ] {
            if !value.is_empty() {
                args.push(flag.into());
                args.push(value.into());
            }
        }
        if let Some(d) = v.duration {
            args.push("--duration".into());
            args.push(format!("{d}"));
        }
    }
    let mut cmd = mshell_process::outliving_shell("mshellctl", &args, "yt");
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Err(err) = cmd.spawn_reaped() {
        tracing::warn!(?err, "yt: mshellctl play failed");
        toast(PROVIDER, "Could not start mshellctl play");
    }
}

fn mshellctl_forget(id: Option<&str>) {
    let args: Vec<&str> = match id {
        Some(id) => vec!["play", "forget", id],
        None => vec!["play", "forget", "--all"],
    };
    if let Err(err) = std::process::Command::new("mshellctl")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn_reaped()
    {
        tracing::warn!(?err, "yt: mshellctl play forget failed");
    }
}

fn open_url(url: &str) {
    if let Err(err) = mshell_process::desktop::open(url) {
        tracing::warn!(?err, "yt: opening the browser failed");
        toast(PROVIDER, "Could not open the browser");
    }
}

fn copy_text(text: String) {
    use std::io::Write as _;
    std::thread::spawn(move || {
        match std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                toast(PROVIDER, "Link copied");
            }
            Err(err) => tracing::warn!(?err, "yt: wl-copy failed"),
        }
    });
}

/// Shared mutable state the row closures touch (they outlive `search`).
struct State {
    library: Library,
    /// `library.json` didn't parse: show it, never save over it.
    broken: Option<String>,
    file: LibraryFile,
}

pub struct YoutubeProvider {
    state: Rc<RefCell<State>>,
    progress: RefCell<Progress>,
    progress_path: PathBuf,
    /// `progress.json`'s mtime at the last read — `search` re-reads only
    /// when the tracker has written since.
    progress_mtime: Cell<Option<std::time::SystemTime>>,
    fetcher: Fetcher,
    thumbs: Thumbs,
    /// Videos / channels seen in listings this session, for the
    /// `yt !<id>` page and channel names.
    known: RefCell<HashMap<String, VideoView>>,
    channels: RefCell<HashMap<String, Channel>>,
    set_search: Option<Rc<dyn Fn(&str) + 'static>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
    pruned: Cell<bool>,
}

impl YoutubeProvider {
    pub fn new() -> Self {
        Self::with_paths(
            LibraryFile::default_location(),
            Progress::default_path(),
            thumbs::cache_dir(),
        )
    }

    pub fn with_paths(library: LibraryFile, progress_path: PathBuf, thumbs_dir: PathBuf) -> Self {
        Self::build(library, progress_path, Thumbs::new(thumbs_dir))
    }

    fn build(library: LibraryFile, progress_path: PathBuf, thumbs: Thumbs) -> Self {
        let loaded = library.load();
        Self {
            state: Rc::new(RefCell::new(State {
                library: loaded.library,
                broken: loaded.broken,
                file: library,
            })),
            progress: RefCell::new(Progress::load(&progress_path)),
            progress_mtime: Cell::new(mtime(&progress_path)),
            progress_path,
            fetcher: Fetcher::new(),
            thumbs,
            known: RefCell::default(),
            channels: RefCell::default(),
            set_search: None,
            notifier: Arc::default(),
            pruned: Cell::new(false),
        }
    }

    /// Lets navigation rows (channels, lists, a video's actions) rewrite
    /// the launcher query.
    pub fn with_search_setter(mut self, set_search: Rc<dyn Fn(&str) + 'static>) -> Self {
        self.set_search = Some(set_search);
        self
    }

    fn repaint(&self) -> Rc<dyn Fn()> {
        let notifier = Arc::clone(&self.notifier);
        Rc::new(move || {
            if let Ok(slot) = notifier.lock()
                && let Some(n) = slot.as_ref()
            {
                n();
            }
        })
    }

    fn seed(&self, text: String) -> Rc<dyn Fn()> {
        let setter = self.set_search.clone();
        Rc::new(move || {
            if let Some(set) = &setter {
                set(&text);
            }
        })
    }

    // ── rows ────────────────────────────────────────────────────────────

    fn info(id: &str, name: impl Into<String>, description: impl Into<String>) -> LauncherItem {
        LauncherItem {
            id: format!("yt:info:{id}"),
            name: name.into(),
            description: description.into(),
            icon: "dialog-information-symbolic".into(),
            icon_is_path: false,
            score: 300.0,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    fn nav(
        &self,
        id: &str,
        name: String,
        description: String,
        icon: &str,
        score: f64,
        target: String,
    ) -> LauncherItem {
        LauncherItem {
            id: format!("yt:nav:{id}"),
            name,
            description,
            icon: icon.into(),
            icon_is_path: false,
            score,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: self.seed(target),
        }
    }

    fn icon_for(&self, key: &str, url: Option<&str>) -> (String, bool) {
        match self.thumbs.get(key, url) {
            Some(path) => (path.to_string_lossy().into_owned(), true),
            None => (ICON.to_string(), false),
        }
    }

    /// One video row. `ctx` tags where it is listed (`s`earch, `l`ater,
    /// `h`istory, `c`ontinue, `f`eed, `u`rl) so Ctrl+Delete knows which
    /// list to edit.
    fn video_row(&self, v: &VideoView, ctx: char, score: f64) -> LauncherItem {
        self.known.borrow_mut().insert(v.id.clone(), v.clone());
        let mut parts: Vec<String> = Vec::new();
        if !v.channel.is_empty() {
            parts.push(v.channel.clone());
        }
        if v.live {
            parts.push("LIVE".into());
        } else if let Some(d) = v.duration {
            parts.push(format_duration(d));
        }
        if let Some(p) = self.progress.borrow().videos.get(&v.id) {
            if p.resumable() {
                parts.push(format!("▶ from {}", format_duration(p.position)));
            } else if p.completed {
                parts.push("watched".into());
            }
        }
        if self.state.borrow().library.is_saved(&v.id) && ctx != 'l' {
            parts.push("in Watch later".into());
        }
        let (icon, icon_is_path) = self.icon_for(&v.id, Some(&listing::video_thumbnail(&v.id)));
        let video = v.clone();
        LauncherItem {
            id: format!("yt:v:{ctx}:{}", v.id),
            name: v.title.clone(),
            description: parts.join(" · "),
            icon,
            icon_is_path,
            score,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(move || mshellctl_play("play", &video.url(), Some(&video))),
        }
    }

    fn channel_row(&self, c: &Channel, followers: Option<u64>, score: f64) -> LauncherItem {
        self.channels.borrow_mut().insert(c.id.clone(), c.clone());
        let mut desc = vec!["Channel".to_string()];
        if let Some(n) = followers {
            desc.push(format!("{} subscribers", format_count(n)));
        }
        if self.state.borrow().library.is_favourite(&c.id) {
            desc.push("★ favourite".into());
        }
        let (icon, icon_is_path) = self.icon_for(&c.id, c.image.as_deref());
        let mut row = self.nav(
            &format!("ch:{}", c.id),
            c.name.clone(),
            desc.join(" · "),
            "avatar-default-symbolic",
            score,
            format!("{PREFIX} @{} ", c.id),
        );
        row.icon = if icon_is_path {
            icon
        } else {
            "avatar-default-symbolic".into()
        };
        row.icon_is_path = icon_is_path;
        row
    }

    fn playlist_row(&self, e: &Entry, score: f64) -> LauncherItem {
        let url = format!("https://www.youtube.com/playlist?list={}", e.id);
        let mut desc = vec!["Playlist".to_string()];
        if !e.channel.is_empty() {
            desc.push(e.channel.clone());
        }
        desc.push("Enter: play all · Ctrl+Enter: queue all".into());
        LauncherItem {
            id: format!("yt:p:{}", e.id),
            name: e.title.clone(),
            description: desc.join(" · "),
            icon: "view-list-symbolic".into(),
            icon_is_path: false,
            score,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(move || mshellctl_play("play", &url, None)),
        }
    }

    fn entry_rows(&self, entries: &[Entry], ctx: char, base: f64) -> Vec<LauncherItem> {
        entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let score = base - i as f64;
                match e.kind {
                    Kind::Video => self.video_row(&VideoView::from_entry(e), ctx, score),
                    Kind::Channel => self.channel_row(
                        &Channel {
                            id: e.id.clone(),
                            name: e.title.clone(),
                            image: e.image.clone(),
                        },
                        e.followers,
                        score,
                    ),
                    Kind::Playlist => self.playlist_row(e, score),
                }
            })
            .collect()
    }

    fn fetch_rows(
        &self,
        key: &str,
        requests: Vec<Request>,
        limit: usize,
        what: &str,
        ctx: char,
    ) -> Vec<LauncherItem> {
        let fetched = match self.fetcher.peek(key) {
            Some(cached) => cached,
            None if !margo_fs::path::on_path("yt-dlp") => {
                return vec![Self::info(
                    "noytdlp",
                    "yt-dlp is not installed",
                    "Install the yt-dlp package to search YouTube",
                )];
            }
            None => self.fetcher.get(key, requests, limit),
        };
        match fetched {
            Fetch::Loading => vec![Self::info(
                "loading",
                format!("Searching YouTube for {what}…"),
                "Results appear in a moment",
            )],
            Fetch::Failed(err) => vec![Self::info("error", "YouTube search failed", err)],
            Fetch::Ready(entries) if entries.is_empty() => {
                vec![Self::info(
                    "empty",
                    "Nothing found",
                    format!("No results for {what}"),
                )]
            }
            Fetch::Ready(entries) => {
                for e in &entries {
                    if e.kind == Kind::Video
                        && listing::valid_channel_id(&e.channel_id)
                        && !e.channel.is_empty()
                    {
                        self.channels
                            .borrow_mut()
                            .entry(e.channel_id.clone())
                            .or_insert_with(|| Channel {
                                id: e.channel_id.clone(),
                                name: e.channel.clone(),
                                image: None,
                            });
                    }
                }
                self.entry_rows(&entries, ctx, 200.0)
            }
        }
    }

    // ── views ───────────────────────────────────────────────────────────

    fn home(&self) -> Vec<LauncherItem> {
        let mut rows = Vec::new();
        if let Some(err) = &self.state.borrow().broken {
            rows.push(Self::info(
                "broken",
                "library.json could not be read",
                format!("{err} — fix or move it; Watch later is read-only until then"),
            ));
        }
        let progress = self.progress.borrow().clone();
        for (i, (id, p)) in progress
            .continue_watching()
            .into_iter()
            .take(HOME_CONTINUE)
            .enumerate()
        {
            rows.push(self.video_row(&VideoView::from_progress(id, p), 'c', 250.0 - i as f64));
        }
        let (later, favs) = {
            let st = self.state.borrow();
            (st.library.watch_later.len(), st.library.favourites.len())
        };
        let history = progress.history().len();
        rows.push(self.nav(
            "later",
            "Watch later".into(),
            format!("{later} saved videos"),
            "document-open-recent-symbolic",
            200.0,
            format!("{PREFIX} :later "),
        ));
        if favs > 0 {
            rows.push(self.nav(
                "feed",
                "Favourite channels feed".into(),
                format!("Latest uploads from {favs} channels"),
                "starred-symbolic",
                199.0,
                format!("{PREFIX} :feed"),
            ));
        }
        rows.push(self.nav(
            "channels",
            "Favourite channels".into(),
            format!("{favs} channels"),
            "avatar-default-symbolic",
            198.0,
            format!("{PREFIX} :channels "),
        ));
        rows.push(self.nav(
            "history",
            "History".into(),
            format!("{history} played videos"),
            "document-open-recent-symbolic",
            197.0,
            format!("{PREFIX} :history "),
        ));
        let mut hint = Self::info(
            "hint",
            "Search YouTube",
            "Type after `yt ` — or paste a link; Ctrl+Enter on a video shows its actions",
        );
        hint.score = 100.0;
        rows.push(hint);
        rows
    }

    fn later(&self, filter: &str) -> Vec<LauncherItem> {
        let needle = filter.to_lowercase();
        let saved: Vec<VideoView> = self
            .state
            .borrow()
            .library
            .watch_later
            .iter()
            .map(VideoView::from_saved)
            .collect();
        let rows: Vec<LauncherItem> = saved
            .iter()
            .filter(|v| matches(v, &needle))
            .enumerate()
            .map(|(i, v)| self.video_row(v, 'l', 200.0 - i as f64 * 0.01))
            .collect();
        if rows.is_empty() {
            return vec![Self::info(
                "nolater",
                "Watch later is empty",
                "Ctrl+Enter on any video → Save to Watch later",
            )];
        }
        rows
    }

    fn history(&self, filter: &str) -> Vec<LauncherItem> {
        let needle = filter.to_lowercase();
        let progress = self.progress.borrow().clone();
        let mut rows: Vec<LauncherItem> = progress
            .history()
            .into_iter()
            .map(|(id, p)| VideoView::from_progress(id, p))
            .filter(|v| matches(v, &needle))
            .take(HISTORY_ROWS)
            .enumerate()
            .map(|(i, v)| self.video_row(&v, 'h', 200.0 - i as f64 * 0.01))
            .collect();
        if rows.is_empty() {
            return vec![Self::info(
                "nohistory",
                "No history yet",
                "Videos you play from here are listed with their resume point",
            )];
        }
        if needle.is_empty() {
            rows.push(self.nav(
                "clearhist",
                "Clear history…".into(),
                "Forget every played video and resume point".into(),
                "edit-clear-all-symbolic",
                0.0,
                format!("{PREFIX} :clear-history"),
            ));
        }
        rows
    }

    fn clear_history(&self) -> Vec<LauncherItem> {
        let n = self.progress.borrow().videos.len();
        let repaint = self.repaint();
        vec![LauncherItem {
            id: "yt:act:clearhist".into(),
            name: format!("Clear history — forget {n} videos"),
            description: "Removes every history entry and saved position. Cannot be undone.".into(),
            icon: "edit-clear-all-symbolic".into(),
            icon_is_path: false,
            score: 200.0,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(move || {
                mshellctl_forget(None);
                toast(PROVIDER, "History cleared");
                repaint();
            }),
        }]
    }

    fn channels_view(&self, filter: &str) -> Vec<LauncherItem> {
        let needle = filter.to_lowercase();
        let favs = self.state.borrow().library.favourites.clone();
        let rows: Vec<LauncherItem> = favs
            .iter()
            .filter(|c| needle.is_empty() || c.name.to_lowercase().contains(&needle))
            .enumerate()
            .map(|(i, c)| self.channel_row(c, None, 200.0 - i as f64 * 0.01))
            .collect();
        if rows.is_empty() {
            return vec![Self::info(
                "nofav",
                "No favourite channels",
                "Open a channel (`yt @…`) and choose ★ Add to favourites",
            )];
        }
        rows
    }

    fn feed(&self, filter: &str) -> Vec<LauncherItem> {
        let favs = self.state.borrow().library.favourites.clone();
        if favs.is_empty() {
            return self.channels_view("");
        }
        let key = format!(
            "feed:{}",
            favs.iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        let requests = favs
            .iter()
            .map(|c| Request::Channel(c.id.clone()))
            .collect();
        let needle = filter.to_lowercase();
        self.fetch_rows(&key, requests, FEED_PER_CHANNEL, "your channels", 'f')
            .into_iter()
            .filter(|r| {
                needle.is_empty()
                    || r.id.starts_with("yt:info:")
                    || r.name.to_lowercase().contains(&needle)
                    || r.description.to_lowercase().contains(&needle)
            })
            .collect()
    }

    fn channel_name(&self, id: &str) -> Option<String> {
        if let Some(c) = self
            .state
            .borrow()
            .library
            .favourites
            .iter()
            .find(|c| c.id == id)
        {
            return Some(c.name.clone());
        }
        self.channels.borrow().get(id).map(|c| c.name.clone())
    }

    fn channel(&self, id: &str, filter: &str) -> Vec<LauncherItem> {
        let name = self
            .channel_name(id)
            .unwrap_or_else(|| "this channel".into());
        let is_fav = self.state.borrow().library.is_favourite(id);
        let channel = self.channels.borrow().get(id).cloned().unwrap_or(Channel {
            id: id.to_string(),
            name: name.clone(),
            image: None,
        });
        let state = Rc::clone(&self.state);
        let repaint = self.repaint();
        let fav_row = LauncherItem {
            id: format!("yt:act:fav:{id}"),
            name: if is_fav {
                format!("★ Remove {name} from favourites")
            } else {
                format!("☆ Add {name} to favourite channels")
            },
            description: "Favourites feed: `yt :feed`".into(),
            icon: "starred-symbolic".into(),
            icon_is_path: false,
            score: 300.0,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(move || {
                let added = with_library(&state, |lib| lib.toggle_favourite(channel.clone()));
                if let Some(added) = added {
                    toast(
                        PROVIDER,
                        if added {
                            "Added to favourite channels"
                        } else {
                            "Removed from favourite channels"
                        },
                    );
                }
                repaint();
            }),
        };
        let (key, req, what) = if filter.is_empty() {
            (
                format!("ch:{id}"),
                Request::Channel(id.to_string()),
                format!("{name}'s videos"),
            )
        } else {
            (
                format!("chs:{id}:{filter}"),
                Request::ChannelSearch(id.to_string(), filter.to_string()),
                format!("“{filter}” on {name}"),
            )
        };
        let mut rows = vec![fav_row];
        rows.extend(self.fetch_rows(&key, vec![req], PAGE, &what, 's'));
        rows
    }

    fn lookup(&self, id: &str) -> VideoView {
        if let Some(v) = self.known.borrow().get(id) {
            return v.clone();
        }
        if let Some(v) = self
            .state
            .borrow()
            .library
            .watch_later
            .iter()
            .find(|v| v.id == id)
        {
            return VideoView::from_saved(v);
        }
        if let Some(p) = self.progress.borrow().videos.get(id) {
            return VideoView::from_progress(id, p);
        }
        VideoView {
            id: id.to_string(),
            title: format!("YouTube · {id}"),
            channel: String::new(),
            channel_id: String::new(),
            duration: None,
            live: false,
        }
    }

    /// `yt !<id>`: every action on one video.
    fn video_page(&self, id: &str) -> Vec<LauncherItem> {
        let v = self.lookup(id);
        let saved = self.state.borrow().library.is_saved(id);
        let resume = self
            .progress
            .borrow()
            .videos
            .get(id)
            .filter(|p| p.resumable())
            .map(|p| p.position);
        let act = |tag: &str,
                   name: String,
                   description: String,
                   icon: &str,
                   score: f64,
                   f: Rc<dyn Fn()>| LauncherItem {
            id: format!("yt:act:{tag}:{id}"),
            name,
            description,
            icon: icon.into(),
            icon_is_path: false,
            score,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: f,
        };
        let mut rows = Vec::new();
        let (vp, vq, vs) = (v.clone(), v.clone(), v.clone());
        rows.push(act(
            "play",
            format!("Play — {}", v.title),
            match resume {
                Some(pos) => format!("Resumes from {}", format_duration(pos)),
                None => "In the mpv window".into(),
            },
            "media-playback-start-symbolic",
            300.0,
            Rc::new(move || mshellctl_play("play", &vp.url(), Some(&vp))),
        ));
        if resume.is_some() {
            rows.push(act(
                "start",
                "Play from the start".into(),
                "Ignore the saved position".into(),
                "media-skip-backward-symbolic",
                299.0,
                Rc::new(move || mshellctl_play("play", &format!("{}&t=0", vs.url()), Some(&vs))),
            ));
        }
        rows.push(act(
            "queue",
            "Add to queue".into(),
            "Plays after the current video, in the same window".into(),
            "list-add-symbolic",
            298.0,
            Rc::new(move || mshellctl_play("queue", &vq.url(), Some(&vq))),
        ));
        let state = Rc::clone(&self.state);
        let repaint = self.repaint();
        let vl = v.clone();
        rows.push(act(
            "later",
            if saved {
                "Remove from Watch later".into()
            } else {
                "Save to Watch later".into()
            },
            "Watch later: `yt :later`".into(),
            "document-open-recent-symbolic",
            297.0,
            Rc::new(move || {
                if let Some(added) =
                    with_library(&state, |lib| lib.toggle_watch_later(vl.to_saved()))
                {
                    toast(
                        PROVIDER,
                        if added {
                            "Saved to Watch later"
                        } else {
                            "Removed from Watch later"
                        },
                    );
                }
                repaint();
            }),
        ));
        if listing::valid_channel_id(&v.channel_id) {
            let label = if v.channel.is_empty() {
                "the channel".to_string()
            } else {
                v.channel.clone()
            };
            if !v.channel.is_empty() {
                self.channels
                    .borrow_mut()
                    .entry(v.channel_id.clone())
                    .or_insert_with(|| Channel {
                        id: v.channel_id.clone(),
                        name: v.channel.clone(),
                        image: None,
                    });
            }
            rows.push(self.nav(
                &format!("vch:{}", v.channel_id),
                format!("Open {label}"),
                "Latest uploads · ★ favourite".into(),
                "avatar-default-symbolic",
                296.0,
                format!("{PREFIX} @{} ", v.channel_id),
            ));
        }
        let (vc, vb) = (v.url(), v.url());
        rows.push(act(
            "copy",
            "Copy link".into(),
            v.url(),
            "edit-copy-symbolic",
            295.0,
            Rc::new(move || copy_text(vc.clone())),
        ));
        rows.push(act(
            "browser",
            "Open in the browser".into(),
            v.url(),
            "web-browser-symbolic",
            294.0,
            Rc::new(move || open_url(&vb)),
        ));
        rows
    }

    fn url_rows(&self, url: &str) -> Vec<LauncherItem> {
        if let Some(id) = listing::video_id_from_url(url) {
            // A pasted timestamp wins over the saved position — pass the
            // URL through unchanged for Play.
            let mut rows = self.video_page(&id);
            if let Some(play) = rows.first_mut() {
                let mut v = self.lookup(&id);
                let link = url.to_string();
                v.id = id.clone();
                play.on_activate = Rc::new(move || mshellctl_play("play", &link, Some(&v)));
            }
            return rows;
        }
        if mshell_youtube_host(url) {
            let (a, b) = (url.to_string(), url.to_string());
            return vec![
                LauncherItem {
                    id: "yt:u:play".into(),
                    name: "Play link in mpv".into(),
                    description: url.into(),
                    icon: "media-playback-start-symbolic".into(),
                    icon_is_path: false,
                    score: 300.0,
                    provider_name: PROVIDER.into(),
                    usage_key: None,
                    on_activate: Rc::new(move || mshellctl_play("play", &a, None)),
                },
                LauncherItem {
                    id: "yt:u:queue".into(),
                    name: "Add link to the queue".into(),
                    description: url.into(),
                    icon: "list-add-symbolic".into(),
                    icon_is_path: false,
                    score: 299.0,
                    provider_name: PROVIDER.into(),
                    usage_key: None,
                    on_activate: Rc::new(move || mshellctl_play("queue", &b, None)),
                },
            ];
        }
        vec![Self::info(
            "notyt",
            "Not a YouTube link",
            "Paste a youtube.com or youtu.be URL",
        )]
    }

    /// Re-read `progress.json` when it changed (a `stat` otherwise).
    fn reload_progress(&self) {
        let now = mtime(&self.progress_path);
        if now != self.progress_mtime.get() || now.is_none() {
            *self.progress.borrow_mut() = Progress::load(&self.progress_path);
            self.progress_mtime.set(now);
        }
    }
}

fn mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// youtube.com / youtu.be host (playlists, channels, …).
fn mshell_youtube_host(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or("");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.strip_prefix("www.").unwrap_or(host);
    matches!(
        host,
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be"
    )
}

/// Edit the library and save it; `None` (with a toast) when the file on
/// disk is broken and must not be overwritten.
fn with_library<T>(state: &Rc<RefCell<State>>, f: impl FnOnce(&mut Library) -> T) -> Option<T> {
    let mut st = state.borrow_mut();
    if st.broken.is_some() {
        toast(PROVIDER, "library.json does not parse — not saving over it");
        return None;
    }
    let out = f(&mut st.library);
    if let Err(err) = st.file.save(&st.library) {
        tracing::warn!(%err, "yt: saving library.json failed");
        toast(PROVIDER, format!("Could not save: {err}"));
    }
    Some(out)
}

impl Default for YoutubeProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn prefix_rest(query: &str) -> Option<&str> {
    let q = query.trim_start();
    let rest = q.strip_prefix(PREFIX)?;
    (rest.is_empty() || rest.starts_with(' ')).then_some(rest)
}

impl Provider for YoutubeProvider {
    fn name(&self) -> &str {
        PROVIDER
    }

    fn category(&self) -> &str {
        "Search"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        prefix_rest(query).is_some()
    }

    fn bypasses_category_for_query(&self, query: &str) -> bool {
        prefix_rest(query).is_some_and(|r| !r.trim().is_empty())
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        self.fetcher.set_notifier(Arc::clone(&notifier));
        self.thumbs.set_notifier(Arc::clone(&notifier));
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    fn on_opened(&mut self) {
        let loaded = self.state.borrow().file.load();
        {
            let mut st = self.state.borrow_mut();
            st.library = loaded.library;
            st.broken = loaded.broken;
        }
        self.reload_progress();
        self.fetcher.drop_failures();
        if self.thumbs.fetching() && !self.pruned.replace(true) {
            let dir = self.thumbs.dir().to_path_buf();
            std::thread::spawn(move || thumbs::prune(&dir));
        }
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "yt:palette".into(),
            name: "yt <query>".into(),
            description: "Search YouTube and play in mpv (Watch later, channels, history)".into(),
            icon: ICON.into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(rest) = prefix_rest(query) else {
            return Vec::new();
        };
        // The tracker writes progress while mpv plays; a cheap re-read
        // keeps "▶ from 2:03" current between opens.
        self.reload_progress();
        match route(rest) {
            Route::Home => self.home(),
            Route::Search(q) => self.fetch_rows(
                &format!("s:{q}"),
                vec![Request::Search(q.clone())],
                PAGE,
                &format!("“{q}”"),
                's',
            ),
            Route::Url(url) => self.url_rows(&url),
            Route::Channel { id, filter } => self.channel(&id, &filter),
            Route::Later(f) => self.later(&f),
            Route::Feed(f) => self.feed(&f),
            Route::Channels(f) => self.channels_view(&f),
            Route::History(f) => self.history(&f),
            Route::ClearHistory => self.clear_history(),
            Route::Video(id) => self.video_page(&id),
            Route::Hint(text) => vec![Self::info(
                "hint",
                text,
                "yt <text> searches · yt :later · yt :feed · yt :history",
            )],
        }
    }

    /// Search tab — one row that seeds the prefix (no YouTube request is
    /// made for generic Search-tab typing).
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        let trimmed = filter.trim();
        if self.bypasses_category_for_query(trimmed) {
            return self.search(trimmed);
        }
        let needle = trimmed.to_ascii_lowercase();
        if !(needle.is_empty()
            || ["yt", "youtube", "video", "mpv"]
                .iter()
                .any(|k| k.contains(&needle) || needle.contains(k)))
        {
            return Vec::new();
        }
        vec![self.nav(
            "engine",
            "YouTube".into(),
            "yt <query> — search, Watch later, channels, history".into(),
            ICON,
            204.0,
            format!("{PREFIX} "),
        )]
    }

    /// Ctrl+Enter on a video → its actions page; on a playlist → queue it.
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        if let Some(rest) = item.id.strip_prefix("yt:v:") {
            let id = rest.split_once(':')?.1;
            return Some(self.seed(format!("{PREFIX} !{id}")));
        }
        if let Some(list) = item.id.strip_prefix("yt:p:") {
            let url = format!("https://www.youtube.com/playlist?list={list}");
            return Some(Rc::new(move || mshellctl_play("queue", &url, None)));
        }
        None
    }

    fn keeps_open(&self, item: &LauncherItem, alt: bool) -> bool {
        let id = item.id.as_str();
        id.starts_with("yt:nav:")
            || id.starts_with("yt:info:")
            || id.starts_with("yt:act:later:")
            || id.starts_with("yt:act:fav:")
            || id == "yt:act:clearhist"
            || (alt && id.starts_with("yt:v:"))
    }

    fn can_delete(&self, item: &LauncherItem) -> bool {
        if let Some(id) = item.id.strip_prefix("yt:nav:ch:") {
            return self.state.borrow().library.is_favourite(id);
        }
        ["yt:v:l:", "yt:v:h:", "yt:v:c:"]
            .iter()
            .any(|p| item.id.starts_with(p))
    }

    fn delete_item(&mut self, item: &LauncherItem) {
        if let Some(id) = item.id.strip_prefix("yt:v:l:") {
            if with_library(&self.state, |lib| lib.remove_watch_later(id)) == Some(true) {
                toast(PROVIDER, "Removed from Watch later");
            }
        } else if let Some(id) = item
            .id
            .strip_prefix("yt:v:h:")
            .or_else(|| item.id.strip_prefix("yt:v:c:"))
        {
            mshellctl_forget(Some(id));
            // Hide it now; the file catches up when mshellctl has run.
            self.progress.borrow_mut().videos.remove(id);
        } else if let Some(id) = item.id.strip_prefix("yt:nav:ch:")
            && with_library(&self.state, |lib| {
                let before = lib.favourites.len();
                lib.favourites.retain(|c| c.id != id);
                before != lib.favourites.len()
            }) == Some(true)
        {
            toast(PROVIDER, "Removed from favourite channels");
        }
    }

    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        Some(crate::LauncherPreview::text(
            item.name.clone(),
            item.description.clone(),
        ))
    }
}

#[cfg(test)]
mod tests;
