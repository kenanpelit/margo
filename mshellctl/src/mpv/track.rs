//! `mshellctl play __track` — the YouTube resume / history tracker.
//!
//! Started next to the mpv companion whenever `mshellctl play` loads a
//! YouTube video (one instance, guarded by a lock file). It subscribes to
//! mpv's JSON IPC (`observe_property` on path / time-pos / duration /
//! media-title / pause) and keeps `progress.json` current:
//!
//! * the playing video's position every [`SAVE_EVERY`], on pause, and
//!   when the file ends (replace / playlist-next / quit);
//! * played to the end (`end-file` reason `eof`) → `completed`, so the
//!   next play starts over;
//! * the title mpv resolved through yt-dlp, and a last-played stamp, so
//!   the launcher's history is ordered by what was actually watched.
//!
//! Non-YouTube files in the same window are ignored. The tracker exits
//! when mpv closes its socket. Decisions live in [`Tracker::on_line`], a
//! pure state machine tested against event streams captured from mpv
//! 0.41 on this machine (`fixtures/`).

use super::ipc as mpv_ipc;
use super::youtube::{ProgressEntry, ProgressStore, now_secs, video_id};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Save cadence while a video plays.
const SAVE_EVERY: Duration = Duration::from_secs(5);

/// A change to persist.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// `id` loaded (stamp last-played, keep its title).
    Started { id: String, title: Option<String> },
    /// Save the position.
    Position {
        id: String,
        position: f64,
        duration: Option<f64>,
        title: Option<String>,
    },
    /// Played to the end.
    Completed { id: String, duration: Option<f64> },
}

/// What the tracker knows about the current file.
#[derive(Debug, Default)]
pub struct Tracker {
    id: Option<String>,
    position: Option<f64>,
    duration: Option<f64>,
    title: Option<String>,
    loaded: bool,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn position_update(&self) -> Option<Update> {
        let id = self.id.clone()?;
        let position = self.position.filter(|p| *p > 0.0)?;
        Some(Update::Position {
            id,
            position,
            duration: self.duration,
            title: self.title.clone(),
        })
    }

    /// The periodic save (also run when the socket closes).
    pub fn flush(&self) -> Option<Update> {
        if self.loaded {
            self.position_update()
        } else {
            None
        }
    }

    /// Feed one IPC line; returns what to persist now.
    pub fn on_line(&mut self, line: &str) -> Vec<Update> {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return Vec::new();
        };
        match v.get("event").and_then(|e| e.as_str()) {
            Some("property-change") => {
                let data = v.get("data");
                match v.get("name").and_then(|n| n.as_str()) {
                    Some("path") => {
                        if let Some(path) = data.and_then(|d| d.as_str()) {
                            *self = Tracker {
                                id: video_id(path),
                                ..Tracker::default()
                            };
                        }
                    }
                    Some("time-pos") => {
                        if let Some(p) = data.and_then(|d| d.as_f64()) {
                            self.position = Some(p);
                            // Only a loaded file reports a position — this
                            // also covers a tracker that subscribed after
                            // `file-loaded` had already fired.
                            self.loaded = true;
                        }
                    }
                    Some("duration") => {
                        if let Some(d) = data.and_then(|d| d.as_f64()) {
                            self.duration = Some(d);
                        }
                    }
                    Some("media-title") => {
                        // Before yt-dlp answers mpv shows the URL tail
                        // (`watch?v=…`); keep only a real title.
                        if let Some(t) = data.and_then(|d| d.as_str())
                            && !t.starts_with("watch?")
                            && Some(t) != self.id.as_deref()
                        {
                            self.title = Some(t.to_string());
                        }
                    }
                    Some("pause") if data.and_then(|d| d.as_bool()) == Some(true) => {
                        return self.flush().into_iter().collect();
                    }
                    _ => {}
                }
                Vec::new()
            }
            Some("file-loaded") => {
                self.loaded = true;
                match &self.id {
                    Some(id) => vec![Update::Started {
                        id: id.clone(),
                        title: self.title.clone(),
                    }],
                    None => Vec::new(),
                }
            }
            Some("end-file") => {
                let out = match (&self.id, v.get("reason").and_then(|r| r.as_str())) {
                    (Some(id), Some("eof")) if self.loaded => vec![Update::Completed {
                        id: id.clone(),
                        duration: self.duration,
                    }],
                    _ if self.loaded => self.position_update().into_iter().collect(),
                    _ => Vec::new(),
                };
                self.id = None;
                self.loaded = false;
                out
            }
            _ => Vec::new(),
        }
    }
}

