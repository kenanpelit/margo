// SPDX-License-Identifier: GPL-3.0-or-later
//! The supplementary `org.margo.Tune` D-Bus interface — the library / queue
//! surface that standard MPRIS can't express. Consumed by the margo shell's
//! dedicated bar pill + menu; MPRIS (via `mpris-server`) stays the interop
//! path for everything else.

use crate::audio::RepeatMode;
use crate::bridge::{AppCommand, CommandSender, SharedSnapshot};
use std::path::PathBuf;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

pub const IFACE_NAME: &str = "org.margo.Tune";
pub const OBJECT_PATH: &str = "/org/margo/Tune";

struct TuneService {
    snap: SharedSnapshot,
    tx: CommandSender,
}

impl TuneService {
    fn send(&self, cmd: AppCommand) {
        let _ = self.tx.send_blocking(cmd);
    }
}

#[interface(name = "org.margo.Tune")]
impl TuneService {
    // ── Now-playing (so a consumer needs only this one interface) ────

    #[zbus(property)]
    async fn playing(&self) -> bool {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .playing
    }

    #[zbus(property)]
    async fn has_song(&self) -> bool {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .has_song
    }

    #[zbus(property)]
    async fn title(&self) -> String {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .title
            .clone()
    }

    #[zbus(property)]
    async fn artist(&self) -> String {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .artist
            .clone()
    }

    #[zbus(property)]
    async fn album(&self) -> String {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .album
            .clone()
    }

    /// The current track's embedded lyrics text, or `""` when the file's
    /// tags don't carry any.
    #[zbus(property)]
    async fn embedded_lyrics(&self) -> String {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .lyrics
            .clone()
    }

    /// Absolute path to the current track's cached cover art, or `""`.
    #[zbus(property)]
    async fn cover_art(&self) -> String {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cover_art
            .clone()
    }

    #[zbus(property)]
    async fn position(&self) -> u64 {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .position_secs
    }

    #[zbus(property)]
    async fn duration(&self) -> u64 {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .duration_secs
    }

    // ── Library / queue ─────────────────────────────────────────────

    #[zbus(property)]
    async fn library_roots(&self) -> Vec<String> {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .library_roots
            .clone()
    }

    #[zbus(property)]
    async fn scanning(&self) -> bool {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .scanning
    }

    /// `(done, total)` while a scan is running, `(0, 0)` otherwise.
    #[zbus(property)]
    async fn scan_progress(&self) -> (u32, u32) {
        let s = self
            .snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (s.scan_done, s.scan_total)
    }

    #[zbus(property)]
    async fn queue_length(&self) -> u32 {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .queue_len
    }

    #[zbus(property)]
    async fn current_index(&self) -> i64 {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current_index
    }

    /// (title, artist, duration_secs) for every song in the queue, in
    /// queue order.
    #[zbus(property)]
    async fn queue_entries(&self) -> Vec<(String, String, u64)> {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .queue_entries
            .clone()
    }

    #[zbus(property)]
    async fn repeat_mode(&self) -> String {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .repeat
            .to_string()
    }

    #[zbus(property)]
    async fn shuffle(&self) -> bool {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shuffle
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .volume
    }

    /// Sticky playback rate (0.5..=2.0; 1.0 = normal speed).
    #[zbus(property)]
    async fn rate(&self) -> f64 {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .rate
    }

    /// Names of the saved playlists.
    #[zbus(property)]
    async fn playlists(&self) -> Vec<String> {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .playlists
            .clone()
    }

    // ── Methods (actions, applied on the GTK main context) ───────────

    async fn play_pause(&self) {
        self.send(AppCommand::PlayPause);
    }

    async fn next(&self) {
        self.send(AppCommand::Next);
    }

    async fn previous(&self) {
        self.send(AppCommand::Previous);
    }

    async fn stop(&self) {
        self.send(AppCommand::Stop);
    }

    async fn seek(&self, position_secs: u64) {
        self.send(AppCommand::SeekAbs(position_secs));
    }

    async fn set_volume(&self, volume: f64) {
        self.send(AppCommand::SetVolume(volume));
    }

    async fn set_rate(&self, rate: f64) {
        self.send(AppCommand::SetRate(rate));
    }

    /// Load a saved playlist by name.
    async fn load_playlist(&self, name: String) {
        self.send(AppCommand::LoadPlaylist(name));
    }

    /// Open a `.m3u` / `.m3u8` / `.pls` file.
    async fn open_playlist(&self, path: String) {
        self.send(AppCommand::OpenPlaylist(PathBuf::from(path)));
    }

    /// Save the current queue to the library under this name.
    async fn save_playlist(&self, name: String) {
        self.send(AppCommand::SavePlaylist(name));
    }

    async fn set_library_roots(&self, roots: Vec<String>) {
        self.send(AppCommand::SetLibraryRoots(
            roots.into_iter().map(PathBuf::from).collect(),
        ));
    }

    async fn play_folder(&self, path: String) {
        self.send(AppCommand::PlayFolder(PathBuf::from(path)));
    }

    async fn rescan_library(&self) {
        self.send(AppCommand::RescanLibrary);
    }

    async fn play_index(&self, index: u32) {
        self.send(AppCommand::PlayIndex(index));
    }

    async fn remove_index(&self, index: u32) {
        self.send(AppCommand::RemoveIndex(index));
    }

    async fn set_repeat_mode(&self, mode: String) {
        let mode = match mode.as_str() {
            "repeat-all" => RepeatMode::RepeatAll,
            "repeat-one" => RepeatMode::RepeatOne,
            _ => RepeatMode::Consecutive,
        };
        self.send(AppCommand::SetRepeat(mode));
    }

    async fn set_shuffle(&self, shuffle: bool) {
        self.send(AppCommand::SetShuffle(shuffle));
    }

    async fn raise(&self) {
        self.send(AppCommand::Raise);
    }

    /// Show the window if hidden, hide it if visible (creating it if it
    /// was never opened — e.g. after `mtune --hidden`). Bind to a key
    /// for a quick peek at the player.
    async fn toggle_window(&self) {
        self.send(AppCommand::ToggleWindow);
    }

    async fn quit(&self) {
        self.send(AppCommand::Quit);
    }

    // ── Signal ──────────────────────────────────────────────────────

    /// Something a consumer's view depends on has changed (coalesced by the
    /// caller).
    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Register the supplementary `org.margo.Tune` interface on `conn` —
/// mtune's MPRIS server connection. It is served there rather than on a
/// connection of its own because the GApplication already owns the bare
/// `org.margo.Tune` bus name, so a fresh zbus connection could never
/// claim it; the shell reaches this interface via the MPRIS well-known
/// name (`org.mpris.MediaPlayer2.org.margo.Tune`).
///
/// Returns the connection (to keep alive and emit `Changed` on), or
/// `None` if registration failed — mtune still runs; only the shell's
/// dedicated pill loses its live feed.
pub async fn serve_on(
    conn: &Connection,
    snap: SharedSnapshot,
    tx: CommandSender,
) -> Option<Connection> {
    match conn
        .object_server()
        .at(OBJECT_PATH, TuneService { snap, tx })
        .await
    {
        Ok(_) => Some(conn.clone()),
        Err(e) => {
            log::warn!("mtune: could not serve {IFACE_NAME} at {OBJECT_PATH}: {e}");
            None
        }
    }
}

/// Emit the coalesced `Changed` signal. Call after updating the snapshot.
pub async fn emit_changed(conn: &Connection) {
    if let Ok(emitter) = SignalEmitter::new(conn, OBJECT_PATH) {
        let _ = TuneService::changed(&emitter).await;
    }
}
