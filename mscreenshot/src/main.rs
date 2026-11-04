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

mod ocr;
mod scan;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
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
                    MSCREENSHOT_OCR_LANG   OCR languages (eng+tur; default\n  \
                                           eng+tur when installed)\n  \
                    MSCREENSHOT_OCR_PSM    tesseract --psm (default 6)\n  \
                  \n\
                  REQUIRED RUNTIME TOOLS:\n  \
                    grim slurp wl-clipboard\n  \
                  OPTIONAL EDITOR:\n  \
                    swappy / satty / gimp / krita\n\
                  OPTIONAL TOOLS:\n  \
                    tesseract + tesseract-data-<lang> (ocr)\n  \
                    zbar (qr)"
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

    /// All outputs (whole layout) → edit → save.
    All,
    /// All outputs → save to clipboard.
    Ac,
    /// All outputs → save to disk.
    Af,

    /// Region select → OCR with tesseract → copy the text.
    ///
    /// Languages: `--lang`, else `MSCREENSHOT_OCR_LANG`, else `eng+tur`
    /// narrowed to the installed tessdata packs. Nothing is saved; the
    /// temporary capture is deleted once the text is read.
    Ocr {
        /// tesseract languages, `+`-joined (e.g. `eng+tur`).
        #[arg(long, short = 'l')]
        lang: Option<String>,
        /// tesseract page segmentation mode (0-13, default 6 = one
        /// uniform block of text).
        #[arg(long)]
        psm: Option<String>,
        /// OCR the focused output instead of a selected region.
        #[arg(long)]
        screen: bool,
    },

    /// Region select → decode QR codes / barcodes (zbarimg) → copy the
    /// payload. A single http(s) URL gets an "Open" notification action.
    Qr,

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
    /// Internal: detached helper for the QR result notification; opens
    /// `url` when the user clicks "Open". Not for direct invocation.
    #[command(hide = true)]
    NotifyUrl {
        title: String,
        body: String,
        url: String,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum CaptureSource {
    Region,
    Screen,
    Window,
    AllOutputs,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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
    let cli = Cli::parse();
    if let Err(err) = run(&cli) {
        eprintln!("mscreenshot: {err:#}");
        notify_with_urgency(
            failure_title(&cli.cmd),
            &format!("{err:#}"),
            "critical",
            "dialog-error",
        );
        std::process::exit(1);
    }
}

/// Title of the error notification — names the tool the user ran, so an
/// OCR or scan failure doesn't read as a failed screenshot.
fn failure_title(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::Ocr { .. } => "Text recognition failed",
        Cmd::Qr => "Code scan failed",
        _ => "Screenshot failed",
    }
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Open => return open_latest(),
        Cmd::Dir => return open_save_dir(),
        Cmd::NotifyHandle { title, body, path } => {
            return run_notify_handle(title, body, path);
        }
        Cmd::NotifyUrl { title, body, url } => return run_notify_url(title, body, url),
        Cmd::Qr => return run_qr(cli),
        Cmd::Ocr { lang, psm, screen } => {
            return run_ocr(cli, lang.as_deref(), psm.as_deref(), *screen);
        }
        _ => {}
    }

    let Some((source, mode)) = capture_plan(&cli.cmd) else {
        unreachable!("non-capture commands are handled earlier and short-circuit before this match")
    };

    require_capture_tools(source)?;
    if matches!(mode, DeliveryMode::ClipOnly | DeliveryMode::EditSaveClip) {
        require("wl-copy")?;
    }

    let label = match source {
        CaptureSource::Region => "Region screenshot",
        CaptureSource::Screen => "Screen screenshot",
        CaptureSource::Window => "Window screenshot",
        CaptureSource::AllOutputs => "Full screenshot",
    };

    countdown(cli.delay);

    // Step 1: capture into a temp file.
    let temp = make_temp_png()?;
    capture(source, &temp, cli.output.as_deref())
        .with_context(|| format!("capture ({:?})", source))?;

    // Step 2: deliver per mode.
    deliver(mode, label, &temp)
}

