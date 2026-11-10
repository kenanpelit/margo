//! `fs <query>` — fuzzy file and folder search.
//!
//! Ported from noctalia's `file-search` plugin, rebuilt native: instead of
//! piping through `fzf`, the provider builds an in-memory path index off
//! the GTK thread and fuzzy-matches it with nucleo (the matcher the rest
//! of the launcher uses).
//!
//! **Where** (Settings → Launcher → File search, `launcher.file_search`;
//! also switchable from an empty `fs` query) — see [`scope`]:
//!
//! | Scope | Covers |
//! |---|---|
//! | Home only (default) | `$HOME` |
//! | Home + folders | `$HOME` plus the extra folders (`fs +<path>` adds one) |
//! | Whole system | `/`, minus pseudo / tmpfs / network / FUSE / snapshot mounts |
//!
//! "Include USB disks" adds the mounted USB / removable volumes to any
//! scope. Each combination keeps its own index on disk ([`store`]), so
//! switching back never walks again. With USB disks on, the index is built
//! only by "Rebuild search index" — plugging a disk in only marks it out
//! of date.
//!
//! Index source per root, best first (detected at runtime, see [`index`]):
//! 1. `fd` — fast walk, honouring `.gitignore`, skipping hidden entries and
//!    build trees; home and extra folders follow symlinked folders.
//! 2. `plocate` — paths from the updatedb database (as fresh as the last
//!    `updatedb` run). USB disks and roots under updatedb's `PRUNEPATHS`
//!    are not in it, and listing the database for `/` is slower than
//!    walking: those take the built-in walk, as does a root plocate
//!    returns nothing for (e.g. a bind-mounted `/home` updatedb skipped).
//! 3. A bounded std walk when neither tool is installed.
//!
//! | Key | Action |
//! |---|---|
//! | Enter | open with the MIME default (`xdg-open`); folders open in the file manager |
//! | Ctrl+Enter | reveal in the file manager (`FileManager1.ShowItems`, else open the parent folder) |
//!
//! A filename match outranks a match that only hits the parent folders,
//! and opened paths get the usual frecency boost.

pub mod index;
pub mod mounts;
pub mod scope;
pub mod store;

use crate::providers::bg_cache::BgCache;
use crate::providers::tools::tildify;
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
    scoring::make_matcher,
};
pub use index::{Backend, Entry, Index, choose_backend};
use index::{BuildSpec, Cancel, MAX_INDEX, MAX_INDEX_SYSTEM, build_index};
use margo_fs::path::on_path as command_exists;
use nucleo_matcher::{
    Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
pub use scope::{FolderError, FsSettings, Refresh, SearchScope};
use scope::{compute_roots, plan_key, tilde, validate_new_folder};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use store::{BuildFn, IndexStore, Job, View};

const NAME: &str = "Files";
const PREFIX: &str = "fs";
/// Result rows per query.
const MAX_RESULTS: usize = 40;
/// The held index (~30 MB for 200k paths) is dropped after this long
/// without an `fs` query; it stays on disk for the next one.
const INDEX_IDLE: Duration = scope::HOME_TTL;
/// A mount scan for "Include USB disks" stays good this long.
const USB_PROBE_TTL: Duration = Duration::from_secs(30);
/// Longest a walk may run before it is cancelled: home and folders (and
/// `/`) are local and take seconds; spinning USB disks can take minutes.
const BUILD_TIMEOUT: Duration = Duration::from_secs(600);
const BUILD_TIMEOUT_USB: Duration = Duration::from_secs(1800);

/// Case-insensitive ordered-subsequence test — a cheap prefilter so the
/// nucleo scorer only runs on plausible candidates out of ~10⁵ paths.
/// `needle` must already be lowercase. ASCII needles take a byte loop.
fn subsequence_ci(hay: &str, needle: &str) -> bool {
    if needle.is_ascii() {
        let mut hay = hay.bytes();
        return needle
            .bytes()
            .all(|n| hay.any(|h| h.to_ascii_lowercase() == n));
    }
    let mut hay = hay.chars().flat_map(char::to_lowercase);
    needle.chars().all(|n| hay.any(|h| h == n))
}

/// Rank `entries` for `query`. Each whitespace-separated token must be a
/// subsequence of the path. A filename match scores its nucleo score
/// plus a bonus, a parent-folder-only match its path score; ties go to
/// the shorter path.
pub fn rank<'a>(entries: &'a [Entry], query: &str, limit: usize) -> Vec<(&'a Entry, f64)> {
    let lower = query.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    // Parse once and reuse the UTF-32 buffer: per-item parsing dominated
    // the cost on a ~200k-path home.
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = make_matcher();
    let mut buf = Vec::new();
    let mut score = |hay: &str| {
        pattern
            .score(Utf32Str::new(hay, &mut buf), &mut matcher)
            .map(f64::from)
    };
    let mut hits: Vec<(&Entry, f64)> = entries
        .iter()
        .filter(|e| tokens.iter().all(|t| subsequence_ci(&e.path, t)))
        .filter_map(|e| {
            let name = e.file_name();
            // Only pay for a filename score when the name can match.
            let name_score = if tokens.iter().all(|t| subsequence_ci(name, t)) {
                score(name)
            } else {
                None
            };
            match name_score {
                Some(s) => Some((e, s + 1000.0)),
                None => score(&e.path).map(|s| (e, s)),
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.path.len().cmp(&b.0.path.len()))
            .then_with(|| a.0.path.cmp(&b.0.path))
    });
    hits.truncate(limit);
    hits
}

