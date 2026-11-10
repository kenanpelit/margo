//! What the `fs` index covers: the search-scope setting, the roots and
//! prunes it turns into, the extra-folder validation and the cache key
//! that keeps one index per combination. Pure; the provider feeds it the
//! mount probes from [`super::mounts`].

use super::index::skipped_name;
use super::mounts::is_under;
use std::fmt;
use std::path::{Component, Path};
use std::time::Duration;

/// Where the search looks (`launcher.file_search.scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchScope {
    /// Your home folder — the original behaviour.
    #[default]
    Home,
    /// Home plus the extra folders.
    HomeAndFolders,
    /// All of `/`, minus pseudo / network / snapshot / removable mounts.
    System,
}

impl SearchScope {
    pub const ALL: [SearchScope; 3] = [Self::Home, Self::HomeAndFolders, Self::System];

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home only",
            Self::HomeAndFolders => "Home + folders",
            Self::System => "Whole system",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::HomeAndFolders => "folders",
            Self::System => "system",
        }
    }
}

/// The user's file-search settings (mirrors `launcher.file_search`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FsSettings {
    pub scope: SearchScope,
    /// Extra folders as configured (absolute or `~/…`).
    pub folders: Vec<String>,
    pub include_usb: bool,
}

/// How an index is kept fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// Rebuilt in the background once older than this (served stale
    /// meanwhile), and built on first use.
    Auto(Duration),
    /// Built only by an explicit "Rebuild search index"; a stale one is
    /// flagged "out of date". USB disks can take minutes to walk.
    Manual,
}

/// Home-only re-index period (the original behaviour).
pub const HOME_TTL: Duration = Duration::from_secs(300);
/// Whole-system re-index period: a walk of `/` costs far more than home.
pub const SYSTEM_TTL: Duration = Duration::from_secs(1800);

impl FsSettings {
    pub fn refresh(&self) -> Refresh {
        if self.include_usb {
            Refresh::Manual
        } else if self.scope == SearchScope::System {
            Refresh::Auto(SYSTEM_TTL)
        } else {
            Refresh::Auto(HOME_TTL)
        }
    }

    /// Short description of the combination, e.g. "Home + folders + USB disks".
    pub fn label(&self) -> String {
        if self.include_usb {
            format!("{} + USB disks", self.scope.label())
        } else {
            self.scope.label().to_string()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    Home,
    Folder,
    Usb,
    System,
}

/// One walk of the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub path: String,
    pub kind: RootKind,
    /// Absolute paths below `path` the walk must not enter.
    pub prune: Vec<String>,
}

/// Why a folder can't be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderError {
    Empty,
    NotAbsolute(String),
    Missing(String),
    NotADirectory(String),
    Duplicate(String),
    /// Already searched as part of another root.
    Covered {
        path: String,
        by: String,
    },
}

impl fmt::Display for FolderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Type a folder path"),
            Self::NotAbsolute(p) => write!(f, "“{p}” is not an absolute path (start with / or ~/)"),
            Self::Missing(p) => write!(f, "{p} does not exist"),
            Self::NotADirectory(p) => write!(f, "{p} is not a folder"),
            Self::Duplicate(p) => write!(f, "{p} is already in the list"),
            Self::Covered { path, by } => write!(f, "{path} is already searched as part of {by}"),
        }
    }
}