/// grim always; slurp for a region unless a geometry source that doesn't
/// need it is available.
fn require_capture_tools(source: CaptureSource) -> Result<()> {
    require("grim")?;
    if matches!(source, CaptureSource::Region) {
        // slurp is only required when MARGO_REGION_GEOM isn't set —
        // margo's W2.1 in-compositor selector pre-fills the env so
        // we skip slurp entirely. Outside margo (or with the env
        // unset) the legacy slurp-spawning path applies.
        let pre_geom = std::env::var("MARGO_REGION_GEOM").unwrap_or_default();
        if pre_geom.trim().is_empty() && !which("mshellctl") {
            require("slurp")?;
        }
    }
    Ok(())
}

/// `--delay N` countdown: pop one notification announcing the delay (so
/// the user knows the capture isn't frozen), sleep, then proceed. We don't
/// tick down per-second — that floods the notification stack and is
/// louder than useful.
fn countdown(delay: Option<u32>) {
    if let Some(seconds) = delay
        && seconds > 0
    {
        notify(
            "Screenshot",
            &format!("Capturing in {seconds}s — set up the menu / tooltip now"),
        );
        std::thread::sleep(std::time::Duration::from_secs(seconds as u64));
    }
}

fn deliver(mode: DeliveryMode, label: &str, temp: &Path) -> Result<()> {
    match mode {
        DeliveryMode::SaveOnly => {
            let final_path = save_final(temp)?;
            notify_save(label, &final_path);
        }
        DeliveryMode::ClipOnly => {
            copy_to_clipboard(temp)?;
            notify_clip(label);
            // temp dropped; clipboard worker holds the bytes
            // until the next selection replaces it.
        }
        DeliveryMode::EditSave => {
            let final_path = match edit(temp)? {
                Some(p) => p,
                None => {
                    // No editor available — just save raw.
                    save_final(temp)?
                }
            };
            notify_save(label, &final_path);
        }
        DeliveryMode::EditSaveClip => {
            let final_path = match edit(temp)? {
                Some(p) => p,
                None => save_final(temp)?,
            };
            copy_to_clipboard(&final_path)?;
            notify_save_clip(label, &final_path);
        }
    }
    Ok(())
}

/// What a capture subcommand grabs and where the result goes. `None`
/// for the non-capture commands (`open`, `dir`, `notify-handle`).
fn capture_plan(cmd: &Cmd) -> Option<(CaptureSource, DeliveryMode)> {
    Some(match cmd {
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
        Cmd::All => (CaptureSource::AllOutputs, DeliveryMode::EditSave),
        Cmd::Ac => (CaptureSource::AllOutputs, DeliveryMode::ClipOnly),
        Cmd::Af => (CaptureSource::AllOutputs, DeliveryMode::SaveOnly),
        Cmd::Ocr { .. }
        | Cmd::Qr
        | Cmd::NotifyUrl { .. }
        | Cmd::Open
        | Cmd::Dir
        | Cmd::NotifyHandle { .. } => return None,
    })
}

// ── Region tools (OCR) ──────────────────────────────────────

/// A capture that only feeds a tool (OCR, …): it is never the deliverable,
/// so it is deleted on every path — success, failure, or an early `?`.
/// Region contents can be sensitive (a password manager, a chat), so
/// leaving them in `$XDG_RUNTIME_DIR` is not an option.
struct ToolCapture(PathBuf);

impl Drop for ToolCapture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn capture_for_tool(cli: &Cli, source: CaptureSource) -> Result<ToolCapture> {
    require_capture_tools(source)?;
    countdown(cli.delay);
    let shot = ToolCapture(make_temp_png()?);
    capture(source, &shot.0, cli.output.as_deref())
        .with_context(|| format!("capture ({source:?})"))?;
    Ok(shot)
}

