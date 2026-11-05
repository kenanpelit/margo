//! `zwlr_foreign_toplevel_management_v1` — write-side toplevel manager.
//!
//! margo already advertises the read-only `ext-foreign-toplevel-list-v1`
//! via smithay's `ForeignToplevelListState`. This module adds the wlr
//! *write-side* manager so taskbars / docks (mshell, Waybar) can act on
//! toplevels — activate, close, (un)fullscreen, (un)minimize — not merely
//! list them.
//!
//! Design is **additive**: the smithay ext-list keeps running untouched.
//! We track our own toplevel set, refreshed once per repaint by
//! [`refresh`], which diffs `MargoState::clients` and emits
//! title / app_id / state / closed accordingly. Window actions route
//! through the [`WlrForeignToplevelHandler`] trait (impl in
//! `state/handlers/wlr_foreign_toplevel.rs`).
//!
//! Ported from niri's `protocols/foreign_toplevel.rs` (wlr half only),
//! in the porting style margo already used for `output_management.rs`.
//! v1 limitation: no `output_enter` / `output_leave` tracking — clients
//! get title / app_id / state, which covers the taskbar use case.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::seat::WaylandFocus;

use crate::state::MargoState;

const WLR_MANAGEMENT_VERSION: u32 = 3;

/// Per-toplevel state mirrored to every bound manager instance.
struct ToplevelData {
    title: String,
    app_id: String,
    /// wlr `state` payload (array of `zwlr_foreign_toplevel_handle_v1::State`).
    states: Vec<u32>,
    instances: HashSet<ZwlrForeignToplevelHandleV1>,
}

pub struct WlrForeignToplevelState {
    display: DisplayHandle,
    manager_instances: HashSet<ZwlrForeignToplevelManagerV1>,
    toplevels: HashMap<WlSurface, ToplevelData>,
    /// [`toplevel_fingerprint`] of the client list the last `refresh`
    /// mirrored; an unchanged list skips the snapshot and diff.
    last_fingerprint: Option<u64>,
}

#[derive(Clone)]
pub struct WlrForeignToplevelGlobalData {
    filter: Arc<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

/// Window actions the wlr manager drives. Implemented for `MargoState` in
/// `state/handlers/wlr_foreign_toplevel.rs`.
pub trait WlrForeignToplevelHandler {
    fn wlr_foreign_toplevel_state(&mut self) -> &mut WlrForeignToplevelState;
    fn wlr_ftl_activate(&mut self, surface: WlSurface);
    fn wlr_ftl_close(&mut self, surface: WlSurface);
    fn wlr_ftl_set_fullscreen(&mut self, surface: WlSurface);
    fn wlr_ftl_unset_fullscreen(&mut self, surface: WlSurface);
    fn wlr_ftl_set_minimized(&mut self, surface: WlSurface);
    fn wlr_ftl_unset_minimized(&mut self, surface: WlSurface);
}

impl WlrForeignToplevelState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrForeignToplevelManagerV1, WlrForeignToplevelGlobalData>,
        D: Dispatch<ZwlrForeignToplevelManagerV1, ()>,
        D: Dispatch<ZwlrForeignToplevelHandleV1, ()>,
        D: WlrForeignToplevelHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = WlrForeignToplevelGlobalData {
            filter: Arc::new(filter),
        };
        display.create_global::<D, ZwlrForeignToplevelManagerV1, _>(
            WLR_MANAGEMENT_VERSION,
            global_data,
        );
        Self {
            display: display.clone(),
            manager_instances: HashSet::new(),
            toplevels: HashMap::new(),
            last_fingerprint: None,
        }
    }
}

impl ToplevelData {
    fn state_bytes(&self) -> Vec<u8> {
        self.states.iter().flat_map(|s| s.to_ne_bytes()).collect()
    }

    /// Create a fresh handle for `manager` and prime it with current state.
    fn add_instance<D>(&mut self, display: &DisplayHandle, manager: &ZwlrForeignToplevelManagerV1)
    where
        D: Dispatch<ZwlrForeignToplevelHandleV1, ()> + 'static,
    {
        let Some(client) = manager.client() else {
            return;
        };
        let Ok(toplevel) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, D>(
            display,
            manager.version(),
            (),
        ) else {
            return;
        };
        manager.toplevel(&toplevel);
        toplevel.title(self.title.clone());
        toplevel.app_id(self.app_id.clone());
        toplevel.state(self.state_bytes());
        toplevel.done();
        self.instances.insert(toplevel);
    }
}

/// One toplevel's mirrored fields, in [`toplevel_fingerprint`] order:
/// surface, title, app id, then the `fullscreen`, `focused` and
/// `minimized` states.
pub type ToplevelKey<'a, S> = (S, &'a str, &'a str, bool, bool, bool);

