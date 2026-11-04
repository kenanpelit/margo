//! Media players (MPRIS + native MPD): the `media list/status` snapshots.

use super::*;

// ── Media players (MPRIS + native MPD, picked/driven via `media_picker`) ────
use crate::media_picker::playback_label;

#[derive(serde::Serialize)]
pub(super) struct PlayerSnapshot {
    pub(super) identity: String,
    pub(super) state: String,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) active: bool,
}

/// Every known player (every MPRIS player + native MPD when connected),
/// with `active` marking whichever `media_picker::pick_active("")` would
/// act on next — the same one `mshellctl media toggle`/`next`/`prev` (no
/// explicit target) drives.
pub(super) fn media_snapshot() -> Vec<PlayerSnapshot> {
    let svc = media_service();
    let active_id = media_picker::pick_active("").map(|p| p.id());

    let mut snaps: Vec<PlayerSnapshot> = svc
        .players()
        .into_iter()
        .map(|p| PlayerSnapshot {
            active: active_id.as_deref() == Some(&format!("mpris:{}", p.id.bus_name())),
            identity: p.identity.get(),
            state: playback_label(p.playback_state.get()).to_string(),
            title: p.metadata.title.get(),
            artist: p.metadata.artist.get(),
        })
        .collect();

    let mpd = mpd_service().player.clone();
    if mpd.connected.get() {
        snaps.push(PlayerSnapshot {
            active: active_id.as_deref() == Some("mpd"),
            identity: "MPD".to_string(),
            state: playback_label(mpd.playback_state.get()).to_string(),
            title: mpd.title.get(),
            artist: mpd.artist.get(),
        });
    }
    snaps
}

/// `mshellctl media list` body.
pub(super) fn render_media_list(as_json: bool) -> String {
    let snap = media_snapshot();
    if as_json {
        return serde_json::to_string_pretty(&snap).unwrap_or_else(|_| "[]".into());
    }
    if snap.is_empty() {
        return "No media players".to_string();
    }
    snap.iter()
        .map(|p| {
            let icon = if p.state == "Playing" { "▶" } else { "⏸" };
            let track = match (p.title.as_str(), p.artist.as_str()) {
                ("", "") => String::new(),
                (t, "") => format!("  — {t}"),
                (t, a) => format!("  — {t} · {a}"),
            };
            let tag = if p.active { "  [active]" } else { "" };
            format!("{icon} {} [{}]{track}{tag}", p.identity, p.state)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `mshellctl media status` body — the active player.
pub(super) fn render_media_status(as_json: bool) -> String {
    let active = media_snapshot().into_iter().find(|p| p.active);
    if as_json {
        return serde_json::to_string_pretty(&active).unwrap_or_else(|_| "null".into());
    }
    match active {
        None => "No active media player".to_string(),
        Some(p) => {
            let track = match (p.title.as_str(), p.artist.as_str()) {
                ("", "") => String::new(),
                (t, "") => format!(" — {t}"),
                (t, a) => format!(" — {t} · {a}"),
            };
            format!("{} [{}]{track}", p.identity, p.state)
        }
    }
}