fn run_ocr(cli: &Cli, lang: Option<&str>, psm: Option<&str>, screen: bool) -> Result<()> {
    if !which("tesseract") {
        bail!("OCR needs `tesseract` (pacman -S tesseract tesseract-data-eng)");
    }
    require("wl-copy")?;
    let env_lang = std::env::var("MSCREENSHOT_OCR_LANG").ok();
    let requested = lang.or(env_lang.as_deref());
    let installed = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .map(|o| ocr::parse_list_langs(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let langs = ocr::resolve_langs(requested, &installed).map_err(|missing| {
        let pkgs: Vec<String> = missing.iter().map(|l| ocr::tessdata_package(l)).collect();
        anyhow!(
            "OCR language data not installed: {} (pacman -S {})",
            missing.join(", "),
            pkgs.join(" ")
        )
    })?;
    let env_psm = std::env::var("MSCREENSHOT_OCR_PSM").ok();
    let psm = ocr::resolve_psm(psm.or(env_psm.as_deref()));

    let source = if screen {
        CaptureSource::Screen
    } else {
        CaptureSource::Region
    };
    let shot = capture_for_tool(cli, source)?;
    let out = Command::new("tesseract")
        .args(ocr::tesseract_argv(&shot.0.to_string_lossy(), &langs, psm))
        .stdin(Stdio::null())
        .output()
        .context("spawn tesseract")?;
    drop(shot);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "tesseract exited {}: {}",
            out.status,
            err.lines().last().unwrap_or("").trim()
        );
    }
    let text = ocr::clean_text(&String::from_utf8_lossy(&out.stdout));
    if text.is_empty() {
        notify_tool("Text recognition", "No text recognised in the selection");
        return Ok(());
    }
    copy_text_to_clipboard(&text)?;
    notify_tool(
        &format!("Text copied ({langs})"),
        &ocr::escape_markup(&ocr::preview(&text, ocr::PREVIEW_CHARS)),
    );
    Ok(())
}

fn run_qr(cli: &Cli) -> Result<()> {
    if !which("zbarimg") {
        bail!("QR / barcode scan needs `zbarimg` (pacman -S zbar)");
    }
    require("wl-copy")?;
    let shot = capture_for_tool(cli, CaptureSource::Region)?;
    // `-q` drops the "scanned N barcode symbols" summary; no `--raw`, so
    // the `TYPE:` prefixes keep the boundary between codes.
    let out = Command::new("zbarimg")
        .arg("-q")
        .arg(&shot.0)
        .stdin(Stdio::null())
        .output()
        .context("spawn zbarimg")?;
    drop(shot);
    if out.status.code() == Some(scan::ZBAR_NOTHING_FOUND) {
        notify_tool("Code scan", "No QR code or barcode found in the selection");
        return Ok(());
    }
    if !out.status.success() {
        bail!(
            "zbarimg exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("")
                .trim()
        );
    }
    let symbols = scan::parse_zbarimg(&String::from_utf8_lossy(&out.stdout));
    if symbols.is_empty() {
        notify_tool("Code scan", "No QR code or barcode found in the selection");
        return Ok(());
    }
    let text = scan::clipboard_text(&symbols);
    copy_text_to_clipboard(&text)?;
    let title = scan::title(&symbols);
    let body = ocr::escape_markup(&ocr::preview(&text, ocr::PREVIEW_CHARS));
    match scan::openable_url(&symbols) {
        Some(url) => spawn_notify_url(&title, &body, url),
        None => notify_tool(&title, &body),
    }
    Ok(())
}