/// Shortest filter worth searching: one character matches most of a home
/// directory and only costs time.
const MIN_QUERY_CHARS: usize = 2;

/// Ranked results for one query.
type Hits = Vec<(Entry, f64)>;

#[derive(Default)]
struct SearchState {
    /// Latest query the UI asked for (with the index to search).
    wanted: Option<(String, Arc<Index>)>,
    /// Last finished search, with the index it ran over: a refreshed or
    /// switched index must not be answered from the old one's hits. Weak,
    /// so a finished search never keeps a dropped index's paths alive.
    done: Option<(String, Weak<Index>, Hits)>,
    running: bool,
}

/// Runs [`rank`] on a worker thread — on a ~200k-path home a broad query
/// takes hundreds of ms, far too long for the GTK thread on every
/// keystroke. One worker at a time; it always picks up the newest wanted
/// query, so fast typing skips the intermediate ones.
#[derive(Default)]
struct Searcher {
    state: Arc<Mutex<SearchState>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

impl Searcher {
    /// Finished hits for `query`, or `None` while they're computed (the
    /// notifier repaints when they land). Also returns the last finished
    /// result set for showing while typing.
    fn request(&self, query: &str, index: &Arc<Index>) -> (Option<Hits>, Option<Hits>) {
        let Ok(mut st) = self.state.lock() else {
            return (None, None);
        };
        if let Some((q, searched, hits)) = &st.done
            && q == query
            && Weak::ptr_eq(searched, &Arc::downgrade(index))
        {
            return (Some(hits.clone()), None);
        }
        let previous = st.done.as_ref().map(|(_, _, h)| h.clone());
        st.wanted = Some((query.to_string(), Arc::clone(index)));
        if !st.running {
            st.running = true;
            let state = Arc::clone(&self.state);
            let notifier = Arc::clone(&self.notifier);
            std::thread::spawn(move || {
                loop {
                    let job = state.lock().ok().and_then(|mut s| {
                        let job = s.wanted.take();
                        if job.is_none() {
                            s.running = false;
                        }
                        job
                    });
                    let Some((q, index)) = job else { break };
                    let hits: Hits = rank(&index.entries, &q, MAX_RESULTS)
                        .into_iter()
                        .map(|(e, s)| (e.clone(), s))
                        .collect();
                    if let Ok(mut s) = state.lock() {
                        s.done = Some((q, Arc::downgrade(&index), hits));
                    }
                    if let Ok(slot) = notifier.lock()
                        && let Some(notify) = slot.as_ref()
                    {
                        notify();
                    }
                }
            });
        }
        (None, previous)
    }

