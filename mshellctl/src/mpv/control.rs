//! mpv companion window control: orchestrates margo (via `mctl::ipc_client`)
//! + mpv's JSON IPC. Ported from `mplay`'s `control.rs` — `start`/`toggle`/
//! `stop`/`snap`/`pin`/`focus` only; `play`/`download`/the video-wallpaper
//! engine stay on `mplay` for now (a separate yt-dlp-adjacent scope).
//!
//! The decision math lives in `geometry`; this module is the
//! side-effecting glue (verified manually against a live compositor).

use anyhow::{Result, bail};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use super::geometry::{Corner, Rect, nearest_corner};
use super::ipc as mpv_ipc;
use super::margo_client as margo;

const APP_ID: &str = "mpv";

fn env_i32(key: &str, default: i32) -> i32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn default_w() -> i32 {
    env_i32("MARGO_MPV_WIDTH", 640)
}
fn default_h() -> i32 {
    env_i32("MARGO_MPV_HEIGHT", 360)
}
fn margin_x() -> i32 {
    env_i32("MARGO_MPV_MARGIN_X", 32)
}
fn margin_y() -> i32 {
    env_i32("MARGO_MPV_MARGIN_Y", 96)
}

fn have(tool: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Best-effort desktop notification.
fn notify(body: &str) {
    if have("notify-send") {
        let _ = Command::new("notify-send")
            .args(["-t", "1200", "mshellctl", body])
            .status();
    }
}

fn mpv_running() -> bool {
    Command::new("pgrep")
        .args(["-x", "mpv"])
        .stdout(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Launch mpv (pseudo-gui + IPC socket), idle (no source loaded yet).
fn spawn_mpv() -> Result<()> {
    if !have("mpv") {
        bail!("mpv bulunamadı");
    }
    let sock = mpv_ipc::socket_path();
    let _ = std::fs::remove_file(&sock);
    let autofit = format!("{}x{}", default_w(), default_h());
    let args = [
        "--player-operation-mode=pseudo-gui".to_string(),
        format!("--input-ipc-server={}", sock.display()),
        "--idle".to_string(),
        format!("--autofit={autofit}"),
        format!("--autofit-larger={autofit}"),
    ];

    let (program, lead): (&str, Vec<&str>) = if have("mullvad-exclude") {
        ("mullvad-exclude", vec!["mpv"])
    } else {
        ("mpv", vec![])
    };
    Command::new(program)
        .args(lead)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

// ── commands ─────────────────────────────────────────────────────────────

pub fn start() -> Result<()> {
    if mpv_running() && mpv_ipc::socket_ready() {
        notify("MPV zaten çalışıyor");
        return Ok(());
    }
    spawn_mpv()?;
    notify(&format!("MPV başlatıldı ({}x{})", default_w(), default_h()));
    Ok(())
}

/// Rich play/pause notification: ▶/⏸ icon + the media title, coalesced
/// into a single updating popup (like the volume OSD).
fn notify_media(playing: bool, title: &str) {
    if !have("notify-send") {
        return;
    }
    let (icon, summary) = if playing {
        ("media-playback-start", "▶  Oynatılıyor")
    } else {
        ("media-playback-pause", "⏸  Duraklatıldı")
    };
    let _ = Command::new("notify-send")
        .args([
            "-t",
            "1500",
            "-i",
            icon,
            "-h",
            "string:x-canonical-private-synchronous:mshellctl-mpv",
            summary,
            title,
        ])
        .status();
}

pub fn toggle() -> Result<()> {
    if !mpv_running() || !mpv_ipc::socket_ready() {
        bail!("MPV çalışmıyor");
    }
    mpv_ipc::toggle_pause()?;
    // Let mpv apply the cycle, then report the resulting state + title.
    sleep(Duration::from_millis(40));
    let playing = !mpv_ipc::get_bool("pause").unwrap_or(false);
    let title = mpv_ipc::get_string("media-title").unwrap_or_default();
    notify_media(playing, &title);
    Ok(())
}

/// Hop monitors + tags + focusstack until the mpv window is focused.
pub fn focus() -> Result<()> {
    let clients = margo::clients()?;
    let mpv = margo::find_client(&clients, APP_ID)
        .ok_or_else(|| anyhow::anyhow!("MPV penceresi bulunamadı"))?;

    // 1. Hop to mpv's monitor.
    let mut hops = 0;
    loop {
        let mons = margo::monitors()?;
        let active = margo::active_output(&mons);
        if active.map(|o| o.name) == Some(mpv.monitor.clone()) || hops >= 4 {
            break;
        }
        let _ = margo::dispatch("focusmon", &["1"]);
        sleep(Duration::from_millis(40));
        hops += 1;
    }

    // 2. Switch view if mpv's tags don't intersect the active mask.
    let mons = margo::monitors()?;
    if let Some(active) = margo::active_output(&mons)
        && (mpv.tags & active.active_tag_mask) == 0
    {
        let lowest = mpv.tags & mpv.tags.wrapping_neg();
        let _ = margo::dispatch("view", &[&lowest.to_string()]);
        sleep(Duration::from_millis(40));
    }

    // 3. Cycle focus until mpv is focused.
    for _ in 0..20 {
        let f = margo::focused()?;
        if margo::parse_focused(&f).map(|c| c.app_id).as_deref() == Some(APP_ID) {
            return Ok(());
        }
        let _ = margo::dispatch("focusstack", &["1"]);
        sleep(Duration::from_millis(30));
    }
    bail!("MPV odaklanamadı")
}

fn ensure_floating() -> Result<()> {
    let f = margo::focused()?;
    let floating = margo::parse_focused(&f)
        .map(|c| c.floating)
        .unwrap_or(false);
    if !floating {
        let _ = margo::dispatch("togglefloating", &[]);
        sleep(Duration::from_millis(50));
    }
    Ok(())
}

pub fn snap() -> Result<()> {
    focus()?;
    ensure_floating()?;

    let mut f = margo::parse_focused(&margo::focused()?)
        .ok_or_else(|| anyhow::anyhow!("Odaktaki pencere okunamadı"))?;

    // Shrink a tiled-sized window down to the floating default first.
    if f.width > 700 || f.height > 500 {
        let dw = default_w() - f.width;
        let dh = default_h() - f.height;
        let _ = margo::dispatch("resizewin", &["--", &dw.to_string(), &dh.to_string()]);
        sleep(Duration::from_millis(50));
        f = margo::parse_focused(&margo::focused()?)
            .ok_or_else(|| anyhow::anyhow!("Odaktaki pencere okunamadı"))?;
    }

    let mons = margo::monitors()?;
    let out = margo::find_output(&mons, &f.monitor)
        .ok_or_else(|| anyhow::anyhow!("Output {} bulunamadı", f.monitor))?;
    let area = Rect {
        x: out.x,
        y: out.y,
        w: out.width,
        h: out.height,
    };
    let current = nearest_corner(f.x, f.y, f.width, f.height, area, margin_x(), margin_y());
    let next: Corner = current.next();
    let (tx, ty) = next.position(area, f.width, f.height, margin_x(), margin_y());
    let dx = tx - f.x;
    let dy = ty - f.y;
    margo::dispatch("movewin", &["--", &dx.to_string(), &dy.to_string()])?;
    notify(&format!("{current:?} → {next:?}"));
    Ok(())
}

pub fn pin() -> Result<()> {
    focus()?;
    ensure_floating()?;
    margo::dispatch("togglesticky", &[])?;
    notify("mpv sabitleme toggle");
    Ok(())
}

pub fn stop() -> Result<()> {
    if mpv_ipc::socket_ready() {
        let _ = mpv_ipc::quit();
    } else {
        let _ = Command::new("pkill").args(["-x", "mpv"]).status();
    }
    notify("MPV kapatıldı");
    Ok(())
}
