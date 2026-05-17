//! `mscreenshot` — margo's screenshot helper.
//!
//! Replaces the old `scripts/screenshot` bash helper and the
//! short-lived in-compositor capture path. Lives in the margo
//! workspace as a sibling binary to `mctl` and `mlayout`,
//! so it ships in the same package and can be invoked by the
//! compositor's `screenshot-*` dispatch actions OR directly by
//! the user from a terminal / keybind.
//!
//! ## Pipeline
//!
//! Each subcommand follows the same shape:
//!
//!   1. Capture into a temp PNG via `grim` (full output, focused
//!      window, or `slurp`-selected region).
//!   2. Optionally pipe through an annotation editor — the
//!      first of `swappy` / `satty` / `gimp` / `krita` that's
//!      on `$PATH`. The editor writes back to the user's
//!      `$XDG_PICTURES_DIR/Screenshots/screenshot_TS.png`.
//!   3. Optionally copy the final PNG to the clipboard via
//!      `wl-copy --type image/png`.
//!   4. Fire a `notify-send` so the user sees the saved path.
//!
//! Compatibility: every subcommand the old bash helper accepted
//! (`rec`, `area`, `screen`, `window`, plus the short aliases
//! `rc`/`rf`/`ri`/`sc`/`sf`/`si`/`sec`/`wc`/`wf`/`wi` and the
//! `open`/`dir` shortcuts) is preserved. The dispatch wiring in
//! `margo/src/dispatch/mod.rs` now calls `mscreenshot <mode>`
//! instead of `margo-screenshot <mode>`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "mscreenshot",
    version,
    about = "Screenshot helper for margo — capture, edit, save, clipboard.",
    long_about = "Screenshot helper for margo. Each subcommand spawns the\n\
                  appropriate underlying tool (grim / slurp / wl-copy / an\n\
                  editor like swappy or satty) and ships the result through\n\
                  the standard save+clipboard pipeline.\n\
                  \n\
                  ENVIRONMENT:\n  \
                    SCREENSHOT_SAVE_DIR    override save directory\n  \
                    SCREENSHOT_EDITOR      force editor (swappy/satty/...)\n  \
                    SCREENSHOT_NO_EDIT     skip the edit step (1/true)\n  \
                  \n\
                  REQUIRED RUNTIME TOOLS:\n  \
                    grim slurp wl-clipboard\n  \
                  OPTIONAL EDITOR:\n  \
                    swappy / satty / gimp / krita"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Wait N seconds before capturing. Useful for catching menus,
    /// tooltips, or any pop-up that closes when focus moves to a
    /// selector. Shows a single notification announcing the delay
    /// so the user knows what's happening.
    #[arg(long, short = 'd', global = true)]
    delay: Option<u32>,
    /// Override capture output for `screen`/`sc`/`sf`/`si`/`sec`.
    /// Default behaviour reads the focused output from `mctl status`.
    /// Useful for multi-monitor users who want a specific monitor
    /// captured regardless of focus. Has no effect on region or
    /// window modes.
    #[arg(long, short = 'o', global = true)]
    output: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Region select → edit → save + clipboard.
    Rec,
    /// Region select → save (no clipboard, no edit).
    Area,
    /// Region select → save + edit, no clipboard.
    Ri,
    /// Region select → save to clipboard only.
    Rc,
    /// Region select → save to disk only.
    Rf,

    /// Focused output → edit → save.
    Screen,
    /// Focused output → save to clipboard.
    Sc,
    /// Focused output → save to disk.
    Sf,
    /// Focused output → save + edit.
    Si,
    /// Focused output → edit + clipboard.
    Sec,

    /// Focused window → edit → save.
    Window,
    /// Focused window → save to clipboard.
    Wc,
    /// Focused window → save to disk.
    Wf,
    /// Focused window → save + edit.
    Wi,

    /// Open the most recently saved screenshot via xdg-open.
    Open,
    /// Open the screenshot save directory via xdg-open.
    Dir,
    /// Internal: spawned as a detached helper after a save to
    /// drive a `notify-send --wait --action ...` and execute the
    /// user's button click (Open / Folder / Delete). Not for
    /// direct invocation.
    #[command(hide = true)]
    NotifyHandle {
        /// Notification title to display.
        title: String,
        /// Notification body to display.
        body: String,
        /// Path of the saved screenshot — passed to xdg-open
        /// on "Open" and rm on "Delete".
        path: String,
    },
}