    #[cfg(test)]
    fn wait_idle(&self) {
        for _ in 0..1000 {
            if self.state.lock().map(|s| !s.running).unwrap_or(true) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// `file://` URI with every byte outside the unreserved set (and `/`)
/// percent-encoded, as FileManager1.ShowItems expects.
pub fn file_uri(path: &str) -> String {
    let mut out = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// gdbus argv revealing `path` (selected) in the file manager.
pub fn reveal_argv(path: &str) -> Vec<String> {
    [
        "gdbus",
        "call",
        "--session",
        "--dest",
        "org.freedesktop.FileManager1",
        "--object-path",
        "/org/freedesktop/FileManager1",
        "--method",
        "org.freedesktop.FileManager1.ShowItems",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!("['{}']", file_uri(path)), String::new()])
    .collect()
}

fn open_path(path: &str) {
    if let Err(err) = mshell_process::desktop::open(path) {
        tracing::warn!(?err, path, "file search: xdg-open failed");
        toast("Files", format!("Could not open {path}"));
    }
}

/// Reveal via FileManager1; when no file manager implements it (or gdbus
/// is missing) open the parent folder instead. Off-thread: gdbus blocks
/// until the file manager answers.
fn reveal_path(path: String) {
    std::thread::spawn(move || {
        let argv = reveal_argv(&path);
        let ok = Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            let parent = Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".into());
            open_path(&parent);
        }
    });
}

/// Settings source: the live config in the shell, defaults in tests.
type SettingsFn = Box<dyn Fn() -> FsSettings>;
/// Persists changed settings (the shell writes `launcher.file_search`).
type SaveFn = Rc<dyn Fn(FsSettings)>;

/// Latest-wins off-thread `stat` of a folder typed after `fs +`, so the
/// GTK thread never touches a (possibly network) path.
#[derive(Default)]
struct DirProbe {
    /// Path asked for and its answer once known (`None` = nothing there).
    slot: Arc<Mutex<(String, Option<Option<bool>>)>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

impl DirProbe {
    fn get(&self, path: &str) -> Option<Option<bool>> {
        let Ok(mut slot) = self.slot.lock() else {
            return None;
        };
        if slot.0 == path {
            return slot.1;
        }
        *slot = (path.to_string(), None);
        let shared = Arc::clone(&self.slot);
        let notifier = Arc::clone(&self.notifier);
        let path = path.to_string();
        std::thread::spawn(move || {
            let kind = std::fs::metadata(&path).ok().map(|m| m.is_dir());
            if let Ok(mut slot) = shared.lock()
                && slot.0 == path
            {
                slot.1 = Some(kind);
            }
            if let Ok(n) = notifier.lock()
                && let Some(n) = n.as_ref()
            {
                n();
            }
        });
        None
    }
}

/// The production build: probe mounts, plan the roots, walk them, with a
/// watchdog that cancels a walk running past its timeout.
fn production_build(preferred: Backend) -> BuildFn {
    Arc::new(move |job: &Job, cancel: &Cancel| {
        let s = &job.settings;
        let system = s.scope == SearchScope::System;
        let usb = if s.include_usb || system {
            mounts::removable_mounts()
        } else {
            Vec::new()
        };
        let prune = if system {
            mounts::system_prunes(&mounts::read_mounts(), &usb)
        } else {
            Vec::new()
        };
        let (roots, missing) = compute_roots(s, &job.home, &usb, &prune, |p| Path::new(p).is_dir());
        let spec = BuildSpec {
            roots,
            usb: if s.include_usb { usb } else { Vec::new() },
            missing,
            preferred,
            updatedb_prune: if preferred == Backend::Plocate {
                mounts::updatedb_prunepaths()
            } else {
                Vec::new()
            },
            max: if system { MAX_INDEX_SYSTEM } else { MAX_INDEX },
        };
        let timeout = if s.include_usb {
            BUILD_TIMEOUT_USB
        } else {
            BUILD_TIMEOUT
        };
        let done = std::sync::atomic::AtomicBool::new(false);
        let timed_out = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|sc| {
            sc.spawn(|| {
                let start = Instant::now();
                while !done.load(std::sync::atomic::Ordering::Acquire) {
                    if start.elapsed() > timeout {
                        timed_out.store(true, std::sync::atomic::Ordering::Release);
                        cancel.cancel();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            });
            let started = Instant::now();
            let built = build_index(&spec, cancel);
            done.store(true, std::sync::atomic::Ordering::Release);
            if timed_out.load(std::sync::atomic::Ordering::Acquire) {
                tracing::warn!(label = %job.label, "file search: index walk timed out");
                toast(
                    "File search",
                    format!(
                        "Indexing {} took longer than {} min and was stopped",
                        job.label,
                        timeout.as_secs() / 60
                    ),
                );
            } else if let Some(i) = &built {
                tracing::info!(
                    label = %job.label,
                    paths = i.entries.len(),
                    ms = started.elapsed().as_millis() as u64,
                    "file search: indexed"
                );
            }
            built
        })
    })
}

pub struct FileSearchProvider {
    home: PathBuf,
    preferred: Backend,
    settings: SettingsFn,
    save: Option<SaveFn>,
    set_search: Option<Rc<dyn Fn(&str) + 'static>>,
    store: Rc<IndexStore>,
    usb_probe: BgCache<Arc<Vec<String>>>,
    dir_probe: DirProbe,
    searcher: Searcher,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

impl FileSearchProvider {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let preferred = choose_backend(command_exists("fd"), command_exists("plocate"));
        let dir = index::cache_dir();
        Self::with_store(
            home,
            preferred,
            IndexStore::new(Some(dir), production_build(preferred), INDEX_IDLE),
        )
    }

    fn with_store(home: PathBuf, preferred: Backend, store: IndexStore) -> Self {
        Self {
            home,
            preferred,
            settings: Box::new(FsSettings::default),
            save: None,
            set_search: None,
            store: Rc::new(store),
            usb_probe: BgCache::new(USB_PROBE_TTL),
            dir_probe: DirProbe::default(),
            searcher: Searcher::default(),
            notifier: Arc::new(Mutex::new(None)),
        }
    }

    /// Read the scope settings from `settings` on every query and write
    /// changes made from the launcher (scope, USB toggle, added folders)
    /// through `save`.
    pub fn with_settings(mut self, settings: SettingsFn, save: SaveFn) -> Self {
        self.settings = settings;
        self.save = Some(save);
        self
    }

    /// Lets "Add search folder" put `fs +` in the search box.
    pub fn with_search_setter(mut self, set_search: Rc<dyn Fn(&str) + 'static>) -> Self {
        self.set_search = Some(set_search);
        self
    }

    #[cfg(test)]
    fn seeded(root: &str, entries: Vec<Entry>) -> Self {
        let p = Self::for_test(root, Arc::new(|_: &Job, _: &Cancel| None));
        let job = p.job();
        p.store.seed(&job.key, Index::from_entries(entries));
        p
    }

    #[cfg(test)]
    fn for_test(home: &str, build: BuildFn) -> Self {
        Self::with_store(
            PathBuf::from(home),
            Backend::Fd,
            IndexStore::new(None, build, Duration::from_secs(60)),
        )
    }

    fn home_str(&self) -> String {
        self.home.to_string_lossy().into_owned()
    }

    fn job(&self) -> Job {
        let settings = (self.settings)();
        let home = self.home_str();
        Job {
            key: plan_key(&settings, &home),
            refresh: settings.refresh(),
            label: settings.label(),
            settings,
            home,
        }
    }

    /// Mounted USB volumes (cached probe), when USB disks are on.
    fn usb_now(&self, settings: &FsSettings) -> Option<Arc<Vec<String>>> {
        if !settings.include_usb {
            return None;
        }
        self.usb_probe.get(|| Arc::new(mounts::removable_mounts()))
    }

    fn repaint(notifier: &Mutex<Option<RefreshNotifier>>) {
        if let Ok(n) = notifier.lock()
            && let Some(n) = n.as_ref()
        {
            n();
        }
    }

    fn hint(&self, id: &str, name: String, description: String, icon: &str) -> LauncherItem {
        LauncherItem {
            id: format!("fs:{id}"),
            name,
            description,
            icon: icon.into(),
            icon_is_path: false,
            score: 100.0,
            provider_name: NAME.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    fn action(
        &self,
        id: &str,
        name: String,
        description: String,
        icon: &str,
        score: f64,
        on_activate: Rc<dyn Fn() + 'static>,
    ) -> LauncherItem {
        LauncherItem {
            score,
            on_activate,
            ..self.hint(&format!("act:{id}"), name, description, icon)
        }
    }

    /// "Rebuild search index", or "Cancel indexing" while this job builds.
    fn rebuild_row(&self, job: &Job, view: &View, out_of_date: bool, score: f64) -> LauncherItem {
        let notifier = Arc::clone(&self.notifier);
        let store = Rc::clone(&self.store);
        if let Some((_, started, true)) = &view.building {
            return self.action(
                "cancel",
                "Cancel indexing".into(),
                format!(
                    "Indexing {} for {} s — Enter stops it",
                    job.label,
                    started.elapsed().as_secs()
                ),
                "process-stop-symbolic",
                score,
                Rc::new(move || {
                    store.cancel();
                    Self::repaint(&notifier);
                }),
            );
        }
        let mut desc = if out_of_date {
            format!("Out of date — walk {} again", job.label)
        } else {
            format!("Walk {} now", job.label)
        };
        if let Some((other, _, false)) = &view.building {
            desc.push_str(&format!(" (stops indexing {other})"));
        }
        let job = job.clone();
        self.action(
            "rebuild",
            "Rebuild search index".into(),
            desc,
            "view-refresh-symbolic",
            score,
            Rc::new(move || {
                store.rebuild(&job);
                Self::repaint(&notifier);
            }),
        )
    }

    /// The scope / USB / add-folder rows of an empty `fs` query.
    fn setting_rows(&self, settings: &FsSettings) -> Vec<LauncherItem> {
        let mut rows = Vec::new();
        let Some(save) = self.save.clone() else {
            return rows;
        };
        let next = match settings.scope {
            SearchScope::Home => SearchScope::HomeAndFolders,
            SearchScope::HomeAndFolders => SearchScope::System,
            SearchScope::System => SearchScope::Home,
        };
        let folders = scope::expanded_folders(&settings.folders, &self.home_str());
        let mut desc = format!("Enter switches to {}", next.label());
        if settings.scope == SearchScope::HomeAndFolders {
            desc = if folders.is_empty() {
                format!("No folders yet — {desc}")
            } else {
                let home = self.home_str();
                let list: Vec<String> = folders.iter().map(|f| tilde(f, &home)).collect();
                format!("{} · {desc}", list.join(", "))
            };
        }
        let notifier = Arc::clone(&self.notifier);
        let s = settings.clone();
        let save_scope = Rc::clone(&save);
        rows.push(self.action(
            "scope",
            format!("Search scope: {}", settings.scope.label()),
            desc,
            "find-location-symbolic",
            90.0,
            Rc::new(move || {
                save_scope(FsSettings {
                    scope: next,
                    ..s.clone()
                });
                Self::repaint(&notifier);
            }),
        ));
        let notifier = Arc::clone(&self.notifier);
        let s = settings.clone();
        let save_usb = Rc::clone(&save);
        rows.push(self.action(
            "usb",
            format!(
                "USB disks: {}",
                if settings.include_usb { "on" } else { "off" }
            ),
            if settings.include_usb {
                "Enter stops searching mounted USB / removable disks".into()
            } else {
                "Enter adds mounted USB / removable disks (indexed only on Rebuild)".into()
            },
            "drive-removable-media-symbolic",
            89.0,
            Rc::new(move || {
                save_usb(FsSettings {
                    include_usb: !s.include_usb,
                    ..s.clone()
                });
                Self::repaint(&notifier);
            }),
        ));
        if let Some(set) = self.set_search.clone() {
            rows.push(self.action(
                "addfolder",
                "Add search folder…".into(),
                "Type fs +<path>, e.g. fs +/media/data or fs +~/.config".into(),
                "folder-new-symbolic",
                88.0,
                Rc::new(move || set(&format!("{PREFIX} +"))),
            ));
        }
        rows
    }

    /// `fs +<path>`: validate (off-thread stat) and offer to add it.
    fn add_folder_rows(&self, raw: &str, settings: &FsSettings) -> Vec<LauncherItem> {
        let home = self.home_str();
        let expanded = match scope::expand_path(raw, &home) {
            Ok(p) => p,
            Err(e) => {
                return vec![self.hint(
                    "addinvalid",
                    "Add search folder".into(),
                    e.to_string(),
                    "dialog-warning-symbolic",
                )];
            }
        };
        let Some(kind) = self.dir_probe.get(&expanded) else {
            return vec![self.hint(
                "addchecking",
                "Add search folder".into(),
                format!("Checking {}…", tilde(&expanded, &home)),
                "content-loading-symbolic",
            )];
        };
        match validate_new_folder(raw, &home, &settings.folders, |_| kind) {
            Err(e) => vec![self.hint(
                "addinvalid",
                "Add search folder".into(),
                e.to_string(),
                "dialog-warning-symbolic",
            )],
            Ok(stored) => {
                let Some(save) = self.save.clone() else {
                    return Vec::new();
                };
                let mut next = settings.clone();
                next.folders.push(stored.clone());
                // Adding a folder means searching it.
                next.scope = SearchScope::HomeAndFolders;
                let notifier = Arc::clone(&self.notifier);
                let set = self.set_search.clone();
                let switch = if settings.scope == SearchScope::HomeAndFolders {
                    String::new()
                } else {
                    " and switch to Home + folders".into()
                };
                vec![self.action(
                    "add",
                    format!("Add search folder {stored}"),
                    format!("Enter adds it{switch}"),
                    "folder-new-symbolic",
                    100.0,
                    Rc::new(move || {
                        save(next.clone());
                        toast(
                            "File search",
                            format!("Added {stored} to the search folders"),
                        );
                        if let Some(set) = &set {
                            set(&format!("{PREFIX} "));
                        }
                        Self::repaint(&notifier);
                    }),
                )]
            }
        }
    }

    /// The status line of an empty `fs` query for a held index.
    fn status(&self, job: &Job, index: &Index, out_of_date: bool) -> String {
        let mut parts = vec![format!(
            "{} paths · {} via {}",
            index.entries.len(),
            job.label,
            index.backend_label()
        )];
        if index.capped {
            parts.push("capped".into());
        }
        if out_of_date {
            parts.push("out of date: USB disks changed".into());
        }
        if !index.missing.is_empty() {
            let home = self.home_str();
            let list: Vec<String> = index.missing.iter().map(|m| tilde(m, &home)).collect();
            parts.push(format!("not found: {}", list.join(", ")));
        }
        let walked = |b| index.walks.iter().any(|w| w.backend == b);
        if walked(Backend::Plocate) && walked(Backend::Walk) {
            parts.push("USB disks / folders outside updatedb walked directly".into());
        }
        parts.join(" · ")
    }

    fn row(&self, e: &Entry, score: f64) -> LauncherItem {
        let is_dir = e.is_dir.unwrap_or_else(|| Path::new(&e.path).is_dir());
        let parent = Path::new(&e.path)
            .parent()
            .map(|p| tildify(p, &self.home))
            .unwrap_or_default();
        let path = e.path.clone();
        LauncherItem {
            id: format!("fs:path:{}", e.path),
            name: e.file_name().to_string(),
            description: parent,
            icon: if is_dir {
                "folder-symbolic"
            } else {
                "text-x-generic-symbolic"
            }
            .into(),
            icon_is_path: false,
            score: 100.0 + score / 10.0,
            provider_name: NAME.into(),
            usage_key: Some(format!("fs:{}", e.path)),
            on_activate: Rc::new(move || open_path(&path)),
        }
    }
}

impl Default for FileSearchProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn strip_prefix(query: &str) -> Option<&str> {
    let q = query.trim_start();
    if q == PREFIX {
        Some("")
    } else {
        q.strip_prefix(PREFIX)
            .filter(|r| r.starts_with(char::is_whitespace))
    }
}

impl Provider for FileSearchProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn category(&self) -> &str {
        "Search"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        strip_prefix(query).is_some()
    }

    fn commands(&self) -> Vec<LauncherItem> {
        let scope = (self.settings)().label();
        vec![self.hint(
            "palette",
            PREFIX.into(),
            format!("Find files and folders — {scope} (Enter open · Ctrl+Enter reveal)"),
            "system-search-symbolic",
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(rest) = strip_prefix(query) else {
            return Vec::new();
        };
        let filter = rest.trim();
        let job = self.job();
        if let Some(raw) = filter.strip_prefix('+') {
            return self.add_folder_rows(raw, &job.settings);
        }
        let view = self.store.view(&job);
        let out_of_date = match (&view.index, self.usb_now(&job.settings)) {
            (Some(index), Some(now)) => index.usb != *now,
            _ => false,
        };
        let Some(index) = view.index.clone() else {
            let mut rows = Vec::new();
            if view.never_built {
                rows.push(self.hint(
                    "unindexed",
                    format!("{} is not indexed yet", job.label),
                    "USB disks are only walked on command — choose Rebuild search index".into(),
                    "drive-removable-media-symbolic",
                ));
            } else {
                let what = match &view.building {
                    Some((other, _, false)) => format!("waiting for the {other} index"),
                    _ => format!("{} via {}", job.label, self.preferred.label()),
                };
                rows.push(self.hint(
                    "indexing",
                    if view.loading {
                        "Loading the index…".into()
                    } else {
                        "Indexing files…".into()
                    },
                    what,
                    "content-loading-symbolic",
                ));
            }
            if !view.loading {
                rows.push(self.rebuild_row(&job, &view, false, 95.0));
            }
            if filter.is_empty() {
                rows.extend(self.setting_rows(&job.settings));
            }
            return rows;
        };
        if filter.is_empty() {
            let mut rows = vec![self.hint(
                "help",
                "Type to search files and folders".into(),
                self.status(&job, &index, out_of_date),
                "system-search-symbolic",
            )];
            rows.push(self.rebuild_row(&job, &view, out_of_date, 95.0));
            rows.extend(self.setting_rows(&job.settings));
            return rows;
        }
        if filter.chars().count() < MIN_QUERY_CHARS {
            return vec![self.hint(
                "short",
                "Keep typing…".into(),
                format!("At least {MIN_QUERY_CHARS} characters"),
                "system-search-symbolic",
            )];
        }
        let (hits, previous) = self.searcher.request(filter, &index);
        let mut rows: Vec<LauncherItem> = match hits {
            Some(hits) => hits.iter().map(|(e, s)| self.row(e, *s)).collect(),
            None => {
                // Show the previous query's rows while this one runs, so
                // the list doesn't flash empty on every keystroke.
                let rows: Vec<LauncherItem> = previous
                    .unwrap_or_default()
                    .iter()
                    .map(|(e, s)| self.row(e, *s))
                    .collect();
                if rows.is_empty() {
                    return vec![self.hint(
                        "searching",
                        "Searching…".into(),
                        filter.to_string(),
                        "content-loading-symbolic",
                    )];
                }
                rows
            }
        };
        if rows.is_empty() {
            rows.push(self.hint(
                "nomatch",
                "No matching files".into(),
                filter.to_string(),
                "edit-find-symbolic",
            ));
        }
        // An out-of-date index offers the rebuild next to its results.
        if out_of_date || matches!(view.building, Some((_, _, true))) {
            rows.push(self.rebuild_row(&job, &view, out_of_date, 1.0));
        }
        rows
    }

    /// Ctrl+Enter → reveal the path in the file manager.
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let path = item.id.strip_prefix("fs:path:")?.to_string();
        Some(Rc::new(move || reveal_path(path.clone())))
    }

    /// The scope / USB / rebuild rows relabel themselves; the launcher
    /// stays open for them.
    fn keeps_open(&self, item: &LauncherItem, _alt: bool) -> bool {
        item.id.starts_with("fs:act:")
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.searcher.notifier.lock() {
            *slot = Some(notifier.clone());
        }
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier.clone());
        }
        if let Ok(mut slot) = self.dir_probe.notifier.lock() {
            *slot = Some(notifier.clone());
        }
        self.usb_probe.set_notifier(notifier.clone());
        self.store.set_notifier(notifier);
    }

    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            // The Search tab shares space with web engines; an empty file
            // hint there is noise.
            Vec::new()
        } else {
            self.search(&format!("{PREFIX} {filter}"))
                .into_iter()
                .filter(|r| !r.id.starts_with("fs:act:"))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn e(path: &str) -> Entry {
        Entry {
            path: path.into(),
            is_dir: Some(false),
        }
    }

    #[test]
    fn filename_match_beats_folder_match_and_fuzzy_works() {
        let entries = vec![
            e("/home/u/report/notes.txt"),
            e("/home/u/docs/report.pdf"),
            e("/home/u/zzz.txt"),
        ];
        let ranked: Vec<&str> = rank(&entries, "report", 10)
            .iter()
            .map(|(e, _)| e.path.as_str())
            .collect();
        assert_eq!(
            ranked,
            vec!["/home/u/docs/report.pdf", "/home/u/report/notes.txt"]
        );
        let fuzzy: Vec<&str> = rank(&entries, "rprt", 10)
            .iter()
            .map(|(e, _)| e.path.as_str())
            .collect();
        assert_eq!(fuzzy.first(), Some(&"/home/u/docs/report.pdf"));
        assert!(rank(&entries, "qqq", 10).is_empty());
        // Tokens AND together.
        let both: Vec<&str> = rank(&entries, "docs pdf", 10)
            .iter()
            .map(|(e, _)| e.path.as_str())
            .collect();
        assert_eq!(both, vec!["/home/u/docs/report.pdf"]);
    }

    /// A refreshed index must not be answered from the previous index's
    /// hits for the same query.
    #[test]
    fn searcher_does_not_serve_hits_of_a_replaced_index() {
        let searcher = Searcher::default();
        let old = Arc::new(Index::from_entries(vec![e("/home/u/old-report.txt")]));
        let new = Arc::new(Index::from_entries(vec![e("/home/u/new-report.txt")]));
        let run = |index: &Arc<Index>| {
            if let (Some(hits), _) = searcher.request("report", index) {
                return hits;
            }
            searcher.wait_idle();
            searcher.request("report", index).0.unwrap_or_default()
        };
        assert_eq!(run(&old)[0].0.path, "/home/u/old-report.txt");
        assert_eq!(run(&new)[0].0.path, "/home/u/new-report.txt");
    }

    #[test]
    fn subsequence_is_case_insensitive_and_unicode_safe() {
        assert!(subsequence_ci("/home/u/Özet.ODT", "özodt"));
        assert!(!subsequence_ci("abc", "acb"));
    }

    #[test]
    fn reveal_uses_filemanager1_with_encoded_uri() {
        let a = reveal_argv("/home/u/My File's.txt");
        assert!(a.contains(&"org.freedesktop.FileManager1.ShowItems".to_string()));
        assert_eq!(a[a.len() - 2], "['file:///home/u/My%20File%27s.txt']");
        assert_eq!(a[a.len() - 1], "");
    }

    #[test]
    fn rows_open_and_reveal() {
        let p = FileSearchProvider::seeded(
            "/home/u",
            vec![
                Entry {
                    path: "/home/u/docs".into(),
                    is_dir: Some(true),
                },
                e("/home/u/docs/report.pdf"),
            ],
        );
        assert!(p.handles_command("fs"));
        assert!(p.handles_command("fs rep"));
        assert!(!p.handles_command("fsck"));
        let search = |q: &str| {
            let first = p.search(q);
            if first.iter().any(|r| r.id == "fs:searching")
                || first.iter().any(|r| r.id.starts_with("fs:path:"))
            {
                p.searcher.wait_idle();
            }
            p.search(q)
        };
        assert_eq!(p.search("fs r")[0].name, "Keep typing…");
        let rows = search("fs report");
        assert_eq!(rows[0].name, "report.pdf");
        assert_eq!(rows[0].description, "~/docs");
        assert_eq!(rows[0].icon, "text-x-generic-symbolic");
        assert!(p.alt_action(&rows[0]).is_some());
        let dir = search("fs docs");
        assert_eq!(dir[0].icon, "folder-symbolic");
        let empty = p.search("fs");
        assert!(empty[0].name.starts_with("Type to search"));
        assert!(empty[0].description.contains("Home only"));
        assert!(p.alt_action(&empty[0]).is_none());
        // No settings writer: no scope rows, but the rebuild row is there.
        assert!(empty.iter().any(|r| r.id == "fs:act:rebuild"));
        assert!(!empty.iter().any(|r| r.id == "fs:act:scope"));
        assert!(p.keeps_open(&empty[1], false));
        assert!(!p.keeps_open(&rows[0], false));
    }

    /// A provider whose settings live in a cell the launcher rows write.
    fn with_cell(p: FileSearchProvider, cell: &Rc<RefCell<FsSettings>>) -> FileSearchProvider {
        let read = Rc::clone(cell);
        let write = Rc::clone(cell);
        p.with_settings(
            Box::new(move || read.borrow().clone()),
            Rc::new(move |s| *write.borrow_mut() = s),
        )
    }

    fn activate(rows: &[LauncherItem], id: &str) {
        let row = rows.iter().find(|r| r.id == id).unwrap_or_else(|| {
            panic!(
                "no {id} in {:?}",
                rows.iter().map(|r| &r.id).collect::<Vec<_>>()
            )
        });
        (row.on_activate)();
    }

    #[test]
    fn launcher_rows_switch_scope_and_usb() {
        let cell = Rc::new(RefCell::new(FsSettings::default()));
        let p = with_cell(
            FileSearchProvider::for_test("/home/u", Arc::new(|_: &Job, _: &Cancel| None)),
            &cell,
        );
        let rows = p.search("fs");
        activate(&rows, "fs:act:scope");
        assert_eq!(cell.borrow().scope, SearchScope::HomeAndFolders);
        activate(&p.search("fs"), "fs:act:scope");
        assert_eq!(cell.borrow().scope, SearchScope::System);
        activate(&p.search("fs"), "fs:act:scope");
        assert_eq!(cell.borrow().scope, SearchScope::Home);
        activate(&p.search("fs"), "fs:act:usb");
        assert!(cell.borrow().include_usb);
        assert!(p.commands()[0].description.contains("USB disks"));
    }

    #[test]
    fn usb_scope_waits_for_an_explicit_rebuild() {
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let r2 = Arc::clone(&runs);
        let build: BuildFn = Arc::new(move |_: &Job, _: &Cancel| {
            r2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(Index::from_entries(vec![e("/home/u/x.txt")]))
        });
        let cell = Rc::new(RefCell::new(FsSettings {
            include_usb: true,
            ..FsSettings::default()
        }));
        let p = with_cell(FileSearchProvider::for_test("/home/u", build), &cell);
        p.search("fs");
        p.store.wait_idle();
        let rows = p.search("fs");
        assert_eq!(rows[0].id, "fs:unindexed");
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 0);
        activate(&rows, "fs:act:rebuild");
        p.store.wait_idle();
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(p.search("fs")[0].id, "fs:help");
    }

    #[test]
    fn add_folder_from_the_launcher() {
        let dir = std::env::temp_dir().join(format!("fs-add-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir_s = dir.to_string_lossy().into_owned();
        let cell = Rc::new(RefCell::new(FsSettings::default()));
        let typed = Rc::new(RefCell::new(String::new()));
        let t2 = Rc::clone(&typed);
        let p = with_cell(
            FileSearchProvider::for_test("/home/u", Arc::new(|_: &Job, _: &Cancel| None)),
            &cell,
        )
        .with_search_setter(Rc::new(move |t| *t2.borrow_mut() = t.to_string()));
        activate(&p.search("fs"), "fs:act:addfolder");
        assert_eq!(*typed.borrow(), "fs +");
        assert_eq!(p.search("fs +relative")[0].id, "fs:addinvalid");
        let q = format!("fs +{dir_s}");
        let mut rows = p.search(&q);
        for _ in 0..200 {
            if rows[0].id != "fs:addchecking" {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            rows = p.search(&q);
        }
        activate(&rows, "fs:act:add");
        assert_eq!(cell.borrow().folders, vec![dir_s.clone()]);
        assert_eq!(cell.borrow().scope, SearchScope::HomeAndFolders);
        assert_eq!(*typed.borrow(), "fs ");
        // The same folder again is refused.
        assert_eq!(p.search(&q)[0].id, "fs:addinvalid");
        let missing = format!("fs +{dir_s}/nope");
        let mut rows = p.search(&missing);
        for _ in 0..200 {
            if rows[0].id != "fs:addchecking" {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            rows = p.search(&missing);
        }
        assert!(rows[0].description.contains("does not exist"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