/// `raw` (`~`, `~/…` or absolute) as a lexically normalised absolute
/// path: no `.` / empty components, no trailing `/`. `..` is refused
/// rather than resolved.
pub fn expand_path(raw: &str, home: &str) -> Result<String, FolderError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(FolderError::Empty);
    }
    let joined = if raw == "~" {
        home.to_string()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        format!("{}/{rest}", home.trim_end_matches('/'))
    } else {
        raw.to_string()
    };
    if !joined.starts_with('/') || joined.chars().any(char::is_control) {
        return Err(FolderError::NotAbsolute(raw.to_string()));
    }
    let mut parts = Vec::new();
    for c in Path::new(&joined).components() {
        match c {
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(FolderError::NotAbsolute(raw.to_string()));
            }
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

/// `path` as shown and stored: `~/…` under home.
pub fn tilde(path: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    if path == home {
        "~".into()
    } else if let Some(rest) = path.strip_prefix(home).and_then(|r| r.strip_prefix('/')) {
        format!("~/{rest}")
    } else {
        path.to_string()
    }
}

/// Does a walk of `root` (pruning `prune`) already list `path`? Only when
/// `path` is below it, outside its prunes, and reached without passing a
/// hidden or excluded folder (a walk skips those — so `~/.config/x` can
/// still be added next to home).
pub fn covers(root: &str, prune: &[String], path: &str) -> bool {
    if !is_under(path, root) || prune.iter().any(|p| is_under(path, p)) {
        return false;
    }
    let rel = if root == "/" {
        &path[1..]
    } else {
        path[root.len()..].trim_start_matches('/')
    };
    rel.split('/')
        .filter(|c| !c.is_empty())
        .all(|c| !skipped_name(c))
}

/// The configured extra folders, expanded, invalid entries dropped,
/// duplicates removed (configured order kept).
pub fn expanded_folders(folders: &[String], home: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in folders {
        if let Ok(p) = expand_path(f, home)
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    out
}

/// Validate a folder before adding it to `existing` (the configured
/// list). `is_dir` answers `Some(true)` for a folder, `Some(false)` for
/// another file type and `None` when nothing is there. Returns the form
/// to store (`~/…` under home).
pub fn validate_new_folder(
    raw: &str,
    home: &str,
    existing: &[String],
    is_dir: impl Fn(&str) -> Option<bool>,
) -> Result<String, FolderError> {
    let path = expand_path(raw, home)?;
    match is_dir(&path) {
        None => return Err(FolderError::Missing(path)),
        Some(false) => return Err(FolderError::NotADirectory(path)),
        Some(true) => {}
    }
    let current = expanded_folders(existing, home);
    if current.contains(&path) {
        return Err(FolderError::Duplicate(tilde(&path, home)));
    }
    let home_n = expand_path(home, "/").unwrap_or_else(|_| home.to_string());
    if covers(&home_n, &[], &path) {
        return Err(FolderError::Covered {
            path: tilde(&path, home),
            by: "your home folder".into(),
        });
    }
    if let Some(by) = current.iter().find(|c| covers(c, &[], &path)) {
        return Err(FolderError::Covered {
            path: tilde(&path, home),
            by: tilde(by, home),
        });
    }
    Ok(tilde(&path, home))
}

/// The index slot of a combination: mode, (for Home + folders) the
/// folder list, and the USB toggle. Two settings with the same key share
/// one index; switching back to a key reuses its index from disk. The
/// mounted USB volumes are not part of it: a different set marks the
/// index out of date instead (see [`super::store`]).
pub fn plan_key(settings: &FsSettings, home: &str) -> String {
    let folders = if settings.scope == SearchScope::HomeAndFolders {
        let mut f = expanded_folders(&settings.folders, home);
        f.sort();
        f.join("\u{1f}")
    } else {
        String::new()
    };
    let home = if settings.scope == SearchScope::System {
        ""
    } else {
        home
    };
    format!(
        "v1|{}|{home}|{folders}|usb={}",
        settings.scope.tag(),
        u8::from(settings.include_usb)
    )
}

/// The roots a build walks for `settings`: home (or `/` with
/// `system_prune`), the existing extra folders, then the USB mounts.
/// A root another root already lists is dropped; ancestors first. Also
/// returns the configured folders that are missing (not folders).
pub fn compute_roots(
    settings: &FsSettings,
    home: &str,
    usb: &[String],
    system_prune: &[String],
    is_dir: impl Fn(&str) -> bool,
) -> (Vec<Root>, Vec<String>) {
    let mut candidates: Vec<Root> = Vec::new();
    let mut missing = Vec::new();
    let home_n = expand_path(home, "/").unwrap_or_else(|_| "/".into());
    match settings.scope {
        SearchScope::System => candidates.push(Root {
            path: "/".into(),
            kind: RootKind::System,
            prune: system_prune.to_vec(),
        }),
        SearchScope::Home | SearchScope::HomeAndFolders => {
            candidates.push(Root {
                path: home_n,
                kind: RootKind::Home,
                prune: Vec::new(),
            });
        }
    }
    if settings.scope == SearchScope::HomeAndFolders {
        for f in expanded_folders(&settings.folders, home) {
            if is_dir(&f) {
                candidates.push(Root {
                    path: f,
                    kind: RootKind::Folder,
                    prune: Vec::new(),
                });
            } else {
                missing.push(f);
            }
        }
    }
    if settings.include_usb {
        for m in usb {
            candidates.push(Root {
                path: m.trim_end_matches('/').to_string(),
                kind: RootKind::Usb,
                prune: Vec::new(),
            });
        }
    }
    // Ancestors first, so a nested root meets the root that covers it.
    candidates.sort_by(|a, b| a.path.len().cmp(&b.path.len()).then(a.path.cmp(&b.path)));
    let mut kept: Vec<Root> = Vec::new();
    for r in candidates {
        if !kept.iter().any(|k| covers(&k.path, &k.prune, &r.path)) {
            kept.push(r);
        }
    }
    // Home first (it keeps its share of the index cap), then by path.
    kept.sort_by_key(|r| {
        (
            r.kind != RootKind::Home && r.kind != RootKind::System,
            r.path.clone(),
        )
    });
    (kept, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/u";

    fn s(scope: SearchScope, folders: &[&str], usb: bool) -> FsSettings {
        FsSettings {
            scope,
            folders: folders.iter().map(|f| f.to_string()).collect(),
            include_usb: usb,
        }
    }

    fn paths(r: &[Root]) -> Vec<&str> {
        r.iter().map(|r| r.path.as_str()).collect()
    }

    #[test]
    fn expand_tilde_and_normalise() {
        assert_eq!(expand_path("~", HOME).unwrap(), "/home/u");
        assert_eq!(expand_path(" ~/work/ ", HOME).unwrap(), "/home/u/work");
        assert_eq!(
            expand_path("/media//data/./x/", HOME).unwrap(),
            "/media/data/x"
        );
        assert_eq!(expand_path("/", HOME).unwrap(), "/");
        assert_eq!(expand_path("", HOME), Err(FolderError::Empty));
        assert!(matches!(
            expand_path("work", HOME),
            Err(FolderError::NotAbsolute(_))
        ));
        assert!(matches!(
            expand_path("~user/x", HOME),
            Err(FolderError::NotAbsolute(_))
        ));
        assert!(matches!(
            expand_path("/a/../etc", HOME),
            Err(FolderError::NotAbsolute(_))
        ));
        assert_eq!(tilde("/home/u/work", HOME), "~/work");
        assert_eq!(tilde("/home/uu", HOME), "/home/uu");
    }

    #[test]
    fn new_folder_validation() {
        let dirs = |p: &str| match p {
            "/media/data"
            | "/media/data/photos"
            | "/home/u/work"
            | "/home/u/.config/app"
            | "/srv" => Some(true),
            "/media/file.txt" => Some(false),
            _ => None,
        };
        let existing = vec!["/media/data".to_string()];
        assert_eq!(
            validate_new_folder("/srv/", HOME, &existing, dirs).unwrap(),
            "/srv"
        );
        assert_eq!(
            validate_new_folder("/nope", HOME, &existing, dirs),
            Err(FolderError::Missing("/nope".into()))
        );
        assert_eq!(
            validate_new_folder("/media/file.txt", HOME, &existing, dirs),
            Err(FolderError::NotADirectory("/media/file.txt".into()))
        );
        assert_eq!(
            validate_new_folder("/media/data/", HOME, &existing, dirs),
            Err(FolderError::Duplicate("/media/data".into()))
        );
        assert_eq!(
            validate_new_folder("/media/data/photos", HOME, &existing, dirs),
            Err(FolderError::Covered {
                path: "/media/data/photos".into(),
                by: "/media/data".into()
            })
        );
        assert!(matches!(
            validate_new_folder("~/work", HOME, &existing, dirs),
            Err(FolderError::Covered { .. })
        ));
        // A hidden folder is not reached by the home walk: it can be added.
        assert_eq!(
            validate_new_folder("~/.config/app", HOME, &existing, dirs).unwrap(),
            "~/.config/app"
        );
        assert_eq!(
            validate_new_folder("  ", HOME, &existing, dirs),
            Err(FolderError::Empty)
        );
    }

    #[test]
    fn roots_per_mode() {
        let all = |_: &str| true;
        let usb = vec!["/run/media/u/STICK".to_string()];
        let prune = vec!["/proc".to_string(), "/run".to_string()];

        let (r, _) = compute_roots(
            &s(SearchScope::Home, &["/srv"], false),
            HOME,
            &usb,
            &prune,
            all,
        );
        assert_eq!(paths(&r), vec!["/home/u"]);

        let (r, missing) = compute_roots(
            &s(
                SearchScope::HomeAndFolders,
                &["/srv", "~/.config/app", "/gone", "/srv/x", "~/docs"],
                false,
            ),
            HOME,
            &usb,
            &prune,
            |p| p != "/gone",
        );
        assert_eq!(paths(&r), vec!["/home/u", "/home/u/.config/app", "/srv"]);
        assert_eq!(missing, vec!["/gone".to_string()]);

        let (r, _) = compute_roots(
            &s(SearchScope::System, &["/srv"], true),
            HOME,
            &usb,
            &prune,
            all,
        );
        // The stick sits under the pruned /run: it stays its own root.
        assert_eq!(paths(&r), vec!["/", "/run/media/u/STICK"]);
        assert_eq!(r[0].prune, prune);
        assert_eq!(r[1].kind, RootKind::Usb);

        let (r, _) = compute_roots(&s(SearchScope::Home, &[], true), HOME, &usb, &prune, all);
        assert_eq!(paths(&r), vec!["/home/u", "/run/media/u/STICK"]);
    }

    #[test]
    fn a_folder_containing_home_replaces_it() {
        let (r, _) = compute_roots(
            &s(SearchScope::HomeAndFolders, &["/home"], false),
            HOME,
            &[],
            &[],
            |_| true,
        );
        assert_eq!(paths(&r), vec!["/home"]);
    }

    #[test]
    fn cache_keys_follow_the_combination() {
        let home = plan_key(&s(SearchScope::Home, &["/srv"], false), HOME);
        // Folders only matter in Home + folders mode.
        assert_eq!(home, plan_key(&s(SearchScope::Home, &[], false), HOME));
        let f1 = plan_key(
            &s(SearchScope::HomeAndFolders, &["/srv", "/media/x"], false),
            HOME,
        );
        let f2 = plan_key(
            &s(SearchScope::HomeAndFolders, &["/media/x/", "/srv"], false),
            HOME,
        );
        assert_eq!(f1, f2, "order and spelling don't split the index");
        assert_ne!(
            f1,
            plan_key(&s(SearchScope::HomeAndFolders, &["/srv"], false), HOME)
        );
        assert_ne!(home, f1);
        let usb = plan_key(&s(SearchScope::Home, &[], true), HOME);
        assert_ne!(home, usb);
        let sys = plan_key(&s(SearchScope::System, &[], false), HOME);
        assert_ne!(sys, home);
        assert_eq!(
            sys,
            plan_key(&s(SearchScope::System, &["/srv"], false), "/home/other")
        );
    }

    #[test]
    fn refresh_policy() {
        assert_eq!(
            s(SearchScope::Home, &[], false).refresh(),
            Refresh::Auto(HOME_TTL)
        );
        assert_eq!(
            s(SearchScope::HomeAndFolders, &[], false).refresh(),
            Refresh::Auto(HOME_TTL)
        );
        assert_eq!(
            s(SearchScope::System, &[], false).refresh(),
            Refresh::Auto(SYSTEM_TTL)
        );
        assert_eq!(s(SearchScope::Home, &[], true).refresh(), Refresh::Manual);
        assert_eq!(
            s(SearchScope::System, &[], true).label(),
            "Whole system + USB disks"
        );
    }
}
