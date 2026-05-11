//! margo compositor backend.
//!
//! Event channel is the atomic `state.json` snapshot margo rewrites
//! every time compositor state changes. We watch the file with
//! inotify; each `IN_MOVED_TO` (margo writes via tmp-rename) triggers
//! a re-read + broadcast of a fresh `CompositorState`.
//!
//! Dispatch (commands TO the compositor) goes through
//! `mctl dispatch …` as a subprocess. mctl is a workspace sibling
//! so the binary is always co-located; we don't open a second IPC
//! layer just to call back into mctl's existing one.
//!
//! State model — margo is tag-based. We map margo tags 1..=9 to
//! ashell-style "workspaces" so the existing workspaces module
//! renders without modification:
//!
//!   * `id` / `index` = tag number (1..=9)
//!   * `name` = "1".."9"
//!   * `monitor` = output name
//!   * `windows` = count of clients on that tag for that monitor
//!
//! "Active workspace" maps to the lowest-bit set in the active
//! tagmask; multi-bit tagmasks (margo lets you view several tags
//! simultaneously) show the lowest as active.

use super::types::{
    ActiveWindow, CompositorCommand, CompositorEvent, CompositorMonitor, CompositorService,
    CompositorState, CompositorWorkspace,
};
use crate::services::ServiceEvent;
use anyhow::{Context, Result, anyhow};
use inotify::{Inotify, WatchMask};
use serde_json::Value;
use std::{
    env,
    path::PathBuf,
    process::Stdio,
    time::{Duration, SystemTime},
};
use tokio::{process::Command, sync::broadcast, time::sleep};
use tokio_stream::StreamExt;

/// `$XDG_RUNTIME_DIR/margo/state.json` — same path
/// `margo/src/state.rs::state_file_path()` writes to.
fn state_file_path() -> PathBuf {
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    base.join("margo").join("state.json")
}

pub fn is_available() -> bool {
    if state_file_path().exists() {
        return true;
    }
    env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.eq_ignore_ascii_case("margo"))
        .unwrap_or(false)
}

pub async fn execute_command(cmd: CompositorCommand) -> Result<()> {
    // Translate margo-shell's compositor command vocabulary into
    // mctl dispatch invocations. mctl is a workspace sibling so we
    // just shell out and let it forward into margo's dwl-ipc
    // dispatch table; that keeps the surface honest — anything you
    // can do from a keybind, you can do here, and vice-versa.
    let (action, args): (String, Vec<String>) = match cmd {
        // margo tags are 1-indexed bitmasks: tag N == 1 << (N - 1).
        // `mctl dispatch view <mask>` switches the active monitor's
        // tagset to that mask.
        CompositorCommand::FocusWorkspace(id) => {
            if !(1..=9).contains(&id) {
                return Err(anyhow!(
                    "Margo backend: workspace id {id} out of range (expected 1..=9 = tags)"
                ));
            }
            let mask = 1u32 << (id as u32 - 1);
            ("view".into(), vec![mask.to_string()])
        }
        CompositorCommand::ScrollWorkspace(dir) => {
            // Margo has `viewrelative` — shift the active tagset by
            // ±1 along the 1..=9 ring.
            ("viewrelative".into(), vec![dir.to_string()])
        }
        CompositorCommand::NextLayout => ("switchlayout".into(), Vec::new()),
        CompositorCommand::CustomDispatch(action, args) => {
            // Most commonly used for `spawn`. Margo's spawn slot is
            // the 4th positional, with 3 empty filler args ahead of
            // it; mctl knows the slot mapping so just forward.
            if action == "spawn" {
                (
                    "spawn".into(),
                    vec![String::new(), String::new(), String::new(), args],
                )
            } else {
                let owned_args = if args.is_empty() {
                    Vec::new()
                } else {
                    vec![args]
                };
                (action, owned_args)
            }
        }
    };

    let mut cmd_builder = Command::new("mctl");
    cmd_builder.arg("dispatch").arg(&action);
    for a in &args {
        cmd_builder.arg(a);
    }
    let status = cmd_builder
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("spawning mctl")?;
    if !status.success() {
        return Err(anyhow!(
            "mctl dispatch {action} {args:?} failed (exit code {:?})",
            status.code()
        ));
    }
    Ok(())
}

