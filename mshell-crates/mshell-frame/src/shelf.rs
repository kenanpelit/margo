//! The file shelf behind the Shelf pill + menu: a list of file paths the
//! user dropped in, to drag back out into another app later (a browser
//! upload, a chat, a mail). It keeps references only — removing an item
//! never touches the file.
//!
//! One owner task on the shared tokio runtime holds the list: the pill and
//! the menu send [`ShelfCommand`]s and watch the published [`ShelfState`],
//! and the task saves every change to `$XDG_STATE_HOME/margo/shelf.json`
//! (atomic write, off the GTK thread). A file that doesn't parse is kept
//! (with a `.bak` copy) and never overwritten (`margo_fs::side_file`).
//! [`Shelf`] itself is pure and unit-tested.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::{mpsc, watch};
use tracing::warn;

/// Items kept at most (oldest fall off).
pub(crate) const MAX_ITEMS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShelfItem {
    pub(crate) path: PathBuf,
    /// Unix seconds it was (last) added.
    #[serde(default)]
    pub(crate) added: i64,
}

/// The shelf file: `{ "version": 1, "items": [...] }`, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shelf {
    #[serde(default = "version")]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) items: Vec<ShelfItem>,
}

fn version() -> u32 {
    1
}

impl Default for Shelf {
    fn default() -> Self {
        Self {
            version: version(),
            items: Vec::new(),
        }
    }
}

/// `/a/b/` and `/a/b` are one entry; only absolute paths are kept.
fn normalize(p: &Path) -> Option<PathBuf> {
    let s = p.to_str()?;
    let trimmed = if s.len() > 1 {
        s.trim_end_matches('/')
    } else {
        s
    };
    let p = PathBuf::from(if trimmed.is_empty() { "/" } else { trimmed });
    p.is_absolute().then_some(p)
}

impl Shelf {
    pub(crate) fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut s: Self = serde_json::from_str(text)?;
        // Drop what a hand edit may have broken: relative paths, duplicates.
        let mut seen = std::collections::HashSet::new();
        s.items.retain_mut(|i| match normalize(&i.path) {
            Some(p) if seen.insert(p.clone()) => {
                i.path = p;
                true
            }
            _ => false,
        });
        s.items.truncate(MAX_ITEMS);
        Ok(s)
    }

    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string_pretty(&Self {
            version: 1,
            items: self.items.clone(),
        })
        .unwrap_or_default()
    }

    /// Put `paths` on top, in the given order; one already on the shelf
    /// moves up instead of appearing twice. Returns how many were new.
    pub(crate) fn add(&mut self, paths: &[PathBuf], now: i64) -> usize {
        let mut new = 0;
        for p in paths.iter().rev() {
            let Some(p) = normalize(p) else { continue };
            match self.items.iter().position(|i| i.path == p) {
                Some(at) => {
                    self.items.remove(at);
                }
                None => new += 1,
            }
            self.items.insert(
                0,
                ShelfItem {
                    path: p,
                    added: now,
                },
            );
        }
        self.items.truncate(MAX_ITEMS);
        new
    }

    pub(crate) fn remove(&mut self, path: &Path) -> bool {
        let n = self.items.len();
        self.items.retain(|i| i.path != path);
        self.items.len() != n
    }

    #[cfg(test)]
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        self.items.iter().map(|i| i.path.clone()).collect()
    }
}

/// `text/uri-list` for `paths` (RFC 2483: CRLF-separated `file://` URIs,
/// percent-encoded).
pub(crate) fn uri_list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| file_uri(p))
        .map(|u| u + "\r\n")
        .collect()
}

/// `file://` URI of an absolute path, bytes outside the unreserved /
/// path set percent-encoded.
pub(crate) fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// GNOME's clipboard format for copied files (pasting in Nautilus and
/// friends copies the files).
pub(crate) fn gnome_copied_files(paths: &[PathBuf]) -> String {
    let mut s = String::from("copy");
    for p in paths {
        s.push('\n');
        s.push_str(&file_uri(p));
    }
    s
}

// ── The owner task ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum ShelfCommand {
    Add(Vec<PathBuf>),
    Remove(PathBuf),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ShelfState {
    pub(crate) items: Vec<ShelfItem>,
    /// The saved file didn't parse: shown in the menu; nothing is saved
    /// over it until it is fixed or moved away.
    pub(crate) broken: Option<String>,
}

struct Handle {
    state: watch::Sender<ShelfState>,
    cmd: mpsc::UnboundedSender<ShelfCommand>,
}

fn shelf_path() -> PathBuf {
    margo_fs::xdg::state_home().join("margo/shelf.json")
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn handle() -> &'static Handle {
    static H: OnceLock<Handle> = OnceLock::new();
    H.get_or_init(|| {
        let (state, _) = watch::channel(ShelfState::default());
        let (cmd, rx) = mpsc::unbounded_channel();
        mshell_services::tokio_rt_spawn(run(shelf_path(), state.clone(), rx));
        Handle { state, cmd }
    })
}

pub(crate) fn subscribe() -> watch::Receiver<ShelfState> {
    handle().state.subscribe()
}

pub(crate) fn send(cmd: ShelfCommand) {
    let _ = handle().cmd.send(cmd);
}

