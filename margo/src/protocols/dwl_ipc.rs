#![allow(dead_code, unreachable_patterns)]
//! dwl-ipc-unstable-v2 server implementation.
//!
//! Implements the IPC protocol that lets external clients (mmsg/mctl) query and
//! control the compositor: get/set tags and layouts, watch state changes, and
//! dispatch compositor actions.

use smithay::{
    output::Output,
    reexports::wayland_server::{
        backend::ClientId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
    },
};
use tracing::debug;

use crate::{
    layout::LayoutId,
    protocols::generated::dwl_ipc::{
        zdwl_ipc_manager_v2::{self, ZdwlIpcManagerV2},
        zdwl_ipc_output_v2::{self, TagState, ZdwlIpcOutputV2},
    },
    state::MargoState,
    MAX_TAGS,
};

/// All layout IDs in a fixed order matching the IPC layout index.
const ALL_LAYOUTS: &[LayoutId] = &[
    LayoutId::Tile,
    LayoutId::Scroller,
    LayoutId::Grid,
    LayoutId::Monocle,
    LayoutId::Deck,
    LayoutId::CenterTile,
    LayoutId::RightTile,
    LayoutId::VerticalScroller,
    LayoutId::VerticalTile,
    LayoutId::VerticalGrid,
    LayoutId::VerticalDeck,
    LayoutId::TgMix,
    LayoutId::Canvas,
    LayoutId::Dwindle,
];

// ── Per-monitor IPC state ─────────────────────────────────────────────────────

/// Resources registered for one monitor.
#[derive(Debug, Default)]
pub struct DwlIpcState {
    pub outputs: Vec<ZdwlIpcOutputV2>,
}

impl DwlIpcState {
    pub fn new() -> Self {
        DwlIpcState::default()
    }
}

// ── Global user-data (none needed) ───────────────────────────────────────────

#[derive(Debug)]
pub struct DwlIpcGlobalData;

// ── GlobalDispatch: binding the zdwl_ipc_manager_v2 global ───────────────────

impl GlobalDispatch<ZdwlIpcManagerV2, DwlIpcGlobalData> for MargoState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZdwlIpcManagerV2>,
        _global_data: &DwlIpcGlobalData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        // Announce all known layouts
        for layout in ALL_LAYOUTS.iter() {
            manager.layout(layout.name().to_string());
        }
        // Announce tag count
        manager.tags(MAX_TAGS as u32);
    }
}

// ── Dispatch: ZdwlIpcManagerV2 requests ──────────────────────────────────────

impl Dispatch<ZdwlIpcManagerV2, ()> for MargoState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZdwlIpcManagerV2,
        request: zdwl_ipc_manager_v2::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zdwl_ipc_manager_v2::Request::GetOutput { id, output } => {
                let ipc_output = data_init.init(id, ());

                // Find which monitor this wl_output belongs to
                let mon_idx = match Output::from_resource(&output) { Some(smithay_output) => {
                    state
                        .monitors
                        .iter()
                        .position(|m| m.output == smithay_output)
                        .unwrap_or(0)
                } _ => {
                    0
                }};

                // Send initial state for this output
                if mon_idx < state.monitors.len() {
                    send_monitor_state(state, mon_idx, &ipc_output);
                }

                // Register resource
                if mon_idx < state.monitors.len() {
                    state.monitors[mon_idx].dwl_ipc.outputs.push(ipc_output);
                }
            }
            zdwl_ipc_manager_v2::Request::Release => {}
            _ => {}
        }
    }

    fn destroyed(_state: &mut Self, _client: ClientId, _resource: &ZdwlIpcManagerV2, _data: &()) {
    }
}

// ── Dispatch: ZdwlIpcOutputV2 requests ───────────────────────────────────────

