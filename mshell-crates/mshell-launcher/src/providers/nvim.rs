//! `nv <query>` / `nvim <query>` — reopen Neovim sessions and projects.
//!
//! Native port of the `nvim-projects` community plugin. Rows come from:
//!
//! * **session files** in `$XDG_STATE_HOME/nvim/sessions` and
//!   `$XDG_DATA_HOME/nvim/sessions`, their file names decoded back into the
//!   project directory ([`decode_session_name`]: persistence.nvim /
//!   persisted.nvim `%`-paths with a `%%branch` suffix, auto-session
//!   URL-encoding with a `|branch`, neovim-session-manager `__` paths);
//! * `~/.config/nvim` itself;
//! * optionally every sub-directory of `launcher.nvim_projects_dir`
//!   (Settings → Launcher → Neovim projects; blank = off).
//!
//! Sessions whose directory no longer exists are hidden (an old session
//! would only fail its `cd`). Rows sort by last use (the session file's
//! mtime), then fuzzy-match the name and path once you type.
//!
//! | Key | Action |
//! |---|---|
//! | Enter | open the user's terminal in the project running `nvim -S <session>` (a plain folder runs `nvim .`) |
//! | Ctrl+Enter | open the folder fresh (`nvim .`), ignoring its session |
//!
//! The session path goes through [`vim_fnameescape`]: `-S` hands its
//! argument to `:source` unescaped, and persistence names are full of `%`
//! (Vim's "current file") — unescaped, nvim fails with E499.
//!
//! The scan (a few hundred `stat`s) runs on a worker thread through
//! [`BgCache`], never on the GTK thread `search` runs on.

use crate::providers::bg_cache::BgCache;
use crate::providers::tools::tildify;
use crate::scoring::{fuzzy_score, make_matcher};
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use mshell_process::SpawnReaped;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

const PROVIDER: &str = "Neovim";
const ICON: &str = "nvim";
/// Rows shown before typing narrows the list.
const BROWSE_LIMIT: usize = 50;
const SCAN_TTL: Duration = Duration::from_secs(5);
/// Sub-directories read from the projects dir (a home dir set by mistake
/// must not produce thousands of rows).
const MAX_PROJECT_DIRS: usize = 500;

/// A session file name decoded back into what it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSession {
    pub dir: PathBuf,
    /// Git branch for per-branch sessions.
    pub branch: Option<String>,
}

/// Percent-decode `%XX` escapes (auto-session's file names).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode a session file name into its project directory (+ branch).
/// `None` for names that are no known plugin's absolute-path encoding.
pub fn decode_session_name(file_name: &str) -> Option<DecodedSession> {
    let stem = file_name.strip_suffix(".vim").unwrap_or(file_name);
    let (path, branch) = if stem.len() >= 3 && stem[..3].eq_ignore_ascii_case("%2F") {
        // auto-session: URL-encoded path, `|` (%7C) before the branch.
        let decoded = percent_decode(stem);
        match decoded.split_once('|') {
            Some((p, b)) => (p.to_string(), Some(b.to_string())),
            None => (decoded, None),
        }
    } else if stem.starts_with("__") {
        // neovim-session-manager: `/` written as `__`.
        (stem.replace("__", "/"), None)
    } else if stem.starts_with('%') {
        // persistence.nvim / persisted.nvim: `/` written as `%`, then
        // `%%<branch>` (the branch's own `/` also as `%`).
        match stem.split_once("%%") {
            Some((p, b)) => (p.replace('%', "/"), Some(b.replace('%', "/"))),
            None => (stem.replace('%', "/"), None),
        }
    } else {
        return None;
    };
    let dir = PathBuf::from(path);
    if !dir.is_absolute() {
        return None;
    }
    Some(DecodedSession {
        dir,
        branch: branch.filter(|b| !b.is_empty()),
    })
}