/// Hash of everything `refresh` mirrors, per toplevel in order. Equal
/// fingerprints mean `refresh` would send nothing. Hashing borrows; it
/// allocates nothing.
pub fn toplevel_fingerprint<'a, S: Hash + 'a>(
    toplevels: impl Iterator<Item = ToplevelKey<'a, S>>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for key in toplevels {
        key.hash(&mut hasher);
    }
    hasher.finish()
}

/// The wlr `state` array for a toplevel: fullscreen, activated (focused)
/// and minimized, in protocol enum values.
fn wire_states(fullscreen: bool, focused: bool, minimized: bool) -> Vec<u32> {
    use zwlr_foreign_toplevel_handle_v1::State;
    [
        (fullscreen, State::Fullscreen),
        (focused, State::Activated),
        (minimized, State::Minimized),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, s)| s as u32)
    .collect()
}

/// Diff `state.clients` against the tracked toplevel set and emit the wlr
/// events. Called once per repaint batch from
/// `MargoState::finish_repaint_batch`, so it first compares a cheap fingerprint of the client list and returns
/// when nothing changed — the full pass clones every title and app id and
/// builds a HashSet of surfaces.
pub fn refresh(state: &mut MargoState) {
    let focused_idx = state.focused_client_idx();
    let fingerprint =
        toplevel_fingerprint(state.clients.iter().enumerate().filter_map(|(i, client)| {
            let surface = client.window.wl_surface()?;
            Some((
                surface.id(),
                client.title.as_str(),
                client.app_id.as_str(),
                client.is_fullscreen,
                Some(i) == focused_idx,
                client.is_minimized,
            ))
        }));
    if state.protocols.wlr_foreign_toplevel.last_fingerprint == Some(fingerprint) {
        return;
    }
    state.protocols.wlr_foreign_toplevel.last_fingerprint = Some(fingerprint);

    struct Snap {
        surface: WlSurface,
        title: String,
        app_id: String,
        states: Vec<u32>,
    }

    let mut snaps: Vec<Snap> = Vec::with_capacity(state.clients.len());
    let mut alive: HashSet<WlSurface> = HashSet::new();
    for (i, client) in state.clients.iter().enumerate() {
        let Some(surface) = client.window.wl_surface().map(|s| s.into_owned()) else {
            continue;
        };
        let states = wire_states(
            client.is_fullscreen,
            Some(i) == focused_idx,
            client.is_minimized,
        );
        alive.insert(surface.clone());
        snaps.push(Snap {
            surface,
            title: crate::utils::clamp_wire_str(&client.title).to_owned(),
            app_id: crate::utils::clamp_wire_str(&client.app_id).to_owned(),
            states,
        });
    }

    let proto = &mut state.protocols.wlr_foreign_toplevel;

    // Closed windows: notify + drop.
    proto.toplevels.retain(|surface, data| {
        if alive.contains(surface) {
            return true;
        }
        for inst in &data.instances {
            inst.closed();
        }
        false
    });

    // Snapshot the manager list + display once to avoid aliasing `proto`
    // while we hold an entry borrow.
    let managers: Vec<ZwlrForeignToplevelManagerV1> =
        proto.manager_instances.iter().cloned().collect();
    let display = proto.display.clone();

    for snap in snaps {
        match proto.toplevels.entry(snap.surface) {
            Entry::Occupied(entry) => {
                let data = entry.into_mut();
                let mut changed = false;
                if data.title != snap.title {
                    data.title = snap.title.clone();
                    for inst in &data.instances {
                        inst.title(snap.title.clone());
                    }
                    changed = true;
                }
                if data.app_id != snap.app_id {
                    data.app_id = snap.app_id.clone();
                    for inst in &data.instances {
                        inst.app_id(snap.app_id.clone());
                    }
                    changed = true;
                }
                if data.states != snap.states {
                    data.states = snap.states.clone();
                    let bytes = data.state_bytes();
                    for inst in &data.instances {
                        inst.state(bytes.clone());
                    }
                    changed = true;
                }
                if changed {
                    for inst in &data.instances {
                        inst.done();
                    }
                }
            }
            Entry::Vacant(entry) => {
                let mut data = ToplevelData {
                    title: snap.title,
                    app_id: snap.app_id,
                    states: snap.states,
                    instances: HashSet::new(),
                };
                for manager in &managers {
                    data.add_instance::<MargoState>(&display, manager);
                }
                entry.insert(data);
            }
        }
    }
}

impl<D> GlobalDispatch<ZwlrForeignToplevelManagerV1, WlrForeignToplevelGlobalData, D>
    for WlrForeignToplevelState
