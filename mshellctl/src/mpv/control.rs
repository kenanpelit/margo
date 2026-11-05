//! mpv companion window control: orchestrates margo (via `mctl::ipc_client`)
//! + mpv's JSON IPC. Ported from `mplay`'s `control.rs` — everything except
//! the video-wallpaper engine (`mplay`'s embedded-libmpv `paper` module —
//! its own raw EGL/Wayland renderer — stays on `mplay` pending a decision
//! on whether/how to embed that inside mshell's own GTK4/EGL stack).
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
use super::youtube::{self, Hints, ProgressStore};
use super::{track, ytdl};

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

/// Best-effort desktop notification.
fn notify(body: &str) {
    if margo_fs::path::on_path("notify-send") {
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

fn read_clipboard() -> String {
    if !margo_fs::path::on_path("wl-paste") {
        return String::new();
    }
    Command::new("wl-paste")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
}

/// Resolve a source from an explicit argument, else the clipboard.
pub fn resolve_source(arg: Option<&str>) -> String {
    let raw = match arg {
        Some(a) if !a.trim().is_empty() => a.to_string(),
        _ => read_clipboard(),
    };
    ytdl::normalize_source(&raw)
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn runtime_dir() -> std::path::PathBuf {
    margo_fs::xdg::runtime_dir()
        .unwrap_or_else(|| {
            std::path::PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
        })
        .join("mshellctl-mpv")
}

/// Single-quote a path for safe embedding in a `/bin/sh` script.
fn sh_quote(p: &std::path::Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Materialize a tiny wrapper that hands mpv's `ytdl_hook` back to our own
/// hidden `mshellctl play __ytdlp` subcommand. The wrapper lives in the
/// runtime dir and points at the *current* mshellctl binary — no
/// hard-coded dotfiles path.
fn ensure_ytdl_shim() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("ytdl-shim");
    let script = format!("#!/bin/sh\nexec {} play __ytdlp \"$@\"\n", sh_quote(&exe));
    std::fs::write(&path, script).ok()?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// Launch mpv (pseudo-gui + IPC socket), optionally loading `source`
/// (from `start` seconds — a per-file option, so later queue entries
/// still start at 0).
fn spawn_mpv(source: Option<&str>, start: Option<f64>) -> Result<()> {
    if !margo_fs::path::on_path("mpv") {
        bail!("mpv bulunamadı");
    }
    let sock = mpv_ipc::socket_path();
    let _ = std::fs::remove_file(&sock);
    let autofit = format!("{}x{}", default_w(), default_h());
    let mut args: Vec<String> = vec![
        "--player-operation-mode=pseudo-gui".into(),
        format!("--input-ipc-server={}", sock.display()),
        "--idle".into(),
        format!("--autofit={autofit}"),
        format!("--autofit-larger={autofit}"),
    ];
    if source.is_some() {
        args.push("--no-audio-display".into());
        if let Some(shim) = ensure_ytdl_shim() {
            args.push(format!("--script-opts-append=ytdl_hook-ytdl_path={shim}"));
        }
    }

    let (program, lead): (&str, Vec<String>) = if margo_fs::path::on_path("mullvad-exclude") {
        ("mullvad-exclude", vec!["mpv".into()])
    } else {
        ("mpv", vec![])
    };
    let mut cmd = Command::new(program);
    cmd.args(lead).args(&args);
    if let Some(src) = source {
        match start {
            Some(secs) => {
                cmd.args(["--{", &format!("--start={secs:.1}"), src, "--}"]);
            }
            None => {
                cmd.arg(src);
            }
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()?;
    Ok(())
}

// ── commands ─────────────────────────────────────────────────────────────

pub fn start() -> Result<()> {
    if mpv_running() && mpv_ipc::socket_ready() {
        notify("MPV zaten çalışıyor");
        return Ok(());
    }
    spawn_mpv(None, None)?;
    notify(&format!("MPV başlatıldı ({}x{})", default_w(), default_h()));
    Ok(())
}

/// How a source enters the companion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    /// Replace what plays now.
    Replace,
    /// Append to the playlist (start playing when idle) — one window.
    Queue,
}

/// What to hand mpv for `src`: the target and its per-file start.
/// YouTube videos resume from their saved position unless the URL has
/// its own timestamp or the video was watched to the end.
fn prepare(src: &str, hints: &Hints) -> (String, Option<f64>, Option<String>) {
    if !ytdl::is_youtube_url(src) {
        return (src.to_string(), None, None);
    }
    let target = format!("ytdl://{src}");
    let Some(id) = youtube::video_id(src) else {
        return (target, None, None); // playlist / channel URL
    };
    let store = ProgressStore::default_location();
    let start = if youtube::has_timestamp(src) {
        None
    } else {
        store.load().videos.get(&id).and_then(youtube::resume_start)
    };
    if let Err(err) = store.touch(&id, hints) {
        tracing::warn!(%err, "youtube: recording history failed");
    }
    (target, start, Some(id))
}

/// `m:ss` / `h:mm:ss`.
fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Play (`Replace`) or queue `arg`; YouTube videos get resume + history
/// and the position tracker.
pub fn load(arg: Option<&str>, mode: LoadMode, hints: &Hints) -> Result<()> {
    let src = resolve_source(arg);
    if src.is_empty() {
        bail!("Oynatılacak kaynak yok (argüman/pano boş)");
    }
    let (target, start, yt_id) = prepare(&src, hints);
    let resumed = start.map(|s| format!(" — kaldığı yerden {}", clock(s)));
    if mpv_running() && mpv_ipc::socket_ready() {
        let options = start.map(|s| format!("start={s:.1}")).unwrap_or_default();
        match mode {
            LoadMode::Replace => {
                mpv_ipc::loadfile_with(&target, "replace", &options)?;
                // A paused window would load the new video paused.
                let _ = mpv_ipc::send_command(mpv_ipc::socket_path(), &["set", "pause", "no"]);
                notify(&format!(
                    "Yüklendi (replace){}",
                    resumed.unwrap_or_default()
                ));
            }
            LoadMode::Queue => {
                mpv_ipc::loadfile_with(&target, "append-play", &options)?;
                notify("Sıraya eklendi");
            }
        }
    } else {
        spawn_mpv(Some(&target), start)?;
        notify(&format!("Oynatılıyor{}", resumed.unwrap_or_default()));
    }
    if yt_id.is_some() {
        track::ensure_running();
    }
    Ok(())
}

/// Drop a video's history + resume point (`None` = all of them).
pub fn forget(id: Option<&str>) -> Result<()> {
    if let Some(id) = id
        && !youtube::valid_video_id(id)
    {
        bail!("geçersiz video kimliği: {id}");
    }
    ProgressStore::default_location().forget(id)?;
    Ok(())
}

pub fn download(arg: Option<&str>) -> Result<()> {
    if !margo_fs::path::on_path("yt-dlp") {
        bail!("yt-dlp bulunamadı");
    }
    let src = resolve_source(arg);
    if !ytdl::is_youtube_url(&src) {
        bail!("Argümandaki/panodaki URL YouTube değil");
    }
    let dir = format!("{}/Downloads", home());
    std::fs::create_dir_all(&dir).ok();
    let status = Command::new("yt-dlp")
        .current_dir(&dir)
        .args([
            "-f",
            "bestvideo+bestaudio/best",
            "--merge-output-format",
            "mp4",
            "--embed-thumbnail",
            "--add-metadata",
            &src,
        ])
        .status()?;
    if status.success() {
        notify(&format!("İndirildi: {dir}"));
        Ok(())
    } else {
        bail!("yt-dlp başarısız")
    }
}

/// Rich play/pause notification: ▶/⏸ icon + the media title, coalesced
/// into a single updating popup (like the volume OSD).
fn notify_media(playing: bool, title: &str) {
    if !margo_fs::path::on_path("notify-send") {
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
        let _ = margo::dispatch("resizewin", &[&dw.to_string(), &dh.to_string()]);
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
    margo::dispatch("movewin", &[&dx.to_string(), &dy.to_string()])?;
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
