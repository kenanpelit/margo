//! Reactive sync layer — turns margo's pushed state frames into
//! reactive property updates on a [`MargoService`].
//!
//! The shell subscribes to `watch state` on margo's IPC socket
//! (`$MARGO_SOCKET` / `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`). margo
//! pushes a full snapshot frame on every meaningful change (focus /
//! tag / arrange / hotplug / config reload), so the shell updates
//! event-driven with zero idle wakeups and no file polling. The task
//! reconnects with a short backoff so it survives a compositor
//! restart, and holds only a `Weak` reference so the service still
//! drops cleanly when the last `Arc<MargoService>` is released.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::state_json::{RawClient, StateJson, lowest_tag, monitor_id, socket_path};
use crate::{
    Address, Client, ClientLocation, ClientSize, FullscreenMode, MargoEvent, MargoService, Monitor,
    Reactive, Workspace, WorkspaceInfo,
};

/// Backoff between socket reconnect attempts.
const RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// Spawn the reactive sync task: a `watch state` subscription on
/// margo's IPC socket. The task holds only a `Weak` reference to the
/// service so it still drops cleanly when the last `Arc<MargoService>`
/// is released. Frames arrive only on real state changes (margo pushes
/// on its dirty-flush), so there is no polling and `apply` runs exactly
/// as often as the compositor changes.
pub(crate) fn spawn(service: &Arc<MargoService>) {
    let weak = Arc::downgrade(service);
    tokio::spawn(async move {
        // Subscribe to the compositor's IPC socket with `watch state`:
        // margo pushes a full snapshot frame on every change (focus /
        // tag / arrange / hotplug / config reload), so there is no
        // polling and no idle wakeups. Reconnect with a short backoff
        // so the shell survives a compositor restart.
        loop {
            if weak.upgrade().is_none() {
                break;
            }
            let stream = match tokio::net::UnixStream::connect(socket_path()).await {
                Ok(s) => s,
                Err(_) => {
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
            };
            let (rd, mut wr) = stream.into_split();
            if wr.write_all(b"watch state\n").await.is_err() {
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
            let mut lines = BufReader::new(rd).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                if let Ok(state) = serde_json::from_str::<StateJson>(&line) {
                    apply(&service, &state);
                }
            }
            // Connection closed (compositor exit / restart) — back off
            // and reconnect.
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
}

/// One-shot `get state` over the socket — used to prime the service on
/// startup so widgets see populated state on their first paint. Returns
/// `None` when margo isn't running yet (the `watch` task will fill in
/// once it connects).
pub(crate) async fn fetch_state_once() -> Option<StateJson> {
    let stream = tokio::net::UnixStream::connect(socket_path()).await.ok()?;
    let (rd, mut wr) = stream.into_split();
    wr.write_all(b"get state\n").await.ok()?;
    let mut lines = BufReader::new(rd).lines();
    let line = lines.next_line().await.ok()??;
    serde_json::from_str::<StateJson>(&line).ok()
}

/// Project a freshly-deserialized [`StateJson`] onto the service's
/// reactive properties. Idempotent — re-running with an identical
/// state should be a no-op as far as widget subscribers are
/// concerned (the underlying `Reactive::set` always broadcasts,
/// but consumers all `.get()`-snapshot every render, so a duplicate
/// notification is harmless).
pub(crate) fn apply_snapshot(service: &MargoService, state: &StateJson) {
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
fn apply(service: &MargoService, state: &StateJson) {
    // Track which entities were added / removed / focus-changed so we
    // can emit synthetic typed events on `service.event_tx`. The
    // OkShell widget watchers expect those events
    // (`MargoEvent::WorkspaceV2`, `CreateWorkspaceV2`, `OpenWindow`,
    // …) and would otherwise sit forever on `events.next().await`
    // because the poll loop only pushes through `Reactive::set`.
    let mut emitted: Vec<MargoEvent> = Vec::new();

    // Mirror the active keyboard-layout name (change-only — `set`
    // broadcasts unconditionally, so guard on an actual change).
    if service.keyboard_layout.get() != state.keyboard_layout {
        service.keyboard_layout.set(state.keyboard_layout.clone());
    }

    let prev_focused_tag: Option<i64> = service
        .monitors
        .get()
        .iter()
        .find(|m| m.focused.get())
        .map(|m| m.active_workspace.get().id);

    // ── Workspaces ───────────────────────────────────────────────────
    let current_ws = service.workspaces.get();
    let mut next_ws: Vec<Arc<Workspace>> = Vec::with_capacity(state.tag_count as usize);
    let mut workspace_by_id: HashMap<i64, Arc<Workspace>> = HashMap::new();

    // Per-tag window count + fullscreen flag, folded in one pass over clients
    // (walking only each client's set tag-bits) instead of two full O(clients)
    // scans per tag inside the loop below. This is the O(tags × clients) tag
    // math the loop used to do; now it is ~O(clients).
    let tag_count = state.tag_count as usize;
    let tag_mask: u32 = if tag_count >= 32 {
        u32::MAX
    } else {
        (1u32 << tag_count as u32) - 1
    };
    let mut windows_per_tag = vec![0u16; tag_count + 1];
    let mut fullscreen_per_tag = vec![false; tag_count + 1];
    for c in &state.clients {
        let mut bits = c.tags & tag_mask;
        while bits != 0 {
            let idx = (bits.trailing_zeros() + 1) as usize;
            bits &= bits - 1;
            if !c.minimized {
                windows_per_tag[idx] = windows_per_tag[idx].saturating_add(1);
            }
            if c.fullscreen {
                fullscreen_per_tag[idx] = true;
            }
        }
    }

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
        let windows = windows_per_tag[tag as usize];
        let fullscreen = fullscreen_per_tag[tag as usize];
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
        let last_window: Option<Address> = last.map(client_address);
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
    // Workspace lifecycle events (Create / Destroy) so widget watchers
    // light up. Compare ID sets between prev/next; emit per added or
    // removed tag. mshell config locks `tag_count = 9`, so in steady
    // state these only fire on the very first apply (current_ws is
    // empty → all 9 fire `CreateWorkspaceV2`).
    {
        use std::collections::BTreeSet;
        let prev_ids: BTreeSet<i64> = current_ws.iter().map(|w| w.id.get()).collect();
        let next_ids: BTreeSet<i64> = next_ws.iter().map(|w| w.id.get()).collect();
        for id in next_ids.difference(&prev_ids) {
            emitted.push(MargoEvent::CreateWorkspaceV2 {
                id: *id,
                name: id.to_string(),
            });
        }
        for id in prev_ids.difference(&next_ids) {
            emitted.push(MargoEvent::DestroyWorkspaceV2 {
                id: *id,
                name: id.to_string(),
            });
        }
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
    // Monitor hotplug events.
    {
        use std::collections::BTreeSet;
        let prev_ids: BTreeSet<i64> = current_mons.iter().map(|m| m.id.get()).collect();
        let next_ids: BTreeSet<i64> = next_mons.iter().map(|m| m.id.get()).collect();
        for id in next_ids.difference(&prev_ids) {
            let name = next_mons
                .iter()
                .find(|m| m.id.get() == *id)
                .map(|m| m.name.get())
                .unwrap_or_default();
            emitted.push(MargoEvent::MonitorAddedV2 {
                id: *id,
                name: name.clone(),
                description: name,
            });
        }
        for id in prev_ids.difference(&next_ids) {
            let name = current_mons
                .iter()
                .find(|m| m.id.get() == *id)
                .map(|m| m.name.get())
                .unwrap_or_default();
            emitted.push(MargoEvent::MonitorRemovedV2 { id: *id, name });
        }
    }
    if vec_membership_differs(&current_mons, &next_mons) {
        service.monitors.set(next_mons);
    }

    // Active workspace change — emit `WorkspaceV2` so MargoTags
    // ActiveWorkspaceChanged arm fires when the user tag-switches.
    let new_focused_tag: Option<i64> = service
        .monitors
        .get()
        .iter()
        .find(|m| m.focused.get())
        .map(|m| m.active_workspace.get().id);
    if new_focused_tag != prev_focused_tag
        && let Some(id) = new_focused_tag
    {
        emitted.push(MargoEvent::WorkspaceV2 {
            id,
            name: id.to_string(),
        });
    }

    // ── Clients ──────────────────────────────────────────────────────
    let current_clients = service.clients.get();
    // Index current clients by address so the per-client reuse lookup below is
    // O(1) rather than a linear scan — the whole pass was O(clients²), the part
    // that bites with many windows + frequent title/focus churn. Addresses are
    // unique per window, so the map is 1:1 with `current_clients`.
    let current_by_address: HashMap<Address, &Arc<Client>> = current_clients
        .iter()
        .map(|c| (c.address.get(), c))
        .collect();
    let mut next_clients: Vec<Arc<Client>> = Vec::with_capacity(state.clients.len());
    for c in &state.clients {
        let addr = client_address(c);
        let new_client = if let Some(&existing) = current_by_address.get(&addr) {
            update_client_in_place(existing, c, state);
            Arc::clone(existing)
        } else {
            build_client(c, state, &workspace_by_id)
        };
        next_clients.push(new_client);
    }
    // Client lifecycle + active-window events. mshell-port's
    // MargoDock watcher listens for `ActiveWindowV2`; the dock list
    // itself reads via `clients.watch()` so we don't need per-client
    // OpenWindow / CloseWindow events but emit them anyway for
    // upstream-shape compatibility.
    {
        use std::collections::HashSet;
        let prev_addrs: HashSet<Address> =
            current_clients.iter().map(|c| c.address.get()).collect();
        let next_addrs: HashSet<Address> = next_clients.iter().map(|c| c.address.get()).collect();
        for addr in next_addrs.difference(&prev_addrs) {
            if let Some(c) = next_clients.iter().find(|cl| cl.address.get() == *addr) {
                emitted.push(MargoEvent::OpenWindow {
                    address: addr.clone(),
                    workspace_name: c.workspace.get().name,
                    class: c.class.get(),
                    title: c.title.get(),
                });
            }
        }
        for addr in prev_addrs.difference(&next_addrs) {
            emitted.push(MargoEvent::CloseWindow {
                address: addr.clone(),
            });
        }
    }
    // Active window change.
    let prev_focused: Option<Address> = current_clients
        .iter()
        .find(|c| c.focus_history_id.get() == 0)
        .map(|c| c.address.get());
    let new_focused: Option<Address> = next_clients
        .iter()
        .find(|c| c.focus_history_id.get() == 0)
        .map(|c| c.address.get());
    if prev_focused != new_focused
        && let Some(addr) = new_focused
    {
        emitted.push(MargoEvent::ActiveWindowV2 { address: addr });
    }
    // Globally focused client — `state.json`'s `focused_idx`
    // indexes into `state.clients`, and `next_clients` is built in
    // that same order, so `next_clients[focused_idx]` is the one
    // focused window. Only re-publish when the focused *client*
    // actually changes (same Arc is reused across title edits, so
    // typing doesn't spuriously fire `focused_client`).
    let focused_client = state
        .focused_idx
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|idx| next_clients.get(idx).cloned());
    let focused_changed = match (service.focused_client.get(), &focused_client) {
        (Some(prev), Some(next)) => prev.address.get() != next.address.get(),
        (None, None) => false,
        _ => true,
    };
    if focused_changed {
        service.focused_client.set(focused_client);
    }
    if vec_membership_differs(&current_clients, &next_clients) {
        service.clients.set(next_clients);
    }

    // Broadcast all events at end (after all reactive sets, so
    // subscribers that `.get()`-snapshot on receipt see the final
    // state, not a half-applied one).
    if !emitted.is_empty() {
        let tx = &service.event_tx;
        for event in emitted {
            let _ = tx.send(event);
        }
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

/// Deterministic, **stable** Address for a margo client, derived from the
/// compositor's monotonic per-window `id`. The id is never reused within a
/// run, so — unlike the old `monitor_idx` + slot-`idx` synthesis — a window's
/// address survives another window closing (which shifts every later `idx`),
/// which is what keeps `Arc<Client>` identity and widget state from being lost
/// and avoids spurious close/open churn. Falls back to the positional
/// synthesis only when `id` is absent (`id == 0`: an older margo that predates
/// the snapshot field).
fn client_address(c: &RawClient) -> Address {
    if c.id != 0 {
        return Address::new(c.id.to_string());
    }
    Address::new(format!(
        "{:04x}{:08x}",
        (c.monitor_idx as u16),
        (c.idx as u32)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The core of the stable-id fix: a window's Address must depend only on
    /// its `id`, never on the positional slot `idx` — otherwise another window
    /// closing (which shifts every later `idx`) would silently re-address
    /// surviving windows, dropping their `Arc<Client>` identity and firing
    /// spurious close/open events.
    #[test]
    fn client_address_is_stable_across_slot_shift() {
        let at_slot = |id: u64, idx: i32| RawClient {
            id,
            idx,
            monitor_idx: 0,
            ..Default::default()
        };
        // Same window (same id), different slot → identical address.
        assert_eq!(
            client_address(&at_slot(5, 0)),
            client_address(&at_slot(5, 3))
        );
        // Distinct windows → distinct addresses.
        assert_ne!(
            client_address(&at_slot(5, 0)),
            client_address(&at_slot(6, 0))
        );
        // The address round-trips to the id for race-free `focuswindowid`.
        assert_eq!(client_address(&at_slot(42, 7)).margo_id(), Some(42));
    }

    /// Without a compositor id (older snapshot) the address falls back to the
    /// positional synthesis, so it stays non-empty and distinct per slot.
    #[test]
    fn client_address_falls_back_without_id() {
        let legacy = |monitor_idx: i32, idx: i32| RawClient {
            id: 0,
            monitor_idx,
            idx,
            ..Default::default()
        };
        assert_ne!(client_address(&legacy(0, 0)), client_address(&legacy(0, 1)));
    }
}
