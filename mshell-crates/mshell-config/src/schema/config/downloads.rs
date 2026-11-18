//! `downloads.*` — the media downloader (yt-dlp) behind the Downloads pill,
//! its menu and the launcher's `dl <url>` / YouTube "Download" actions.

use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct DownloadsConfig {
    /// Folder for video downloads. Empty: the XDG videos folder
    /// (`xdg-user-dir VIDEOS`, else `~/Videos`). `~` is expanded.
    pub video_dir: String,
    /// Folder for audio downloads. Empty: the XDG music folder.
    pub audio_dir: String,
    /// Audio format of an audio download: `mp3` (default), `opus`, `m4a`
    /// or `best` (whatever the site has, no re-encode).
    pub audio_format: String,
    /// Highest video height preferred, e.g. `1080`; `best` takes the best.
    pub video_quality: String,
    /// Download a whole playlist when the link names one (off: only the
    /// video the link points at).
    pub playlist: bool,
    /// Extra yt-dlp arguments, split on spaces, added last (they win).
    pub extra_args: String,
    /// Notify when a download finishes or fails.
    pub notify: bool,
}

impl Default for DownloadsConfig {
    fn default() -> Self {
        Self {
            video_dir: String::new(),
            audio_dir: String::new(),
            audio_format: "mp3".to_string(),
            video_quality: "best".to_string(),
            playlist: false,
            extra_args: String::new(),
            notify: true,
        }
    }
}

/// The `downloads.audio_format` values offered in Settings.
pub const AUDIO_FORMATS: &[&str] = &["mp3", "opus", "m4a", "best"];
/// The `downloads.video_quality` values offered in Settings.
pub const VIDEO_QUALITIES: &[&str] = &["best", "2160", "1440", "1080", "720", "480"];
