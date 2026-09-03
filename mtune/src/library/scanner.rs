// SPDX-License-Identifier: GPL-3.0-or-later
//! Recursive folder scan. Runs on a worker thread and streams playable
//! file paths back over a channel so a large library never blocks startup.

use crate::library::config::LibrarySection;
use ignore::WalkBuilder;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum ScanMsg {
    /// One playable file, in a stable per-directory order.
    Found(PathBuf),
    /// The scan finished; `total` is the number of `Found` messages sent.
    Done { total: usize },
}

/// The synchronous core: walk `roots` (recursive per `lib.recursive`),
/// keep only playable files, sorted within each root.
pub fn scan_blocking(roots: &[PathBuf], lib: &LibrarySection) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        let mut builder = WalkBuilder::new(root);
        builder
            .standard_filters(false) // no .gitignore semantics
            .hidden(true) // skip dotfiles
            .follow_links(false)
            .max_depth(if lib.recursive { None } else { Some(1) });
        let mut batch: Vec<PathBuf> = builder
            .build()
            .filter_map(|r| r.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.into_path())
            .filter(|p| lib.is_playable(p))
            .collect();
        batch.sort();
        out.extend(batch);
    }
    out
}

/// Spawn a worker thread that walks the roots and streams `ScanMsg`s.
/// The thread exits early if the receiver is dropped.
pub fn scan(roots: Vec<PathBuf>, lib: LibrarySection) -> async_channel::Receiver<ScanMsg> {
    let (tx, rx) = async_channel::unbounded();
    let spawned = std::thread::Builder::new()
        .name("mtune-scan".into())
        .spawn(move || {
            let files = scan_blocking(&roots, &lib);
            let total = files.len();
            for f in files {
                if tx.send_blocking(ScanMsg::Found(f)).is_err() {
                    return; // receiver dropped
                }
            }
            let _ = tx.send_blocking(ScanMsg::Done { total });
        });
    if let Err(e) = spawned {
        log::error!("mtune: could not spawn the library-scan thread: {e}");
        // rx will simply never yield; callers treat that as an empty library.
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("Album A")).unwrap();
        fs::create_dir_all(r.join("Album B/disc2")).unwrap();
        fs::write(r.join("Album A/01.mp3"), b"x").unwrap();
        fs::write(r.join("Album A/02.flac"), b"x").unwrap();
        fs::write(r.join("Album A/cover.jpg"), b"x").unwrap();
        fs::write(r.join("Album B/1.ogg"), b"x").unwrap();
        fs::write(r.join("Album B/disc2/2.ogg"), b"x").unwrap();
        fs::write(r.join("top.wav"), b"x").unwrap();
        d
    }

    #[test]
    fn recursive_finds_all_playable_skips_others() {
        let d = tree();
        let lib = LibrarySection::default(); // recursive = true
        let found = scan_blocking(&[d.path().to_path_buf()], &lib);
        assert_eq!(found.len(), 5); // 4 in albums + top.wav; cover.jpg excluded
        assert!(found.iter().any(|p| p.ends_with("Album B/disc2/2.ogg")));
    }

    #[test]
    fn non_recursive_stays_top_level() {
        let d = tree();
        let lib = LibrarySection {
            recursive: false,
            ..Default::default()
        };
        let found = scan_blocking(&[d.path().to_path_buf()], &lib);
        assert_eq!(found, vec![d.path().join("top.wav")]);
    }

    #[test]
    fn results_are_sorted_stably() {
        let d = tree();
        let lib = LibrarySection::default();
        let a = scan_blocking(&[d.path().to_path_buf()], &lib);
        let b = scan_blocking(&[d.path().to_path_buf()], &lib);
        assert_eq!(a, b);
        let ia = a
            .iter()
            .position(|p| p.ends_with("Album A/01.mp3"))
            .unwrap();
        let ib = a
            .iter()
            .position(|p| p.ends_with("Album A/02.flac"))
            .unwrap();
        assert!(ia < ib);
    }

    #[test]
    fn async_scan_streams_then_done() {
        let d = tree();
        let rx = scan(vec![d.path().to_path_buf()], LibrarySection::default());
        let mut found = 0usize;
        loop {
            match rx.recv_blocking().unwrap() {
                ScanMsg::Found(_) => found += 1,
                ScanMsg::Done { total } => {
                    assert_eq!(total, found);
                    assert_eq!(total, 5);
                    break;
                }
            }
        }
    }
}