impl Dispatch<ZdwlIpcOutputV2, ()> for MargoState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZdwlIpcOutputV2,
        request: zdwl_ipc_output_v2::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Find which monitor this output resource belongs to
        let mon_idx = state
            .monitors
            .iter()
            .position(|m| m.dwl_ipc.outputs.iter().any(|o| o == resource));

        match request {
            zdwl_ipc_output_v2::Request::SetTags { tagmask, toggle_tagset } => {
                let idx = mon_idx.unwrap_or(0);
                if idx < state.monitors.len() && tagmask != 0 {
                    let sel = if toggle_tagset != 0 {
                        1 - state.monitors[idx].seltags
                    } else {
                        state.monitors[idx].seltags
                    };
                    state.monitors[idx].tagset[sel] = tagmask;
                    state.arrange_monitor(idx);
                    broadcast_monitor(state, idx);
                }
            }
            zdwl_ipc_output_v2::Request::SetClientTags { and_tags, xor_tags } => {
                let idx = mon_idx.unwrap_or(0);
                if let Some(cidx) = state.focused_client_idx() {
                    if state.clients[cidx].monitor == idx {
                        let current = state.clients[cidx].tags;
                        let new = (current & and_tags) ^ xor_tags;
                        if new != 0 {
                            state.clients[cidx].old_tags = state.clients[cidx].tags;
                            state.clients[cidx].is_tag_switching = true;
                            state.clients[cidx].animation.running = false;
                            state.clients[cidx].tags = new;
                            state.arrange_monitor(idx);
                            broadcast_monitor(state, idx);
                        }
                    }
                }
            }
            zdwl_ipc_output_v2::Request::SetLayout { index } => {
                let idx = mon_idx.unwrap_or(0);
                if idx < state.monitors.len() {
                    if let Some(layout) = ALL_LAYOUTS.get(index as usize) {
                        state.set_layout(layout.name());
                        broadcast_monitor(state, idx);
                    }
                }
            }
            zdwl_ipc_output_v2::Request::Quit => {
                state.should_quit = true;
            }
            zdwl_ipc_output_v2::Request::Dispatch {
                dispatch,
                arg1,
                arg2,
                arg3,
                arg4,
                arg5,
            } => {
                debug!("ipc dispatch: {dispatch}");
                // Parse args: arg1=i, arg2=i2, arg3=f, arg4=ui, arg5=v
                let arg = margo_config::Arg {
                    i: arg1.parse().unwrap_or(0),
                    i2: arg2.parse().unwrap_or(0),
                    f: arg3.parse().unwrap_or(0.0),
                    f2: 0.0,
                    v: if arg4.is_empty() { None } else { Some(arg4) },
                    v2: if arg5.is_empty() { None } else { Some(arg5) },
                    v3: None,
                    ui: 0,
                    ui2: 0,
                };
                crate::dispatch::dispatch_action(state, &dispatch, &arg);
            }
            zdwl_ipc_output_v2::Request::Release => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &ZdwlIpcOutputV2, _data: &()) {
        // Remove resource from whichever monitor holds it
        for mon in state.monitors.iter_mut() {
            mon.dwl_ipc.outputs.retain(|o| o != resource);
        }
    }
}

// ── Broadcast helpers ─────────────────────────────────────────────────────────

/// Send full state for monitor `mon_idx` to one output resource, then frame.
fn send_monitor_state(state: &MargoState, mon_idx: usize, out: &ZdwlIpcOutputV2) {
    let focused_monitor = state.focused_monitor();
    let mon = &state.monitors[mon_idx];
    let tagset = mon.current_tagset();

    // active
    out.active((mon_idx == focused_monitor) as u32);

    // per-tag state
    for tag in 0..MAX_TAGS {
        let bit = 1u32 << tag;
        let occupied =
            state.clients.iter().any(|c| c.monitor == mon_idx && (c.tags & bit) != 0) as u32;
        let focused = state
            .focused_client_idx()
            .map(|i| (state.clients[i].tags & bit) != 0 && state.clients[i].monitor == mon_idx)
            .unwrap_or(false) as u32;

        let tag_state = if (tagset & bit) != 0 {
            TagState::Active
        } else if state
            .clients
            .iter()
            .any(|c| c.monitor == mon_idx && c.is_urgent && (c.tags & bit) != 0)
        {
            TagState::Urgent
        } else {
            TagState::None
        };

        out.tag(tag as u32, tag_state, occupied, focused);
    }

    // layout
    let layout = mon.current_layout();
    let layout_idx = ALL_LAYOUTS
        .iter()
        .position(|&l| l == layout)
        .unwrap_or(0) as u32;
    out.layout(layout_idx);
    out.layout_symbol(layout.symbol().to_string());

    // focused client info
    let (title, appid, fullscreen, floating, x, y, width, height) = state
        .focused_client_idx()
        .filter(|&i| state.clients[i].monitor == mon_idx)
        .map(|i| {
            let c = &state.clients[i];
            (
                c.title.clone(),
                c.app_id.clone(),
                c.is_fullscreen as u32,
                c.is_floating as u32,
                c.geom.x,
                c.geom.y,
                c.geom.width,
                c.geom.height,
            )
        })
        .unwrap_or_default();

    out.title(title);
    out.appid(appid);
    out.fullscreen(fullscreen);
    out.floating(floating);
    out.x(x);
    out.y(y);
    out.width(width);
    out.height(height);

    // keymode
    out.keymode(state.input_keyboard.mode.clone());

    out.frame();
}

/// Broadcast state for monitor `mon_idx` to all registered output resources.
pub fn broadcast_monitor(state: &MargoState, mon_idx: usize) {
    if mon_idx >= state.monitors.len() {
        return;
    }
    let resources: Vec<ZdwlIpcOutputV2> = state.monitors[mon_idx].dwl_ipc.outputs.clone();
    for out in &resources {
        send_monitor_state(state, mon_idx, out);
    }
}

/// Broadcast to all monitors.
pub fn broadcast_all(state: &MargoState) {
    for mon_idx in 0..state.monitors.len() {
        broadcast_monitor(state, mon_idx);
    }
}
