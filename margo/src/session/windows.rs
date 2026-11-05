//! Per-tag app lists in the session snapshot (workspace restore).
//!
//! Native port of the workspace-restore community plugin onto margo's
//! session snapshot. `session_save` records every open window — app id,
//! output, tags, floating and the command line it runs with — next to the
//! per-tag layouts. With `session_restore_windows = 1`, `session_load`
//! (and crash recovery, and `session_restore_at_login = 1` at startup)
//! then:
//!
//!  * moves already-open windows back to their saved output and tags,
//!  * relaunches saved apps that aren't open (each once, from its saved
//!    command line — no shell), and
//!  * places the windows those launches open on their saved tag even if a
//!    window rule would send them elsewhere (the plugin's exec rule), for
//!    [`PLACEMENT_TTL`].
//!
//! `session_auto_add = 1` adds a newly opened app that isn't on the saved
//! list yet, on the tag it opened on — it only adds, flushed with the
//! minute's autosave.
//!
//! Restoring never closes windows and never touches scratchpads; window
//! sizes and positions are not restored, only which output and tag each
//! window is on. The on-disk format stays version 1: `windows` is a new
//! defaulted field, so older snapshots load (with no window list) and an
//! older margo reading a new file ignores it.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::SessionSnapshot;
use crate::state::MargoState;

/// How long a relaunched app's placement waits for its first window.
pub const PLACEMENT_TTL: Duration = Duration::from_secs(90);

/// One saved window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowEntry {
    pub app_id: String,
    /// Output (connector) name it was on.
    pub monitor: String,
    /// Tag mask it was on.
    pub tags: u32,
    #[serde(default)]
    pub floating: bool,
    /// argv to relaunch it with (`/proc/<pid>/cmdline`). Empty when unknown,
    /// or for the second+ window of one process (one launch reopens the
    /// app; its own session restore brings the other windows).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
}

/// An open window, as the restore planner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindow {
    pub id: u64,
    pub app_id: String,
    pub monitor: String,
    pub tags: u32,
}

/// What a restore does.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    /// `(window id, entry index)`: move that window to that entry's place.
    pub moves: Vec<(u64, usize)>,
    /// Entry indices with no open window: launch (when they have a
    /// command) and wait for their window.
    pub missing: Vec<usize>,
}

/// Match saved entries to open windows, per app id, in order: a window
/// already in its place keeps it; the rest pair up in list order and move;
/// entries left over are missing. Windows beyond the saved count stay put.
pub fn plan_restore(entries: &[WindowEntry], open: &[OpenWindow]) -> Plan {
    let mut plan = Plan::default();
    let mut taken = vec![false; open.len()];
    let mut unmatched: Vec<usize> = Vec::new();
    // Pass 1: windows already where an entry wants them.
    for (ei, e) in entries.iter().enumerate() {
        let hit = open.iter().enumerate().position(|(wi, w)| {
            !taken[wi] && w.app_id == e.app_id && w.monitor == e.monitor && w.tags & e.tags != 0
        });
        match hit {
            Some(wi) => taken[wi] = true,
            None => unmatched.push(ei),
        }
    }
    // Pass 2: the rest pair up with any window of the app, else missing.
    for ei in unmatched {
        let e = &entries[ei];
        match (0..open.len()).find(|&wi| !taken[wi] && open[wi].app_id == e.app_id) {
            Some(wi) => {
                taken[wi] = true;
                plan.moves.push((open[wi].id, ei));
            }
            None => plan.missing.push(ei),
        }
    }
    plan
}

/// argv from a `/proc/<pid>/cmdline` body (NUL-separated, NUL-terminated).
pub fn parse_cmdline(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}

/// A saved window waiting for its relaunched app to map.
#[derive(Debug, Clone)]
struct Placement {
    entry: WindowEntry,
    until: Instant,
}

