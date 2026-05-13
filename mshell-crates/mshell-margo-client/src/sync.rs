//! Background poll loop that turns margo's `state.json` snapshot
//! stream into reactive property updates on a [`HyprlandService`].
//!
//! state.json is rewritten by margo on every meaningful change
//! (focus / tag / arrange / hotplug / config reload), so polling
//! at a steady cadence is sufficient to drive the bar widgets —
//! we don't need an inotify watcher. 250 ms balances UI
//! responsiveness against syscall cost; a tag-switch animation
//! at 60 fps lasts ~280 ms, so a single poll-tick is guaranteed
//! to surface a focus change inside one animation frame.
//!
//! The loop runs forever; ownership cleanup happens when the
//! `Arc<HyprlandService>` is dropped (the closure captures a
//! `Weak<HyprlandService>` and exits as soon as the upgrade
//! fails).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::interval;

use crate::state_json::{lowest_tag, monitor_id, read, RawClient, StateJson};
use crate::{
    Address, Client, ClientLocation, ClientSize, FullscreenMode, HyprlandService, Monitor,
    Reactive, Workspace, WorkspaceInfo,
};

const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Spawn the background poll loop. The task holds only a `Weak`
/// reference to the service so the service still drops cleanly.
pub(crate) fn spawn(service: &Arc<HyprlandService>) {
    let weak = Arc::downgrade(service);
    tokio::spawn(async move {
        let mut ticker = interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let Some(service) = weak.upgrade() else { break };
            if let Some(state) = read() {
                apply(&service, &state);
            }
        }
    });
}

/// Project a freshly-deserialized [`StateJson`] onto the service's
/// reactive properties. Idempotent — re-running with an identical
/// state should be a no-op as far as widget subscribers are
/// concerned (the underlying `Reactive::set` always broadcasts,
/// but consumers all `.get()`-snapshot every render, so a duplicate
/// notification is harmless).
pub(crate) fn apply_snapshot(service: &HyprlandService, state: &StateJson) {
    apply(service, state);
}

fn apply(service: &HyprlandService, state: &StateJson) {
    // ── Build the workspace fleet ─────────────────────────────────
    //
    // margo has a fixed 9 tags. Each tag becomes a `Workspace` with
    // ID = bit position + 1 (so tag 1 → ws 1, tag 9 → ws 9). The
    // workspace's `monitor` field reads "first output whose
    // active_tag_mask has the bit set"; this matches the upstream
    // semantic of "monitor that owns this workspace right now".
    let mut workspaces: Vec<Arc<Workspace>> = Vec::with_capacity(state.tag_count as usize);
    let mut workspace_by_id: HashMap<i64, Arc<Workspace>> = HashMap::new();

    for tag in 1..=state.tag_count {
        let ws_id = tag as i64;
        let bit = 1u32 << (tag - 1);
        // Owner monitor: first output that has this tag active OR
        // occupied; fall back to active_output.
        let owner = state
            .outputs
            .iter()
            .find(|o| (o.active_tag_mask | o.occupied_tag_mask) & bit != 0)
            .map(|o| (o.name.clone(), monitor_id(&o.name)))
            .unwrap_or_else(|| {
                (
                    state.active_output.clone(),
                    monitor_id(&state.active_output),
                )
            });
        // Window count on this tag (every client whose `tags` mask
        // includes our bit).
        let windows: u16 = state
            .clients
            .iter()
            .filter(|c| c.tags & bit != 0 && !c.minimized)
            .count()
            .min(u16::MAX as usize) as u16;
        let fullscreen = state
            .clients
            .iter()
            .any(|c| c.tags & bit != 0 && c.fullscreen);
        // Last-window: latest focus_history entry on the owner
        // monitor whose tag mask includes this bit.
        let last = state
            .outputs
            .iter()
            .find(|o| o.name == owner.0)
            .and_then(|o| {
                o.focus_history.iter().find_map(|app_id| {
                    state
                        .clients
                        .iter()
                        .find(|c| c.app_id == *app_id && c.tags & bit != 0)
                })
            });
        let last_window: Option<Address> = last.map(|c| client_address(c));
        let last_window_title: String = last.map(|c| c.title.clone()).unwrap_or_default();

        let ws = Arc::new(Workspace {
            id: Reactive::new(ws_id),
            name: Reactive::new(format!("{tag}")),
            monitor: Reactive::new(owner.0.clone()),
            monitor_id: Reactive::new(Some(owner.1)),
            windows: Reactive::new(windows),
            fullscreen: Reactive::new(fullscreen),
            last_window: Reactive::new(last_window),
            last_window_title: Reactive::new(last_window_title),
            persistent: Reactive::new(true),
            tiled_layout: Reactive::new(
                state
                    .outputs
                    .iter()
                    .find(|o| o.name == owner.0)
                    .and_then(|o| state.layouts.get(o.layout_idx).cloned())
                    .unwrap_or_default(),
            ),
        });
        workspace_by_id.insert(ws_id, Arc::clone(&ws));
        workspaces.push(ws);
    }
    service.workspaces.set(workspaces);

    // ── Build the monitor fleet ───────────────────────────────────
    let monitors: Vec<Arc<Monitor>> = state
        .outputs
        .iter()
        .map(|o| {
            let mid = monitor_id(&o.name);
            let active_tag = lowest_tag(o.active_tag_mask);
            let ws_info = WorkspaceInfo {
                id: active_tag as i64,
                name: if active_tag == 0 {
                    String::new()
                } else {
                    active_tag.to_string()
                },
            };
            let refresh_rate = o
                .mode
                .as_ref()
                .map(|m| m.refresh_mhz as f32 / 1000.0)
                .unwrap_or(60.0);
            Arc::new(Monitor {
                id: Reactive::new(mid),
                name: Reactive::new(o.name.clone()),
                description: Reactive::new(o.name.clone()),
                make: Reactive::new(String::new()),
                model: Reactive::new(String::new()),
                serial: Reactive::new(String::new()),
                width: Reactive::new(o.width as u32),
                height: Reactive::new(o.height as u32),
                refresh_rate: Reactive::new(refresh_rate),
                x: Reactive::new(o.x),
                y: Reactive::new(o.y),
                active_workspace: Reactive::new(ws_info.clone()),
                // Margo has no notion of a "special" workspace —
                // give the same value as active so widgets that
                // read it never see uninitialised data.
                special_workspace: Reactive::new(ws_info),
                scale: Reactive::new(o.scale),
                focused: Reactive::new(o.active),
                dpms_status: Reactive::new(true),
                vrr: Reactive::new(false),
            })
        })
        .collect();
    service.monitors.set(monitors);

    // ── Build the client fleet ────────────────────────────────────
    let clients: Vec<Arc<Client>> = state
        .clients
        .iter()
        .map(|c| build_client(c, state, &workspace_by_id))
        .collect();
    service.clients.set(clients);
}