where
    D: GlobalDispatch<ZwlrForeignToplevelManagerV1, WlrForeignToplevelGlobalData>,
    D: Dispatch<ZwlrForeignToplevelManagerV1, ()>,
    D: Dispatch<ZwlrForeignToplevelHandleV1, ()>,
    D: WlrForeignToplevelHandler,
    D: 'static,
{
    fn bind(
        state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &WlrForeignToplevelGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());
        let proto = state.wlr_foreign_toplevel_state();
        let display = proto.display.clone();
        for data in proto.toplevels.values_mut() {
            data.add_instance::<D>(&display, &manager);
        }
        proto.manager_instances.insert(manager);
    }

    fn can_view(client: Client, global_data: &WlrForeignToplevelGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrForeignToplevelManagerV1, (), D> for WlrForeignToplevelState
where
    D: Dispatch<ZwlrForeignToplevelManagerV1, ()>,
    D: WlrForeignToplevelHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: <ZwlrForeignToplevelManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            resource.finished();
            state
                .wlr_foreign_toplevel_state()
                .manager_instances
                .remove(resource);
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ZwlrForeignToplevelManagerV1,
        _data: &(),
    ) {
        state
            .wlr_foreign_toplevel_state()
            .manager_instances
            .remove(resource);
    }
}

impl<D> Dispatch<ZwlrForeignToplevelHandleV1, (), D> for WlrForeignToplevelState
where
    D: Dispatch<ZwlrForeignToplevelHandleV1, ()>,
    D: WlrForeignToplevelHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrForeignToplevelHandleV1,
        request: <ZwlrForeignToplevelHandleV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request;

        let proto = state.wlr_foreign_toplevel_state();
        let Some(surface) = proto
            .toplevels
            .iter()
            .find(|(_, data)| data.instances.contains(resource))
            .map(|(surface, _)| surface.clone())
        else {
            return;
        };

        match request {
            Request::Activate { .. } => state.wlr_ftl_activate(surface),
            Request::Close => state.wlr_ftl_close(surface),
            Request::SetFullscreen { .. } => state.wlr_ftl_set_fullscreen(surface),
            Request::UnsetFullscreen => state.wlr_ftl_unset_fullscreen(surface),
            Request::SetMinimized => state.wlr_ftl_set_minimized(surface),
            Request::UnsetMinimized => state.wlr_ftl_unset_minimized(surface),
            // margo is a tiling WM — no maximize concept.
            Request::SetMaximized
            | Request::UnsetMaximized
            | Request::SetRectangle { .. }
            | Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ZwlrForeignToplevelHandleV1,
        _data: &(),
    ) {
        for data in state.wlr_foreign_toplevel_state().toplevels.values_mut() {
            data.instances.remove(resource);
        }
    }
}

#[macro_export]
macro_rules! delegate_wlr_foreign_toplevel {
    ($ty:ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1: $crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelGlobalData
        ] => $crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelState);
        smithay::reexports::wayland_server::delegate_dispatch!($ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1: ()
        ] => $crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelState);
        smithay::reexports::wayland_server::delegate_dispatch!($ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1: ()
        ] => $crate::protocols::wlr_foreign_toplevel::WlrForeignToplevelState);
    };
}

#[cfg(test)]
mod tests {
    use super::{toplevel_fingerprint, wire_states};
    use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::State;

    #[test]
    fn wire_states_carry_minimized() {
        assert!(wire_states(false, false, false).is_empty());
        assert_eq!(
            wire_states(true, true, true),
            vec![
                State::Fullscreen as u32,
                State::Activated as u32,
                State::Minimized as u32
            ]
        );
        assert_eq!(
            wire_states(false, false, true),
            vec![State::Minimized as u32]
        );
    }

    #[test]
    fn fingerprint_tracks_every_mirrored_field() {
        let base = [
            (1_u32, "Firefox", "firefox", false, true, false),
            (2, "kitty", "kitty", false, false, false),
        ];
        let fp =
            |list: &[super::ToplevelKey<'static, u32>]| toplevel_fingerprint(list.iter().copied());
        assert_eq!(fp(&base), fp(&base), "unchanged list → refresh skipped");
        let mut retitled = base;
        retitled[0].1 = "Firefox — new tab";
        assert_ne!(fp(&base), fp(&retitled));
        let mut refocused = base;
        refocused[0].4 = false;
        refocused[1].4 = true;
        assert_ne!(fp(&base), fp(&refocused));
        let mut fullscreen = base;
        fullscreen[1].3 = true;
        assert_ne!(fp(&base), fp(&fullscreen));
        let mut minimized = base;
        minimized[1].5 = true;
        assert_ne!(fp(&base), fp(&minimized));
        assert_ne!(fp(&base), fp(&base[..1]), "closed window");
    }
}