/// Session-restore state that lives as long as margo runs
/// (`MargoState::session`).
#[derive(Debug, Default)]
pub struct SessionRuntime {
    /// Snapshot loaded from the crash autosave at startup — applied by
    /// the autosave timer once outputs are mapped, then cleared.
    pub pending_crash_restore: Option<SessionSnapshot>,
    /// `session.json` loaded at startup for `session_restore_at_login`,
    /// applied the same way.
    pub pending_login_restore: Option<SessionSnapshot>,
    placements: Vec<Placement>,
    /// `session_auto_add`: new apps waiting to be added to `session.json`.
    pub auto_add: Vec<WindowEntry>,
}

/// What [`MargoState::restore_session_windows`] did, for the notification.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RestoreReport {
    pub moved: usize,
    pub launched: usize,
}

impl SessionRuntime {
    /// The first live placement for `app_id`, consumed.
    fn take_placement(&mut self, app_id: &str, now: Instant) -> Option<WindowEntry> {
        self.placements.retain(|p| p.until > now);
        let i = self
            .placements
            .iter()
            .position(|p| p.entry.app_id == app_id)?;
        Some(self.placements.remove(i).entry)
    }

    /// Queue `entry` for `session_auto_add` unless that app is queued.
    fn queue_auto_add(&mut self, entry: WindowEntry) {
        if !self.auto_add.iter().any(|e| e.app_id == entry.app_id) {
            self.auto_add.push(entry);
        }
    }
}

/// Add the queued apps that `snap` doesn't list yet. Returns whether it
/// changed.
pub fn merge_auto_add(snap: &mut SessionSnapshot, queued: Vec<WindowEntry>) -> bool {
    let mut changed = false;
    for e in queued {
        if !snap.windows.iter().any(|w| w.app_id == e.app_id) {
            snap.windows.push(e);
            changed = true;
        }
    }
    changed
}

fn read_cmdline(pid: u32) -> Vec<String> {
    if pid == 0 {
        return Vec::new();
    }
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|raw| parse_cmdline(&raw))
        .unwrap_or_default()
}

impl MargoState {
    /// Saved entries for the open windows (see [`WindowEntry::command`]).
    pub(crate) fn capture_session_windows(&mut self) -> Vec<WindowEntry> {
        let mut seen_pids = std::collections::HashSet::new();
        let mut out = Vec::new();
        for i in 0..self.clients.len() {
            let c = &self.clients[i];
            if c.app_id.is_empty() || c.is_in_scratchpad || c.is_initial_map_pending || c.is_killing
            {
                continue;
            }
            let pid = c.pid;
            if self.clients[i].launch_argv.is_none() {
                self.clients[i].launch_argv = Some(read_cmdline(pid));
            }
            let c = &self.clients[i];
            let first_of_process = pid == 0 || seen_pids.insert(pid);
            out.push(WindowEntry {
                app_id: c.app_id.clone(),
                monitor: self
                    .monitors
                    .get(c.monitor)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
                tags: c.tags & crate::TAGMASK,
                floating: c.is_floating,
                command: if first_of_process {
                    c.launch_argv.clone().unwrap_or_default()
                } else {
                    Vec::new()
                },
            });
        }
        out
    }

    /// Put open windows back and relaunch the missing apps of `snap`.
    pub(crate) fn restore_session_windows(&mut self, snap: &SessionSnapshot) -> RestoreReport {
        let mut report = RestoreReport::default();
        if snap.windows.is_empty() {
            return report;
        }
        let open: Vec<OpenWindow> = self
            .clients
            .iter()
            .filter(|c| !c.is_in_scratchpad && !c.is_initial_map_pending && !c.is_killing)
            .map(|c| OpenWindow {
                id: c.id,
                app_id: c.app_id.clone(),
                monitor: self
                    .monitors
                    .get(c.monitor)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
                tags: c.tags,
            })
            .collect();
        let plan = plan_restore(&snap.windows, &open);
        for (id, ei) in plan.moves {
            let Some(idx) = self.clients.iter().position(|c| c.id == id) else {
                continue;
            };
            let e = &snap.windows[ei];
            let tags = e.tags & crate::TAGMASK;
            if tags == 0 {
                continue;
            }
            let target = self
                .monitors
                .iter()
                .position(|m| m.name == e.monitor)
                .unwrap_or(self.clients[idx].monitor);
            self.move_client_to_monitor(idx, target, tags);
            report.moved += 1;
        }
        let now = Instant::now();
        for ei in plan.missing {
            let e = snap.windows[ei].clone();
            if !e.command.is_empty() {
                match crate::utils::spawn(&e.command) {
                    Ok(()) => report.launched += 1,
                    Err(err) => tracing::warn!(
                        target: "session",
                        app_id = %e.app_id,
                        error = ?err,
                        "relaunch failed"
                    ),
                }
            }
            // Without a command the app may still reopen the window itself
            // (a browser's second window): place it when it does.
            self.session.placements.push(Placement {
                entry: e,
                until: now + PLACEMENT_TTL,
            });
        }
        if report.moved > 0 {
            self.arrange_all();
            self.request_repaint();
            self.mark_state_dirty();
        }
        tracing::info!(
            target: "session",
            moved = report.moved,
            launched = report.launched,
            "restored session windows"
        );
        report
    }

