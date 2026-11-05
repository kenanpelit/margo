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

use crate::state_json::{RawClient, RawOutput, StateJson, lowest_tag, monitor_id, socket_path};
use crate::{
    ActiveLayout, Address, Client, ClientLocation, ClientSize, FullscreenMode, MargoEvent,
    MargoService, Monitor, Reactive, Workspace, WorkspaceInfo,
};

/// Backoff between socket reconnect attempts.
const RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// While frames keep failing to parse, log every Nth failure after the first.
const LOG_EVERY: u64 = 100;

/// How much of a bad frame goes into the log line.
const EXCERPT_BYTES: usize = 200;

/// Counts consecutive frames that failed to parse, so a schema skew between
/// margo and mshell (every frame failing) is logged once and then every
/// [`LOG_EVERY`]th time instead of being dropped silently or flooding the log.
#[derive(Debug, Default)]
struct FrameErrors {
    consecutive: u64,
}

impl FrameErrors {
    /// A frame failed. Returns the failure count when this one should be
    /// logged.
    fn on_error(&mut self) -> Option<u64> {
        self.consecutive += 1;
        (self.consecutive == 1 || self.consecutive.is_multiple_of(LOG_EVERY))
            .then_some(self.consecutive)
    }

    /// A frame parsed. Returns true when this ends a run of failures.
    fn on_ok(&mut self) -> bool {
        std::mem::take(&mut self.consecutive) > 0
    }
}

