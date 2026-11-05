//! `fs <query>` — fuzzy file and folder search under `$HOME`.
//!
//! Ported from noctalia's `file-search` plugin, rebuilt native: instead of
//! piping through `fzf`, the provider builds an in-memory path index off
//! the GTK thread and fuzzy-matches it with nucleo (the matcher the rest
//! of the launcher uses).
//!
//! Index source, best first (detected at runtime):
//! 1. `fd` — fast walk of `$HOME`, following symlinked folders, honouring
//!    `.gitignore`, skipping hidden entries and build trees.
//! 2. `plocate` — every non-hidden path under `$HOME` from the updatedb
//!    database (as fresh as the last `updatedb` run).
//! 3. A bounded std walk when neither tool is installed.
//!
//! | Key | Action |
//! |---|---|
//! | Enter | open with the MIME default (`xdg-open`); folders open in the file manager |
//! | Ctrl+Enter | reveal in the file manager (`FileManager1.ShowItems`, else open the parent folder) |
//!
//! A filename match outranks a match that only hits the parent folders,
//! and opened paths get the usual frecency boost.

use crate::providers::bg_cache::BgCache;
use crate::providers::tools::{command_exists, tildify};
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
    scoring::make_matcher,
};
use mshell_process::SpawnReaped;
use nucleo_matcher::{
    Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

const NAME: &str = "Files";
const PREFIX: &str = "fs";
/// Result rows per query.
const MAX_RESULTS: usize = 40;
/// Hard cap on indexed paths so a pathological tree can't eat memory.
const MAX_INDEX: usize = 400_000;
/// Re-index at most this often; a stale index is still served while the
/// refresh runs.
const INDEX_TTL: Duration = Duration::from_secs(300);
/// The index (~30 MB for 200k paths) is dropped after this long without an
/// `fs` query, instead of staying resident for the rest of the session. An
/// index this old would be rebuilt by the next query anyway.
const INDEX_IDLE: Duration = INDEX_TTL;
/// Folders never worth indexing.
const EXCLUDES: &[&str] = &["node_modules", "target", ".git", "__pycache__"];
/// Depth limit for the std fallback walk.
const WALK_DEPTH: usize = 8;

/// Where the index came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Fd,
    Plocate,
    Walk,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Backend::Fd => "fd",
            Backend::Plocate => "plocate",
            Backend::Walk => "built-in walk",
        }
    }
}

/// Best available backend.
pub fn choose_backend(has_fd: bool, has_plocate: bool) -> Backend {
    if has_fd {
        Backend::Fd
    } else if has_plocate {
        Backend::Plocate
    } else {
        Backend::Walk
    }
}

/// One indexed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    /// `Some` when the backend told us (fd marks folders with a trailing
    /// `/`); `None` → stat lazily for the few rows shown.
    pub is_dir: Option<bool>,
}

impl Entry {
    fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Debug, Default)]
pub struct Index {
    pub entries: Vec<Entry>,
    pub backend: Option<Backend>,
}

/// One `fd` output line: an absolute path, folders with a trailing `/`
/// (fd ≥ 9).
fn fd_entry(l: &str) -> Option<Entry> {
    if l.is_empty() {
        return None;
    }
    Some(match l.strip_suffix('/') {
        Some(dir) if !dir.is_empty() => Entry {
            path: dir.to_string(),
            is_dir: Some(true),
        },
        _ => Entry {
            path: l.to_string(),
            is_dir: Some(false),
        },
    })
}

/// Parse `fd` output: one absolute path per line, folders with a trailing
/// `/` (fd ≥ 9).
pub fn parse_fd(stdout: &str) -> Vec<Entry> {
    stdout
        .lines()
        .filter_map(fd_entry)
        .take(MAX_INDEX)
        .collect()
}

/// One `plocate -r ^<root>/` line, unless it is hidden or under an
/// excluded folder (plocate has no exclude flag) — matching what fd would
/// list.
fn locate_entry(l: &str, root: &str) -> Option<Entry> {
    let rel = l.strip_prefix(root)?;
    let keep = !rel.is_empty()
        && rel
            .split('/')
            .all(|c| !c.starts_with('.') && !EXCLUDES.contains(&c));
    keep.then(|| Entry {
        path: l.to_string(),
        is_dir: None,
    })
}

/// Parse `plocate -r ^<root>/` output (see [`locate_entry`]).
pub fn parse_locate(stdout: &str, root: &str) -> Vec<Entry> {
    stdout
        .lines()
        .filter_map(|l| locate_entry(l, root))
        .take(MAX_INDEX)
        .collect()
}

/// Read entries line by line (one reused buffer), at most [`MAX_INDEX`].
/// Returns the entries and whether the cap stopped the read.
fn collect_entries(
    mut reader: impl BufRead,
    mut entry: impl FnMut(&str) -> Option<Entry>,
) -> (Vec<Entry>, bool) {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    while out.len() < MAX_INDEX {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return (out, false),
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buf);
        if let Some(e) = entry(line.trim_end_matches('\n')) {
            out.push(e);
        }
    }
    (out, true)
}