    /// A relaunched window's saved place, consumed: `(monitor, tags)`.
    /// Called while a new window is first placed (after its window rules,
    /// which it overrides like the plugin's exec rule).
    pub(crate) fn session_take_placement(&mut self, app_id: &str) -> Option<(usize, u32)> {
        if app_id.is_empty() {
            return None;
        }
        let e = self.session.take_placement(app_id, Instant::now())?;
        let tags = e.tags & crate::TAGMASK;
        let mon = self.monitors.iter().position(|m| m.name == e.monitor)?;
        (tags != 0).then_some((mon, tags))
    }

    /// `session_auto_add`: queue a newly mapped app for the saved list.
    pub(crate) fn session_window_mapped(&mut self, idx: usize) {
        if !self.config.session_auto_add || idx >= self.clients.len() {
            return;
        }
        let c = &self.clients[idx];
        // Dialogs and other floating windows are not part of a layout.
        if c.app_id.is_empty() || c.is_floating || c.is_in_scratchpad {
            return;
        }
        let pid = c.pid;
        if c.launch_argv.is_none() {
            self.clients[idx].launch_argv = Some(read_cmdline(pid));
        }
        let c = &self.clients[idx];
        let entry = WindowEntry {
            app_id: c.app_id.clone(),
            monitor: self
                .monitors
                .get(c.monitor)
                .map(|m| m.name.clone())
                .unwrap_or_default(),
            tags: c.tags & crate::TAGMASK,
            floating: false,
            command: c.launch_argv.clone().unwrap_or_default(),
        };
        self.session.queue_auto_add(entry);
    }