/// Persist one update (locked read-modify-write).
pub fn apply(store: &ProgressStore, update: &Update) -> std::io::Result<()> {
    let now = now_secs();
    store.update(|data| {
        let (id, f): (&str, Box<dyn FnOnce(&mut ProgressEntry)>) = match update {
            Update::Started { id, title } => (
                id,
                Box::new(move |e| {
                    if let Some(t) = title {
                        e.title = t.clone();
                    }
                }),
            ),
            Update::Position {
                id,
                position,
                duration,
                title,
            } => (
                id,
                Box::new(move |e| {
                    e.position = *position;
                    e.completed = false;
                    if duration.is_some() {
                        e.duration = *duration;
                    }
                    if let Some(t) = title {
                        e.title = t.clone();
                    }
                }),
            ),
            Update::Completed { id, duration } => (
                id,
                Box::new(move |e| {
                    e.position = 0.0;
                    e.completed = true;
                    if duration.is_some() {
                        e.duration = *duration;
                    }
                }),
            ),
        };
        let entry = data.videos.entry(id.to_string()).or_default();
        f(entry);
        entry.last_played = now;
    })
}

/// Lock that keeps the tracker single-instance; released on exit.
fn single_instance() -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let dir = margo_fs::xdg::runtime_dir_or_tmp().join("mshellctl-mpv");
    std::fs::create_dir_all(&dir).ok()?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("track.lock"))
        .ok()?;
    // SAFETY: flock on a descriptor we own; non-blocking.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (rc == 0).then_some(file)
}

