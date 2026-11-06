//! Icon themes: discovery, `index.theme` parsing and icon lookup per the
//! freedesktop Icon Theme spec, so the picker can show each theme's own
//! folder / file / app / status icons. Pure text parsing plus directory
//! reads; nothing here touches GTK.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The icons a theme card shows: (what it stands for, names to try in
/// order). Names are the spec's plus the common app ids themes ship.
pub(crate) const SAMPLES: &[(&str, &[&str])] = &[
    ("Folder", &["folder", "inode-directory"]),
    ("Home", &["user-home", "folder-home"]),
    ("Document", &["text-x-generic", "x-office-document"]),
    ("Image", &["image-x-generic"]),
    (
        "Files",
        &["org.gnome.Nautilus", "system-file-manager", "file-manager"],
    ),
    (
        "Browser",
        &["firefox", "web-browser", "internet-web-browser"],
    ),
    (
        "Terminal",
        &["utilities-terminal", "org.gnome.Terminal", "terminal"],
    ),
    (
        "Settings",
        &["preferences-system", "org.gnome.Settings", "systemsettings"],
    ),
];

/// How a sub-directory's icons match a requested size (spec `Type=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirType {
    Fixed,
    Scalable { min: u32, max: u32 },
    Threshold(u32),
}

/// One `[sub/dir]` section of an `index.theme`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubDir {
    pub path: String,
    pub size: u32,
    pub scale: u32,
    pub kind: DirType,
}

impl SubDir {
    /// The spec's DirectorySizeDistance for a `want`-px icon at scale 1.
    pub(crate) fn distance(&self, want: u32) -> u32 {
        let size = self.size * self.scale;
        // Outside the [lo, hi] range: how far; inside: 0.
        let outside = |lo: u32, hi: u32| lo.saturating_sub(want) + want.saturating_sub(hi);
        match self.kind {
            DirType::Fixed => size.abs_diff(want),
            DirType::Scalable { min, max } => outside(min * self.scale, max * self.scale),
            DirType::Threshold(t) => outside(
                self.size.saturating_sub(t) * self.scale,
                (self.size + t) * self.scale,
            ),
        }
    }
}

/// What an `index.theme` says about a theme.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Index {
    pub name: String,
    pub comment: String,
    pub inherits: Vec<String>,
    pub hidden: bool,
    pub subdirs: Vec<SubDir>,
}

/// `[section]` → `key = value` pairs, in file order.
fn sections(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = Some(name.trim().to_string());
            continue;
        }
        if let (Some(section), Some((k, v))) = (&current, line.split_once('=')) {
            out.entry(section.clone())
                .or_default()
                .entry(k.trim().to_string())
                .or_insert_with(|| v.trim().to_string());
        }
    }
    out
}

fn list(value: Option<&String>) -> Vec<String> {
    value
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Parse an `index.theme`. `None` when it has no `[Icon Theme]` section or
/// no icon directories (a cursor-only theme).
pub(crate) fn parse_index(text: &str) -> Option<Index> {
    let sections = sections(text);
    let head = sections.get("Icon Theme")?;
    let num = |m: &HashMap<String, String>, k: &str| m.get(k).and_then(|v| v.parse::<u32>().ok());
    let mut dirs = list(head.get("Directories"));
    dirs.extend(list(head.get("ScaledDirectories")));
    let subdirs: Vec<SubDir> = dirs
        .into_iter()
        .filter_map(|path| {
            let s = sections.get(&path)?;
            let size = num(s, "Size")?;
            let kind = match s.get("Type").map(String::as_str) {
                Some("Fixed") => DirType::Fixed,
                Some("Scalable") => DirType::Scalable {
                    min: num(s, "MinSize").unwrap_or(size),
                    max: num(s, "MaxSize").unwrap_or(size),
                },
                _ => DirType::Threshold(num(s, "Threshold").unwrap_or(2)),
            };
            Some(SubDir {
                path,
                size,
                scale: num(s, "Scale").unwrap_or(1).max(1),
                kind,
            })
        })
        .collect();
    if subdirs.is_empty() {
        return None;
    }
    Some(Index {
        name: head.get("Name").cloned().unwrap_or_default(),
        comment: head.get("Comment").cloned().unwrap_or_default(),
        inherits: list(head.get("Inherits")),
        hidden: head
            .get("Hidden")
            .is_some_and(|v| v.eq_ignore_ascii_case("true")),
        subdirs,
    })
}

/// One installed icon theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IconTheme {
    /// Directory name — what every config key stores.
    pub id: String,
    /// Every base directory that has a copy of it, in lookup order.
    pub dirs: Vec<PathBuf>,
    pub index: Index,
}

