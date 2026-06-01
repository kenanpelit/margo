//! `mplay media` — smart multi-player media controller. Routes a
//! transport command to the best active player (MPRIS via playerctl, MPD
//! via mpc, mpv via its IPC socket), with scoring + last-player memory +
//! Spotify autostart, then shows a rich notification. Port of osc-media.sh.

pub mod player;
pub mod status;

mod mpd;
mod mpris;
mod mpv;
mod notify;
mod spotify;

use anyhow::{Result, anyhow};
use player::{Kind, candidate_score, is_browser};
use status::{Command, Status};
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

/// Now-playing metadata for the notification.
#[derive(Default)]
pub struct Meta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: String,
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })))
        .join("mplay")
}

fn last_player_file() -> PathBuf {
    runtime_dir().join("last-player")
}

fn read_last_player() -> String {
    std::fs::read_to_string(last_player_file())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn write_last_player(id: &str) {
    let dir = runtime_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("last-player"), id);
    }
}

fn basename(s: &str) -> String {
    s.rsplit('/')
        .next()
        .unwrap_or(s)
        .split('?')
        .next()
        .unwrap_or(s)
        .to_string()
}

fn status_of(kind: &Kind) -> Status {
    match kind {
        Kind::Mpris(p) => mpris::status(p),
        Kind::Mpd => mpd::status(),
        Kind::Mpv => mpv::status(),
    }
}

fn meta_of(kind: &Kind) -> Meta {
    match kind {
        Kind::Mpris(p) => {
            let title = mpris::metadata(p, "title")
                .or_else(|| mpris::metadata(p, "xesam:url").map(|u| basename(&u)))
                .unwrap_or_default();
            Meta {
                title,
                artist: mpris::metadata(p, "artist").unwrap_or_default(),
                album: mpris::metadata(p, "album").unwrap_or_default(),
                art_url: mpris::metadata(p, "mpris:artUrl").unwrap_or_default(),
            }
        }
        Kind::Mpd => {
            let title = mpd::current("%title%")
                .filter(|s| !s.is_empty())
                .or_else(|| mpd::current("%file%").map(|f| basename(&f)))
                .unwrap_or_default();
            Meta {
                title,
                artist: mpd::current("%artist%").unwrap_or_default(),
                album: mpd::current("%album%").unwrap_or_default(),
                art_url: String::new(),
            }
        }
        Kind::Mpv => {
            let (t, a, al) = mpv::metadata();
            Meta {
                title: t.unwrap_or_default(),
                artist: a.unwrap_or_default(),
                album: al.unwrap_or_default(),
                art_url: String::new(),
            }
        }
    }
}

fn execute(kind: &Kind, cmd: Command) -> bool {
    match kind {
        Kind::Mpris(p) => mpris::control(p, cmd),
        Kind::Mpd => mpd::control(cmd),
        Kind::Mpv => mpv::control(cmd),
    }
}

/// Best MPRIS player whose name passes `filter`, by score.
fn pick_best_mpris(filter: impl Fn(&str) -> bool, cmd: Command, last_id: &str) -> Option<Kind> {
    mpris::list()
        .into_iter()
        .filter(|p| filter(p))
        .map(|p| {
            let st = mpris::status(&p);
            let k = Kind::Mpris(p);
            (candidate_score(&k, st, cmd, last_id), k)
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, k)| k)
}

fn resolve_explicit(target: &str, cmd: Command, last_id: &str) -> Option<Kind> {
    match target.to_ascii_lowercase().as_str() {
        "mpv" => mpv::available().then_some(Kind::Mpv),
        "mpd" | "mpc" => mpd::available().then_some(Kind::Mpd),
        "spotify" => spotify::ensure_ready(cmd).map(Kind::Mpris),
        "browser" => pick_best_mpris(is_browser, cmd, last_id),
        other => {
            let needle = other.to_string();
            pick_best_mpris(
                move |n| n.to_ascii_lowercase().contains(&needle),
                cmd,
                last_id,
            )
        }
    }
}

fn pick_active(target: Option<&str>, cmd: Command, last_id: &str) -> Option<Kind> {
    if let Some(t) = target {
        return resolve_explicit(t, cmd, last_id);
    }
    let mut cands: Vec<Kind> = Vec::new();
    if mpv::available() {
        cands.push(Kind::Mpv);
    }
    if mpd::available() {
        cands.push(Kind::Mpd);
    }
    for p in mpris::list() {
        cands.push(Kind::Mpris(p));
    }
    cands
        .into_iter()
        .map(|k| {
            let st = status_of(&k);
            (candidate_score(&k, st, cmd, last_id), k)
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, k)| k)
}

/// Run a media transport command against `target` (or the best active
/// player when `None`).
pub fn run(cmd: Command, target: Option<&str>) -> Result<()> {
    let last_id = read_last_player();
    let kind = match pick_active(target, cmd, &last_id) {
        Some(k) => k,
        None => {
            notify::error("Kontrol edilebilir bir medya oynatıcı bulunamadı.");
            return Err(anyhow!("no controllable media player found"));
        }
    };

    if !cmd.is_status() {
        if !execute(&kind, cmd) {
            notify::error(&format!(
                "{} için komut başarısız oldu.",
                notify::player_pretty(kind.name())
            ));
            return Err(anyhow!("media control command failed"));
        }
        sleep(Duration::from_millis(150));
    }

    let st = status_of(&kind);
    let meta = meta_of(&kind);
    write_last_player(&kind.id());
    notify::media(kind.name(), cmd, st, &meta);
    Ok(())
}
