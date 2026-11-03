//! `ext-workspace-v1` server state — margo tag model.
//!
//! Smithay has no native impl; this is hand-rolled, ported from niri's
//! `protocols/ext_workspace.rs` and adapted to margo's dwl-style tags.
//!
//! Mapping (see road_map.md §15.10 P5):
//!
//! - **Workspace group = output (monitor).** One group per connected
//!   monitor.
//! - **9 fixed workspaces per group**, one per tag bit (margo/dwl use a
//!   9-tag bitmask). Workspaces never appear/disappear at runtime —
//!   only their `active` state flips. Groups come and go with monitors.
//! - **active** = the tag bit is set in the monitor's current tagset.
//!   margo tags are a bitmask, so several workspaces can be active at
//!   once on one monitor — which the protocol's per-workspace `Active`
//!   state allows.
//! - **id** = `"<connector>:<n>"` (stable across the session), **name**
//!   = `"<n>"` (1-based), **coordinates** = `[monitor_index, tag_index]`.
//! - `activate` → warp focus to the workspace's monitor + `view_tag`.
//! - `assign` / `remove` / `create_workspace` → no-op (margo's tag set
//!   is fixed per monitor).
//!
//! Tag state is also exposed over margo's IPC socket; this protocol
//! lets standard ext-workspace shells (sfwbar, ironbar, …) show margo
//! workspaces. Refreshed once per repaint by [`refresh`].

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::mem;

use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use crate::state::MargoState;

const VERSION: u32 = 1;

/// Number of tags margo exposes per monitor (dwl default).
pub const WORKSPACE_COUNT: usize = 9;

/// Stable key for one workspace: which output it lives on + tag index 0..9.
type WsKey = (Output, usize);

pub trait ExtWorkspaceHandler {
    fn ext_workspace_manager_state(&mut self) -> &mut ExtWorkspaceManagerState;
    /// Make the `tag_idx`-th tag visible on `output`'s monitor.
    fn activate_workspace(&mut self, output: Output, tag_idx: usize);
}

/// Queued, applied on `manager.commit`.
enum Action {
    Activate(WsKey),
}

pub struct ExtWorkspaceManagerState {
    display: DisplayHandle,
    instances: HashMap<ExtWorkspaceManagerV1, Vec<Action>>,
    groups: HashMap<Output, GroupData>,
    workspaces: HashMap<WsKey, WorkspaceData>,
    /// [`monitor_fingerprint`] of the monitors the last `refresh` mirrored;
    /// unchanged monitors skip the snapshot and diff.
    last_fingerprint: Option<u64>,
}

struct GroupData {
    instances: Vec<ExtWorkspaceGroupHandleV1>,
}

struct WorkspaceData {
    id: String,
    name: String,
    coordinates: [u32; 2],
    active: bool,
    output: Output,
    instances: Vec<ExtWorkspaceHandleV1>,
}

pub struct ExtWorkspaceGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

impl WorkspaceData {
    fn proto_state(&self) -> ext_workspace_handle_v1::State {
        let mut s = ext_workspace_handle_v1::State::empty();
        if self.active {
            s |= ext_workspace_handle_v1::State::Active;
        }
        s
    }

    fn add_instance<D>(
        &mut self,
        display: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
    ) -> Option<&ExtWorkspaceHandleV1>
    where
        D: Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1> + 'static,
    {
        // Fails when the client is already gone; skip it rather than panic.
        let workspace = match client.create_resource::<ExtWorkspaceHandleV1, _, D>(
            display,
            manager.version(),
            manager.clone(),
        ) {
            Ok(workspace) => workspace,
            Err(err) => {
                tracing::debug!(?err, "ext_workspace: client gone, skipping workspace");
                return None;
            }
        };
        manager.workspace(&workspace);
        workspace.id(self.id.clone());
        workspace.name(self.name.clone());
        workspace.coordinates(
            self.coordinates
                .iter()
                .flat_map(|x| x.to_ne_bytes())
                .collect(),
        );
        workspace.state(self.proto_state());
        workspace.capabilities(ext_workspace_handle_v1::WorkspaceCapabilities::Activate);
        self.instances.push(workspace);
        self.instances.last()
    }
}

