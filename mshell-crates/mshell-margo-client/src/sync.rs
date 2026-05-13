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

use crate::state_json::{lowest_tag, monitor_id, read_raw, RawClient, StateJson};
use crate::{
    Address, Client, ClientLocation, ClientSize, FullscreenMode, HyprlandService, Monitor,
    Reactive, Workspace, WorkspaceInfo,
};

const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Spawn the background poll loop. The task holds only a `Weak`
/// reference to the service so the service still drops cleanly.
///
/// We compare the raw `state.json` bytes against the last applied
/// snapshot and skip the apply path entirely when they match.
/// `apply()` rebuilds every `Workspace` / `Monitor` / `Client` `Arc`
/// from scratch and calls `Reactive::set(new_vec)`; the `Reactive`
/// notification fires on every `set` regardless of whether the
/// underlying value changed (Vec<Arc<_>> equality is by pointer, so
/// even an unchanged snapshot looks "new"). Without this short-
/// circuit, the bar's downstream reactive subscribers (focused-tag
/// pill, dock, layout) all repaint four times a second, and the
/// repaint coalesces visibly with frequent state.json writes on the
/// margo side (window title changes during typing, focus updates,
/// dwl-ipc broadcasts) — the user sees the bar flickering on every
/// keystroke in another window.
pub(crate) fn spawn(service: &Arc<HyprlandService>) {
    let weak = Arc::downgrade(service);
    tokio::spawn(async move {
        let mut ticker = interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_raw: Option<String> = None;
        loop {
            ticker.tick().await;
            let Some(service) = weak.upgrade() else { break };
            let Some(raw) = read_raw() else { continue };
            if last_raw.as_deref() == Some(&raw) {
                continue;
            }
            let parsed: Option<StateJson> = serde_json::from_str(&raw).ok();
            if let Some(state) = parsed {
                apply(&service, &state);
                last_raw = Some(raw);
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

/// Project a snapshot onto the service's reactive properties **with
/// stable Arc identity**. This is the critical bit that distinguishes
/// the polling path from "rebuild the world four times a second":
///
/// * Walk the existing `Vec<Arc<Workspace>>` and try to match by
///   `id.get()` against the new snapshot's tag-derived ID.
///   - Match → update the matched Arc's `Reactive<_>` fields in
///     place. `Reactive::set` is a no-op-broadcast if you don't
///     end up changing the value, so a tag whose
///     `windows` count didn't change won't ripple a notification
///     down to its subscribers.
///   - No match → allocate a new `Arc<Workspace>`.
/// * Only call `service.workspaces.set(new_vec)` when the *membership*
///   actually changed (length differs or at least one Arc is new), so
///   reactive subscribers of the *vec itself* don't repaint on every
///   poll. Same dance for monitors / clients.
///
/// Without this, every state.json poll allocated fresh
/// `Arc<Workspace>` / `Arc<Monitor>` / `Arc<Client>` instances even
/// when state was identical, and `Reactive::set` broadcast the new
/// `Vec<Arc<_>>` to every subscriber. mshell's bar widgets — which
/// subscribe to all three — repainted from scratch four times a
/// second, and the resulting GTK damage commits hit margo's render
/// path faster than the monitor refresh rate. The user perceived
/// this as the "every-keystroke" bar flicker that mshell on Hyprland
/// (wayle-hyprland mutates Arcs in place) and on niri does not have.
fn apply(service: &HyprlandService, state: &StateJson) {
    // ── Workspaces ───────────────────────────────────────────────────
    let current_ws = service.workspaces.get();
    let mut next_ws: Vec<Arc<Workspace>> = Vec::with_capacity(state.tag_count as usize);
    let mut workspace_by_id: HashMap<i64, Arc<Workspace>> = HashMap::new();

    for tag in 1..=state.tag_count {
        let ws_id = tag as i64;
        let bit = 1u32 << (tag - 1);
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
        let tiled_layout = state
            .outputs
            .iter()
            .find(|o| o.name == owner.0)
            .and_then(|o| state.layouts.get(o.layout_idx).cloned())
            .unwrap_or_default();

        let ws = if let Some(existing) = current_ws.iter().find(|w| w.id.get() == ws_id) {
            // Reuse — only `set` fields that changed; `Reactive::set`
            // always broadcasts on call, so we guard each field by
            // comparing against the current snapshot first.
            if existing.monitor.get() != owner.0 {
                existing.monitor.set(owner.0.clone());
            }
            if existing.monitor_id.get() != Some(owner.1) {
                existing.monitor_id.set(Some(owner.1));
            }
            if existing.windows.get() != windows {
                existing.windows.set(windows);
            }
            if existing.fullscreen.get() != fullscreen {
                existing.fullscreen.set(fullscreen);
            }
            if existing.last_window.get() != last_window {
                existing.last_window.set(last_window.clone());
            }
            if existing.last_window_title.get() != last_window_title {
                existing.last_window_title.set(last_window_title.clone());
            }
            if existing.tiled_layout.get() != tiled_layout {
                existing.tiled_layout.set(tiled_layout.clone());
            }
            Arc::clone(existing)
        } else {
            Arc::new(Workspace {
                id: Reactive::new(ws_id),
                name: Reactive::new(format!("{tag}")),
                monitor: Reactive::new(owner.0.clone()),
                monitor_id: Reactive::new(Some(owner.1)),
                windows: Reactive::new(windows),
                fullscreen: Reactive::new(fullscreen),
                last_window: Reactive::new(last_window),
                last_window_title: Reactive::new(last_window_title),
                persistent: Reactive::new(true),
                tiled_layout: Reactive::new(tiled_layout),
            })
        };
        workspace_by_id.insert(ws_id, Arc::clone(&ws));
        next_ws.push(ws);
    }
    if vec_membership_differs(&current_ws, &next_ws) {
        service.workspaces.set(next_ws);
    }

    // ── Monitors ─────────────────────────────────────────────────────
    let current_mons = service.monitors.get();
    let mut next_mons: Vec<Arc<Monitor>> = Vec::with_capacity(state.outputs.len());
    for o in &state.outputs {
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

        let mon = if let Some(existing) = current_mons.iter().find(|m| m.id.get() == mid) {
            if existing.name.get() != o.name {
                existing.name.set(o.name.clone());
                existing.description.set(o.name.clone());
            }
            if existing.width.get() != o.width as u32 {
                existing.width.set(o.width as u32);
            }
            if existing.height.get() != o.height as u32 {
                existing.height.set(o.height as u32);
            }
            if (existing.refresh_rate.get() - refresh_rate).abs() > f32::EPSILON {
                existing.refresh_rate.set(refresh_rate);
            }
            if existing.x.get() != o.x {
                existing.x.set(o.x);
            }
            if existing.y.get() != o.y {
                existing.y.set(o.y);
            }
            if existing.active_workspace.get() != ws_info {
                existing.active_workspace.set(ws_info.clone());
                existing.special_workspace.set(ws_info.clone());
            }
            if (existing.scale.get() - o.scale).abs() > f32::EPSILON {
                existing.scale.set(o.scale);
            }
            if existing.focused.get() != o.active {
                existing.focused.set(o.active);
            }
            Arc::clone(existing)
        } else {
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
                special_workspace: Reactive::new(ws_info),
                scale: Reactive::new(o.scale),
                focused: Reactive::new(o.active),
                dpms_status: Reactive::new(true),
                vrr: Reactive::new(false),
            })
        };
        next_mons.push(mon);
    }
    if vec_membership_differs(&current_mons, &next_mons) {
        service.monitors.set(next_mons);
    }

    // ── Clients ──────────────────────────────────────────────────────
    let current_clients = service.clients.get();
    let mut next_clients: Vec<Arc<Client>> = Vec::with_capacity(state.clients.len());
    for c in &state.clients {
        let addr = client_address(c);
        let new_client = if let Some(existing) = current_clients
            .iter()
            .find(|cl| cl.address.get() == addr)
        {
            update_client_in_place(existing, c, state);
            Arc::clone(existing)
        } else {
            build_client(c, state, &workspace_by_id)
        };
        next_clients.push(new_client);
    }
    if vec_membership_differs(&current_clients, &next_clients) {
        service.clients.set(next_clients);
    }
}

/// True when the two Vecs have different Arcs (different lengths or
/// at least one position holds a fresh Arc — `Arc::ptr_eq` is the
/// identity check). Used so `Reactive::set` only fires when membership
/// actually changed; per-field updates flow through each Arc's own
/// `Reactive` cells.
fn vec_membership_differs<T>(a: &[Arc<T>], b: &[Arc<T>]) -> bool {
    if a.len() != b.len() {
        return true;
    }
    a.iter().zip(b.iter()).any(|(x, y)| !Arc::ptr_eq(x, y))
}

/// In-place per-field updater for an already-mapped `Client` Arc.
/// Each `set` is guarded against the current value so unchanged
/// fields don't ripple notifications.
fn update_client_in_place(client: &Client, c: &RawClient, state: &StateJson) {
    let mapped = !c.minimized;
    if client.mapped.get() != mapped {
        client.mapped.set(mapped);
    }
    let hidden = c.minimized || c.scratchpad;
    if client.hidden.get() != hidden {
        client.hidden.set(hidden);
    }
    let at = ClientLocation { x: c.x, y: c.y };
    if client.at.get() != at {
        client.at.set(at);
    }
    let size = ClientSize {
        width: c.width,
        height: c.height,
    };
    if client.size.get() != size {
        client.size.set(size);
    }
    let ws_id = lowest_tag(c.tags) as i64;
    let ws_info = WorkspaceInfo {
        id: ws_id,
        name: if ws_id == 0 {
            String::new()
        } else {
            ws_id.to_string()
        },
    };
    if client.workspace.get() != ws_info {
        client.workspace.set(ws_info);
    }
    if client.floating.get() != c.floating {
        client.floating.set(c.floating);
    }
    let mid = monitor_id(&c.monitor);
    if client.monitor.get() != mid {
        client.monitor.set(mid);
    }
    if client.class.get() != c.app_id {
        client.class.set(c.app_id.clone());
    }
    if client.title.get() != c.title {
        client.title.set(c.title.clone());
    }
    if client.pid.get() != c.pid {
        client.pid.set(c.pid);
    }
    if client.pinned.get() != c.global {
        client.pinned.set(c.global);
    }
    let fullscreen_mode = if c.fullscreen {
        FullscreenMode::Fullscreen
    } else {
        FullscreenMode::None
    };
    if client.fullscreen.get() != fullscreen_mode {
        client.fullscreen.set(fullscreen_mode);
        client.fullscreen_client.set(fullscreen_mode);
    }
    let focus_history_id = state
        .outputs
        .iter()
        .find(|o| o.name == c.monitor)
        .and_then(|o| o.focus_history.iter().position(|app| app == &c.app_id))
        .map(|p| p as i32)
        .unwrap_or(-1);
    if client.focus_history_id.get() != focus_history_id {
        client.focus_history_id.set(focus_history_id);
    }
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