async fn run(
    path: PathBuf,
    state: watch::Sender<ShelfState>,
    mut rx: mpsc::UnboundedReceiver<ShelfCommand>,
) {
    use margo_fs::side_file::{self, SideLoad};
    let p = path.clone();
    let loaded = tokio::task::spawn_blocking(move || side_file::load(&p, Shelf::from_json))
        .await
        .unwrap_or(SideLoad::Missing);
    let (mut shelf, broken) = match loaded {
        SideLoad::Loaded(s) => (s, None),
        SideLoad::Missing => (Shelf::default(), None),
        SideLoad::Broken { error, backup } => {
            warn!(%error, ?backup, "shelf: shelf.json doesn't parse; not saving over it");
            (Shelf::default(), Some(error))
        }
    };
    state.send_replace(ShelfState {
        items: shelf.items.clone(),
        broken: broken.clone(),
    });
    while let Some(cmd) = rx.recv().await {
        let changed = match cmd {
            ShelfCommand::Add(paths) => {
                let before = shelf.items.clone();
                shelf.add(&paths, now_secs());
                shelf.items != before
            }
            ShelfCommand::Remove(p) => shelf.remove(&p),
            ShelfCommand::Clear => {
                let had = !shelf.items.is_empty();
                shelf.items.clear();
                had
            }
        };
        if !changed {
            continue;
        }
        let body = shelf.to_json();
        let p = path.clone();
        let saved = tokio::task::spawn_blocking(move || {
            side_file::save(&p, body.as_bytes(), |t| Shelf::from_json(t).map(|_| ()))
        })
        .await;
        let broken = match saved {
            Ok(Ok(())) => None,
            Ok(Err(e)) => {
                warn!(error = %e, "shelf: not saved");
                Some(e.to_string())
            }
            Err(e) => Some(e.to_string()),
        };
        state.send_replace(ShelfState {
            items: shelf.items.clone(),
            broken,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn adding_dedupes_moves_up_and_keeps_order() {
        let mut s = Shelf::default();
        assert_eq!(s.add(&[p("/a"), p("/b/"), p("rel/x")], 1), 2);
        assert_eq!(
            s.paths(),
            [p("/a"), p("/b")],
            "drop order kept; relative refused"
        );
        assert_eq!(s.add(&[p("/c"), p("/b")], 2), 1);
        assert_eq!(s.paths(), [p("/c"), p("/b"), p("/a")]);
        assert_eq!(s.items[1].added, 2, "re-added: new time");
        assert!(s.remove(&p("/b")));
        assert!(!s.remove(&p("/b")));
        assert_eq!(s.add(&[p("/")], 3), 1, "the root itself is a path");
        let many: Vec<PathBuf> = (0..250).map(|i| p(&format!("/f{i}"))).collect();
        s.add(&many, 4);
        assert_eq!(s.items.len(), MAX_ITEMS);
        assert_eq!(s.items[0].path, p("/f0"));
    }

    #[test]
    fn file_round_trips_and_hand_edits_are_cleaned() {
        let mut s = Shelf::default();
        s.add(&[p("/home/u/a b.png")], 7);
        assert_eq!(Shelf::from_json(&s.to_json()).unwrap(), s);
        // The plugin's own format reads as-is.
        let plugin = r#"{"version": 1, "items": [{"path": "/x/", "added": 5}, {"path": "/x"}, {"path": "y"}]}"#;
        let s = Shelf::from_json(plugin).unwrap();
        assert_eq!(s.paths(), [p("/x")]);
        assert_eq!(s.items[0].added, 5);
        assert!(Shelf::from_json("{\"items\": 3}").is_err());
    }

    #[test]
    fn uris_are_percent_encoded() {
        assert_eq!(
            file_uri(Path::new("/home/u/a b#1%.png")),
            "file:///home/u/a%20b%231%25.png"
        );
        assert_eq!(file_uri(Path::new("/tmp/ç")), "file:///tmp/%C3%A7");
        assert_eq!(
            uri_list(&[p("/a"), p("/b c")]),
            "file:///a\r\nfile:///b%20c\r\n"
        );
        assert_eq!(gnome_copied_files(&[p("/a")]), "copy\nfile:///a");
    }

    #[tokio::test]
    async fn the_owner_task_saves_and_refuses_to_clobber_a_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("margo/shelf.json");
        let (state, mut srx) = watch::channel(ShelfState::default());
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(path.clone(), state, rx));
        tx.send(ShelfCommand::Add(vec![p("/a"), p("/b")])).unwrap();
        tx.send(ShelfCommand::Remove(p("/a"))).unwrap();
        let st = srx.wait_for(|s| s.items.len() == 1).await.unwrap().clone();
        assert_eq!(st.items[0].path, p("/b"));
        let on_disk = Shelf::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk.paths(), [p("/b")]);
        drop(tx);
        task.await.unwrap();

        // A broken file is reported, kept (with a .bak) and not replaced.
        std::fs::write(&path, "{ not json").unwrap();
        let (state, mut srx) = watch::channel(ShelfState::default());
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(path.clone(), state, rx));
        srx.wait_for(|s| s.broken.is_some()).await.unwrap();
        tx.send(ShelfCommand::Add(vec![p("/c")])).unwrap();
        let st = srx.wait_for(|s| !s.items.is_empty()).await.unwrap().clone();
        assert!(st.broken.is_some(), "still reported");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        assert!(dir.path().join("margo/shelf.json.bak").exists());
        drop(tx);
        task.await.unwrap();
    }
}