impl IconTheme {
    pub(crate) fn name(&self) -> &str {
        if self.index.name.is_empty() {
            &self.id
        } else {
            &self.index.name
        }
    }
}

/// The selectable icon themes under `roots`: directories whose
/// `index.theme` (from the first root that has the theme) lists icon
/// directories. `default`, `hicolor` (the spec's fallback, not a look) and
/// `Hidden=true` themes other than Adwaita are left out. Sorted by name.
pub(crate) fn discover(roots: &[PathBuf]) -> Vec<IconTheme> {
    let mut themes: Vec<IconTheme> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                continue;
            };
            if let Some(t) = themes.iter_mut().find(|t| t.id == id) {
                t.dirs.push(dir);
                continue;
            }
            if id == "default" || id == "hicolor" || !dir.is_dir() {
                continue;
            }
            let Some(index) = std::fs::read_to_string(dir.join("index.theme"))
                .ok()
                .and_then(|t| parse_index(&t))
            else {
                continue;
            };
            // `Hidden=true` asks to stay out of theme lists, but Adwaita
            // (GTK's default) sets it too and is a look people go back to.
            if index.hidden && id != "Adwaita" {
                continue;
            }
            themes.push(IconTheme {
                id,
                dirs: vec![dir],
                index,
            });
        }
    }
    themes.sort_by_key(|t| t.name().to_lowercase());
    themes
}

const EXTENSIONS: &[&str] = &["svg", "png"];