/// The first [`EXCERPT_BYTES`] of a frame (cut on a char boundary).
fn frame_excerpt(line: &str) -> &str {
    let mut end = line.len().min(EXCERPT_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

fn degraded_tx() -> &'static tokio::sync::watch::Sender<bool> {
    static TX: std::sync::OnceLock<tokio::sync::watch::Sender<bool>> = std::sync::OnceLock::new();
    TX.get_or_init(|| tokio::sync::watch::Sender::new(false))
}

fn set_degraded(degraded: bool) {
    degraded_tx().send_if_modified(|d| std::mem::replace(d, degraded) != degraded);
}

/// `true` while margo's `watch state` frames fail to parse (the shell shows
/// the last good state). The shell toasts on the rising edge so a
/// margo/mshell version skew isn't a silently frozen bar.
pub fn sync_degraded() -> tokio::sync::watch::Receiver<bool> {
    degraded_tx().subscribe()
}

/// Spawn the reactive sync task: a `watch state` subscription on
/// margo's IPC socket. The task holds only a `Weak` reference to the
/// service so it still drops cleanly when the last `Arc<MargoService>`
/// is released. Frames arrive only on real state changes (margo pushes
/// on its dirty-flush), so there is no polling and `apply` runs exactly
/// as often as the compositor changes.
pub(crate) fn spawn(service: &Arc<MargoService>) {
    let weak = Arc::downgrade(service);
    tokio::spawn(async move {
        let mut errors = FrameErrors::default();
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
            tracing::debug!("margo sync: subscribed to `watch state`");
            let mut lines = BufReader::new(rd).lines();
            loop {
                let line = match lines.next_line().await {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!(error = %e, "margo sync: reading the state stream failed; reconnecting");
                        break;
                    }
                };
                let Some(service) = weak.upgrade() else {
                    return;
                };
                match serde_json::from_str::<StateJson>(&line) {
                    Ok(state) => {
                        if errors.on_ok() {
                            tracing::info!("margo sync: state frames parse again");
                            set_degraded(false);
                        }
                        apply(&service, &state);
                    }
                    Err(e) => {
                        if let Some(n) = errors.on_error() {
                            tracing::warn!(
                                error = %e,
                                failures = n,
                                frame = %frame_excerpt(&line),
                                "margo sync: state frame does not parse (margo/mshell version skew?); \
                                 the bar keeps the last good state"
                            );
                        }
                        set_degraded(true);
                    }
                }
            }
            // Connection closed (compositor exit / restart) — back off
            // and reconnect.
            tracing::debug!("margo sync: state stream closed; reconnecting");
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
    if service.session_locked.get() != state.session_locked {
        service.session_locked.set(state.session_locked);
    }
    if service.lock_keys.get() != state.lock_keys {
        service.lock_keys.set(state.lock_keys);
    }
    if service.internal_keyboard.get() != state.internal_keyboard {
        service
            .internal_keyboard
            .set(state.internal_keyboard.clone());
    }
    if service.keymode.get() != state.keymode {
        service.keymode.set(state.keymode.clone());
    }

    let fullscreen_outputs: Vec<String> = state
        .outputs
        .iter()
        .filter(|o| o.exclusive_fullscreen)
        .map(|o| o.name.clone())
        .collect();
    if service.fullscreen_outputs.get() != fullscreen_outputs {
        service.fullscreen_outputs.set(fullscreen_outputs);
    }

    // Mirror the focused output's scroller strip (change-only).
    let scroller = state
        .outputs
        .iter()
        .find(|o| o.name == state.active_output)
        .and_then(|o| o.scroller.clone());
    if service.scroller.get() != scroller {
        service.scroller.set(scroller);
    }

    // Mirror the scratchpad list (change-only).
    let scratchpads = crate::indicators::scratchpad_chips(&state.clients);
    if service.scratchpads.get() != scratchpads {
        service.scratchpads.set(scratchpads);
    }
    let minimized = crate::indicators::minimized_chips(&state.clients);
    if service.minimized.get() != minimized {
        service.minimized.set(minimized);
    }

    // Mirror the active tag's window list (change-only — a title edit on
    // another tag must not repaint the tag row's icons).
    let tag_windows = crate::indicators::active_tag_windows(state);
    if service.active_tag_windows.get() != tag_windows {
        service.active_tag_windows.set(tag_windows);
    }

    // Mirror the focused output's layout (change-only).
    let layout = active_layout(state);
    if service.active_layout.get() != layout {
        service.active_layout.set(layout);
    }

    // Mirror config diagnostics (change-only). This only actually
    // changes on a `mctl reload` — window/focus/tag activity leaves
    // `last_reload_diagnostics` untouched compositor-side — so
    // subscribers can treat a change here as "a reload just happened"
    // without needing a separate reload-specific event.
    if service.config_errors.get() != state.config_errors {
        service.config_errors.set(state.config_errors.clone());
    }

    let prev_focused_tag: Option<i64> = service
        .monitors
        .get()
        .iter()
        .find(|m| m.focused.get())
        .map(|m| m.active_workspace.get().id);

    // ── Workspaces ───────────────────────────────────────────────────
    // Tags are bits of a u32 mask, so at most 32 exist. Clamping keeps a
    // malformed frame from overflowing `1 << (tag - 1)` or allocating a
    // per-tag table sized by an arbitrary number.
    let tag_count_u32 = state.tag_count.min(u32::BITS);
    let current_ws = service.workspaces.get();
    let mut next_ws: Vec<Arc<Workspace>> = Vec::with_capacity(tag_count_u32 as usize);
    let mut workspace_by_id: HashMap<i64, Arc<Workspace>> = HashMap::new();

    // Per-tag window count + fullscreen flag, folded in one pass over clients
    // (walking only each client's set tag-bits) instead of two full O(clients)
    // scans per tag inside the loop below. This is the O(tags × clients) tag
    // math the loop used to do; now it is ~O(clients).
    let tag_count = tag_count_u32 as usize;
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

    for tag in 1..=tag_count_u32 {
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
            .and_then(|o| most_recent_client_on_tag(o, state, bit));
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

        let (logical_w, logical_h) = o.logical_size();
        let (logical_w, logical_h) = (logical_w.max(0) as u32, logical_h.max(0) as u32);
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
            if existing.logical_width.get() != logical_w {
                existing.logical_width.set(logical_w);
            }
            if existing.logical_height.get() != logical_h {
                existing.logical_height.set(logical_h);
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
                logical_width: Reactive::new(logical_w),
                logical_height: Reactive::new(logical_h),
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
    // Active window change. The focused client comes from `focused_idx`
    // (see below); `focus_history_id == 0` can't identify it because
    // every output's MRU head has position 0.
    let focused_client = state
        .focused_idx
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|idx| next_clients.get(idx).cloned());
    let prev_focused: Option<Address> = service.focused_client.get().map(|c| c.address.get());
    let new_focused: Option<Address> = focused_client.as_ref().map(|c| c.address.get());
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
    if client.xwayland.get() != c.xwayland {
        client.xwayland.set(c.xwayland);
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
        .and_then(|o| focus_history_position(o, c))
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
        .and_then(|o| focus_history_position(o, c))
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
        xwayland: Reactive::new(c.xwayland),
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

/// Prefer the stable id projection. The app-id fallback exists only for a
/// rolling upgrade from an older compositor and cannot distinguish two
/// windows owned by the same application.
fn most_recent_client_on_tag<'a>(
    output: &'a RawOutput,
    state: &'a StateJson,
    tag_bit: u32,
) -> Option<&'a RawClient> {
    if !output.focus_history_v2.is_empty() {
        output.focus_history_v2.iter().find_map(|entry| {
            state
                .clients
                .iter()
                .find(|client| client.id == entry.id && client.tags & tag_bit != 0)
        })
    } else {
        output.focus_history.iter().find_map(|app_id| {
            state
                .clients
                .iter()
                .find(|client| client.app_id == *app_id && client.tags & tag_bit != 0)
        })
    }
}

fn focus_history_position(output: &RawOutput, client: &RawClient) -> Option<usize> {
    if !output.focus_history_v2.is_empty() && client.id != 0 {
        output
            .focus_history_v2
            .iter()
            .position(|entry| entry.id == client.id)
    } else {
        output
            .focus_history
            .iter()
            .position(|app_id| app_id == &client.app_id)
    }
}

/// The focused output's layout, or `None` when the active output is
/// missing or its `layout_idx` is past the layouts list.
fn active_layout(state: &StateJson) -> Option<ActiveLayout> {
    let focused = state
        .outputs
        .iter()
        .find(|o| o.name == state.active_output)?;
    let name = state.layouts.get(focused.layout_idx)?;
    Some(ActiveLayout {
        idx: focused.layout_idx,
        name: name.clone(),
    })
}

#[cfg(test)]
mod frame_error_tests {
    use super::*;

    /// Every frame failing (schema skew) logs the first failure and then
    /// every LOG_EVERY-th, and a good frame reports the recovery once.
    #[test]
    fn failures_are_logged_rate_limited_and_recovery_is_reported() {
        let mut e = FrameErrors::default();
        assert_eq!(e.on_error(), Some(1));
        let logged: Vec<u64> = (2..=250).filter_map(|_| e.on_error()).collect();
        assert_eq!(logged, vec![100, 200]);
        assert!(e.on_ok(), "first good frame ends the run");
        assert!(!e.on_ok(), "only once");
        assert_eq!(e.on_error(), Some(1), "a new run logs again");
    }

    #[test]
    fn excerpt_is_bounded_and_cut_on_a_char_boundary() {
        let line = "ş".repeat(300);
        let ex = frame_excerpt(&line);
        assert!(ex.len() <= EXCERPT_BYTES);
        assert!(ex.len() >= EXCERPT_BYTES - 1);
        assert_eq!(frame_excerpt("{}"), "{}");
    }

    /// A frame whose required field has the wrong type is an error the
    /// loop now reports, not something it silently skips.
    #[test]
    fn a_skewed_frame_is_a_parse_error() {
        let skewed =
            r#"{"active_output":"eDP-1","layouts":"tile","outputs":[],"clients":[],"tag_count":9}"#;
        assert!(serde_json::from_str::<StateJson>(skewed).is_err());
    }

    #[test]
    fn degraded_flag_notifies_on_change_only() {
        let mut rx = sync_degraded();
        rx.mark_unchanged();
        set_degraded(true);
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        set_degraded(true);
        assert!(!rx.has_changed().unwrap(), "no repeat notification");
        set_degraded(false);
        assert!(!*rx.borrow_and_update());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_layout_follows_the_focused_output() {
        let mut out = test_output(Vec::new(), Vec::new());
        out.layout_idx = 1;
        let mut other = test_output(Vec::new(), Vec::new());
        other.name = "HDMI-A-1".into();
        other.layout_idx = 2;
        let mut state = test_state(out, Vec::new());
        state.outputs.push(other);
        state.layouts = vec!["tile".into(), "scroller".into(), "grid".into()];
        assert_eq!(
            active_layout(&state),
            Some(ActiveLayout {
                idx: 1,
                name: "scroller".into()
            })
        );
        state.active_output = "HDMI-A-1".into();
        assert_eq!(active_layout(&state).map(|l| l.idx), Some(2));
        // Index past the list (mid-reload) and unknown output → None.
        state.layouts.truncate(2);
        assert_eq!(active_layout(&state), None);
        state.active_output = "nope".into();
        assert_eq!(active_layout(&state), None);
    }

    fn test_output(
        focus_history: Vec<String>,
        focus_history_v2: Vec<crate::state_json::RawFocusHistoryEntry>,
    ) -> RawOutput {
        RawOutput {
            name: "DP-1".into(),
            active: true,
            active_tag_mask: 1,
            occupied_tag_mask: 1,
            focus_history,
            focus_history_v2,
            layout_idx: 0,
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            scale: 1.0,
            logical_width: None,
            logical_height: None,
            exclusive_fullscreen: false,
            mode: None,
            wallpaper: String::new(),
            wallpapers_by_tag: Vec::new(),
            scroller: None,
        }
    }

    fn test_state(output: RawOutput, clients: Vec<RawClient>) -> StateJson {
        StateJson {
            active_output: "DP-1".into(),
            focused_idx: None,
            layouts: Vec::new(),
            circle_layouts: Vec::new(),
            outputs: vec![output],
            clients,
            tag_count: 9,
            keyboard_layout: String::new(),
            session_locked: None,
            lock_keys: None,
            keymode: None,
            config_errors: Vec::new(),
            internal_keyboard: None,
        }
    }

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

    #[test]
    fn focus_history_position_uses_id_for_same_app_windows() {
        let output = test_output(
            vec!["same.app".into(), "same.app".into()],
            vec![
                crate::state_json::RawFocusHistoryEntry {
                    id: 12,
                    app_id: "same.app".into(),
                },
                crate::state_json::RawFocusHistoryEntry {
                    id: 11,
                    app_id: "same.app".into(),
                },
            ],
        );
        let first = RawClient {
            id: 11,
            app_id: "same.app".into(),
            tags: 1,
            ..Default::default()
        };
        let second = RawClient {
            id: 12,
            app_id: "same.app".into(),
            tags: 1,
            ..Default::default()
        };

        assert_eq!(focus_history_position(&output, &second), Some(0));
        assert_eq!(focus_history_position(&output, &first), Some(1));

        let state = test_state(output.clone(), vec![first, second]);
        assert_eq!(
            most_recent_client_on_tag(&output, &state, 1).map(|client| client.id),
            Some(12),
        );
    }

    #[test]
    fn focus_history_helpers_fall_back_to_legacy_projection() {
        let output = test_output(vec!["beta".into(), "alpha".into()], Vec::new());
        let alpha = RawClient {
            app_id: "alpha".into(),
            tags: 1,
            ..Default::default()
        };
        let beta = RawClient {
            app_id: "beta".into(),
            tags: 1,
            ..Default::default()
        };
        let state = test_state(output.clone(), vec![alpha.clone(), beta.clone()]);

        assert_eq!(focus_history_position(&output, &beta), Some(0));
        assert_eq!(focus_history_position(&output, &alpha), Some(1));
        assert_eq!(
            most_recent_client_on_tag(&output, &state, 1).map(|client| client.app_id.as_str()),
            Some("beta"),
        );
    }
}

/// `apply` driven by `watch state`-style frames: a fresh service, a
/// sequence of snapshots, and checks on the reactive model plus the
/// synthetic events it broadcasts.
#[cfg(test)]
mod apply_tests {
    use super::*;
    use crate::state_json::{RawFocusHistoryEntry, RawLockKeys};
    use tokio::sync::broadcast::Receiver;

    fn service() -> MargoService {
        let (event_tx, _) = tokio::sync::broadcast::channel(256);
        MargoService {
            workspaces: Reactive::new(Vec::new()),
            clients: Reactive::new(Vec::new()),
            monitors: Reactive::new(Vec::new()),
            focused_client: Reactive::new(None),
            keyboard_layout: Reactive::new(String::new()),
            session_locked: Reactive::new(None),
            lock_keys: Reactive::new(None),
            keymode: Reactive::new(None),
            internal_keyboard: Reactive::new(None),
            scroller: Reactive::new(None),
            scratchpads: Reactive::new(Vec::new()),
            minimized: Reactive::new(Vec::new()),
            active_tag_windows: Reactive::new(Vec::new()),
            config_errors: Reactive::new(Vec::new()),
            active_layout: Reactive::new(None),
            fullscreen_outputs: Reactive::new(Vec::new()),
            event_tx,
        }
    }

    fn drain(rx: &mut Receiver<MargoEvent>) -> Vec<MargoEvent> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn output(name: &str, active: bool, tag_mask: u32, history: &[u64]) -> RawOutput {
        RawOutput {
            name: name.into(),
            active,
            active_tag_mask: tag_mask,
            occupied_tag_mask: 0,
            focus_history: Vec::new(),
            focus_history_v2: history
                .iter()
                .map(|&id| RawFocusHistoryEntry {
                    id,
                    app_id: String::new(),
                })
                .collect(),
            layout_idx: 0,
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            scale: 1.0,
            logical_width: None,
            logical_height: None,
            exclusive_fullscreen: false,
            mode: None,
            wallpaper: String::new(),
            wallpapers_by_tag: Vec::new(),
            scroller: None,
        }
    }

    fn client(id: u64, app: &str, monitor: &str, tags: u32) -> RawClient {
        RawClient {
            id,
            app_id: app.into(),
            title: format!("{app} window"),
            monitor: monitor.into(),
            tags,
            width: 800,
            height: 600,
            ..Default::default()
        }
    }

    /// Two outputs: DP-1 (focused, tag 1: kitty + firefox), HDMI-A-1
    /// (tag 2: mpv). kitty (id 1) is focused.
    fn frame() -> StateJson {
        let mut dp1 = output("DP-1", true, 0b1, &[1, 2]);
        dp1.occupied_tag_mask = 0b1;
        let mut hdmi = output("HDMI-A-1", false, 0b10, &[3]);
        hdmi.occupied_tag_mask = 0b10;
        hdmi.x = 1920;
        StateJson {
            active_output: "DP-1".into(),
            focused_idx: Some(0),
            layouts: vec!["tile".into(), "scroller".into()],
            circle_layouts: Vec::new(),
            outputs: vec![dp1, hdmi],
            clients: vec![
                client(1, "kitty", "DP-1", 0b1),
                client(2, "firefox", "DP-1", 0b1),
                client(3, "mpv", "HDMI-A-1", 0b10),
            ],
            tag_count: 9,
            keyboard_layout: "English (US)".into(),
            session_locked: None,
            lock_keys: None,
            keymode: None,
            config_errors: Vec::new(),
            internal_keyboard: None,
        }
    }

    fn ws(svc: &MargoService, id: i64) -> Arc<Workspace> {
        svc.workspaces
            .get()
            .into_iter()
            .find(|w| w.id.get() == id)
            .unwrap_or_else(|| panic!("no workspace {id}"))
    }

    fn client_by_id(svc: &MargoService, id: u64) -> Arc<Client> {
        svc.clients
            .get()
            .into_iter()
            .find(|c| c.address.get().margo_id() == Some(id))
            .unwrap_or_else(|| panic!("no client {id}"))
    }

    #[test]
    fn first_frame_builds_the_model_and_announces_everything() {
        let svc = service();
        let mut rx = svc.event_tx.subscribe();
        apply(&svc, &frame());

        assert_eq!(svc.workspaces.get().len(), 9);
        assert_eq!(svc.monitors.get().len(), 2);
        assert_eq!(svc.clients.get().len(), 3);
        assert_eq!(svc.keyboard_layout.get(), "English (US)");

        let tag1 = ws(&svc, 1);
        assert_eq!(tag1.windows.get(), 2);
        assert_eq!(tag1.monitor.get(), "DP-1");
        assert_eq!(tag1.tiled_layout.get(), "tile");
        assert_eq!(tag1.last_window.get(), Some(Address::new("1".into())));
        assert_eq!(tag1.last_window_title.get(), "kitty window");
        let tag2 = ws(&svc, 2);
        assert_eq!(
            (tag2.windows.get(), tag2.monitor.get()),
            (1, "HDMI-A-1".into())
        );
        assert_eq!(ws(&svc, 5).windows.get(), 0);
        assert_eq!(
            ws(&svc, 5).monitor.get(),
            "DP-1",
            "unowned tags sit on the active output"
        );

        let mons = svc.monitors.get();
        let dp1 = mons.iter().find(|m| m.name.get() == "DP-1").unwrap();
        assert!(dp1.focused.get());
        assert_eq!(dp1.active_workspace.get().id, 1);
        assert_eq!(dp1.refresh_rate.get(), 60.0, "no mode → 60 Hz");

        assert_eq!(client_by_id(&svc, 1).focus_history_id.get(), 0);
        assert_eq!(client_by_id(&svc, 2).focus_history_id.get(), 1);
        assert_eq!(
            svc.focused_client.get().map(|c| c.address.get()),
            Some(Address::new("1".into()))
        );

        let events = drain(&mut rx);
        let count = |f: fn(&MargoEvent) -> bool| events.iter().filter(|e| f(e)).count();
        assert_eq!(
            count(|e| matches!(e, MargoEvent::CreateWorkspaceV2 { .. })),
            9
        );
        assert_eq!(count(|e| matches!(e, MargoEvent::MonitorAddedV2 { .. })), 2);
        assert_eq!(count(|e| matches!(e, MargoEvent::OpenWindow { .. })), 3);
        assert_eq!(
            count(|e| matches!(e, MargoEvent::WorkspaceV2 { id: 1, .. })),
            1
        );
        assert_eq!(count(|e| matches!(e, MargoEvent::ActiveWindowV2 { .. })), 1);
    }

    #[test]
    fn identical_frame_is_silent_and_keeps_identity() {
        let svc = service();
        apply(&svc, &frame());
        let (ws_before, mons_before, clients_before) =
            (svc.workspaces.get(), svc.monitors.get(), svc.clients.get());
        let mut rx = svc.event_tx.subscribe();

        apply(&svc, &frame());

        assert!(
            drain(&mut rx).is_empty(),
            "no events for an unchanged frame"
        );
        assert!(!vec_membership_differs(&ws_before, &svc.workspaces.get()));
        assert!(!vec_membership_differs(&mons_before, &svc.monitors.get()));
        assert!(!vec_membership_differs(&clients_before, &svc.clients.get()));
    }

    #[test]
    fn title_change_updates_in_place() {
        let svc = service();
        apply(&svc, &frame());
        let kitty = client_by_id(&svc, 1);
        let mut rx = svc.event_tx.subscribe();

        let mut next = frame();
        next.clients[0].title = "vim".into();
        apply(&svc, &next);

        assert!(Arc::ptr_eq(&kitty, &client_by_id(&svc, 1)));
        assert_eq!(kitty.title.get(), "vim");
        assert_eq!(ws(&svc, 1).last_window_title.get(), "vim");
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn closing_a_window_keeps_the_survivors_identity() {
        let svc = service();
        apply(&svc, &frame());
        let mpv = client_by_id(&svc, 3);
        let mut rx = svc.event_tx.subscribe();

        // firefox closes: every later client's slot `idx` shifts down.
        let mut next = frame();
        next.clients.remove(1);
        for (i, c) in next.clients.iter_mut().enumerate() {
            c.idx = i as i32;
        }
        next.outputs[0].focus_history_v2.truncate(1);
        apply(&svc, &next);

        assert!(Arc::ptr_eq(&mpv, &client_by_id(&svc, 3)));
        assert_eq!(svc.clients.get().len(), 2);
        assert_eq!(ws(&svc, 1).windows.get(), 1);
        let events = drain(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [MargoEvent::CloseWindow { address }] if address.margo_id() == Some(2)
        ));
    }

    #[test]
    fn minimized_and_fullscreen_windows_per_tag() {
        let svc = service();
        let mut f = frame();
        f.clients[1].minimized = true;
        f.clients[2].fullscreen = true;
        apply(&svc, &f);
        assert_eq!(
            ws(&svc, 1).windows.get(),
            1,
            "minimized windows aren't counted"
        );
        assert!(ws(&svc, 2).fullscreen.get());
        assert!(!ws(&svc, 1).fullscreen.get());
        assert!(client_by_id(&svc, 2).hidden.get());
        assert!(!client_by_id(&svc, 2).mapped.get());
    }

    #[test]
    fn global_window_counts_on_every_tag_it_carries() {
        let svc = service();
        let mut f = frame();
        f.clients[2].tags = 0b111;
        apply(&svc, &f);
        assert_eq!(ws(&svc, 1).windows.get(), 3);
        assert_eq!(ws(&svc, 2).windows.get(), 1);
        assert_eq!(ws(&svc, 3).windows.get(), 1);
        assert_eq!(client_by_id(&svc, 3).workspace.get().id, 1, "lowest tag");
    }

    #[test]
    fn tag_switch_emits_workspace_event() {
        let svc = service();
        apply(&svc, &frame());
        let mut rx = svc.event_tx.subscribe();
        let mut next = frame();
        next.outputs[0].active_tag_mask = 0b100;
        apply(&svc, &next);
        let events = drain(&mut rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, MargoEvent::WorkspaceV2 { id: 3, .. })),
            "{events:?}"
        );
    }

    #[test]
    fn compositor_restart_empties_then_rebuilds() {
        let svc = service();
        apply(&svc, &frame());
        let mut rx = svc.event_tx.subscribe();

        // Reconnect to a margo that is still starting: no outputs yet.
        let mut empty = frame();
        empty.outputs.clear();
        empty.clients.clear();
        empty.focused_idx = None;
        apply(&svc, &empty);
        assert!(svc.monitors.get().is_empty());
        assert!(svc.clients.get().is_empty());
        assert!(svc.focused_client.get().is_none());
        assert_eq!(svc.workspaces.get().len(), 9, "tags persist");
        let events = drain(&mut rx);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, MargoEvent::MonitorRemovedV2 { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, MargoEvent::CloseWindow { .. }))
                .count(),
            3
        );

        apply(&svc, &frame());
        assert_eq!(svc.monitors.get().len(), 2);
        assert_eq!(svc.clients.get().len(), 3);
        assert!(svc.focused_client.get().is_some());
    }

    #[test]
    fn focus_moving_to_another_output_emits_active_window() {
        // Every output's MRU head has focus_history_id 0, so picking "the"
        // active window by `focus_history_id == 0` always found the first
        // output's window and never reported focus moving to mpv.
        let svc = service();
        apply(&svc, &frame());
        let mut rx = svc.event_tx.subscribe();

        let mut next = frame();
        next.focused_idx = Some(2);
        next.active_output = "HDMI-A-1".into();
        next.outputs[0].active = false;
        next.outputs[1].active = true;
        apply(&svc, &next);

        let events = drain(&mut rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                MargoEvent::ActiveWindowV2 { address } if address.margo_id() == Some(3)
            )),
            "{events:?}"
        );
        assert_eq!(
            svc.focused_client
                .get()
                .and_then(|c| c.address.get().margo_id()),
            Some(3)
        );

        // …and back.
        let mut rx = svc.event_tx.subscribe();
        apply(&svc, &frame());
        assert!(drain(&mut rx).iter().any(|e| matches!(
            e,
            MargoEvent::ActiveWindowV2 { address } if address.margo_id() == Some(1)
        )));
    }

    #[test]
    fn oversized_tag_count_is_clamped_to_the_mask_width() {
        // A malformed or future frame with more tags than a u32 mask holds
        // overflowed `1 << (tag - 1)` (a panic in debug builds) and sized
        // the per-tag tables by the raw number.
        let svc = service();
        let mut f = frame();
        f.tag_count = 40;
        apply(&svc, &f);
        assert_eq!(svc.workspaces.get().len(), 32);
        assert_eq!(ws(&svc, 1).windows.get(), 2);
        f.tag_count = u32::MAX;
        apply(&svc, &f);
        assert_eq!(svc.workspaces.get().len(), 32);
    }

    #[test]
    fn out_of_range_focus_index_means_nothing_focused() {
        for idx in [Some(-1), Some(99), None] {
            let svc = service();
            let mut f = frame();
            f.focused_idx = idx;
            apply(&svc, &f);
            assert!(svc.focused_client.get().is_none(), "{idx:?}");
        }
    }

    #[test]
    fn tag_count_changes_create_and_destroy_workspaces() {
        let svc = service();
        apply(&svc, &frame());
        let mut rx = svc.event_tx.subscribe();
        let mut f = frame();
        f.tag_count = 4;
        apply(&svc, &f);
        assert_eq!(svc.workspaces.get().len(), 4);
        let destroyed = drain(&mut rx)
            .into_iter()
            .filter(|e| matches!(e, MargoEvent::DestroyWorkspaceV2 { .. }))
            .count();
        assert_eq!(destroyed, 5);
    }

    #[test]
    fn logical_size_and_fullscreen_outputs_are_mirrored() {
        let svc = service();
        let mut out = output("DP-1", true, 1, &[]);
        out.width = 2560;
        out.height = 1600;
        out.scale = 2.0;
        out.logical_width = Some(800);
        out.logical_height = Some(1280);
        out.exclusive_fullscreen = true;
        let mut f = frame();
        f.outputs = vec![out];
        apply(&svc, &f);
        let mon = svc.monitors.get().pop().unwrap();
        assert_eq!((mon.width.get(), mon.height.get()), (2560, 1600));
        assert_eq!(
            (mon.logical_width.get(), mon.logical_height.get()),
            (800, 1280)
        );
        assert_eq!(svc.fullscreen_outputs.get(), vec!["DP-1".to_string()]);

        f.outputs[0].exclusive_fullscreen = false;
        apply(&svc, &f);
        assert!(svc.fullscreen_outputs.get().is_empty());
    }

    #[test]
    fn keyboard_layout_and_config_errors_are_mirrored() {
        let svc = service();
        let mut f = frame();
        f.config_errors = vec![crate::state_json::RawConfigDiagnostic {
            path: "/c/config.conf".into(),
            line: 3,
            severity: "warning".into(),
            code: "W004".into(),
            message: "duplicate bind".into(),
        }];
        apply(&svc, &f);
        assert_eq!(svc.config_errors.get().len(), 1);
        f.config_errors.clear();
        f.keyboard_layout = "Turkish".into();
        apply(&svc, &f);
        assert!(svc.config_errors.get().is_empty());
        assert_eq!(svc.keyboard_layout.get(), "Turkish");
    }

    #[test]
    fn session_lock_state_is_mirrored_and_unknown_on_older_margo() {
        let svc = service();
        let mut f = frame();
        apply(&svc, &f);
        assert_eq!(svc.session_locked.get(), None, "field absent: unknown");
        f.session_locked = Some(true);
        apply(&svc, &f);
        assert_eq!(svc.session_locked.get(), Some(true));
        f.session_locked = Some(false);
        apply(&svc, &f);
        assert_eq!(svc.session_locked.get(), Some(false));
        let parsed: StateJson = serde_json::from_str(
            r#"{"active_output":"","layouts":[],"outputs":[],"clients":[],
                "tag_count":9,"session_locked":true}"#,
        )
        .expect("frame parses");
        assert_eq!(parsed.session_locked, Some(true));
    }

    #[test]
    fn lock_keys_are_mirrored_and_unknown_on_older_margo() {
        let svc = service();
        let mut f = frame();
        apply(&svc, &f);
        assert_eq!(svc.lock_keys.get(), None);
        let caps = RawLockKeys {
            caps: true,
            ..Default::default()
        };
        f.lock_keys = Some(caps);
        apply(&svc, &f);
        assert_eq!(svc.lock_keys.get(), Some(caps));
        let parsed: StateJson = serde_json::from_str(
            r#"{"active_output":"","layouts":[],"outputs":[],"clients":[],
                "tag_count":9,"lock_keys":{"caps":false,"num":true,"scroll":false}}"#,
        )
        .expect("frame parses");
        assert_eq!(
            parsed.lock_keys,
            Some(RawLockKeys {
                num: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn keymode_is_mirrored_and_unknown_on_older_margo() {
        let svc = service();
        let mut f = frame();
        apply(&svc, &f);
        assert_eq!(svc.keymode.get(), None, "older margo: unknown");
        f.keymode = Some("resize".into());
        apply(&svc, &f);
        assert_eq!(svc.keymode.get().as_deref(), Some("resize"));
        let parsed: StateJson = serde_json::from_str(
            r#"{"active_output":"","layouts":[],"outputs":[],"clients":[],
                "tag_count":9,"keymode":"default"}"#,
        )
        .expect("frame parses");
        assert_eq!(parsed.keymode.as_deref(), Some("default"));
    }

    #[test]
    fn internal_keyboard_is_mirrored_and_unknown_on_older_margo() {
        let svc = service();
        let mut f = frame();
        apply(&svc, &f);
        assert_eq!(svc.internal_keyboard.get(), None, "older margo: unknown");
        // A frame captured from margo with the built-in keyboard off.
        let parsed: StateJson = serde_json::from_str(
            r#"{"active_output":"","layouts":[],"outputs":[],"clients":[],"tag_count":9,
                "internal_keyboard":{"name":"AT Translated Set 2 keyboard",
                "disabled":true,"external":true,"auto":false}}"#,
        )
        .expect("frame parses");
        f.internal_keyboard = parsed.internal_keyboard;
        apply(&svc, &f);
        let ikb = svc.internal_keyboard.get().expect("mirrored");
        assert!(ikb.disabled && ikb.external && !ikb.auto);
        assert_eq!(ikb.name.as_deref(), Some("AT Translated Set 2 keyboard"));
    }

    #[test]
    fn scroller_strip_of_the_focused_output_is_mirrored() {
        use crate::state_json::{RawScroller, RawScrollerColumn};
        let svc = service();
        let mut f = frame();
        apply(&svc, &f);
        assert_eq!(svc.scroller.get(), None, "not in the scroller layout");
        let strip = RawScroller {
            viewport_width: 1920,
            columns: vec![RawScrollerColumn {
                id: 1,
                app_id: "kitty".into(),
                x: -900,
                width: 1500,
                focused: false,
            }],
        };
        // Another output's strip is not the bar's (focused-output) strip.
        f.outputs[1].scroller = Some(strip.clone());
        apply(&svc, &f);
        assert_eq!(svc.scroller.get(), None);
        f.outputs[0].scroller = Some(strip.clone());
        apply(&svc, &f);
        assert_eq!(svc.scroller.get(), Some(strip));
        let parsed: StateJson = serde_json::from_str(
            r#"{"active_output":"DP-1","layouts":[],"clients":[],"tag_count":9,
                "outputs":[{"name":"DP-1","active":true,"active_tag_mask":1,
                "occupied_tag_mask":1,"layout_idx":1,"width":1920,"height":1080,
                "x":0,"y":0,"scale":1.0,"scroller":{"viewport_width":1920,
                "columns":[{"id":7,"app_id":"foot","x":12,"width":1500,"focused":true}]}}]}"#,
        )
        .expect("frame with a scroller strip parses");
        let s = parsed.outputs[0].scroller.as_ref().expect("strip");
        assert_eq!((s.columns[0].id, s.columns[0].x), (7, 12));
    }

    #[test]
    fn scratchpads_are_mirrored_change_only() {
        let svc = service();
        let mut f = frame();
        apply(&svc, &f);
        assert!(svc.scratchpads.get().is_empty());
        f.clients[1].scratchpad = true;
        f.clients[1].scratchpad_shown = true;
        apply(&svc, &f);
        let chips = svc.scratchpads.get();
        assert_eq!(chips.len(), 1);
        assert_eq!((chips[0].id, chips[0].shown), (2, true));
        f.clients[1].scratchpad_shown = false;
        apply(&svc, &f);
        assert!(!svc.scratchpads.get()[0].shown, "hide is mirrored");
    }

    #[test]
    fn xwayland_flag_is_mirrored_and_updated_in_place() {
        let svc = service();
        let mut f = frame();
        f.clients[2].xwayland = true;
        apply(&svc, &f);
        assert!(client_by_id(&svc, 3).xwayland.get(), "built with the flag");
        assert!(!client_by_id(&svc, 1).xwayland.get());
        let before = client_by_id(&svc, 3);
        f.clients[2].xwayland = false;
        apply(&svc, &f);
        let after = client_by_id(&svc, 3);
        assert!(Arc::ptr_eq(&before, &after), "same window, same Arc");
        assert!(!after.xwayland.get(), "updated in place");
    }

    #[test]
    fn active_tag_windows_follow_the_focused_output() {
        let svc = service();
        let mut f = frame();
        apply(&svc, &f);
        let ids = |svc: &MargoService| -> Vec<u64> {
            svc.active_tag_windows.get().iter().map(|w| w.id).collect()
        };
        assert_eq!(ids(&svc), [1, 2], "DP-1 tag 1: kitty + firefox");
        assert!(svc.active_tag_windows.get()[0].focused, "kitty is focused");
        // Focus moves to the other output: its tag's windows follow.
        f.active_output = "HDMI-A-1".into();
        f.focused_idx = Some(2);
        apply(&svc, &f);
        assert_eq!(ids(&svc), [3]);
    }

    /// A frame as margo serialises it (extra fields included), parsed and
    /// applied the way the `watch state` task does.
    #[test]
    fn recorded_json_frame_applies() {
        let line = r#"{"version":1,"active_output":"eDP-1","focused_idx":0,
            "layouts":["tile","scroller"],"tag_count":9,"keyboard_layout":"",
            "outputs":[{"name":"eDP-1","active":true,"x":0,"y":0,"width":2560,
              "height":1600,"scale":1.5,"transform":0,
              "mode":{"physical_width":2560,"physical_height":1600,"refresh_mhz":165000},
              "layout_idx":1,"active_tag_mask":4,"prev_tag_mask":1,
              "occupied_tag_mask":5,"is_overview":false,"wallpaper":"",
              "wallpapers_by_tag":[],"scratchpad_visible":0,"scratchpad_hidden":0,
              "focus_history":["foot"],"focus_history_v2":[{"id":17,"app_id":"foot"}]}],
            "clients":[{"id":17,"idx":0,"monitor":"eDP-1","monitor_idx":0,"tags":4,
              "app_id":"foot","title":"~","x":10,"y":20,"width":1000,"height":700,
              "floating":false,"floated_by_layout":false,"fullscreen":false,
              "minimized":false,"urgent":false,"scratchpad":false,"global":false,
              "focused":true,"pid":4242,"scanout":false,"group_id":null,
              "group_active":false}],
            "config_errors":[]}"#;
        let state: StateJson = serde_json::from_str(line).expect("margo frame parses");
        let svc = service();
        apply(&svc, &state);
        let mon = svc.monitors.get().pop().unwrap();
        // An older frame without `logical_*`: derived from mode / scale.
        assert_eq!((mon.width.get(), mon.height.get()), (2560, 1600));
        assert_eq!(
            (mon.logical_width.get(), mon.logical_height.get()),
            (1707, 1067)
        );
        assert!(svc.fullscreen_outputs.get().is_empty());
        assert_eq!(mon.active_workspace.get().id, 3);
        assert_eq!(mon.refresh_rate.get(), 165.0);
        assert_eq!(mon.scale.get(), 1.5);
        assert_eq!(ws(&svc, 3).tiled_layout.get(), "scroller");
        let foot = client_by_id(&svc, 17);
        assert_eq!((foot.pid.get(), foot.workspace.get().id), (4242, 3));
        assert_eq!(foot.at.get(), ClientLocation { x: 10, y: 20 });
        assert_eq!(svc.focused_client.get().map(|c| c.pid.get()), Some(4242));
    }
}
