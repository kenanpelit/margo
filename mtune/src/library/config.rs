// SPDX-License-Identifier: GPL-3.0-or-later
//! `~/.config/margo/mtune.toml` — mtune's own config file (not `margo-config`,
//! not `mshell-config`). Hand-editable and GUI-writable; re-read on change.

use log::warn;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "m4b", "aac", "wav", "wma", "aiff", "ape", "wv",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OnStart {
    /// Restore the last track + position (from GSettings).
    #[default]
    Resume,
    /// Select the top of the library, but do not auto-play.
    Library,
    /// Do nothing; wait for the user.
    Nothing,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LibrarySection {
    /// One or more persistent root folders.
    pub roots: Vec<PathBuf>,
    /// Rescan the roots at launch (vs. trust the cached index).
    pub scan_on_start: bool,
    /// Watch the roots for added/removed files while running.
    pub watch: bool,
    /// Recurse into subdirectories.
    pub recursive: bool,
    /// Filename extensions considered playable (case-insensitive).
    pub extensions: Vec<String>,
}

impl Default for LibrarySection {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            scan_on_start: true,
            watch: true,
            recursive: true,
            extensions: DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl LibrarySection {
    /// The configured roots, `~`-expanded, with non-existent ones dropped.
    pub fn resolved_roots(&self) -> Vec<PathBuf> {
        self.roots
            .iter()
            .map(|p| expand_tilde(p))
            .filter(|p| {
                let ok = p.is_dir();
                if !ok {
                    warn!("library root does not exist: {}", p.display());
                }
                ok
            })
            .collect()
    }

    /// Whether a path's extension is in the playable set.
    pub fn is_playable(&self, path: &Path) -> bool {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)),
            None => false,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct PlaybackSection {
    pub on_start: OnStart,
    /// Times to repeat each track under `RepeatMode::RepeatEach` before
    /// advancing. Settings/CLI via `mshellctl mtune repeat-count`.
    pub repeat_count: u32,
}

impl Default for PlaybackSection {
    fn default() -> Self {
        Self {
            on_start: OnStart::default(),
            repeat_count: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct BehaviourSection {
    /// Keep playing (windowless) after the window is closed.
    pub close_to_tray: bool,
    /// Refuse a second instance; raise the running one instead.
    pub single_instance: bool,
    /// Start with no window — just the tray icon. Overridden per-launch
    /// by `mtune --hidden`. Click the tray (or `mshellctl mtune
    /// toggle-window`) to show the player.
    pub start_hidden: bool,
}

impl Default for BehaviourSection {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            single_instance: true,
            start_hidden: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MtuneConfig {
    pub library: LibrarySection,
    pub playback: PlaybackSection,
    pub behaviour: BehaviourSection,
}

/// Expand a leading `~/` to `$HOME`. Everything else is returned unchanged.
pub fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    p.to_path_buf()
}

impl MtuneConfig {
    /// `$XDG_CONFIG_HOME/margo/mtune.toml`, falling back to `~/.config/…`.
    pub fn path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
            });
        base.join("margo").join("mtune.toml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(p: &Path) -> Self {
        match std::fs::read_to_string(p) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                warn!("mtune.toml parse error ({e}); using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, p: &Path) -> anyhow::Result<()> {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self)?;
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = MtuneConfig::default();
        assert!(c.library.scan_on_start);
        assert!(c.library.watch);
        assert!(c.library.recursive);
        assert_eq!(c.playback.on_start, OnStart::Resume);
        assert_eq!(c.playback.repeat_count, 3);
        assert!(!c.library.extensions.is_empty());
        assert!(c.behaviour.close_to_tray);
        assert!(!c.behaviour.start_hidden);
    }

    #[test]
    fn start_hidden_roundtrips() {
        let mut c = MtuneConfig::default();
        c.behaviour.start_hidden = true;
        let back: MtuneConfig = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert!(back.behaviour.start_hidden);
    }

    #[test]
    fn repeat_count_roundtrips() {
        let mut c = MtuneConfig::default();
        c.playback.repeat_count = 5;
        let back: MtuneConfig = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.playback.repeat_count, 5);
    }

    #[test]
    fn partial_toml_fills_from_defaults() {
        let toml = r#"
            [library]
            roots = ["/music"]
            watch = false
        "#;
        let c: MtuneConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.library.roots, vec![PathBuf::from("/music")]);
        assert!(!c.library.watch);
        assert!(c.library.scan_on_start); // from default
        assert_eq!(c.playback.on_start, OnStart::Resume); // whole section defaulted
    }

    #[test]
    fn roundtrip_preserves_values() {
        let mut c = MtuneConfig::default();
        c.library.roots = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        c.playback.on_start = OnStart::Library;
        let s = toml::to_string(&c).unwrap();
        let back: MtuneConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.library.roots, c.library.roots);
        assert_eq!(back.playback.on_start, OnStart::Library);
    }

    #[test]
    fn tilde_expansion() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_tilde(Path::new("~/Music")),
            PathBuf::from(format!("{home}/Music"))
        );
        assert_eq!(
            expand_tilde(Path::new("/abs/path")),
            PathBuf::from("/abs/path")
        );
    }

    #[test]
    fn is_playable_matches_extension_case_insensitively() {
        let lib = LibrarySection::default();
        assert!(lib.is_playable(Path::new("/x/Song.MP3")));
        assert!(lib.is_playable(Path::new("/x/song.flac")));
        assert!(!lib.is_playable(Path::new("/x/cover.jpg")));
        assert!(!lib.is_playable(Path::new("/x/noext")));
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.toml");
        assert_eq!(
            MtuneConfig::load_from(&p).library.watch,
            MtuneConfig::default().library.watch
        );
    }

    #[test]
    fn save_then_load_from_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mtune.toml");
        let mut c = MtuneConfig::default();
        c.library.roots = vec![PathBuf::from("/music/lib")];
        c.save_to(&p).unwrap();
        let back = MtuneConfig::load_from(&p);
        assert_eq!(back.library.roots, vec![PathBuf::from("/music/lib")]);
    }
}