/// Vim's `fnameescape()` for Unix: backslash every character `:source`
/// would otherwise expand or split on.
pub fn vim_fnameescape(path: &str) -> String {
    const SPECIAL: &str = " \t\n*?[{`$\\%#'\"|!<";
    let mut out = String::with_capacity(path.len() + 8);
    if path.starts_with('+') || path.starts_with('>') || path == "-" {
        out.push('\\');
    }
    for c in path.chars() {
        if SPECIAL.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub dir: PathBuf,
    /// Session file to restore; `None` for a plain folder.
    pub session: Option<PathBuf>,
    pub used: SystemTime,
}

impl Project {
    fn key(&self) -> String {
        match &self.session {
            Some(s) => format!("nvim:s:{}", s.display()),
            None => format!("nvim:d:{}", self.dir.display()),
        }
    }
}

fn basename(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Where to look. Split out so tests can point it at temp dirs.
#[derive(Debug, Clone)]
pub struct ScanRoots {
    pub session_dirs: Vec<PathBuf>,
    pub extra_dirs: Vec<PathBuf>,
    pub projects_dir: Option<PathBuf>,
}

impl ScanRoots {
    fn for_user(projects_dir: Option<PathBuf>) -> Self {
        Self {
            session_dirs: vec![
                margo_fs::xdg::state_home().join("nvim/sessions"),
                margo_fs::xdg::data_home().join("nvim/sessions"),
            ],
            extra_dirs: vec![margo_fs::xdg::config_home().join("nvim")],
            projects_dir,
        }
    }
}

/// Every openable project, most recently used first. Blocking (worker
/// thread only).
pub fn scan(roots: &ScanRoots) -> Vec<Project> {
    let mut out: Vec<Project> = Vec::new();
    for dir in &roots.session_dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let file_name = entry.file_name();
            let Some(decoded) = decode_session_name(&file_name.to_string_lossy()) else {
                continue;
            };
            if !decoded.dir.is_dir() {
                continue; // project gone — its session would only fail to cd
            }
            let session = entry.path();
            let name = match &decoded.branch {
                Some(b) => format!("{} ({b})", basename(&decoded.dir)),
                None => basename(&decoded.dir),
            };
            out.push(Project {
                name,
                used: mtime(&session),
                dir: decoded.dir,
                session: Some(session),
            });
        }
    }
    let mut folders: Vec<PathBuf> = roots
        .extra_dirs
        .iter()
        .filter(|d| d.is_dir())
        .cloned()
        .collect();
    if let Some(root) = &roots.projects_dir
        && let Ok(rd) = std::fs::read_dir(root)
    {
        folders.extend(
            rd.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .take(MAX_PROJECT_DIRS),
        );
    }
    for dir in folders {
        if out.iter().any(|p| p.dir == dir) {
            continue; // a session already opens this project
        }
        out.push(Project {
            name: basename(&dir),
            used: mtime(&dir),
            dir,
            session: None,
        });
    }
    out.sort_by(|a, b| b.used.cmp(&a.used).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Expand a leading `~/` (Settings paths are written that way).
fn expand_home(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    match raw.strip_prefix("~/") {
        Some(rest) => margo_fs::xdg::home().map(|h| h.join(rest)),
        None if raw == "~" => margo_fs::xdg::home(),
        None => Some(PathBuf::from(raw)),
    }
}

/// The argv run inside the terminal for `project`.
pub fn nvim_argv(project: &Project, fresh: bool) -> Vec<String> {
    match (&project.session, fresh) {
        (Some(session), false) => vec![
            "nvim".into(),
            "-S".into(),
            vim_fnameescape(&session.to_string_lossy()),
        ],
        _ => vec!["nvim".into(), ".".into()],
    }
}

fn open_in_terminal(project: &Project, fresh: bool) {
    let argv = nvim_argv(project, fresh);
    match mshell_process::terminal::command(&argv, "nvim") {
        Ok((terminal, mut cmd)) => {
            cmd.current_dir(&project.dir);
            if let Err(err) = cmd.spawn_reaped() {
                tracing::warn!(?err, %terminal, dir = %project.dir.display(), "nvim: terminal spawn failed");
                toast(PROVIDER, format!("Could not start {terminal}"));
            }
        }
        Err(err) => toast(PROVIDER, err.to_string()),
    }
}

pub struct NvimProvider {
    resolver: Option<Box<dyn Fn() -> String>>,
    projects_dir: RefCell<Option<PathBuf>>,
    cache: BgCache<Vec<Project>>,
}

impl NvimProvider {
    /// `projects_dir` is re-read on every launcher open (Settings changes
    /// apply without a restart); it returns the raw setting.
    pub fn new(projects_dir: Box<dyn Fn() -> String>) -> Self {
        let dir = expand_home(&projects_dir());
        Self {
            resolver: Some(projects_dir),
            projects_dir: RefCell::new(dir),
            cache: BgCache::new(SCAN_TTL),
        }
    }

    fn info_row(id: &str, name: &str, description: String) -> LauncherItem {
        LauncherItem {
            id: format!("nvim:{id}"),
            name: name.into(),
            description,
            icon: "dialog-information-symbolic".into(),
            icon_is_path: false,
            score: 100.0,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    fn row(project: &Project, score: f64, home: &Path) -> LauncherItem {
        let kind = if project.session.is_some() {
            "session"
        } else {
            "folder"
        };
        let p = project.clone();
        LauncherItem {
            id: project.key(),
            name: project.name.clone(),
            description: format!("{} · {kind}", tildify(&project.dir, home)),
            icon: ICON.into(),
            icon_is_path: false,
            score,
            provider_name: PROVIDER.into(),
            usage_key: Some(project.key()),
            on_activate: Rc::new(move || open_in_terminal(&p, false)),
        }
    }

    fn projects(&self) -> Option<Vec<Project>> {
        let roots = ScanRoots::for_user(self.projects_dir.borrow().clone());
        self.cache.get(move || scan(&roots))
    }

    fn find(&self, id: &str) -> Option<Project> {
        self.projects()?.into_iter().find(|p| p.key() == id)
    }
}

fn prefix_rest(query: &str) -> Option<&str> {
    let q = query.trim_start();
    ["nvim", "nv"].iter().find_map(|p| {
        let rest = q.strip_prefix(p)?;
        (rest.is_empty() || rest.starts_with(' ')).then_some(rest)
    })
}

/// Rank `projects` for `filter`: recency order when empty, else the best
/// fuzzy score over the name, then the path.
pub fn rank<'a>(projects: &'a [Project], filter: &str) -> Vec<(&'a Project, f64)> {
    if filter.is_empty() {
        return projects
            .iter()
            .take(BROWSE_LIMIT)
            .enumerate()
            .map(|(i, p)| (p, 200.0 - i as f64 * 0.1))
            .collect();
    }
    let mut matcher = make_matcher();
    let mut hits: Vec<(&Project, f64)> = projects
        .iter()
        .filter_map(|p| {
            let by_name = fuzzy_score(&mut matcher, filter, &p.name).map(|s| s + 20.0);
            let by_path = fuzzy_score(&mut matcher, filter, &p.dir.to_string_lossy());
            by_name.or(by_path).map(|s| (p, s))
        })
        .collect();
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    hits
}

impl Provider for NvimProvider {
    fn name(&self) -> &str {
        PROVIDER
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        prefix_rest(query).is_some()
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        self.cache.set_notifier(notifier);
    }

    fn on_opened(&mut self) {
        if let Some(resolver) = &self.resolver {
            *self.projects_dir.borrow_mut() = expand_home(&resolver());
        }
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "nvim:palette".into(),
            name: "nv <project>".into(),
            description: "Reopen a Neovim session or project in the terminal".into(),
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
        if !margo_fs::path::on_path("nvim") {
            return vec![Self::info_row(
                "missing",
                "Neovim is not installed",
                "Install the neovim package to open sessions".into(),
            )];
        }
        let Some(projects) = self.projects() else {
            return vec![Self::info_row(
                "loading",
                "Reading Neovim sessions…",
                String::new(),
            )];
        };
        let home = margo_fs::xdg::home().unwrap_or_default();
        let rows: Vec<LauncherItem> = rank(&projects, rest.trim())
            .into_iter()
            .map(|(p, score)| Self::row(p, score, &home))
            .collect();
        if rows.is_empty() {
            let description = if rest.trim().is_empty() {
                "No sessions in ~/.local/state/nvim/sessions".to_string()
            } else {
                format!("Nothing matches “{}”", rest.trim())
            };
            return vec![Self::info_row("none", "No Neovim projects", description)];
        }
        rows
    }

    /// Ctrl+Enter opens the folder fresh, without restoring the session.
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let project = self.find(&item.id)?;
        Some(Rc::new(move || open_in_terminal(&project, true)))
    }

    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        let Some(project) = self.find(&item.id) else {
            return Some(crate::LauncherPreview::text(
                item.name.clone(),
                item.description.clone(),
            ));
        };
        let body = match &project.session {
            Some(s) => format!(
                "{}\n\nsession: {}\nEnter: restore · Ctrl+Enter: open fresh",
                project.dir.display(),
                s.display()
            ),
            None => format!("{}\n\nEnter: open in nvim", project.dir.display()),
        };
        Some(crate::LauncherPreview::mono(project.name, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(dir: &str, branch: Option<&str>) -> Option<DecodedSession> {
        Some(DecodedSession {
            dir: PathBuf::from(dir),
            branch: branch.map(String::from),
        })
    }

    /// Shapes of real persistence.nvim names from
    /// ~/.local/state/nvim/sessions on this machine (user renamed).
    #[test]
    fn decodes_persistence_names() {
        assert_eq!(
            decode_session_name("%home%user%aur%margo-git.vim"),
            decoded("/home/user/aur/margo-git", None)
        );
        assert_eq!(
            decode_session_name("%home%user%bmc_vnc%vnc.vim"),
            decoded("/home/user/bmc_vnc/vnc", None)
        );
        assert_eq!(
            decode_session_name("%home%user%.cachy%%margo.vim"),
            decoded("/home/user/.cachy", Some("margo"))
        );
        assert_eq!(
            decode_session_name("%repo%archive%.kod%margo%%feature%x.vim"),
            decoded("/repo/archive/.kod/margo", Some("feature/x"))
        );
    }

    #[test]
    fn decodes_auto_session_and_session_manager_names() {
        assert_eq!(
            decode_session_name("%2Fhome%2Fuser%2Fmy%20proj%7Cmain.vim"),
            decoded("/home/user/my proj", Some("main"))
        );
        assert_eq!(
            decode_session_name("%2Fsrv%2Fapp.vim"),
            decoded("/srv/app", None)
        );
        assert_eq!(
            decode_session_name("__home__user__code__app"),
            decoded("/home/user/code/app", None)
        );
        assert_eq!(decode_session_name("Session.vim"), None);
        assert_eq!(decode_session_name("notes.txt"), None);
    }

    /// Verified against nvim 0.11: `nvim -S '<raw path with %>'` fails with
    /// E499, the escaped form sources the file.
    #[test]
    fn fnameescape_escapes_what_source_expands() {
        assert_eq!(
            vim_fnameescape("/s/%home%a b#c%%x.vim"),
            r"/s/\%home\%a\ b\#c\%\%x.vim"
        );
        assert_eq!(vim_fnameescape("/plain/path.vim"), "/plain/path.vim");
        assert_eq!(vim_fnameescape("+x"), r"\+x");
    }

    fn tmp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("mshell-nvim-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn encode(dir: &Path) -> String {
        format!("{}.vim", dir.to_string_lossy().replace('/', "%"))
    }

    #[test]
    fn scan_keeps_live_sessions_newest_first_and_adds_uncovered_folders() {
        let root = tmp("scan");
        let sessions = root.join("sessions");
        let projects = root.join("projects");
        let (alpha, beta, gamma) = (
            projects.join("alpha"),
            projects.join("beta"),
            projects.join("gamma"),
        );
        for d in [&sessions, &alpha, &beta, &gamma, &projects.join(".hidden")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(sessions.join(encode(&alpha)), "cd x").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(sessions.join(encode(&beta)), "cd x").unwrap();
        // A session for a project that was deleted.
        std::fs::write(sessions.join(encode(&projects.join("gone"))), "cd x").unwrap();
        let roots = ScanRoots {
            session_dirs: vec![sessions.clone(), root.join("missing")],
            extra_dirs: vec![],
            projects_dir: Some(projects.clone()),
        };
        let got = scan(&roots);
        let names: Vec<&str> = got.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(&names[..2], ["beta", "alpha"], "{names:?}");
        assert!(names.contains(&"gamma"));
        assert!(!names.contains(&"gone"));
        assert!(!names.contains(&".hidden"));
        // alpha has a session, so no duplicate folder row.
        assert_eq!(names.iter().filter(|n| **n == "alpha").count(), 1);
        let gamma_row = got.iter().find(|p| p.name == "gamma").unwrap();
        assert!(gamma_row.session.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn argv_restores_session_or_opens_folder() {
        let p = Project {
            name: "x".into(),
            dir: PathBuf::from("/p/x"),
            session: Some(PathBuf::from("/s/%p%x.vim")),
            used: SystemTime::UNIX_EPOCH,
        };
        assert_eq!(nvim_argv(&p, false), ["nvim", "-S", r"/s/\%p\%x.vim"]);
        assert_eq!(nvim_argv(&p, true), ["nvim", "."]);
        let folder = Project { session: None, ..p };
        assert_eq!(nvim_argv(&folder, false), ["nvim", "."]);
    }

    #[test]
    fn rank_prefers_name_matches_and_keeps_recency_when_empty() {
        let mk = |name: &str, dir: &str| Project {
            name: name.into(),
            dir: PathBuf::from(dir),
            session: None,
            used: SystemTime::UNIX_EPOCH,
        };
        let ps = vec![mk("dots", "/home/u/margo/dots"), mk("margo", "/repo/margo")];
        let empty = rank(&ps, "");
        assert_eq!(empty[0].0.name, "dots");
        let hits = rank(&ps, "margo");
        assert_eq!(hits[0].0.name, "margo");
        assert_eq!(hits.len(), 2, "path matches count too");
        assert!(rank(&ps, "zzzz").is_empty());
    }

    #[test]
    fn prefixes_are_exact_words() {
        assert_eq!(prefix_rest("nv"), Some(""));
        assert_eq!(prefix_rest("nv margo"), Some(" margo"));
        assert_eq!(prefix_rest("nvim margo"), Some(" margo"));
        assert_eq!(prefix_rest("nvidia"), None);
        assert_eq!(prefix_rest("nvimx"), None);
    }

    #[test]
    fn home_expansion() {
        assert_eq!(expand_home("  "), None);
        assert_eq!(expand_home("/srv/code"), Some(PathBuf::from("/srv/code")));
        if let Some(h) = margo_fs::xdg::home() {
            assert_eq!(expand_home("~/code"), Some(h.join("code")));
        }
    }
}