#[derive(Copy, Clone, Debug)]
enum CaptureSource {
    Region,
    Screen,
    Window,
}

#[derive(Copy, Clone, Debug)]
enum DeliveryMode {
    /// Save to disk; no editor, no clipboard.
    SaveOnly,
    /// Copy to clipboard; no save, no editor.
    ClipOnly,
    /// Edit → save to disk; no clipboard.
    EditSave,
    /// Edit → save → clipboard.
    EditSaveClip,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("mscreenshot: {err:#}");
        notify_failure(&format!("{err:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match &cli.cmd {
        Cmd::Open => return open_latest(),
        Cmd::Dir => return open_save_dir(),
        Cmd::NotifyHandle { title, body, path } => {
            return run_notify_handle(title, body, path);
        }
        _ => {}
    }

    let (source, mode) = match cli.cmd {
        Cmd::Rec => (CaptureSource::Region, DeliveryMode::EditSaveClip),
        Cmd::Area => (CaptureSource::Region, DeliveryMode::SaveOnly),
        Cmd::Ri => (CaptureSource::Region, DeliveryMode::EditSave),
        Cmd::Rc => (CaptureSource::Region, DeliveryMode::ClipOnly),
        Cmd::Rf => (CaptureSource::Region, DeliveryMode::SaveOnly),
        Cmd::Screen => (CaptureSource::Screen, DeliveryMode::EditSave),
        Cmd::Sc => (CaptureSource::Screen, DeliveryMode::ClipOnly),
        Cmd::Sf => (CaptureSource::Screen, DeliveryMode::SaveOnly),
        Cmd::Si => (CaptureSource::Screen, DeliveryMode::EditSave),
        Cmd::Sec => (CaptureSource::Screen, DeliveryMode::EditSaveClip),
        Cmd::Window => (CaptureSource::Window, DeliveryMode::EditSave),
        Cmd::Wc => (CaptureSource::Window, DeliveryMode::ClipOnly),
        Cmd::Wf => (CaptureSource::Window, DeliveryMode::SaveOnly),
        Cmd::Wi => (CaptureSource::Window, DeliveryMode::EditSave),
        Cmd::Open | Cmd::Dir | Cmd::NotifyHandle { .. } => {
            unreachable!("Open/Dir/NotifyHandle handled earlier and short-circuit before this match")
        }
    };

    require("grim")?;
    if matches!(source, CaptureSource::Region) {
        // slurp is only required when MARGO_REGION_GEOM isn't set —
        // margo's W2.1 in-compositor selector pre-fills the env so
        // we skip slurp entirely. Outside margo (or with the env
        // unset) the legacy slurp-spawning path applies.
        let pre_geom = std::env::var("MARGO_REGION_GEOM").unwrap_or_default();
        if pre_geom.trim().is_empty() {
            require("slurp")?;
        }
    }
    if matches!(
        mode,
        DeliveryMode::ClipOnly | DeliveryMode::EditSaveClip
    ) {
        require("wl-copy")?;
    }

    let label = match source {
        CaptureSource::Region => "Region screenshot",
        CaptureSource::Screen => "Screen screenshot",
        CaptureSource::Window => "Window screenshot",
    };

    // `--delay N` countdown: pop one notification announcing the
    // delay (so the user knows the capture isn't frozen), sleep,
    // then proceed. We don't tick down per-second — that floods
    // the notification stack and is louder than useful.
    if let Some(seconds) = cli.delay
        && seconds > 0
    {
        notify(
            "Screenshot",
            &format!("Capturing in {seconds}s — set up the menu / tooltip now"),
        );
        std::thread::sleep(std::time::Duration::from_secs(seconds as u64));
    }

    // Step 1: capture into a temp file.
    let temp = make_temp_png()?;
    capture(source, &temp, cli.output.as_deref())
        .with_context(|| format!("capture ({:?})", source))?;

    // Step 2: deliver per mode.
    match mode {
        DeliveryMode::SaveOnly => {
            let final_path = save_final(&temp)?;
            notify_save(label, &final_path);
        }
        DeliveryMode::ClipOnly => {
            copy_to_clipboard(&temp)?;
            notify_clip(label);
            // temp dropped; clipboard worker holds the bytes
            // until the next selection replaces it.
        }
        DeliveryMode::EditSave => {
            let final_path = match edit(&temp)? {
                Some(p) => p,
                None => {
                    // No editor available — just save raw.
                    save_final(&temp)?
                }
            };
            notify_save(label, &final_path);
        }
        DeliveryMode::EditSaveClip => {
            let final_path = match edit(&temp)? {
                Some(p) => p,
                None => save_final(&temp)?,
            };
            copy_to_clipboard(&final_path)?;
            notify_save_clip(label, &final_path);
        }
    }
    Ok(())
}

// ── Capture step ────────────────────────────────────────────

fn capture(source: CaptureSource, dest: &Path, output_override: Option<&str>) -> Result<()> {
    match source {
        CaptureSource::Region => capture_region(dest),
        CaptureSource::Screen => capture_screen(dest, output_override),
        CaptureSource::Window => capture_window(dest),
    }
}

fn capture_region(dest: &Path) -> Result<()> {
    // Region geometry source preference, in order:
    //
    //   1. `MARGO_REGION_GEOM` env (W2.1 path). The in-compositor
    //      selector still uses this when it spawns mscreenshot
    //      directly post-commit, so it stays the fast path.
    //   2. `mshellctl screenshot select-region` IPC bridge. Reaches
    //      into the running mshell process and reuses the rich
    //      in-shell area selector (preview state, snap-to-window,
    //      aspect chip, Ctrl+S / Ctrl+E shortcuts) instead of
    //      slurp's bare overlay. Only attempted when mshellctl is
    //      on PATH AND mshell is actually running — the call
    //      blocks until the user commits or cancels. Cancel comes
    //      back as an empty stdout, indistinguishable from the
    //      slurp cancel convention.
    //   3. `slurp` fallback. For compositors without mshell or
    //      when mshellctl isn't installed; preserves the original
    //      mscreenshot behaviour from before the bridge landed.
    let geom_owned = std::env::var("MARGO_REGION_GEOM").unwrap_or_default();
    let geom = if !geom_owned.trim().is_empty() {
        geom_owned.trim().to_string()
    } else if let Some(g) = try_mshell_select_region() {
        g
    } else {
        let g = run_capture_stdout(
            "slurp",
            &[
                "-b", "00000055",
                "-c", "f5f5f5ee",
                "-s", "00000000",
                "-w", "3",
            ],
        )?;
        g.trim().to_string()
    };
    if geom.is_empty() {
        bail!("region selection cancelled");
    }
    let status = Command::new("grim")
        .args(["-g", &geom])
        .arg(dest)
        .status()
        .context("spawn grim")?;
    if !status.success() {
        bail!("grim exited {status} for region capture");
    }
    Ok(())
}

/// Try `mshellctl screenshot select-region`. Returns `Some(geom)`
/// when the bridge call succeeded and returned a geometry (which
/// may be an empty string — the user-cancel signal), `None` when
/// mshellctl isn't on PATH or the call itself failed. The caller
/// treats `None` as "fall through to slurp"; an empty-string
/// `Some("")` is treated as a real cancel (the user did launch
/// the selector and dismissed it).
fn try_mshell_select_region() -> Option<String> {
    if !which("mshellctl") {
        return None;
    }
    let out = Command::new("mshellctl")
        .args(["screenshot", "select-region"])
        .output()
        .ok()?;
    if !out.status.success() {
        // mshell probably isn't running — let slurp take over.
        return None;
    }
    let geom = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(geom)
}

fn capture_screen(dest: &Path, output_override: Option<&str>) -> Result<()> {
    // Explicit `--output NAME` wins. Fall back to focused output
    // when the user didn't override.
    let output_name = match output_override {
        Some(name) if !name.is_empty() => Some(name.to_string()),
        _ => focused_output_name().ok(),
    };
    let mut cmd = Command::new("grim");
    if let Some(name) = output_name.filter(|n| !n.is_empty() && n != "null") {
        cmd.args(["-o", &name]);
    }
    cmd.arg(dest);
    let status = cmd.status().context("spawn grim")?;
    if !status.success() {
        bail!("grim exited {status} for screen capture");
    }
    Ok(())
}

fn capture_window(dest: &Path) -> Result<()> {
    let geom = focused_window_geometry();
    match geom {
        Some(g) if !g.is_empty() => {
            let status = Command::new("grim")
                .args(["-g", &g])
                .arg(dest)
                .status()
                .context("spawn grim")?;
            if !status.success() {
                bail!("grim exited {status} for window capture");
            }
            Ok(())
        }
        _ => {
            // No focused window geom; fall back to region select
            // — same UX the old bash helper had.
            eprintln!(
                "(no focused window geometry — falling back to region select)"
            );
            capture_region(dest)
        }
    }
}

// ── mctl status integration ─────────────────────────────────

fn mctl_status_json() -> Result<serde_json::Value> {
    let out = Command::new("mctl")
        .arg("status")
        .arg("--json")
        .output()
        .context("spawn mctl status --json")?;
    if !out.status.success() {
        bail!(
            "mctl status --json failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .context("parse mctl status JSON")?;
    Ok(json)
}

fn focused_output_name() -> Result<String> {
    let json = mctl_status_json()?;
    let outputs = json["outputs"]
        .as_array()
        .ok_or_else(|| anyhow!("mctl status: no outputs array"))?;
    // Prefer an output marked focused; else the active one;
    // else the first.
    let pick = outputs
        .iter()
        .find(|o| {
            o["focused"]
                .as_object()
                .is_some_and(|f| !f.get("title").and_then(|t| t.as_str()).unwrap_or("").is_empty())
        })
        .or_else(|| outputs.iter().find(|o| o["active"].as_bool() == Some(true)))
        .or_else(|| outputs.first());
    let name = pick
        .and_then(|o| o["name"].as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        bail!("no focused output found in mctl status");
    }
    Ok(name)
}

fn focused_window_geometry() -> Option<String> {
    let json = mctl_status_json().ok()?;
    let outputs = json["outputs"].as_array()?;
    for o in outputs {
        let f = o["focused"].as_object()?;
        let x = f.get("x")?.as_i64().unwrap_or(0);
        let y = f.get("y")?.as_i64().unwrap_or(0);
        let w = f.get("width")?.as_i64().unwrap_or(0);
        let h = f.get("height")?.as_i64().unwrap_or(0);
        if w > 0 && h > 0 {
            // grim -g "X,Y WxH"
            return Some(format!("{},{} {}x{}", x, y, w, h));
        }
    }
    None
}

// ── Edit step ───────────────────────────────────────────────

fn edit(input: &Path) -> Result<Option<PathBuf>> {
    if env_truthy("SCREENSHOT_NO_EDIT") {
        return Ok(None);
    }
    let editor = pick_editor();
    let Some(editor) = editor else {
        return Ok(None);
    };
    let output = save_dir()?.join(default_filename());
    ensure_parent(&output)?;
    match editor.as_str() {
        "swappy" => {
            let status = Command::new("swappy")
                .args(["-f"])
                .arg(input)
                .args(["-o"])
                .arg(&output)
                .status()
                .context("spawn swappy")?;
            if !status.success() {
                bail!("swappy exited {status}");
            }
        }
        "satty" => {
            let status = Command::new("satty")
                .args(["--filename"])
                .arg(input)
                .args(["--output-filename"])
                .arg(&output)
                .status()
                .context("spawn satty")?;
            if !status.success() {
                bail!("satty exited {status}");
            }
        }
        "gimp" | "krita" => {
            // Async-style: just open the temp in the editor and
            // notify. The user is expected to save themselves.
            Command::new(&editor)
                .arg(input)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .with_context(|| format!("spawn {editor}"))?;
            return Ok(Some(input.to_path_buf()));
        }
        _ => return Ok(None),
    }
    if output.exists() && output.metadata().map(|m| m.len()).unwrap_or(0) > 0 {
        Ok(Some(output))
    } else {
        // Editor closed without saving — keep the unedited
        // capture so the user still has SOMETHING.
        Ok(None)
    }
}

fn pick_editor() -> Option<String> {
    if let Ok(forced) = std::env::var("SCREENSHOT_EDITOR")
        && !forced.is_empty()
        && which(&forced)
    {
        return Some(forced);
    }
    for cand in ["swappy", "satty", "gimp", "krita"] {
        if which(cand) {
            return Some(cand.to_string());
        }
    }
    None
}

// ── Save / clipboard / temp ─────────────────────────────────

fn save_final(temp: &Path) -> Result<PathBuf> {
    let dest = save_dir()?.join(default_filename());
    ensure_parent(&dest)?;
    std::fs::copy(temp, &dest)
        .with_context(|| format!("copy {} → {}", temp.display(), dest.display()))?;
    Ok(dest)
}

fn copy_to_clipboard(file: &Path) -> Result<()> {
    use std::io::Write;
    let bytes = std::fs::read(file)
        .with_context(|| format!("read {}", file.display()))?;
    let mut child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn wl-copy")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&bytes).context("write to wl-copy")?;
    }
    // wl-copy daemonises itself; don't wait on the child.
    Ok(())
}

fn make_temp_png() -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = runtime_dir.join(format!("mscreenshot-{}", unsafe { libc::getuid() }));
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create temp dir {}", dir.display()))?;
    let stamp = current_timestamp();
    let pid = std::process::id();
    Ok(dir.join(format!("capture_{stamp}_{pid}.png")))
}

fn save_dir() -> Result<PathBuf> {
    if let Some(s) = std::env::var_os("SCREENSHOT_SAVE_DIR") {
        return Ok(PathBuf::from(s));
    }
    if let Some(s) = std::env::var_os("XDG_PICTURES_DIR")
        .filter(|v| !v.is_empty())
    {
        return Ok(PathBuf::from(s).join("Screenshots"));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home).join("Pictures").join("Screenshots"));
    }
    bail!("could not derive a save directory")
}