/// fd argv for indexing `root`.
pub fn fd_argv(root: &str) -> Vec<String> {
    let mut argv: Vec<String> = [
        "fd",
        "--follow",
        "--type",
        "f",
        "--type",
        "d",
        "--color",
        "never",
        "--absolute-path",
        "--max-results",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    argv.push(MAX_INDEX.to_string());
    for ex in EXCLUDES {
        argv.push("--exclude".into());
        argv.push((*ex).into());
    }
    argv.push(".".into());
    argv.push(root.into());
    argv
}

/// Run `argv` and turn its stdout into entries as it streams in. The
/// output (~20 MB for a 200k-path home) is never held whole; `None` when
/// the command fails.
fn run_entries(argv: &[String], entry: impl FnMut(&str) -> Option<Entry>) -> Option<Vec<Entry>> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (mut entries, capped) = collect_entries(BufReader::new(stdout), entry);
    if capped {
        // The rest is not wanted; don't wait for the walk to finish.
        let _ = child.kill();
    }
    let status = child.wait().ok()?;
    (capped || status.success()).then(|| {
        entries.shrink_to_fit();
        entries
    })
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
    if depth > WALK_DEPTH || out.len() >= MAX_INDEX {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || EXCLUDES.contains(&name.as_ref()) {
            continue;
        }
        let path = e.path();
        // Follows symlinks (the owner's ~/Documents etc. are links); the
        // depth cap bounds any loop.
        let is_dir = path.is_dir();
        out.push(Entry {
            path: path.to_string_lossy().into_owned(),
            is_dir: Some(is_dir),
        });
        if is_dir {
            walk(&path, depth + 1, out);
        }
    }
}