/// The best file for any of `names` (tried in order) in `theme` alone,
/// nearest to `want` px.
fn find_in(theme: &IconTheme, names: &[&str], want: u32) -> Option<PathBuf> {
    let mut subdirs: Vec<&SubDir> = theme.index.subdirs.iter().collect();
    subdirs.sort_by_key(|d| (d.distance(want), d.scale != 1, std::cmp::Reverse(d.size)));
    for name in names {
        for sd in &subdirs {
            for base in &theme.dirs {
                for ext in EXTENSIONS {
                    let p = base.join(&sd.path).join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

/// The file `theme_id` shows for `names` at `want` px: from the theme, else
/// from the themes it inherits (depth-first, as the spec looks them up,
/// `hicolor` excluded so a preview shows the theme's own look).
pub(crate) fn find_icon(
    themes: &[IconTheme],
    theme_id: &str,
    names: &[&str],
    want: u32,
) -> Option<PathBuf> {
    let mut seen: Vec<String> = Vec::new();
    let mut stack = vec![theme_id.to_string()];
    while let Some(id) = stack.pop() {
        if seen.contains(&id) || seen.len() > 8 {
            continue;
        }
        seen.push(id.clone());
        let Some(theme) = themes.iter().find(|t| t.id == id) else {
            continue;
        };
        if let Some(p) = find_in(theme, names, want) {
            return Some(p);
        }
        stack.extend(theme.index.inherits.iter().rev().cloned());
    }
    None
}

/// Install the icon theme(s) in `src` into `dest_root`: `src` itself when it
/// has an `index.theme`, else each sub-directory that has one. Each is
/// copied as `dest_root/<dir name>` with its symlinks kept (variants link
/// into their base theme). Themes already in `dest_root` are not
/// overwritten. Returns the installed ids.
pub(crate) fn install_from(src: &Path, dest_root: &Path) -> Result<Vec<String>, String> {
    let is_theme = |d: &Path| {
        std::fs::read_to_string(d.join("index.theme"))
            .ok()
            .and_then(|t| parse_index(&t))
            .is_some()
    };
    let candidates: Vec<PathBuf> = if is_theme(src) {
        vec![src.to_path_buf()]
    } else {
        let mut v: Vec<PathBuf> = std::fs::read_dir(src)
            .map_err(|e| format!("{}: {e}", src.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && is_theme(p))
            .collect();
        v.sort();
        v
    };
    if candidates.is_empty() {
        return Err(format!(
            "{} has no icon theme (no index.theme with icon directories). Extract an archive first.",
            src.display()
        ));
    }
    std::fs::create_dir_all(dest_root).map_err(|e| format!("{}: {e}", dest_root.display()))?;
    let mut installed = Vec::new();
    let mut skipped = Vec::new();
    for dir in candidates {
        let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let dest = dest_root.join(&id);
        if dest.exists() {
            skipped.push(id);
            continue;
        }
        copy_tree(&dir, &dest).map_err(|e| format!("copying {id}: {e}"))?;
        installed.push(id);
    }
    if installed.is_empty() {
        return Err(format!("Already installed: {}.", skipped.join(", ")));
    }
    Ok(installed)
}

/// Copy `src` to `dest` recursively, recreating symlinks as symlinks.
fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)?;
        } else if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines kept from this machine's `/usr/share/icons/kora/index.theme`
    /// (it really repeats the `[panel/16]` section; the first one counts).
    const KORA: &str = "[Icon Theme]\nName=Kora\nComment=Kora icon theme for GNU/Linux\n\nInherits=breeze,hicolor\n\nExample=folder\n\n# KDE Specific Stuff\nDisplayDepth=32\nDirectories=actions/symbolic,apps/scalable,panel/16,places/scalable\n\n[actions/symbolic]\nContext=Actions\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n\n[apps/scalable]\nContext=Applications\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n\n[panel/16]\nContext=Actions\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n\n[panel/16]\nContext=Status\nSize=16\nType=Fixed\n\n[places/scalable]\nContext=Places\nSize=22\nMinSize=8\nMaxSize=512\nType=Scalable\n";

    #[test]
    fn index_parses_dirs_types_and_inherits() {
        let idx = parse_index(KORA).expect("index");
        assert_eq!(idx.name, "Kora");
        assert_eq!(idx.inherits, ["breeze", "hicolor"]);
        assert_eq!(idx.subdirs.len(), 4);
        assert_eq!(idx.subdirs[1].kind, DirType::Scalable { min: 8, max: 512 });
        assert_eq!(
            idx.subdirs[2].kind,
            DirType::Scalable { min: 8, max: 512 },
            "first [panel/16] wins"
        );
        assert_eq!(idx.subdirs[3].size, 22);
        assert!(!idx.hidden);
    }

    #[test]
    fn cursor_only_and_headless_files_are_not_icon_themes() {
        assert_eq!(parse_index("[Icon Theme]\nName=Capitaine Cursors\n"), None);
        assert_eq!(parse_index("[Settings]\nx=1\n"), None);
        // A directory listed without its own section is ignored.
        assert_eq!(parse_index("[Icon Theme]\nDirectories=a\n"), None);
    }

    #[test]
    fn distances_follow_the_spec() {
        let fixed = SubDir {
            path: "48".into(),
            size: 48,
            scale: 1,
            kind: DirType::Fixed,
        };
        assert_eq!(fixed.distance(32), 16);
        let scal = SubDir {
            kind: DirType::Scalable { min: 16, max: 256 },
            ..fixed.clone()
        };
        assert_eq!(scal.distance(32), 0);
        assert_eq!(scal.distance(8), 8);
        let thr = SubDir {
            kind: DirType::Threshold(2),
            ..fixed
        };
        assert_eq!(thr.distance(47), 0);
        assert_eq!(thr.distance(40), 6);
    }

    fn mk_theme(root: &Path, id: &str, index: &str, files: &[&str]) {
        let d = root.join(id);
        std::fs::create_dir_all(&d).expect("mkdir");
        std::fs::write(d.join("index.theme"), index).expect("write");
        for f in files {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(p, b"<svg/>").expect("write");
        }
    }

    const FIXED: &str = "[Icon Theme]\nName=Base\nDirectories=16x16/places,48x48/places\n[16x16/places]\nSize=16\nType=Fixed\n[48x48/places]\nSize=48\nType=Fixed\n";

    #[test]
    fn discover_and_lookup_follow_inherits_and_sizes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user = tmp.path().join("user");
        let sys = tmp.path().join("sys");
        mk_theme(
            &sys,
            "Base",
            FIXED,
            &["16x16/places/folder.png", "48x48/places/folder.svg"],
        );
        mk_theme(
            &user,
            "Base-Dark",
            "[Icon Theme]\nName=Base Dark\nInherits=Base,hicolor\nDirectories=48x48/places\n[48x48/places]\nSize=48\nType=Fixed\n",
            &["48x48/places/user-home.svg"],
        );
        mk_theme(&sys, "hicolor", FIXED, &["48x48/places/folder.svg"]);
        let hidden = FIXED.replace("Name=Base\n", "Name=Base\nHidden=true\n");
        mk_theme(&sys, "Hidden", &hidden, &[]);
        let adwaita = FIXED.replace("Name=Base\n", "Name=Adwaita\nHidden=true\n");
        mk_theme(&sys, "Adwaita", &adwaita, &[]);
        std::fs::create_dir_all(sys.join("Cursors/cursors")).expect("mkdir");
        std::fs::write(sys.join("Cursors/index.theme"), "[Icon Theme]\nName=C\n").expect("write");

        let themes = discover(&[user.clone(), sys.clone()]);
        let ids: Vec<&str> = themes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            ["Adwaita", "Base", "Base-Dark"],
            "Hidden is left out, Adwaita kept"
        );

        let at = |id: &str, names: &[&str], want| find_icon(&themes, id, names, want);
        assert_eq!(
            at("Base", &["folder"], 48),
            Some(sys.join("Base/48x48/places/folder.svg"))
        );
        assert_eq!(
            at("Base", &["folder"], 16),
            Some(sys.join("Base/16x16/places/folder.png"))
        );
        assert_eq!(
            at("Base-Dark", &["user-home", "folder-home"], 48),
            Some(user.join("Base-Dark/48x48/places/user-home.svg"))
        );
        assert_eq!(
            at("Base-Dark", &["folder"], 48),
            Some(sys.join("Base/48x48/places/folder.svg")),
            "inherited from Base"
        );
        assert_eq!(
            at("Base-Dark", &["nope"], 48),
            None,
            "hicolor is not searched"
        );
    }

    #[test]
    fn install_copies_one_or_many_themes_and_keeps_links() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("download");
        mk_theme(&src, "Pack", FIXED, &["48x48/places/folder.svg"]);
        mk_theme(&src, "Pack-Dark", FIXED, &[]);
        std::os::unix::fs::symlink("../../../Pack/48x48/places", src.join("Pack-Dark/linked"))
            .expect("symlink");
        std::fs::create_dir_all(src.join("not-a-theme")).expect("mkdir");
        let dest = tmp.path().join("icons");

        let ids = install_from(&src, &dest).expect("install");
        assert_eq!(ids, ["Pack", "Pack-Dark"]);
        assert!(dest.join("Pack/48x48/places/folder.svg").is_file());
        assert!(
            std::fs::symlink_metadata(dest.join("Pack-Dark/linked"))
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        // A single theme folder installs itself; a repeat is refused.
        let err = install_from(&src.join("Pack"), &dest).expect_err("exists");
        assert!(err.contains("Already installed: Pack"), "{err}");
        let err = install_from(&src.join("not-a-theme"), &dest).expect_err("no theme");
        assert!(err.contains("no icon theme"), "{err}");
    }
}