impl GroupData {
    fn add_instance<D>(
        &mut self,
        display: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
        output: &Output,
    ) -> Option<&ExtWorkspaceGroupHandleV1>
    where
        D: Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1> + 'static,
    {
        // Fails when the client is already gone; skip it rather than panic.
        let group = match client.create_resource::<ExtWorkspaceGroupHandleV1, _, D>(
            display,
            manager.version(),
            manager.clone(),
        ) {
            Ok(group) => group,
            Err(err) => {
                tracing::debug!(?err, "ext_workspace: client gone, skipping group");
                return None;
            }
        };
        manager.workspace_group(&group);
        group.capabilities(ext_workspace_group_handle_v1::GroupCapabilities::empty());
        for wl_output in output.client_outputs(client) {
            group.output_enter(&wl_output);
        }
        self.instances.push(group);
        self.instances.last()
    }
}

impl ExtWorkspaceManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData>,
        D: Dispatch<ExtWorkspaceManagerV1, ()>,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ExtWorkspaceGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ExtWorkspaceManagerV1, _>(VERSION, global_data);
        Self {
            display: display.clone(),
            instances: HashMap::new(),
            groups: HashMap::new(),
            workspaces: HashMap::new(),
            last_fingerprint: None,
        }
    }
}

/// Hash of everything `refresh` mirrors, per monitor in order: output,
/// name, current tagset (the index is the position). Equal fingerprints
/// mean `refresh` would send nothing. Allocates nothing.
pub fn monitor_fingerprint<'a, O: Hash + 'a>(
    monitors: impl Iterator<Item = (&'a O, &'a str, u32)>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (idx, (output, name, tagset)) in monitors.enumerate() {
        (idx, output, name, tagset).hash(&mut hasher);
    }
    hasher.finish()
}

/// Emit `workspace_enter`/`workspace_leave` linking a workspace to its
/// group's handles (per-manager).
fn send_enter_leave(groups: &HashMap<Output, GroupData>, data: &WorkspaceData, enter: bool) {
    let Some(group_data) = groups.get(&data.output) else {
        return;
    };
    for group in &group_data.instances {
        let manager: &ExtWorkspaceManagerV1 = group.data().unwrap();
        for workspace in &data.instances {
            if workspace.data() == Some(manager) {
                if enter {
                    group.workspace_enter(workspace);
                } else {
                    group.workspace_leave(workspace);
                }
            }
        }
    }
}

fn remove_workspace_instances(groups: &HashMap<Output, GroupData>, data: &WorkspaceData) {
    send_enter_leave(groups, data, false);
    for workspace in &data.instances {
        workspace.removed();
    }
}