/// Build the index for `root` with `backend` (blocking; worker thread).
fn build_index(root: PathBuf, backend: Backend) -> Arc<Index> {
    let root_s = root.to_string_lossy().into_owned();
    let entries = match backend {
        Backend::Fd => run_entries(&fd_argv(&root_s), fd_entry),
        Backend::Plocate => {
            let prefix = format!("{}/", root_s.trim_end_matches('/'));
            let argv = vec![
                "plocate".to_string(),
                "-r".into(),
                format!("^{}", regex::escape(&prefix)),
            ];
            run_entries(&argv, |l| locate_entry(l, &prefix))
        }
        Backend::Walk => None,
    };
    let entries = entries.unwrap_or_else(|| {
        let mut out = Vec::new();
        walk(&root, 0, &mut out);
        out
    });
    Arc::new(Index {
        entries,
        backend: Some(backend),
    })
}

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
    /// Last finished search.
    done: Option<(String, Hits)>,
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
        if let Some((q, hits)) = &st.done
            && q == query
        {
            return (Some(hits.clone()), None);
        }
        let previous = st.done.as_ref().map(|(_, h)| h.clone());
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
                        s.done = Some((q, hits));
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
    if let Err(err) = Command::new("xdg-open").arg(path).spawn_reaped() {
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

pub struct FileSearchProvider {
    root: PathBuf,
    backend: Backend,
    cache: BgCache<Arc<Index>>,
    searcher: Searcher,
}

impl FileSearchProvider {
    pub fn new() -> Self {
        let root = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let backend = choose_backend(command_exists("fd"), command_exists("plocate"));
        Self::with_root(root, backend)
    }

    pub fn with_root(root: PathBuf, backend: Backend) -> Self {
        Self {
            root,
            backend,
            cache: BgCache::with_idle_evict(INDEX_TTL, INDEX_IDLE),
            searcher: Searcher::default(),
        }
    }

    #[cfg(test)]
    fn seeded(root: &str, entries: Vec<Entry>) -> Self {
        let p = Self::with_root(PathBuf::from(root), Backend::Fd);
        p.cache.seed(Arc::new(Index {
            entries,
            backend: Some(Backend::Fd),
        }));
        p
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

    fn row(&self, e: &Entry, score: f64) -> LauncherItem {
        let is_dir = e.is_dir.unwrap_or_else(|| Path::new(&e.path).is_dir());
        let parent = Path::new(&e.path)
            .parent()
            .map(|p| tildify(p, &self.root))
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
        vec![self.hint(
            "palette",
            PREFIX.into(),
            "Find files and folders in your home (Enter open · Ctrl+Enter reveal)".into(),
            "system-search-symbolic",
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(rest) = strip_prefix(query) else {
            return Vec::new();
        };
        let root = self.root.clone();
        let backend = self.backend;
        let index = self.cache.get(move || build_index(root, backend));
        let filter = rest.trim();
        let Some(index) = index else {
            return vec![self.hint(
                "indexing",
                "Indexing files…".into(),
                format!(
                    "{} via {}",
                    tildify(&self.root, &self.root),
                    backend.label()
                ),
                "content-loading-symbolic",
            )];
        };
        if filter.is_empty() {
            return vec![self.hint(
                "help",
                "Type to search files and folders".into(),
                format!(
                    "{} paths in ~ via {}",
                    index.entries.len(),
                    index.backend.unwrap_or(backend).label()
                ),
                "system-search-symbolic",
            )];
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
        let Some(hits) = hits else {
            // Show the previous query's rows while this one runs, so the
            // list doesn't flash empty on every keystroke.
            let mut rows: Vec<LauncherItem> = previous
                .unwrap_or_default()
                .iter()
                .map(|(e, s)| self.row(e, *s))
                .collect();
            if rows.is_empty() {
                rows.push(self.hint(
                    "searching",
                    "Searching…".into(),
                    filter.to_string(),
                    "content-loading-symbolic",
                ));
            }
            return rows;
        };
        let rows: Vec<LauncherItem> = hits.iter().map(|(e, s)| self.row(e, *s)).collect();
        if rows.is_empty() {
            vec![self.hint(
                "nomatch",
                "No matching files".into(),
                filter.to_string(),
                "edit-find-symbolic",
            )]
        } else {
            rows
        }
    }

    /// Ctrl+Enter → reveal the path in the file manager.
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let path = item.id.strip_prefix("fs:path:")?.to_string();
        Some(Rc::new(move || reveal_path(path.clone())))
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.searcher.notifier.lock() {
            *slot = Some(notifier.clone());
        }
        self.cache.set_notifier(notifier);
    }

    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            // The Search tab shares space with web engines; an empty file
            // hint there is noise.
            Vec::new()
        } else {
            self.search(&format!("{PREFIX} {filter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `fd --follow … . /home/kenan` lines (fd 10.5) with folder marks.
    const FD_OUT: &str = "/home/kenan/Documents/\n/home/kenan/Documents/Alpemix/\n/home/kenan/Documents/Alpemix/AlpemixListem.ini\n/home/kenan/alpakademi.txt\n/home/kenan/margo-git-r2525.34b3f55c-1-x86_64.pkg.tar.zst\n";

    fn e(path: &str) -> Entry {
        Entry {
            path: path.into(),
            is_dir: Some(false),
        }
    }

    #[test]
    fn streamed_command_output_matches_the_parsers() {
        let sh = |script: &str| vec!["sh".to_string(), "-c".into(), script.into()];
        let script = format!("printf '%s' '{FD_OUT}'");
        assert_eq!(run_entries(&sh(&script), fd_entry), Some(parse_fd(FD_OUT)));
        // A failing command is no index (the walk fallback takes over).
        assert_eq!(run_entries(&sh("echo /x; exit 3"), fd_entry), None);
        // An endless producer stops at the cap and is killed, not waited on.
        let v = run_entries(&sh("yes /home/x"), fd_entry).unwrap();
        assert_eq!(v.len(), MAX_INDEX);
        // A last line without a newline still counts.
        assert_eq!(
            run_entries(&sh("printf '/a/\\n/a/b'"), fd_entry).map(|v| v.len()),
            Some(2)
        );
    }

    #[test]
    fn fd_output_marks_folders() {
        let v = parse_fd(FD_OUT);
        assert_eq!(v.len(), 5);
        assert_eq!(v[0].path, "/home/kenan/Documents");
        assert_eq!(v[0].is_dir, Some(true));
        assert_eq!(v[2].is_dir, Some(false));
        assert_eq!(v[2].file_name(), "AlpemixListem.ini");
    }

    #[test]
    fn locate_output_drops_hidden_excluded_and_root() {
        let out = "/home/u/\n/home/u/a.txt\n/home/u/.cache/x\n/home/u/p/node_modules/y\n/home/u/p/src/main.rs\n/home/uu/other\n";
        let v = parse_locate(out, "/home/u/");
        let paths: Vec<&str> = v.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["/home/u/a.txt", "/home/u/p/src/main.rs"]);
    }

    #[test]
    fn backend_preference() {
        assert_eq!(choose_backend(true, true), Backend::Fd);
        assert_eq!(choose_backend(false, true), Backend::Plocate);
        assert_eq!(choose_backend(false, false), Backend::Walk);
    }

    #[test]
    fn fd_argv_follows_links_and_excludes() {
        let a = fd_argv("/home/u");
        assert!(a.contains(&"--follow".to_string()));
        assert!(
            a.windows(2)
                .any(|w| w[0] == "--exclude" && w[1] == "node_modules")
        );
        assert_eq!(a.last().map(String::as_str), Some("/home/u"));
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
        assert!(p.search("fs")[0].name.starts_with("Type to search"));
        assert!(p.alt_action(&p.search("fs")[0]).is_none());
    }
}