    /// Write the `session_auto_add` queue into `session.json` (the minute's
    /// autosave tick). With no saved session yet, the first one is the
    /// current state. Off the hot path; logged, never fatal.
    pub(crate) fn flush_session_auto_add(&mut self) {
        if self.session.auto_add.is_empty() {
            return;
        }
        let queued = std::mem::take(&mut self.session.auto_add);
        let Ok(path) = super::session_path() else {
            return;
        };
        let mut snap = match super::load_from(&path) {
            Ok(snap) => snap,
            Err(_) if !path.exists() => SessionSnapshot::capture(self),
            Err(e) => {
                tracing::warn!(target: "session", "auto-add: session unreadable, left alone: {e:?}");
                return;
            }
        };
        if merge_auto_add(&mut snap, queued)
            && let Err(e) = super::save_to(&path, &snap)
        {
            tracing::warn!(target: "session", "auto-add: save failed: {e:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(app: &str, mon: &str, tags: u32, cmd: &[&str]) -> WindowEntry {
        WindowEntry {
            app_id: app.into(),
            monitor: mon.into(),
            tags,
            floating: false,
            command: cmd.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn open(id: u64, app: &str, mon: &str, tags: u32) -> OpenWindow {
        OpenWindow {
            id,
            app_id: app.into(),
            monitor: mon.into(),
            tags,
        }
    }

    #[test]
    fn windows_in_place_stay_others_move_missing_launch() {
        let entries = [
            entry("kitty", "DP-3", 1, &["kitty"]),
            entry("firefox", "DP-3", 2, &["firefox"]),
            entry("kitty", "eDP-1", 4, &["kitty"]),
            entry("discord", "eDP-1", 8, &["discord"]),
        ];
        let windows = [
            open(10, "kitty", "eDP-1", 1), // the second kitty, misplaced
            open(11, "kitty", "DP-3", 1),  // already where entry 0 wants it
            open(12, "firefox", "eDP-1", 1),
            open(13, "mpv", "DP-3", 1), // not saved: left alone
        ];
        let plan = plan_restore(&entries, &windows);
        assert_eq!(plan.moves, vec![(12, 1), (10, 2)]);
        assert_eq!(plan.missing, vec![3]);
    }

    #[test]
    fn more_entries_than_windows_launch_the_rest() {
        let entries = [
            entry("kitty", "DP-3", 1, &["kitty"]),
            entry("kitty", "DP-3", 2, &["kitty", "-e", "btop"]),
        ];
        let plan = plan_restore(&entries, &[open(1, "kitty", "DP-3", 2)]);
        assert!(plan.moves.is_empty(), "the open one is entry 1's");
        assert_eq!(plan.missing, vec![0]);
    }

    #[test]
    fn cmdline_parses_like_proc() {
        // `/proc/<pid>/cmdline` of `kitty -e btop` on this machine.
        assert_eq!(
            parse_cmdline(b"kitty\0-e\0btop\0"),
            vec!["kitty", "-e", "btop"]
        );
        assert!(parse_cmdline(b"").is_empty(), "kernel thread / gone");
        assert_eq!(parse_cmdline(b"/usr/bin/foot"), vec!["/usr/bin/foot"]);
    }

    #[test]
    fn placements_expire_and_are_consumed_once() {
        let mut rt = SessionRuntime::default();
        let now = Instant::now();
        rt.placements.push(Placement {
            entry: entry("discord", "eDP-1", 8, &[]),
            until: now + Duration::from_secs(5),
        });
        assert!(rt.take_placement("kitty", now).is_none());
        assert_eq!(rt.take_placement("discord", now).unwrap().tags, 8);
        assert!(rt.take_placement("discord", now).is_none(), "consumed");
        rt.placements.push(Placement {
            entry: entry("discord", "eDP-1", 8, &[]),
            until: now,
        });
        assert!(rt.take_placement("discord", now).is_none(), "expired");
    }

    #[test]
    fn auto_add_only_adds_new_apps() {
        let mut snap = SessionSnapshot {
            version: super::super::CURRENT_VERSION,
            captured_at: String::new(),
            monitors: vec![],
            scratchpads: vec![],
            windows: vec![entry("kitty", "DP-3", 1, &["kitty"])],
        };
        let queued = vec![
            entry("kitty", "eDP-1", 4, &["kitty"]),
            entry("obsidian", "DP-3", 2, &["obsidian"]),
        ];
        assert!(merge_auto_add(&mut snap, queued));
        let apps: Vec<(&str, u32)> = snap
            .windows
            .iter()
            .map(|w| (w.app_id.as_str(), w.tags))
            .collect();
        assert_eq!(
            apps,
            [("kitty", 1), ("obsidian", 2)],
            "kitty keeps its saved tag"
        );
        assert!(!merge_auto_add(
            &mut snap,
            vec![entry("obsidian", "DP-3", 4, &[])]
        ));
        let mut rt = SessionRuntime::default();
        rt.queue_auto_add(entry("a", "DP-3", 1, &[]));
        rt.queue_auto_add(entry("a", "DP-3", 2, &[]));
        assert_eq!(rt.auto_add.len(), 1);
    }

    #[test]
    fn a_version_1_file_without_windows_still_loads() {
        let old = r#"{"version":1,"captured_at":"x","monitors":[],"scratchpads":[]}"#;
        let snap: SessionSnapshot = serde_json::from_str(old).unwrap();
        assert!(snap.windows.is_empty());
        let new = r#"{"version":1,"captured_at":"x","monitors":[],
            "windows":[{"app_id":"kitty","monitor":"DP-3","tags":1}]}"#;
        let snap: SessionSnapshot = serde_json::from_str(new).unwrap();
        assert_eq!(snap.windows[0].command, Vec::<String>::new());
        assert!(!snap.windows[0].floating);
    }
}