fn ensure_parent(p: &Path) -> Result<()> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    Ok(())
}

fn default_filename() -> String {
    format!("screenshot_{}.png", current_timestamp())
}

fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        )
    }
}

// ── Open / dir ──────────────────────────────────────────────

fn open_latest() -> Result<()> {
    let dir = save_dir()?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    entries.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
    });
    let Some(latest) = entries.last() else {
        bail!("no screenshots in {}", dir.display());
    };
    Command::new("xdg-open")
        .arg(latest)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn xdg-open")?;
    notify("Screenshot", &format!("Opened {}", file_basename(latest)));
    Ok(())
}

fn open_save_dir() -> Result<()> {
    let dir = save_dir()?;
    std::fs::create_dir_all(&dir).ok();
    Command::new("xdg-open")
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn xdg-open")?;
    notify("Screenshot", &format!("Opened {}", dir.display()));
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────

fn require(cmd: &str) -> Result<()> {
    if which(cmd) {
        Ok(())
    } else {
        bail!(
            "required tool `{}` not found on PATH (install it; mscreenshot \
             relies on grim/slurp/wl-clipboard for capture+clipboard)",
            cmd
        )
    }
}

fn which(cmd: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|d| std::fs::metadata(d.join(cmd)).map(|m| m.is_file()).unwrap_or(false))
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).unwrap_or_default().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn run_capture_stdout(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("spawn {cmd}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn file_basename(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

// ── Notifications ───────────────────────────────────────────

fn notify_save(label: &str, path: &Path) {
    spawn_notify_action(
        "Screenshot",
        &format!("{label} saved\n{}", file_basename(path)),
        path,
    );
}

fn notify_save_clip(label: &str, path: &Path) {
    spawn_notify_action(
        "Screenshot",
        &format!("{label} saved + copied\n{}", file_basename(path)),
        path,
    );
}

/// Spawn `mscreenshot notify-handle` as a detached background
/// process. The helper drives `notify-send --wait --action ...`
/// which blocks until the notification times out or the user
/// clicks Open / Folder / Delete — keeping that wait off the
/// main process so the screenshot dispatch returns immediately.
fn spawn_notify_action(title: &str, body: &str, path: &Path) {
    // current_exe() resolves to /usr/bin/mscreenshot in installed
    // form. Falls back to the bare name (PATH lookup) if unavailable
    // — that path is unusual but cheap to handle.
    let exe = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("mscreenshot"));
    let _ = Command::new(&exe)
        .args([
            "notify-handle",
            title,
            body,
            &path.display().to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Internal: drive a notify-send with three action buttons and
/// execute whichever the user clicked. Blocks until notify-send
/// returns (timeout or click).
fn run_notify_handle(title: &str, body: &str, path: &str) -> Result<()> {
    // `notify-send --wait` blocks until the notification is
    // dismissed (timeout) or an action is clicked. The action
    // ID is printed to stdout if clicked, nothing if timed out.
    let output = Command::new("notify-send")
        .args(["-a", "mscreenshot"])
        .args(["-i", "image-x-generic"])
        .args(["-u", "normal"])
        .args(["-t", "8000"])
        .args(["--wait"])
        .args(["-A", "open=Open"])
        .args(["-A", "folder=Show in folder"])
        .args(["-A", "delete=Delete"])
        .arg(title)
        .arg(body)
        .output()
        .context("spawn notify-send")?;
    let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if chosen.is_empty() {
        return Ok(()); // timed out, no action
    }
    let p = std::path::Path::new(path);
    match chosen.as_str() {
        "open" => {
            let _ = Command::new("xdg-open").arg(p).spawn();
        }
        "folder" => {
            if let Some(parent) = p.parent() {
                let _ = Command::new("xdg-open").arg(parent).spawn();
            }
        }
        "delete" => {
            if let Err(e) = std::fs::remove_file(p) {
                // Notify the user that the delete failed — silent
                // failure here would be confusing ("I clicked Delete
                // but the file's still there").
                notify_with_urgency(
                    "Screenshot delete failed",
                    &format!("{}: {e}", file_basename(p)),
                    "critical",
                    "dialog-error",
                );
            } else {
                notify("Screenshot deleted", &file_basename(p));
            }
        }
        _ => {} // unknown action token, ignore
    }
    Ok(())
}

fn notify_clip(label: &str) {
    notify("Screenshot", &format!("{label} copied to clipboard"));
}

fn notify_failure(msg: &str) {
    notify_with_urgency("Screenshot failed", msg, "critical", "dialog-error");
}

fn notify(title: &str, body: &str) {
    notify_with_urgency(title, body, "normal", "image-x-generic");
}

fn notify_with_urgency(title: &str, body: &str, urgency: &str, icon: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "mscreenshot"])
        .args(["-i", icon])
        .args(["-u", urgency])
        .args(["-t", "3500"])
        .arg(title)
        .arg(body)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
