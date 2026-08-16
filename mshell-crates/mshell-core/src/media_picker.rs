//! Backend-aware media player picker for `mshellctl media …`.
//!
//! Chooses the best MPRIS or native-MPD player to act on for a transport
//! command, using the same weighted scoring `mplay media` uses (ported from
//! its `player::candidate_score`, itself a port of `osc-media.sh`), then
//! drives it and fires a rich desktop notification. This is deliberately
//! more deliberate than the bar pill's `display_player()` (a simple "first
//! one playing" tie-break for what to *show*) — this picks what to *act
//! on*, which is what `mplay media`'s scoring is actually for. Native MPD
//! support (this crate's `mpd` module) means this now covers everything
//! `mplay media` does without shelling out to `playerctl`/`mpc`.

use mshell_services::media_service;
use mshell_services::mpd::{MpdPlayer, mpd_service};
use std::cell::RefCell;
use std::sync::Arc;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;

/// Either backend's player handle — mirrors `PillSource` in the bar pill
/// widget, but this module scores/picks independently (see module doc):
/// the pill answers "what's worth showing", this answers "what should a
/// transport command act on".
#[derive(Clone)]
pub(crate) enum PlayerHandle {
    Mpris(Arc<Player>),
    Mpd(Arc<MpdPlayer>),
}

/// Browser-hosted MPRIS players (web media) score lower than real apps —
/// same alias list `resolve_explicit`'s `browser` target uses.
const BROWSER_TOKENS: &[&str] = &[
    "firefox",
    "chrome",
    "chromium",
    "brave",
    "edge",
    "vivaldi",
    "opera",
    "webcord",
    "zen",
    "librewolf",
    "waterfox",
    "floorp",
    "helium",
    "thorium",
    "ungoogled",
    "palemoon",
    "midori",
    "epiphany",
    "falkon",
    "qutebrowser",
];

fn is_browser(haystack: &str) -> bool {
    BROWSER_TOKENS.iter().any(|b| haystack.contains(b))
}

pub(crate) fn playback_label(s: PlaybackState) -> &'static str {
    match s {
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Stopped => "Stopped",
    }
}

impl PlayerHandle {
    fn playback_state(&self) -> PlaybackState {
        match self {
            Self::Mpris(p) => p.playback_state.get(),
            Self::Mpd(p) => p.playback_state.get(),
        }
    }

    /// Clean display name — MPRIS identity, or a fixed literal for MPD
    /// (there's only ever one).
    fn identity(&self) -> String {
        match self {
            Self::Mpris(p) => p.identity.get(),
            Self::Mpd(_) => "MPD".to_string(),
        }
    }

    /// Broader lowercase match string for fuzzy target matching + browser
    /// detection. The bus name is the robust signal for MPRIS: a
    /// Chromium/Firefox fork inherits the engine's MPRIS service name even
    /// when it rebrands its Identity — e.g. Helium reports identity
    /// "Helium" but registers as `org.mpris.MediaPlayer2.chromium.instance…`.
    fn haystack(&self) -> String {
        match self {
            Self::Mpris(p) => {
                let bus = p.id.bus_name();
                let bus = bus.strip_prefix("org.mpris.MediaPlayer2.").unwrap_or(bus);
                let desktop = p.desktop_entry.get().unwrap_or_default();
                format!("{} {} {}", p.identity.get(), bus, desktop).to_lowercase()
            }
            Self::Mpd(_) => "mpd music player daemon".to_string(),
        }
    }

    /// Stable id for last-player tie-break memory + the snapshot's
    /// "active" flag.
    pub(crate) fn id(&self) -> String {
        match self {
            Self::Mpris(p) => format!("mpris:{}", p.id.bus_name()),
            Self::Mpd(_) => "mpd".to_string(),
        }
    }

    async fn play_pause(&self) -> bool {
        match self {
            Self::Mpris(p) => p.play_pause().await.is_ok(),
            Self::Mpd(p) => p.play_pause().await.is_ok(),
        }
    }
    async fn next(&self) -> bool {
        match self {
            Self::Mpris(p) => p.next().await.is_ok(),
            Self::Mpd(p) => p.next().await.is_ok(),
        }
    }
    async fn previous(&self) -> bool {
        match self {
            Self::Mpris(p) => p.previous().await.is_ok(),
            Self::Mpd(p) => p.previous().await.is_ok(),
        }
    }
}

thread_local! {
    // GTK-main-thread only (every caller runs inside `glib::spawn_future_local`
    // or a zbus method dispatched on the same context) — no locking needed.
    static LAST_PLAYER: RefCell<String> = const { RefCell::new(String::new()) };
}

fn last_player() -> String {
    LAST_PLAYER.with(|c| c.borrow().clone())
}

fn set_last_player(id: &str) {
    LAST_PLAYER.with(|c| *c.borrow_mut() = id.to_string());
}

