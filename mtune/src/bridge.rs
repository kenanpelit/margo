// SPDX-License-Identifier: GPL-3.0-or-later
//! The `Send` bridge between the GTK main context (which owns the player /
//! window) and the off-thread D-Bus interface + tray.
//!
//! * [`Snapshot`] — a plain-data mirror of playback + library state. The
//!   main thread writes it on every `PlayerState` change; the tray and the
//!   `org.margo.Tune` interface read it.
//! * [`AppCommand`] — actions the tray / interface send back; a
//!   main-context receiver in `Application` applies each to the player.

use crate::audio::RepeatMode;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub has_song: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Absolute path to the current track's cached cover, or empty.
    pub cover_art: String,
    pub position_secs: u64,
    pub duration_secs: u64,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub queue_len: u32,
    pub current_index: i64,
    pub library_roots: Vec<String>,
    pub scanning: bool,
    pub scan_done: u32,
    pub scan_total: u32,
}

pub type SharedSnapshot = Arc<Mutex<Snapshot>>;

pub fn new_shared() -> SharedSnapshot {
    Arc::new(Mutex::new(Snapshot::default()))
}

/// An action from the tray or the `org.margo.Tune` interface, applied on the
/// main context by `Application`.
#[derive(Debug, Clone)]
pub enum AppCommand {
    PlayPause,
    Next,
    Previous,
    Stop,
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    SeekAbs(u64),
    SetVolume(f64),
    /// Show the window if hidden, hide it (to the tray) if visible.
    ToggleWindow,
    Raise,
    Quit,
    PlayFolder(PathBuf),
    SetLibraryRoots(Vec<PathBuf>),
    RescanLibrary,
    PlayIndex(u32),
    RemoveIndex(u32),
}

pub type CommandSender = async_channel::Sender<AppCommand>;
pub type CommandReceiver = async_channel::Receiver<AppCommand>;