/// Diff margo's monitors/tags against the tracked workspace set and emit
/// events. Called once per repaint batch from
/// `MargoState::finish_repaint_batch`.
pub fn refresh(state: &mut MargoState) {
    // Runs after every repaint batch that queued a frame; nearly always
    // nothing changed. New managers are primed in `bind`, not here.
    let fingerprint = monitor_fingerprint(
        state
            .monitors
            .iter()
            .map(|m| (&m.output, m.name.as_str(), m.current_tagset())),
    );
    if state.protocols.ext_workspace.last_fingerprint == Some(fingerprint) {
        return;
    }
    state.protocols.ext_workspace.last_fingerprint = Some(fingerprint);
    // Monitor snapshot — avoid borrowing `state` while we mutate the proto.
    struct MonSnap {
        output: Output,
        name: String,
        tagset: u32,
        mon_idx: usize,
    }
    let mons: Vec<MonSnap> = state
        .monitors
        .iter()
        .enumerate()
        .map(|(i, m)| MonSnap {
            output: m.output.clone(),
            name: m.name.clone(),
            tagset: m.current_tagset(),
            mon_idx: i,
        })
        .collect();
    let live_outputs: HashSet<Output> = mons.iter().map(|m| m.output.clone()).collect();

    let proto = &mut state.protocols.ext_workspace;
    let mut changed = false;

    // Drop workspaces on disconnected monitors.
    proto.workspaces.retain(|(output, _), data| {
        if live_outputs.contains(output) {
            return true;
        }
        remove_workspace_instances(&proto.groups, data);
        changed = true;
        false
    });

    // Drop groups for disconnected monitors.
    proto.workspaces.shrink_to_fit();
    proto.groups.retain(|output, data| {
        if live_outputs.contains(output) {
            return true;
        }
        for group in &data.instances {
            group.removed();
        }
        changed = true;
        false
    });

    // Update / create workspaces.
    for mon in &mons {
        for tag_idx in 0..WORKSPACE_COUNT {
            let active = (mon.tagset & (1 << tag_idx)) != 0;
            match proto.workspaces.entry((mon.output.clone(), tag_idx)) {
                Entry::Occupied(entry) => {
                    let data = entry.into_mut();
                    if data.active != active {
                        data.active = active;
                        let st = data.proto_state();
                        for inst in &data.instances {
                            inst.state(st);
                        }
                        changed = true;
                    }
                    if data.coordinates[0] != mon.mon_idx as u32 {
                        data.coordinates[0] = mon.mon_idx as u32;
                        for inst in &data.instances {
                            inst.coordinates(
                                data.coordinates
                                    .iter()
                                    .flat_map(|x| x.to_ne_bytes())
                                    .collect(),
                            );
                        }
                        changed = true;
                    }
                }
                Entry::Vacant(entry) => {
                    let mut data = WorkspaceData {
                        id: format!("{}:{}", mon.name, tag_idx + 1),
                        name: (tag_idx + 1).to_string(),
                        coordinates: [mon.mon_idx as u32, tag_idx as u32],
                        active,
                        output: mon.output.clone(),
                        instances: Vec::new(),
                    };
                    let display = proto.display.clone();
                    let managers: Vec<ExtWorkspaceManagerV1> =
                        proto.instances.keys().cloned().collect();
                    for manager in &managers {
                        if let Some(client) = manager.client() {
                            data.add_instance::<MargoState>(&display, &client, manager);
                        }
                    }
                    entry.insert(data);
                    changed = true;
                }
            }
        }
    }

    // Create groups + wire enters for new monitors.
    for mon in &mons {
        if proto.groups.contains_key(&mon.output) {
            continue;
        }
        let mut data = GroupData {
            instances: Vec::new(),
        };
        let display = proto.display.clone();
        let managers: Vec<ExtWorkspaceManagerV1> = proto.instances.keys().cloned().collect();
        for manager in &managers {
            if let Some(client) = manager.client() {
                data.add_instance::<MargoState>(&display, &client, manager, &mon.output);
            }
        }
        // workspace_enter for every workspace already on this output.
        for group in &data.instances {
            let manager: &ExtWorkspaceManagerV1 = group.data().unwrap();
            for ((output, _), ws) in proto.workspaces.iter() {
                if output != &mon.output {
                    continue;
                }
                for workspace in &ws.instances {
                    if workspace.data() == Some(manager) {
                        group.workspace_enter(workspace);
                    }
                }
            }
        }
        proto.groups.insert(mon.output.clone(), data);
        changed = true;
    }

    if changed {
        for manager in proto.instances.keys() {
            manager.done();
        }
    }
}

impl<D> GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData, D>
    for ExtWorkspaceManagerState
