//! `player` palette — control MPRIS media players from the launcher.
//!
//! Wraps `playerctl` to enumerate players and produce one row
//! per player + four transport-control rows (play/pause/next/prev
//! on the focused player). Activating a player row makes it the
//! "active" target via `playerctl --player=<name>`.
//!
//! Subprocess-based rather than D-Bus-direct because:
//!   1. mshell-launcher must stay light on async / D-Bus deps
//!   2. `playerctl` already handles every edge case (firefox
//!      claiming MPRIS without metadata, multiple instances, etc.)
//!   3. The launcher use case is one-shot — a 50 ms subprocess
//!      fork is unmeasurable next to the keystroke that triggered
//!      it.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

const SNAPSHOT_TTL: Duration = Duration::from_secs(2);

pub struct PlayerctlProvider {
    cache: RefCell<Option<CachedSnapshot<PlayerSnapshot>>>,
}

impl PlayerctlProvider {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(None),
        }
    }

    fn cached_snapshot(&self) -> PlayerSnapshot {
        let now = Instant::now();
        if let Some(cached) = self.cache.borrow().as_ref()
            && now.duration_since(cached.captured_at) < SNAPSHOT_TTL
        {
            return cached.value.clone();
        }

        let value = snapshot();
        *self.cache.borrow_mut() = Some(CachedSnapshot {
            captured_at: now,
            value: value.clone(),
        });
        value
    }
}

impl Default for PlayerctlProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of currently-registered MPRIS players.
#[derive(Clone)]
struct PlayerSnapshot {
    players: Vec<PlayerInfo>,
}

#[derive(Clone)]
struct PlayerInfo {
    /// `playerctl --list-all` output entry.
    name: String,
    /// Cached metadata so Actions typing does not fork one
    /// `playerctl metadata` per player on every recompute.
    track: Option<String>,
}

struct CachedSnapshot<T> {
    captured_at: Instant,
    value: T,
}

fn snapshot() -> PlayerSnapshot {
    let players = Command::new("playerctl")
        .arg("--list-all")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|name| {
            let track = current_track(&name);
            PlayerInfo { name, track }
        })
        .collect();
    PlayerSnapshot { players }
}

fn current_track(player: &str) -> Option<String> {
    let out = Command::new("playerctl")
        .args([
            "--player",
            player,
            "metadata",
            "--format",
            "{{artist}} — {{title}}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() || text == "—" {
        None
    } else {
        Some(text)
    }
}

fn spawn_player_cmd(args: &[&str]) {
    if let Err(err) = Command::new("playerctl").args(args).spawn() {
        tracing::warn!(?err, ?args, "playerctl spawn failed");
    }
}

impl Provider for PlayerctlProvider {
    fn name(&self) -> &str {
        "Player"
    }

    fn category(&self) -> &str {
        "System"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "player" || q.starts_with("player ") || q == "play" || q.starts_with("play ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "player:palette".into(),
            name: "player".into(),
            description: "Control MPRIS players — play / pause / next / prev".into(),
            icon: "media-playback-start-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Player".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "player" || q.starts_with("player ") || q == "play" || q.starts_with("play ")) {
            return Vec::new();
        }

        let snap = self.cached_snapshot();
        if snap.players.is_empty() {
            return vec![LauncherItem {
                id: "player:none".into(),
                name: "No MPRIS players running".into(),
                description: "Open a player (Spotify, Firefox, mpv, …) to see it here".into(),
                icon: "media-playback-stop-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "Player".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }

        let mut results: Vec<LauncherItem> = Vec::new();

        // Transport controls — always at the top so single-action
        // workflows (just pause whatever's playing) don't need
        // to scroll.
        for (idx, (label, sub, icon)) in [
            (
                "Play / Pause",
                "play-pause",
                "media-playback-start-symbolic",
            ),
            ("Next track", "next", "media-skip-forward-symbolic"),
            ("Previous track", "previous", "media-skip-backward-symbolic"),
            ("Stop", "stop", "media-playback-stop-symbolic"),
        ]
        .iter()
        .enumerate()
        {
            let cmd = sub.to_string();
            let label_owned = label.to_string();
            results.push(LauncherItem {
                id: format!("player:transport:{sub}"),
                name: (*label).into(),
                description: "Acts on the currently-focused player".into(),
                icon: (*icon).into(),
                icon_is_path: false,
                score: 200.0 - idx as f64,
                provider_name: "Player".into(),
                usage_key: Some(format!("player:transport:{sub}")),
                on_activate: Rc::new(move || {
                    spawn_player_cmd(&[&cmd]);
                    toast("Player", label_owned.clone());
                }),
            });
        }

        // One row per player — picks the player + plays it.
        for (idx, player) in snap.players.iter().enumerate() {
            let track = player.track.as_deref().unwrap_or("(no track)");
            let player_clone = player.name.clone();
            let player_label = player.name.clone();
            results.push(LauncherItem {
                id: format!("player:select:{}", player.name),
                name: format!("{} — {track}", player.name),
                description: "Make this the focused player + play".into(),
                icon: "audio-x-generic-symbolic".into(),
                icon_is_path: false,
                score: 180.0 - idx as f64,
                provider_name: "Player".into(),
                usage_key: Some(format!("player:select:{}", player.name)),
                on_activate: Rc::new(move || {
                    spawn_player_cmd(&["--player", &player_clone, "play"]);
                    toast("Now playing", player_label.clone());
                }),
            });
        }

        results
    }

    /// System tab — surface transport controls + per-player rows
    /// without the `player` prefix. The player provider's search()
    /// doesn't itself filter, but the runtime's name-substring
    /// post-filter (active when typing inside a category) handles
    /// the narrowing.
    fn browse(&self, _filter: &str) -> Vec<LauncherItem> {
        self.search("player")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = PlayerctlProvider::new();
        assert!(p.search("firefox").is_empty());
    }

    #[test]
    fn handles_player_prefix() {
        let p = PlayerctlProvider::new();
        assert!(p.handles_command("player"));
        assert!(p.handles_command("player something"));
        assert!(p.handles_command("play"));
        assert!(!p.handles_command("playerctl"));
    }
}
