//! mpv window-control commands: orchestrates margo (`mctl`) + mpv's JSON
//! IPC + helper tools. The decision math lives in `geometry`/`ytdl`/
//! `margo`; this module is the side-effecting glue (verified manually).

use anyhow::{Result, bail};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use crate::geometry::{Corner, Rect, nearest_corner};
use crate::margo;
use crate::mpv_ipc;
use crate::ytdl;

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
            .args(["-t", "1200", "mplay", body])
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
    if !have("wl-paste") {
        return String::new();
    }
    Command::new("wl-paste")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
}

/// Resolve a source from an explicit argument, else the clipboard.
fn resolve_source(arg: Option<&str>) -> String {
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
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
        })
        .join("mplay")
}

/// Single-quote a path for safe embedding in a `/bin/sh` script.
fn sh_quote(p: &std::path::Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Materialize a tiny wrapper that hands mpv's `ytdl_hook` back to our own
/// embedded shim (`mplay ytdlp …`). The wrapper lives in the runtime dir
/// and points at the *current* mplay binary — no hard-coded dotfiles path.
fn ensure_ytdl_shim() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("ytdl-shim");
    let script = format!("#!/bin/sh\nexec {} ytdlp \"$@\"\n", sh_quote(&exe));
    std::fs::write(&path, script).ok()?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// Launch mpv (pseudo-gui + IPC socket), optionally loading `source`.
fn spawn_mpv(source: Option<&str>) -> Result<()> {
    if !have("mpv") {
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

    let (program, lead): (&str, Vec<String>) = if have("mullvad-exclude") {
        ("mullvad-exclude", vec!["mpv".into()])
    } else {
        ("mpv", vec![])
    };
    let mut cmd = Command::new(program);
    cmd.args(lead).args(&args);
    if let Some(src) = source {
        cmd.arg(src);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()?;
    Ok(())
}

// ── commands ───────────────────────────────────────────────

pub fn start() -> Result<()> {
    if mpv_running() && mpv_ipc::socket_ready() {
        notify("MPV zaten çalışıyor");
        return Ok(());
    }
    spawn_mpv(None)?;
    notify(&format!("MPV başlatıldı ({}x{})", default_w(), default_h()));
    Ok(())
}

pub fn toggle() -> Result<()> {
    if !mpv_running() || !mpv_ipc::socket_ready() {
        bail!("MPV çalışmıyor");
    }
    mpv_ipc::toggle_pause()?;
    Ok(())
}

pub fn play(arg: Option<&str>) -> Result<()> {
    let src = resolve_source(arg);
    if src.is_empty() {
        bail!("Oynatılacak kaynak yok (argüman/pano boş)");
    }
    let target = if ytdl::is_youtube_url(&src) {
        format!("ytdl://{src}")
    } else {
        src.clone()
    };
    if mpv_running() && mpv_ipc::socket_ready() {
        mpv_ipc::loadfile(&target, "replace")?;
        notify("Yüklendi (replace)");
    } else {
        spawn_mpv(Some(&target))?;
        notify("Oynatılıyor");
    }
    Ok(())
}

pub fn download(arg: Option<&str>) -> Result<()> {
    if !have("yt-dlp") {
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
