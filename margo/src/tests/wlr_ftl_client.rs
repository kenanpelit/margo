//! Client side of `zwlr_foreign_toplevel_manager_v1` for the fixture: what
//! a taskbar / dock does — list toplevels with their app id and state, and
//! ask the compositor to (un)minimize one.

use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_client::{Connection, Dispatch, QueueHandle, event_created_child};

use super::client::{Client, ClientState};

/// One toplevel as the taskbar sees it (state as of its last `done`).
#[derive(Debug, Clone)]
pub struct TaskbarEntry {
    pub handle: ZwlrForeignToplevelHandleV1,
    pub app_id: String,
    pub states: Vec<u32>,
    pending_states: Vec<u32>,
    pub closed: bool,
}

impl TaskbarEntry {
    pub fn minimized(&self) -> bool {
        self.states
            .contains(&(zwlr_foreign_toplevel_handle_v1::State::Minimized as u32))
    }
}

#[derive(Debug, Default)]
pub struct ForeignToplevelEvents {
    pub entries: Vec<TaskbarEntry>,
}

impl ForeignToplevelEvents {
    /// The live (not closed) toplevel with `app_id`.
    pub fn by_app_id(&self, app_id: &str) -> &TaskbarEntry {
        self.entries
            .iter()
            .find(|e| e.app_id == app_id && !e.closed)
            .unwrap_or_else(|| panic!("taskbar has no {app_id}: {:?}", self.entries))
    }
}

impl Client {
    pub fn bind_foreign_toplevel_manager(&mut self) -> ZwlrForeignToplevelManagerV1 {
        let manager = self.bind_global(3);
        self.connection.flush().expect("client flush");
        manager
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.foreign_toplevels.entries.push(TaskbarEntry {
                handle: toplevel,
                app_id: String::new(),
                states: Vec::new(),
                pending_states: Vec::new(),
                closed: false,
            });
        }
    }

    event_created_child!(ClientState, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state
            .foreign_toplevels
            .entries
            .iter_mut()
            .find(|e| e.handle == *handle)
        else {
            return;
        };
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => entry.app_id = app_id,
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bytes } => {
                entry.pending_states = bytes
                    .chunks_exact(4)
                    .map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                entry.states = entry.pending_states.clone();
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => entry.closed = true,
            _ => {}
        }
    }
}
