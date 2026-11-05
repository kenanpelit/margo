//! CLAUDE.md files: each account's global one (`<config>/CLAUDE.md`) and
//! the project ones (`CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`)
//! of every project that has Claude Code sessions. Only paths and sizes
//! are read here; the menu opens a file in the user's default app.

use super::accounts::Account;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryFile {
    pub(crate) path: PathBuf,
    /// `Global · default` or the project path.
    pub(crate) label: String,
    pub(crate) global: bool,
    pub(crate) bytes: u64,
}

/// The project-level memory files Claude Code reads, in its order.
const PROJECT_FILES: [&str; 3] = ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"];

/// Every existing memory file (`size` returns a file's length, `None`
/// when it is missing). Globals first, then projects in `projects` order.
pub(crate) fn memory_files(
    accounts: &[Account],
    projects: &[String],
    size: impl Fn(&Path) -> Option<u64>,
) -> Vec<MemoryFile> {
    let mut out = Vec::new();
    let multi = accounts.len() > 1;
    for a in accounts {
        let path = a.dir.join("CLAUDE.md");
        if let Some(bytes) = size(&path) {
            out.push(MemoryFile {
                label: if multi {
                    format!("Global · {}", a.label)
                } else {
                    "Global".into()
                },
                path,
                global: true,
                bytes,
            });
        }
    }
    for p in projects.iter().filter(|p| Path::new(p).is_absolute()) {
        for name in PROJECT_FILES {
            let path = Path::new(p).join(name);
            if out.iter().any(|f: &MemoryFile| f.path == path) {
                continue;
            }
            if let Some(bytes) = size(&path) {
                out.push(MemoryFile {
                    path,
                    label: p.clone(),
                    global: false,
                    bytes,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globals_then_existing_project_files() {
        let accounts = [Account {
            dir: PathBuf::from("/h/.claude"),
            label: "default".into(),
            primary: true,
        }];
        let have = [
            "/h/.claude/CLAUDE.md",
            "/p/a/CLAUDE.md",
            "/p/a/CLAUDE.local.md",
            "/p/b/.claude/CLAUDE.md",
        ];
        let size = |p: &Path| have.contains(&p.to_str().unwrap()).then_some(10);
        let v = memory_files(
            &accounts,
            &["/p/a".into(), "/p/b".into(), "/p/a".into(), "rel".into()],
            size,
        );
        let got: Vec<_> = v
            .iter()
            .map(|f| (f.label.as_str(), f.path.to_str().unwrap(), f.global))
            .collect();
        assert_eq!(
            got,
            [
                ("Global", "/h/.claude/CLAUDE.md", true),
                ("/p/a", "/p/a/CLAUDE.md", false),
                ("/p/a", "/p/a/CLAUDE.local.md", false),
                ("/p/b", "/p/b/.claude/CLAUDE.md", false),
            ]
        );
    }
}