/// Start the tracker in the background unless one already runs. The
/// child is the current `mshellctl` binary, detached from our stdio.
pub fn ensure_running() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe)
        .args(["play", "__track"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// The tracker loop (blocking). Waits up to ~15 s for mpv's socket.
pub fn run() -> anyhow::Result<()> {
    let Some(_lock) = single_instance() else {
        return Ok(()); // another tracker owns this mpv
    };
    let path = mpv_ipc::socket_path();
    let mut sock = None;
    for _ in 0..60 {
        if let Ok(s) = UnixStream::connect(&path) {
            sock = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let Some(sock) = sock else {
        anyhow::bail!("mpv IPC socket {} did not appear", path.display());
    };
    {
        let mut w = &sock;
        for (i, prop) in ["path", "time-pos", "duration", "media-title", "pause"]
            .iter()
            .enumerate()
        {
            let cmd = serde_json::json!({ "command": ["observe_property", i + 1, prop] });
            w.write_all(cmd.to_string().as_bytes())?;
            w.write_all(b"\n")?;
        }
    }
    // A read timeout turns the blocking line reader into a ticker for the
    // periodic save.
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;
    let store = ProgressStore::default_location();
    let persist = |u: &Update| {
        if let Err(err) = apply(&store, u) {
            tracing::warn!(%err, "youtube tracker: saving progress failed");
        }
    };
    let mut tracker = Tracker::new();
    let mut reader = BufReader::new(&sock);
    let mut last_save = Instant::now();
    // Bytes, not a String: a read timeout can land mid-line (even
    // mid-character); `read_until` keeps what it got and the next call
    // appends the rest.
    let mut line: Vec<u8> = Vec::new();
    loop {
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break, // mpv closed the socket
            Ok(_) if line.ends_with(b"\n") => {
                for u in tracker.on_line(String::from_utf8_lossy(&line).trim_end()) {
                    persist(&u);
                    last_save = Instant::now();
                }
                line.clear();
            }
            Ok(_) => {} // EOF mid-line; the next read returns 0
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        if last_save.elapsed() >= SAVE_EVERY {
            if let Some(u) = tracker.flush() {
                persist(&u);
            }
            last_save = Instant::now();
        }
    }
    if let Some(u) = tracker.flush() {
        persist(&u);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "5C_HPTJg5ek";

    /// Captured from mpv 0.41 (`--idle --vo=null`, two files queued, both
    /// played to the end); the local file path is replaced by the
    /// `ytdl://` URL `mshellctl play` loads.
    fn fixture(name: &str) -> Vec<String> {
        let raw = match name {
            "eof" => include_str!("fixtures/mpv-ipc-eof.jsonl"),
            _ => include_str!("fixtures/mpv-ipc-stop-quit.jsonl"),
        };
        raw.lines().map(String::from).collect()
    }

    fn run_all(lines: &[String]) -> Vec<Update> {
        let mut t = Tracker::new();
        lines.iter().flat_map(|l| t.on_line(l)).collect()
    }

    #[test]
    fn played_to_the_end_marks_completed() {
        let updates = run_all(&fixture("eof"));
        assert_eq!(
            updates.first(),
            Some(&Update::Started {
                id: ID.into(),
                title: None
            })
        );
        let completed: Vec<_> = updates
            .iter()
            .filter(|u| matches!(u, Update::Completed { .. }))
            .collect();
        assert_eq!(completed.len(), 2, "{updates:?}");
        assert_eq!(
            completed[0],
            &Update::Completed {
                id: ID.into(),
                duration: Some(3.0)
            }
        );
    }

    #[test]
    fn pause_replace_and_quit_save_the_position() {
        let updates = run_all(&fixture("stop-quit"));
        let saves: Vec<(String, f64, Option<String>)> = updates
            .iter()
            .filter_map(|u| match u {
                Update::Position {
                    id,
                    position,
                    title,
                    ..
                } => Some((id.clone(), *position, title.clone())),
                _ => None,
            })
            .collect();
        // Pause → save, replaced (stop) → save. The middle file is not
        // YouTube and is never recorded; the third load quit while mpv
        // still reported time-pos 0, so there was nothing to resume.
        assert_eq!(saves.len(), 2, "{updates:?}");
        let starts = updates
            .iter()
            .filter(|u| matches!(u, Update::Started { .. }))
            .count();
        assert_eq!(starts, 2, "both YouTube loads stamp the history");
        assert!(saves.iter().all(|(id, ..)| id == ID));
        assert!(saves[0].1 >= 5.0, "started at the resume point (start=5)");
        assert_eq!(saves[0].2.as_deref(), Some("Long tone"));
        assert!(
            !updates
                .iter()
                .any(|u| matches!(u, Update::Completed { .. }))
        );
    }

    /// Found end-to-end: `play` loads the video, *then* starts the
    /// tracker, which subscribes after `file-loaded` already fired. mpv
    /// still sends every observed property's current value, so a
    /// reported time-pos must count as "loaded".
    #[test]
    fn a_tracker_attached_mid_file_still_saves() {
        let mut t = Tracker::new();
        for line in [
            format!(
                r#"{{"event":"property-change","id":1,"name":"path","data":"ytdl://https://www.youtube.com/watch?v={ID}"}}"#
            ),
            r#"{"event":"property-change","id":3,"name":"duration","data":90.0}"#.to_string(),
            r#"{"event":"property-change","id":4,"name":"media-title","data":"Ninety"}"#
                .to_string(),
            r#"{"event":"property-change","id":2,"name":"time-pos","data":41.5}"#.to_string(),
        ] {
            t.on_line(&line);
        }
        assert_eq!(
            t.flush(),
            Some(Update::Position {
                id: ID.into(),
                position: 41.5,
                duration: Some(90.0),
                title: Some("Ninety".into())
            })
        );
        let end = t.on_line(r#"{"event":"end-file","reason":"eof","playlist_entry_id":1}"#);
        assert_eq!(
            end,
            vec![Update::Completed {
                id: ID.into(),
                duration: Some(90.0)
            }]
        );
    }

    #[test]
    fn nothing_is_saved_for_other_files_or_garbage() {
        let mut t = Tracker::new();
        assert!(t.on_line("not json").is_empty());
        t.on_line(r#"{"event":"property-change","id":1,"name":"path","data":"/m/a.mkv"}"#);
        t.on_line(r#"{"event":"file-loaded"}"#);
        t.on_line(r#"{"event":"property-change","id":2,"name":"time-pos","data":30.0}"#);
        assert_eq!(t.flush(), None);
        assert!(
            t.on_line(r#"{"event":"end-file","reason":"eof"}"#)
                .is_empty()
        );
    }

    #[test]
    fn url_tail_is_not_taken_for_a_title() {
        let mut t = Tracker::new();
        t.on_line(&format!(
            r#"{{"event":"property-change","id":1,"name":"path","data":"ytdl://https://www.youtube.com/watch?v={ID}"}}"#
        ));
        t.on_line(&format!(
            r#"{{"event":"property-change","id":4,"name":"media-title","data":"watch?v={ID}"}}"#
        ));
        t.on_line(r#"{"event":"file-loaded"}"#);
        t.on_line(r#"{"event":"property-change","id":2,"name":"time-pos","data":12.5}"#);
        assert_eq!(
            t.flush(),
            Some(Update::Position {
                id: ID.into(),
                position: 12.5,
                duration: None,
                title: None
            })
        );
    }

    #[test]
    fn apply_writes_position_then_completion() {
        let dir = std::env::temp_dir().join(format!("mshellctl-track-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = ProgressStore::new(dir.clone());
        apply(
            &store,
            &Update::Position {
                id: ID.into(),
                position: 42.0,
                duration: Some(149.0),
                title: Some("Rust in 100 Seconds".into()),
            },
        )
        .unwrap();
        let e = store.load().videos[ID].clone();
        assert_eq!(
            (e.position, e.completed, e.duration),
            (42.0, false, Some(149.0))
        );
        assert_eq!(e.title, "Rust in 100 Seconds");
        apply(
            &store,
            &Update::Completed {
                id: ID.into(),
                duration: None,
            },
        )
        .unwrap();
        let e = store.load().videos[ID].clone();
        assert_eq!(
            (e.position, e.completed, e.duration),
            (0.0, true, Some(149.0))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