/// Detached `mscreenshot notify-url` so the scan returns at once while the
/// notification waits for its "Open" click.
fn spawn_notify_url(title: &str, body: &str, url: &str) {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mscreenshot"));
    let _ = Command::new(&exe)
        .args(["notify-url", title, body, url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn run_notify_url(title: &str, body: &str, url: &str) -> Result<()> {
    let output = Command::new("notify-send")
        .args(["-a", "mscreenshot"])
        .args(["-i", "edit-paste-symbolic"])
        .args(["-u", "normal"])
        .args(["-t", "8000"])
        .args(["--wait"])
        .args(["-A", "open=Open link"])
        .arg(title)
        .arg(body)
        .output()
        .context("spawn notify-send")?;
    // Re-check: the URL came through argv, and only plain http(s) may
    // reach xdg-open.
    let symbols = [scan::Symbol {
        kind: String::new(),
        data: url.to_string(),
    }];
    if String::from_utf8_lossy(&output.stdout).trim() == "open"
        && let Some(url) = scan::openable_url(&symbols)
    {
        let _ = Command::new("xdg-open")
            .arg(url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    Ok(())
}

// ── Capture step ────────────────────────────────────────────

fn capture(source: CaptureSource, dest: &Path, output_override: Option<&str>) -> Result<()> {
    match source {
        CaptureSource::Region => capture_region(dest),
        CaptureSource::Screen => capture_screen(dest, output_override),
        CaptureSource::Window => capture_window(dest),
        CaptureSource::AllOutputs => capture_all(dest),
    }
}

/// Whole layout — every output composited. `grim` with no `-o` captures
/// the full output layout by default.
fn capture_all(dest: &Path) -> Result<()> {
    let status = Command::new("grim")
        .arg(dest)
        .status()
        .context("spawn grim")?;
    if !status.success() {
        bail!("grim exited {status} for full capture");
    }
    Ok(())
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
                "-b", "00000055", "-c", "f5f5f5ee", "-s", "00000000", "-w", "3",
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
            eprintln!("(no focused window geometry — falling back to region select)");
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
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parse mctl status JSON")?;
    Ok(json)
}

fn focused_output_name() -> Result<String> {
    focused_output_name_in(&mctl_status_json()?)
}

/// Pick the output `screen` mode captures from a `mctl status --json`
/// document.
fn focused_output_name_in(json: &serde_json::Value) -> Result<String> {
    let outputs = json["outputs"]
        .as_array()
        .ok_or_else(|| anyhow!("mctl status: no outputs array"))?;
    // Prefer the active (focused) output; else the first.
    let pick = outputs
        .iter()
        .find(|o| o["active"].as_bool() == Some(true))
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
    focused_window_geometry_in(&mctl_status_json().ok()?)
}

/// `grim -g` geometry (`"X,Y WxH"`) of the focused window in a
/// `mctl status --json` document, `None` when there is none.
fn focused_window_geometry_in(json: &serde_json::Value) -> Option<String> {
    // Focus lives on the client entry (`clients[].focused`); its
    // x/y/width/height are global layout coordinates, which is what
    // `grim -g` takes.
    let c = json["clients"]
        .as_array()?
        .iter()
        .find(|c| c["focused"].as_bool() == Some(true))?;
    let x = c["x"].as_i64().unwrap_or(0);
    let y = c["y"].as_i64().unwrap_or(0);
    let w = c["width"].as_i64().unwrap_or(0);
    let h = c["height"].as_i64().unwrap_or(0);
    // grim -g "X,Y WxH"
    (w > 0 && h > 0).then(|| format!("{x},{y} {w}x{h}"))
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
    pick_editor_from(std::env::var("SCREENSHOT_EDITOR").ok().as_deref(), which)
}

/// Editor choice given the `SCREENSHOT_EDITOR` value and a PATH probe.
fn pick_editor_from(forced: Option<&str>, on_path: impl Fn(&str) -> bool) -> Option<String> {
    if let Some(forced) = forced
        && !forced.is_empty()
        && on_path(forced)
    {
        return Some(forced.to_string());
    }
    // Satty is the preferred annotation tool across margo (and the
    // shell scripts agree — `screenshot` and `scripts/screenshot`
    // both call satty first). swappy stays in the fallback chain
    // for users who still have it on PATH but not satty; gimp/
    // krita are the last-resort openers that let the user save
    // manually.
    for cand in ["satty", "swappy", "gimp", "krita"] {
        if on_path(cand) {
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
    let bytes = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
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

fn copy_text_to_clipboard(text: &str) -> Result<()> {
    use std::io::Write;
    let mut child = Command::new("wl-copy")
        .args(["--type", "text/plain;charset=utf-8"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn wl-copy")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .context("write to wl-copy")?;
    }
    // wl-copy forks a daemon that serves the selection and exits the
    // foreground process once stdin is closed; reap it.
    let _ = child.wait();
    Ok(())
}

fn make_temp_png() -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = runtime_dir.join(format!("mscreenshot-{}", unsafe { libc::getuid() }));
    std::fs::create_dir_all(&dir).with_context(|| format!("create temp dir {}", dir.display()))?;
    let stamp = current_timestamp();
    let pid = std::process::id();
    Ok(dir.join(format!("capture_{stamp}_{pid}.png")))
}

fn save_dir() -> Result<PathBuf> {
    save_dir_from(
        std::env::var_os("SCREENSHOT_SAVE_DIR"),
        std::env::var_os("XDG_PICTURES_DIR"),
        std::env::var_os("HOME"),
    )
}

/// Save directory from `SCREENSHOT_SAVE_DIR`, `XDG_PICTURES_DIR` and
/// `HOME`, in that order of preference.
fn save_dir_from(
    save_override: Option<std::ffi::OsString>,
    pictures: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    if let Some(s) = save_override {
        return Ok(PathBuf::from(s));
    }
    if let Some(s) = pictures.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(s).join("Screenshots"));
    }
    if let Some(home) = home {
        return Ok(PathBuf::from(home).join("Pictures").join("Screenshots"));
    }
    bail!("could not derive a save directory")
}

fn ensure_parent(p: &Path) -> Result<()> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    Ok(())
}

fn default_filename() -> String {
    format!("screenshot_{}.png", current_timestamp())
}

/// `YYYY-MM-DD_HH-MM-SS` in local time (shared clock with mshell's
/// screenshot/recording names and the log files).
fn current_timestamp() -> String {
    margo_logging::local_timestamp("%Y-%m-%d_%H-%M-%S")
}

// ── Open / dir ──────────────────────────────────────────────

fn open_latest() -> Result<()> {
    let dir = save_dir()?;
    let Some(latest) = latest_png(&dir)? else {
        bail!("no screenshots in {}", dir.display());
    };
    let latest = &latest;
    Command::new("xdg-open")
        .arg(latest)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn xdg-open")?;
    notify("Screenshot", &format!("Opened {}", file_basename(latest)));
    Ok(())
}

/// Most recently modified `.png` directly inside `dir`.
fn latest_png(dir: &Path) -> Result<Option<PathBuf>> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    entries.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    Ok(entries.pop())
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
    std::env::split_paths(&path).any(|d| {
        std::fs::metadata(d.join(cmd))
            .map(|m| m.is_file())
            .unwrap_or(false)
    })
}

fn env_truthy(name: &str) -> bool {
    is_truthy(&std::env::var(name).unwrap_or_default())
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
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
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("mscreenshot"));
    let _ = Command::new(&exe)
        .args(["notify-handle", title, body, &path.display().to_string()])
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

/// Result notification for the region tools (OCR, …).
fn notify_tool(title: &str, body: &str) {
    notify_with_urgency(title, body, "normal", "edit-paste-symbolic");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::ffi::OsString;

    fn parse(args: &[&str]) -> std::result::Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("mscreenshot").chain(args.iter().copied()))
    }

    #[test]
    fn every_capture_subcommand_maps_to_its_documented_plan() {
        use CaptureSource::*;
        use DeliveryMode::*;
        let table: &[(&str, CaptureSource, DeliveryMode)] = &[
            ("rec", Region, EditSaveClip),
            ("area", Region, SaveOnly),
            ("ri", Region, EditSave),
            ("rc", Region, ClipOnly),
            ("rf", Region, SaveOnly),
            ("screen", Screen, EditSave),
            ("sc", Screen, ClipOnly),
            ("sf", Screen, SaveOnly),
            ("si", Screen, EditSave),
            ("sec", Screen, EditSaveClip),
            ("window", Window, EditSave),
            ("wc", Window, ClipOnly),
            ("wf", Window, SaveOnly),
            ("wi", Window, EditSave),
            ("all", AllOutputs, EditSave),
            ("ac", AllOutputs, ClipOnly),
            ("af", AllOutputs, SaveOnly),
        ];
        for (name, source, mode) in table {
            let cli = parse(&[name]).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(capture_plan(&cli.cmd), Some((*source, *mode)), "{name}");
        }
    }

    #[test]
    fn non_capture_subcommands_have_no_plan() {
        for args in [
            &["open"][..],
            &["dir"][..],
            &["notify-handle", "t", "b", "/tmp/x.png"][..],
            &["ocr"][..],
            &["qr"][..],
            &["notify-url", "t", "b", "https://x.y"][..],
            &["ocr", "--lang", "eng+tur", "--psm", "7", "--screen"][..],
        ] {
            let cli = parse(args).unwrap();
            assert_eq!(capture_plan(&cli.cmd), None, "{args:?}");
        }
    }

    #[test]
    fn failures_are_titled_after_the_tool() {
        let title = |args: &[&str]| failure_title(&parse(args).unwrap().cmd);
        assert_eq!(title(&["ocr"]), "Text recognition failed");
        assert_eq!(title(&["qr"]), "Code scan failed");
        assert_eq!(title(&["rc"]), "Screenshot failed");
    }

    #[test]
    fn global_flags_parse_before_or_after_the_subcommand() {
        let cli = parse(&["-d", "3", "sc"]).unwrap();
        assert_eq!(cli.delay, Some(3));
        let cli = parse(&["sc", "--delay", "5", "--output", "DP-2"]).unwrap();
        assert_eq!(cli.delay, Some(5));
        assert_eq!(cli.output.as_deref(), Some("DP-2"));
        let cli = parse(&["-o", "eDP-1", "screen"]).unwrap();
        assert_eq!(cli.output.as_deref(), Some("eDP-1"));
        assert_eq!(cli.delay, None);
    }

    #[test]
    fn malformed_command_lines_are_rejected() {
        assert!(parse(&[]).is_err(), "a subcommand is required");
        assert!(parse(&["bogus"]).is_err());
        assert!(parse(&["sc", "--delay", "-1"]).is_err(), "negative delay");
        assert!(parse(&["sc", "--delay", "soon"]).is_err());
        assert!(
            parse(&["notify-handle", "title", "body"]).is_err(),
            "notify-handle needs title, body and path"
        );
    }

    #[test]
    fn output_pick_prefers_active_then_first() {
        let doc = json!({ "outputs": [
            { "name": "DP-1", "active": false },
            { "name": "HDMI-A-1", "active": true },
        ]});
        assert_eq!(focused_output_name_in(&doc).unwrap(), "HDMI-A-1");

        let doc = json!({ "outputs": [
            { "name": "DP-1", "active": false },
            { "name": "DP-2", "active": false },
        ]});
        assert_eq!(focused_output_name_in(&doc).unwrap(), "DP-1");
    }

    #[test]
    fn output_pick_errors_on_missing_or_nameless_outputs() {
        assert!(focused_output_name_in(&json!({})).is_err());
        assert!(focused_output_name_in(&json!({ "outputs": [] })).is_err());
        assert!(focused_output_name_in(&json!({ "outputs": [{ "active": true }] })).is_err());
        assert!(focused_output_name_in(&json!({ "outputs": "DP-1" })).is_err());
    }

    /// `mctl status --json` is margo's `get state` snapshot: focus is a
    /// `focused` flag on the entry in `clients`, outputs carry no window.
    #[test]
    fn window_geometry_comes_from_the_focused_client() {
        let doc = json!({
            "outputs": [{ "name": "DP-1", "active": true, "x": 0, "y": 0 }],
            "clients": [
                { "focused": false, "x": 0, "y": 0, "width": 640, "height": 480 },
                { "focused": true, "x": 1930, "y": -20, "width": 800, "height": 600 },
            ],
        });
        assert_eq!(
            focused_window_geometry_in(&doc).as_deref(),
            Some("1930,-20 800x600")
        );
    }

    #[test]
    fn window_geometry_skips_unfocused_and_degenerate_clients() {
        let doc = json!({ "clients": [
            { "focused": false, "x": 0, "y": 0, "width": 640, "height": 480 },
        ]});
        assert_eq!(focused_window_geometry_in(&doc), None);
        let doc = json!({ "clients": [
            { "focused": true, "x": 0, "y": 0, "width": 0, "height": 480 },
        ]});
        assert_eq!(focused_window_geometry_in(&doc), None);
        let doc = json!({ "clients": [{ "focused": true, "x": 5, "y": 5 }] });
        assert_eq!(focused_window_geometry_in(&doc), None);
    }

    #[test]
    fn window_geometry_is_none_without_a_usable_window() {
        assert_eq!(focused_window_geometry_in(&json!({})), None);
        assert_eq!(
            focused_window_geometry_in(&json!({ "outputs": [], "clients": [] })),
            None
        );
    }

    #[test]
    fn save_dir_precedence() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            save_dir_from(os("/shots"), os("/pics"), os("/home/u")).unwrap(),
            PathBuf::from("/shots")
        );
        assert_eq!(
            save_dir_from(None, os("/pics"), os("/home/u")).unwrap(),
            PathBuf::from("/pics/Screenshots")
        );
        assert_eq!(
            save_dir_from(None, os(""), os("/home/u")).unwrap(),
            PathBuf::from("/home/u/Pictures/Screenshots"),
            "an empty XDG_PICTURES_DIR is ignored"
        );
        assert!(save_dir_from(None, None, None).is_err());
    }

    #[test]
    fn editor_override_wins_only_when_installed() {
        let installed = |set: &'static [&'static str]| move |c: &str| set.contains(&c);
        assert_eq!(
            pick_editor_from(Some("gimp"), installed(&["gimp", "satty"])),
            Some("gimp".into())
        );
        assert_eq!(
            pick_editor_from(Some("pinta"), installed(&["swappy"])),
            Some("swappy".into()),
            "a missing override falls back to the chain"
        );
        assert_eq!(
            pick_editor_from(Some(""), installed(&["krita"])),
            Some("krita".into())
        );
        assert_eq!(
            pick_editor_from(None, installed(&["krita", "swappy", "satty"])),
            Some("satty".into()),
            "satty is preferred over swappy and the openers"
        );
        assert_eq!(pick_editor_from(None, installed(&[])), None);
    }

    #[test]
    fn truthy_values() {
        for v in ["1", "true", "TRUE", "Yes", "on"] {
            assert!(is_truthy(v), "{v}");
        }
        for v in ["", "0", "false", "no", "off", "y", " 1"] {
            assert!(!is_truthy(v), "{v}");
        }
    }

    #[test]
    fn timestamp_and_filename_shape() {
        // `YYYY-MM-DD_HH-MM-SS`: sortable and free of `:` (which some
        // file managers and FAT/exFAT targets reject).
        let ts = current_timestamp();
        assert_eq!(ts.len(), 19, "{ts}");
        for (i, c) in ts.char_indices() {
            match i {
                4 | 7 => assert_eq!(c, '-', "{ts}"),
                10 => assert_eq!(c, '_', "{ts}"),
                13 | 16 => assert_eq!(c, '-', "{ts}"),
                _ => assert!(c.is_ascii_digit(), "{ts}"),
            }
        }
        let name = default_filename();
        assert!(
            name.starts_with("screenshot_") && name.ends_with(".png"),
            "{name}"
        );
    }

    #[test]
    fn latest_png_ignores_other_files_and_picks_newest() {
        let dir = std::env::temp_dir().join(format!("mscreenshot-latest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(latest_png(&dir).unwrap(), None);

        let old = dir.join("a.png");
        let new = dir.join("b.png");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(&new, b"x").unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();
        assert_eq!(latest_png(&dir).unwrap(), Some(new));

        assert!(latest_png(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_basename_handles_bare_and_rootless_paths() {
        assert_eq!(file_basename(Path::new("/a/b/shot.png")), "shot.png");
        assert_eq!(file_basename(Path::new("shot.png")), "shot.png");
        assert_eq!(file_basename(Path::new("/")), "");
    }
}
