// SPDX-License-Identifier: GPL-3.0-or-later
//! Playlist files — read `.m3u` / `.m3u8` / `.pls`, write extended `.m3u`,
//! and a small on-disk library under `~/.config/margo/mtune/playlists/`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use gtk::glib;
use gtk::prelude::*;
use log::debug;

use crate::audio::Queue;

/// `~/.config/margo/mtune/playlists/` — where "Save Playlist" writes and
/// the shell's playlist list reads. Created on demand.
pub fn library_dir() -> PathBuf {
    let mut d = glib::user_config_dir();
    d.push("margo");
    d.push("mtune");
    d.push("playlists");
    d
}

/// Path of a saved playlist by display name.
pub fn saved_path(name: &str) -> PathBuf {
    library_dir().join(format!("{}.m3u", sanitize(name)))
}

/// Names of the saved playlists, sorted, case-insensitive.
pub fn saved_names() -> Vec<String> {
    let Ok(rd) = fs::read_dir(library_dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let p = e.path();
            match p.extension().and_then(|x| x.to_str()) {
                Some("m3u") | Some("m3u8") | Some("pls") => {
                    p.file_stem().and_then(|s| s.to_str()).map(str::to_owned)
                }
                _ => None,
            }
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Parse a playlist file into the audio-file paths it references.
/// Relative entries resolve against the playlist file's directory.
pub fn parse(path: &Path) -> Vec<PathBuf> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "pls" => parse_pls(path),
        _ => parse_m3u(path),
    }
}

fn parse_m3u(path: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(path) else {
        debug!("playlist: cannot read {}", path.display());
        return Vec::new();
    };
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| resolve(l, base))
        .collect()
}

fn parse_pls(path: &Path) -> Vec<PathBuf> {
    let kf = glib::KeyFile::new();
    if kf.load_from_file(path, glib::KeyFileFlags::NONE).is_err() {
        return Vec::new();
    }
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    let n = kf.int64("playlist", "NumberOfEntries").unwrap_or(0).max(0) as usize;
    (1..=n)
        .filter_map(|i| kf.value("playlist", &format!("File{i}")).ok())
        .map(|v| resolve(v.as_str(), base))
        .collect()
}

fn resolve(entry: &str, base: &Path) -> PathBuf {
    if let Some(rest) = entry.strip_prefix("file://") {
        let unescaped = glib::uri_unescape_string(rest, None::<&str>)
            .map(|g| g.to_string())
            .unwrap_or_else(|| rest.to_string());
        return PathBuf::from(unescaped);
    }
    let p = PathBuf::from(entry);
    if p.is_absolute() { p } else { base.join(p) }
}

/// Write the current queue as an extended `.m3u` (`#EXTM3U` + `#EXTINF`).
pub fn write_m3u(path: &Path, queue: &Queue) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::File::create(path)?;
    writeln!(f, "#EXTM3U")?;
    for i in 0..queue.n_songs() {
        let Some(song) = queue.song_at(i) else {
            continue;
        };
        let Some(fp) = song.file().path() else {
            continue;
        };
        let title = song.title();
        let artist = song.artist();
        let label = if artist.is_empty() {
            title.clone()
        } else {
            format!("{artist} - {title}")
        };
        writeln!(f, "#EXTINF:{},{}", song.duration(), label)?;
        writeln!(f, "{}", fp.display())?;
    }
    Ok(())
}

const RESUME_PREFIX: &str = "#TUNE-RESUME:";

/// The 0-based track index a playlist last left off at, if the file
/// carries a `#TUNE-RESUME:` comment (written by `update_resume_index`).
pub fn resume_index(path: &Path) -> Option<u32> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(RESUME_PREFIX))
        .and_then(|n| n.trim().parse().ok())
}

/// Rewrite just the `#TUNE-RESUME:` line in `path` -- every other line
/// (the `#EXTM3U` header, `#EXTINF` metadata, song paths) is copied
/// through unchanged. `index == 0` removes the line entirely, keeping a
/// never-resumed or freshly-saved playlist's file pristine. A missing or
/// unreadable file is a silent no-op (nothing to update).
pub fn update_resume_index(path: &Path, index: u32) -> std::io::Result<()> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(());
    };
    let mut out = String::with_capacity(text.len() + 16);
    let mut inserted = false;
    for line in text.lines() {
        if line.starts_with(RESUME_PREFIX) {
            if index > 0 && !inserted {
                out.push_str(&format!("{RESUME_PREFIX}{index}\n"));
                inserted = true;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if !inserted && line.starts_with("#EXTM3U") && index > 0 {
            out.push_str(&format!("{RESUME_PREFIX}{index}\n"));
            inserted = true;
        }
    }
    if index > 0 && !inserted {
        // No #EXTM3U header (a bare/legacy playlist) -- prepend it.
        out = format!("{RESUME_PREFIX}{index}\n{out}");
    }
    fs::write(path, out)
}

/// Save the queue to the library under `name`, returning its path.
pub fn save(name: &str, queue: &Queue) -> std::io::Result<PathBuf> {
    let path = saved_path(name);
    write_m3u(&path, queue)?;
    Ok(path)
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "playlist".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture(dir: &std::path::Path, body: &str) -> PathBuf {
        let path = dir.join("fixture.m3u");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn resume_index_absent_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(dir.path(), "#EXTM3U\n/song1.mp3\n/song2.mp3\n");
        assert_eq!(resume_index(&path), None);
    }

    #[test]
    fn resume_index_reads_the_comment() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "#EXTM3U\n#TUNE-RESUME:2\n/song1.mp3\n/song2.mp3\n/song3.mp3\n",
        );
        assert_eq!(resume_index(&path), Some(2));
    }

    #[test]
    fn update_resume_index_inserts_then_updates_without_touching_songs() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(dir.path(), "#EXTM3U\n/song1.mp3\n/song2.mp3\n");

        update_resume_index(&path, 1).unwrap();
        assert_eq!(resume_index(&path), Some(1));
        let songs = parse(&path);
        assert_eq!(songs.len(), 2);

        update_resume_index(&path, 0).unwrap();
        // Index 0 removes the line entirely (keeps a pristine file for
        // the common/never-resumed case).
        assert_eq!(resume_index(&path), None);
        assert_eq!(parse(&path).len(), 2);
    }
}