pub async fn run_listener(
    tx: &broadcast::Sender<ServiceEvent<CompositorService>>,
) -> Result<()> {
    let path = state_file_path();
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("state.json has no parent dir: {}", path.display()))?
        .to_path_buf();

    // inotify-watch the *directory*. margo writes via tmp-rename
    // ($XDG_RUNTIME_DIR/margo/state.tmp.<pid> → state.json), so a
    // simple file-level IN_MODIFY misses the swap; IN_MOVED_TO on
    // the parent dir catches the rename atomically.
    let inotify = Inotify::init().context("inotify init")?;
    inotify
        .watches()
        .add(
            &dir,
            WatchMask::MOVED_TO | WatchMask::CREATE | WatchMask::MODIFY,
        )
        .with_context(|| format!("inotify add_watch {}", dir.display()))?;

    // Send one snapshot immediately so the bar isn't blank during
    // the first idle period (state.json may not change for several
    // seconds in steady state).
    if let Ok(state) = read_state(&path).await {
        let svc = CompositorService { state };
        let _ = tx.send(ServiceEvent::Init(svc));
    }

    let mut buffer = [0u8; 1024];
    let mut stream = inotify.into_event_stream(&mut buffer)?;
    let mut last_emit = SystemTime::UNIX_EPOCH;

    while let Some(event) = stream.next().await {
        let event = event.context("inotify stream")?;
        let Some(name) = event.name else { continue };
        if name != "state.json" {
            continue;
        }
        // Debounce: margo writes the file on every focus/tag/border
        // tick. inotify will deliver an event per write; collapse
        // bursts within 50 ms into one broadcast to avoid spamming
        // the UI thread when the user holds a key.
        if let Ok(elapsed) = last_emit.elapsed() {
            if elapsed < Duration::from_millis(50) {
                sleep(Duration::from_millis(50) - elapsed).await;
            }
        }
        match read_state(&path).await {
            Ok(state) => {
                let _ = tx.send(ServiceEvent::Update(CompositorEvent::StateChanged(
                    Box::new(state),
                )));
                last_emit = SystemTime::now();
            }
            Err(e) => {
                log::warn!("margo: failed to read state.json: {e:#}");
            }
        }
    }
    Err(anyhow!("inotify stream ended unexpectedly"))
}

async fn read_state(path: &PathBuf) -> Result<CompositorState> {
    let raw = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let json: Value = serde_json::from_slice(&raw).context("parse state.json")?;
    Ok(state_from_json(&json))
}

/// Translate one `state.json` snapshot into the
/// `CompositorState`. We synthesize one workspace per tag bit
/// (1..=9) per monitor; window counts come from the clients array.
fn state_from_json(json: &Value) -> CompositorState {
    let outputs = json
        .get("outputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let clients_json = json
        .get("clients")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let focused_idx = json.get("focused_idx").and_then(Value::as_i64);

    let mut workspaces: Vec<CompositorWorkspace> = Vec::new();
    let mut monitors: Vec<CompositorMonitor> = Vec::new();
    let mut active_workspace_id: Option<i32> = None;
    let mut active_window: Option<ActiveWindow> = None;
    let mut wallpapers: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for (mon_idx, out) in outputs.iter().enumerate() {
        let mon_name = out
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let active_mask = out
            .get("active_tag_mask")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let active_output = out
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Per-output wallpaper (active tag's path); empty string means
        // "no wallpaper set". margo rewrites state.json on every tag
        // change, so this map updates automatically with margo's
        // per-tag wallpaper rules.
        if let Some(wp) = out.get("wallpaper").and_then(Value::as_str) {
            wallpapers.insert(mon_name.clone(), wp.to_string());
        }

        let lowest = if active_mask == 0 {
            0
        } else {
            active_mask.trailing_zeros() + 1
        };
        let mon_id = mon_idx as i128;
        for tag in 1..=9u32 {
            let bit = 1u32 << (tag - 1);
            let bit_set = (active_mask & bit) != 0;
            let window_count = clients_json
                .iter()
                .filter(|c| {
                    c.get("monitor_idx").and_then(Value::as_i64) == Some(mon_idx as i64)
                        && c.get("tags").and_then(Value::as_u64).unwrap_or(0) as u32 & bit != 0
                })
                .count() as u16;
            workspaces.push(CompositorWorkspace {
                id: tag as i32,
                index: tag as i32,
                name: tag.to_string(),
                monitor: mon_name.clone(),
                monitor_id: Some(mon_id),
                windows: window_count,
            });
            if bit_set && active_output && tag == lowest {
                active_workspace_id = Some(tag as i32);
            }
        }

        monitors.push(CompositorMonitor {
            id: mon_id,
            name: mon_name.clone(),
            active_workspace_id: lowest as i32,
        });
    }

    if let Some(idx) = focused_idx {
        if let Some(client) = clients_json.get(idx as usize) {
            active_window = Some(ActiveWindow {
                title: client
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                class: client
                    .get("app_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                address: client
                    .get("idx")
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            });
        }
    }

    CompositorState {
        workspaces,
        monitors,
        active_workspace_id,
        active_window,
        keyboard_layout: String::new(),
        wallpapers,
    }
}