where
    D: GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData>,
    D: Dispatch<ExtWorkspaceManagerV1, ()>,
    D: Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1>,
    D: Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1>,
    D: ExtWorkspaceHandler,
    D: 'static,
{
    fn bind(
        state: &mut D,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _global_data: &ExtWorkspaceGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());
        let proto = state.ext_workspace_manager_state();

        // Existing workspaces → new client.
        let mut new_workspaces: HashMap<Output, Vec<ExtWorkspaceHandleV1>> = HashMap::new();
        for ((output, _), data) in proto.workspaces.iter_mut() {
            let Some(workspace) = data.add_instance::<D>(handle, client, &manager).cloned() else {
                continue;
            };
            new_workspaces
                .entry(output.clone())
                .or_default()
                .push(workspace);
        }

        // Existing groups → new client, wiring enters.
        for (output, group_data) in proto.groups.iter_mut() {
            let Some(group) = group_data
                .add_instance::<D>(handle, client, &manager, output)
                .cloned()
            else {
                continue;
            };
            for workspace in new_workspaces.get(output).into_iter().flatten() {
                group.workspace_enter(workspace);
            }
        }

        manager.done();
        proto.instances.insert(manager, Vec::new());
    }

    fn can_view(client: Client, global_data: &ExtWorkspaceGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ExtWorkspaceManagerV1, (), D> for ExtWorkspaceManagerState
where
    D: Dispatch<ExtWorkspaceManagerV1, ()>,
    D: ExtWorkspaceHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ExtWorkspaceManagerV1,
        request: <ExtWorkspaceManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_workspace_manager_v1::Request::Commit => {
                let proto = state.ext_workspace_manager_state();
                let Some(actions) = proto.instances.get_mut(resource) else {
                    return;
                };
                let actions = mem::take(actions);
                for action in actions {
                    match action {
                        Action::Activate((output, tag_idx)) => {
                            state.activate_workspace(output, tag_idx)
                        }
                    }
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                resource.finished();
                let proto = state.ext_workspace_manager_state();
                proto.instances.retain(|m, _| m != resource);
                for data in proto.groups.values_mut() {
                    data.instances.retain(|i| i.data() != Some(resource));
                }
                for data in proto.workspaces.values_mut() {
                    data.instances.retain(|i| i.data() != Some(resource));
                }
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &ExtWorkspaceManagerV1, _data: &()) {
        state
            .ext_workspace_manager_state()
            .instances
            .retain(|m, _| m != resource);
    }
}

impl<D> Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1, D> for ExtWorkspaceManagerState
where
    D: Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1>,
    D: ExtWorkspaceHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ExtWorkspaceHandleV1,
        request: <ExtWorkspaceHandleV1 as Resource>::Request,
        data: &ExtWorkspaceManagerV1,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let proto = state.ext_workspace_manager_state();
        let Some(key) = proto
            .workspaces
            .iter()
            .find(|(_, d)| d.instances.contains(resource))
            .map(|(key, _)| key.clone())
        else {
            return;
        };

        if let ext_workspace_handle_v1::Request::Activate = request {
            if let Some(actions) = proto.instances.get_mut(data) {
                actions.push(Action::Activate(key));
            }
        }
        // deactivate / assign / remove / destroy → no-op (fixed tag set).
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtWorkspaceHandleV1,
        _data: &ExtWorkspaceManagerV1,
    ) {
        for data in state.ext_workspace_manager_state().workspaces.values_mut() {
            data.instances.retain(|i| i != resource);
        }
    }
}

impl<D> Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1, D> for ExtWorkspaceManagerState
where
    D: Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1>,
    D: ExtWorkspaceHandler,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ExtWorkspaceGroupHandleV1,
        _request: <ExtWorkspaceGroupHandleV1 as Resource>::Request,
        _data: &ExtWorkspaceManagerV1,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // create_workspace / destroy → no-op (margo's tag set is fixed).
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtWorkspaceGroupHandleV1,
        _data: &ExtWorkspaceManagerV1,
    ) {
        for data in state.ext_workspace_manager_state().groups.values_mut() {
            data.instances.retain(|i| i != resource);
        }
    }
}

#[macro_export]
macro_rules! delegate_ext_workspace {
    ($ty:ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1: $crate::protocols::ext_workspace::ExtWorkspaceGlobalData
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1: ()
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_handle_v1::ExtWorkspaceHandleV1: smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1: smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
    };
}

#[cfg(test)]
mod fingerprint_tests {
    use super::monitor_fingerprint;

    #[test]
    fn fingerprint_tracks_outputs_names_and_tagsets() {
        let fp = |list: &[(u32, &'static str, u32)]| {
            monitor_fingerprint(list.iter().map(|(o, n, t)| (o, *n, *t)))
        };
        let base = [(1_u32, "DP-1", 0b1), (2, "DP-2", 0b10)];
        assert_eq!(fp(&base), fp(&base));
        assert_ne!(
            fp(&base),
            fp(&[(1, "DP-1", 0b11), (2, "DP-2", 0b10)]),
            "tag change"
        );
        assert_ne!(
            fp(&base),
            fp(&[(3, "DP-1", 0b1), (2, "DP-2", 0b10)]),
            "replugged output"
        );
        assert_ne!(fp(&base), fp(&base[..1]), "unplugged output");
        assert_ne!(
            fp(&base),
            fp(&[(2, "DP-2", 0b10), (1, "DP-1", 0b1)]),
            "reordered"
        );
    }
}