/// Deterministic, stable Address for a margo client. Hyprland's
/// addresses are 64-bit hex strings; we synthesize one from the
/// client's monitor index + slot index (margo's `idx` field is
/// per-output, so combine with `monitor_idx` to disambiguate).
/// PID would be ideal but margo currently publishes pid = 0.
fn client_address(c: &RawClient) -> Address {
    Address::new(format!(
        "{:04x}{:08x}",
        (c.monitor_idx as u16),
        (c.idx as u32),
    ))
}

fn build_client(
    c: &RawClient,
    state: &StateJson,
    workspace_by_id: &HashMap<i64, Arc<Workspace>>,
) -> Arc<Client> {
    let ws_id = lowest_tag(c.tags) as i64;
    let ws_info = WorkspaceInfo {
        id: ws_id,
        name: if ws_id == 0 {
            String::new()
        } else {
            ws_id.to_string()
        },
    };
    // For `client.workspace.get() -> WorkspaceInfo` callers
    // (window_selector) — they read `.id` directly.
    let _ = workspace_by_id; // reserved for future use (workspace.last_window backref)

    let monitor_h = monitor_id(&c.monitor);
    let fullscreen_mode = if c.fullscreen {
        FullscreenMode::Fullscreen
    } else {
        FullscreenMode::None
    };

    // focus_history_id: position in the active output's
    // focus_history (0 = most-recent). -1 = not in history.
    let focus_history_id = state
        .outputs
        .iter()
        .find(|o| o.name == c.monitor)
        .and_then(|o| o.focus_history.iter().position(|app| app == &c.app_id))
        .map(|p| p as i32)
        .unwrap_or(-1);

    Arc::new(Client {
        address: Reactive::new(client_address(c)),
        mapped: Reactive::new(!c.minimized),
        hidden: Reactive::new(c.minimized || c.scratchpad),
        at: Reactive::new(ClientLocation { x: c.x, y: c.y }),
        size: Reactive::new(ClientSize {
            width: c.width,
            height: c.height,
        }),
        workspace: Reactive::new(ws_info),
        floating: Reactive::new(c.floating),
        monitor: Reactive::new(monitor_h),
        class: Reactive::new(c.app_id.clone()),
        title: Reactive::new(c.title.clone()),
        initial_class: Reactive::new(c.app_id.clone()),
        initial_title: Reactive::new(c.title.clone()),
        pid: Reactive::new(c.pid),
        xwayland: Reactive::new(false),
        pinned: Reactive::new(c.global),
        fullscreen: Reactive::new(fullscreen_mode),
        fullscreen_client: Reactive::new(fullscreen_mode),
        over_fullscreen: Reactive::new(false),
        grouped: Reactive::new(Vec::new()),
        tags: Reactive::new(Vec::new()),
        swallowing: Reactive::new(None),
        focus_history_id: Reactive::new(focus_history_id),
        inhibiting_idle: Reactive::new(false),
        xdg_tag: Reactive::new(None),
        xdg_description: Reactive::new(None),
        stable_id: Reactive::new(c.app_id.clone()),
    })
}