/// Rank a candidate player for auto-detect; higher wins. Ported from
/// `mplay`'s `player::candidate_score`: status base, backend/app-name
/// bonuses, and a last-used-player bonus that breaks ties toward whatever
/// you were just controlling.
fn candidate_score(handle: &PlayerHandle, last_id: &str) -> i32 {
    let hay = handle.haystack();
    let browser = is_browser(&hay);

    let mut score = match handle.playback_state() {
        PlaybackState::Playing => 300,
        PlaybackState::Paused => 180,
        PlaybackState::Stopped => 40,
    };

    match handle {
        PlayerHandle::Mpd(_) => score += 40,
        PlayerHandle::Mpris(_) => score += if browser { 8 } else { 35 },
    }

    if hay.contains("spotify") {
        score += 35;
    } else if hay.contains("vlc") {
        score += 28;
    } else if browser {
        score += 10;
    }

    if handle.id() == last_id {
        score += 90;
    }

    if handle.playback_state() == PlaybackState::Playing {
        score += 18;
    }

    score
}

/// Every MPRIS player matching `filter` (applied to the identity/bus/desktop
/// haystack), scored, best first.
fn best_mpris(filter: impl Fn(&str) -> bool, last_id: &str) -> Option<PlayerHandle> {
    media_service()
        .players()
        .into_iter()
        .map(PlayerHandle::Mpris)
        .filter(|h| filter(&h.haystack()))
        .max_by_key(|h| candidate_score(h, last_id))
}

/// Resolve an explicit target fragment: `mpd`/`mpc` route straight to the
/// native MPD backend (only when actually connected), `browser` uses the
/// alias list, anything else is a case-insensitive substring match against
/// every MPRIS player's identity/bus-name/desktop-entry.
fn resolve_explicit(target: &str, last_id: &str) -> Option<PlayerHandle> {
    match target {
        "mpd" | "mpc" => {
            let mpd = mpd_service().player.clone();
            mpd.connected.get().then(|| PlayerHandle::Mpd(mpd))
        }
        "browser" => best_mpris(is_browser, last_id),
        other => {
            let needle = other.to_string();
            best_mpris(move |hay| hay.contains(&needle), last_id)
        }
    }
}

/// Pick the best player to act on: an explicit `target` fragment (or the
/// `mpd`/`mpc`/`browser` aliases) when given, else the highest-scoring
/// candidate across every connected backend (every MPRIS player + native
/// MPD when connected).
pub(crate) fn pick_active(target: &str) -> Option<PlayerHandle> {
    let last_id = last_player();
    let t = target.trim().to_lowercase();
    if !t.is_empty() {
        return resolve_explicit(&t, &last_id);
    }

    let mpd = mpd_service().player.clone();
    let mpd_candidate = mpd.connected.get().then(|| PlayerHandle::Mpd(mpd));

    media_service()
        .players()
        .into_iter()
        .map(PlayerHandle::Mpris)
        .chain(mpd_candidate)
        .max_by_key(|h| candidate_score(h, &last_id))
}

/// Drive `handle`'s transport, remember it as the last-used player, and
/// fire a rich desktop notification once the new state has settled.
pub(crate) async fn toggle(handle: PlayerHandle) -> bool {
    let ok = handle.play_pause().await;
    if ok {
        set_last_player(&handle.id());
        notify(handle).await;
    }
    ok
}
pub(crate) async fn next(handle: PlayerHandle) -> bool {
    let ok = handle.next().await;
    if ok {
        set_last_player(&handle.id());
        notify(handle).await;
    }
    ok
}
pub(crate) async fn previous(handle: PlayerHandle) -> bool {
    let ok = handle.previous().await;
    if ok {
        set_last_player(&handle.id());
        notify(handle).await;
    }
    ok
}

/// Fire-and-forget desktop notification (replaces the previous one via the
/// synchronous hint so rapid switches don't stack), mirroring osc-soundctl /
/// `mplay media`'s own notify. Toast the player + current track after a
/// media action. Spawned with a short settle delay because MPRIS/MPD push
/// the new track / playback state asynchronously after the command
/// returns — reading immediately would name the *previous* track.
async fn notify(handle: PlayerHandle) {
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;

    let name = handle.identity();
    let state = handle.playback_state();
    let (title, artist, art_path) = match &handle {
        PlayerHandle::Mpris(p) => {
            let art = p
                .metadata
                .cover_art
                .get()
                .or_else(|| p.metadata.art_url.get());
            (p.metadata.title.get(), p.metadata.artist.get(), art)
        }
        PlayerHandle::Mpd(p) => (p.title.get(), p.artist.get(), p.cover_art.get()),
    };

    let glyph = match state {
        PlaybackState::Playing => "▶",
        PlaybackState::Paused => "⏸",
        PlaybackState::Stopped => "⏹",
    };
    let body = match (title.trim(), artist.trim()) {
        ("", "") => format!("{glyph} {}", playback_label(state)),
        (t, "") => format!("{glyph} {t}"),
        (t, a) => format!("{glyph} {t} · {a}"),
    };
    let icon = art_path
        .map(|p| p.trim_start_matches("file://").to_string())
        .filter(|p| std::path::Path::new(p).is_file())
        .unwrap_or_else(|| "multimedia-player-symbolic".to_string());
    let _ = tokio::process::Command::new("notify-send")
        .args([
            "-a",
            "mshell",
            "-i",
            &icon,
            "-h",
            "string:x-canonical-private-synchronous:mshell-media",
            &name,
            &body,
        ])
        .status()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_tokens_catch_common_forks() {
        assert!(is_browser(
            "helium org.mpris.mediaplayer2.chromium.instance1"
        ));
        assert!(is_browser("firefox org.mpris.mediaplayer2.firefox"));
        assert!(!is_browser("spotify org.mpris.mediaplayer2.spotify"));
    }
}
