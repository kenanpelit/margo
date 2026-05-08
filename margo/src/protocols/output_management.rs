//! Server-side `wlr_output_management_v1` implementation.
//!
//! Adapted from niri's [`protocols/output_management.rs`], simplified for
//! margo: we don't track per-client config persistence, no VRR, no mode
//! switching (mode changes need DRM-level re-modeset which is risky and
//! out of scope for the first cut). What we DO support:
//!
//!   * Read-only topology discovery — clients (kanshi, wlr-randr,
//!     swayrr, way-displays, …) see one [`ZwlrOutputHeadV1`] per
//!     connected output, with the full mode list and the current
//!     scale / transform / position. They can `done()` on the
//!     [`ZwlrOutputManagerV1`] and use the topology however they
//!     want.
//!   * Apply requests — clients hand us a
//!     [`ZwlrOutputConfigurationV1`] of per-head pending changes,
//!     and we accept the subset we know how to apply: `scale`,
//!     `transform`, `position`. Anything else (mode change,
//!     enable/disable, adaptive_sync) gets the configuration
//!     `cancelled()` so the client knows to fall back. The accepted
//!     subset is what `wlr-randr --scale` / `--transform` /
//!     `--pos` reach for, and it covers the common kanshi profile
//!     types ("at this docked layout, scale eDP-1 by 1.5").
//!
//! Re-modeset can land later; the protocol surface this exposes is
//! already what kanshi-style autoconfig needs to drive scale and
//! position changes.
//!
//! Mode and enable changes that we can't honour come through as
//! [`OutputConfigurationHeadState::Ok`] but the parent
//! `apply_pending_config` returns `Err`, which we fold back into
//! `cancelled()`.

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_output::Transform as WlTransform;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};

/// Bumped on output topology change so clients know to invalidate
/// their cached configuration tokens.
const VERSION: u32 = 4;

/// Snapshot of a single output, used to:
///   1. Send the right `head` events to a newly-bound manager.
///   2. Diff against future updates so we only send `done()` if
///      something actually changed.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputSnapshot {
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub serial_number: String,
    pub physical_size: (i32, i32),
    pub pos: (i32, i32),
    pub scale: f64,
    pub transform: WlTransform,
    pub modes: Vec<ModeSnapshot>,
    pub current_mode: Option<usize>,
    pub preferred_mode: Option<usize>,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModeSnapshot {
    pub width: i32,
    pub height: i32,
    pub refresh: i32, // mHz
}

#[derive(Debug, Default)]
struct ClientData {
    heads: HashMap<String, (ZwlrOutputHeadV1, Vec<ZwlrOutputModeV1>)>,
    configs: HashMap<ZwlrOutputConfigurationV1, ConfigurationState>,
    manager: Option<ZwlrOutputManagerV1>,
}

#[derive(Debug)]
enum ConfigurationState {
    /// Client is still building this configuration: each head it
    /// references gets stashed here as it's introduced via
    /// `enable_head` / `disable_head`. Atomic apply happens on
    /// `apply` or `test`.
    Ongoing(HashMap<String, PendingHeadConfig>),
    /// `apply` / `test` has fired and the client must not modify
    /// the configuration further.
    Finished,
}

#[derive(Debug, Default)]
pub struct PendingHeadConfig {
    enabled: bool,
    scale: Option<f64>,
    transform: Option<WlTransform>,
    position: Option<(i32, i32)>,
    /// `(width, height, refresh_mhz)` — accepted into the pending
    /// state but rejected at apply time because we don't do mode
    /// changes yet.
    mode: Option<(i32, i32, i32)>,
}

pub enum OutputConfigurationHeadState {
    /// Head was disabled (or its head ref dropped) — the
    /// configuration's outcome has already been decided.
    Cancelled,
    /// Per-head configuration object, still building. Holds the
    /// output name and a back-pointer to its parent so
    /// `set_scale` / `set_transform` / `set_position` /
    /// `set_mode` can reach into the parent's pending state.
    Ok(String, ZwlrOutputConfigurationV1),
}

pub struct OutputManagementManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait OutputManagementHandler {
    fn output_management_state(&mut self) -> &mut OutputManagementManagerState;

    /// Called when a client `apply`s a configuration we accepted.
    /// Returns `true` if the compositor was actually able to put the
    /// requested settings into effect, `false` to make us send a
    /// `cancelled()` to the client.
    fn apply_output_pending(&mut self, pending: HashMap<String, PendingHeadConfig>) -> bool;
}

pub struct OutputManagementManagerState {
    display: DisplayHandle,
    serial: u32,
    clients: HashMap<ClientId, ClientData>,
    current: HashMap<String, OutputSnapshot>,
}

impl OutputManagementManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementManagerGlobalData>,
        D: Dispatch<ZwlrOutputManagerV1, ()>,
        D: Dispatch<ZwlrOutputHeadV1, String>,
        D: Dispatch<ZwlrOutputModeV1, ()>,
        D: Dispatch<ZwlrOutputConfigurationV1, u32>,
        D: Dispatch<ZwlrOutputConfigurationHeadV1, OutputConfigurationHeadState>,
        D: OutputManagementHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        display.create_global::<D, ZwlrOutputManagerV1, _>(
            VERSION,
            OutputManagementManagerGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            display: display.clone(),
            serial: 0,
            clients: HashMap::new(),
            current: HashMap::new(),
        }
    }

    /// Refresh the compositor-side view of all outputs and notify
    /// every bound manager about the change. Call this whenever
    /// scale/position/transform/mode list changes, or when an
    /// output is added/removed.
    pub fn snapshot_changed(&mut self, new: HashMap<String, OutputSnapshot>) {
        if new == self.current {
            return;
        }
        self.serial = self.serial.wrapping_add(1);
        self.current = new;
        // Send the full new state to every connected client. For
        // simplicity we don't try to compute minimal diffs (niri
        // does); we simply fire fresh head/mode/done events, which
        // clients are required to handle as a state replacement.
        let serial = self.serial;
        let display = self.display.clone();
        for (_client_id, data) in self.clients.iter_mut() {
            // Tear down the old heads that no longer exist or
            // changed identity. Simpler: finish them all and
            // republish, the protocol allows that.
            for (_name, (head, modes)) in data.heads.drain() {
                for mode in modes {
                    mode.finished();
                }
                head.finished();
            }
            if let Some(manager) = &data.manager {
                if let Some(client) = manager.client() {
                    for snap in self.current.values() {
                        publish_head(&display, &client, manager, snap, &mut data.heads);
                    }
                    manager.done(serial);
                }
            }
        }
    }
}

/// Send one `head` (and its modes) to the manager.
fn publish_head(
    display: &DisplayHandle,
    client: &Client,
    manager: &ZwlrOutputManagerV1,
    snap: &OutputSnapshot,
    heads: &mut HashMap<String, (ZwlrOutputHeadV1, Vec<ZwlrOutputModeV1>)>,
) {
    let Ok(head) = client.create_resource::<ZwlrOutputHeadV1, _, crate::state::MargoState>(
        display,
        manager.version(),
        snap.name.clone(),
    ) else {
        return;
    };
    manager.head(&head);
    head.name(snap.name.clone());
    head.description(snap.description.clone());
    if head.version() >= zwlr_output_head_v1::EVT_MAKE_SINCE {
        head.make(snap.make.clone());
        head.model(snap.model.clone());
        head.serial_number(snap.serial_number.clone());
    }
    head.physical_size(snap.physical_size.0, snap.physical_size.1);
    head.position(snap.pos.0, snap.pos.1);
    head.scale(snap.scale);
    head.transform(snap.transform);
    head.enabled(if snap.enabled { 1 } else { 0 });

    let mut modes_out = Vec::with_capacity(snap.modes.len());
    for (i, m) in snap.modes.iter().enumerate() {
        let Ok(mode) = client.create_resource::<ZwlrOutputModeV1, _, crate::state::MargoState>(
            display,
            head.version(),
            (),
        ) else {
            continue;
        };
        head.mode(&mode);
        mode.size(m.width, m.height);
        mode.refresh(m.refresh);
        if Some(i) == snap.preferred_mode {
            mode.preferred();
        }
        if Some(i) == snap.current_mode {
            head.current_mode(&mode);
        }
        modes_out.push(mode);
    }

    heads.insert(snap.name.clone(), (head, modes_out));
}

// ── Manager dispatch ─────────────────────────────────────────────────────────

impl<D> GlobalDispatch<ZwlrOutputManagerV1, OutputManagementManagerGlobalData, D>
    for OutputManagementManagerState
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementManagerGlobalData>,
    D: Dispatch<ZwlrOutputManagerV1, ()>,
    D: Dispatch<ZwlrOutputHeadV1, String>,
    D: Dispatch<ZwlrOutputModeV1, ()>,
    D: Dispatch<ZwlrOutputConfigurationV1, u32>,
    D: Dispatch<ZwlrOutputConfigurationHeadV1, OutputConfigurationHeadState>,
    D: OutputManagementHandler + 'static,
{
    fn bind(
        state: &mut D,
        _handle: &DisplayHandle,
        client: &Client,
        new: New<ZwlrOutputManagerV1>,
        _global_data: &OutputManagementManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(new, ());
        let mgr_state = state.output_management_state();
        let display = mgr_state.display.clone();
        let serial = mgr_state.serial;
        let snaps: Vec<OutputSnapshot> = mgr_state.current.values().cloned().collect();
        let entry = mgr_state.clients.entry(client.id()).or_default();
        for snap in &snaps {
            publish_head(&display, client, &manager, snap, &mut entry.heads);
        }
        manager.done(serial);
        entry.manager = Some(manager);
    }

    fn can_view(client: Client, global_data: &OutputManagementManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrOutputManagerV1, (), D> for OutputManagementManagerState
where
    D: Dispatch<ZwlrOutputManagerV1, ()>,
    D: Dispatch<ZwlrOutputConfigurationV1, u32>,
    D: Dispatch<ZwlrOutputConfigurationHeadV1, OutputConfigurationHeadState>,
    D: OutputManagementHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                let cfg = data_init.init(id, serial);
                if serial != state.output_management_state().serial {
                    // Stale token — cancel immediately. Client will
                    // re-fetch state and try again.
                    cfg.cancelled();
                    if let Some(c) = state
                        .output_management_state()
                        .clients
                        .get_mut(&manager.client().unwrap().id())
                    {
                        c.configs.insert(cfg, ConfigurationState::Finished);
                    }
                    return;
                }
                if let Some(c) = state
                    .output_management_state()
                    .clients
                    .get_mut(&manager.client().unwrap().id())
                {
                    c.configs
                        .insert(cfg, ConfigurationState::Ongoing(HashMap::new()));
                }
            }
            zwlr_output_manager_v1::Request::Stop => {
                manager.finished();
                if let Some(client) = manager.client() {
                    state.output_management_state().clients.remove(&client.id());
                }
            }
            _ => {}
        }
    }
}

// ── Head dispatch (read-only — clients can't send requests on heads) ─────────

impl<D> Dispatch<ZwlrOutputHeadV1, String, D> for OutputManagementManagerState
where
    D: Dispatch<ZwlrOutputHeadV1, String>,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        head: &ZwlrOutputHeadV1,
        request: zwlr_output_head_v1::Request,
        _name: &String,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let zwlr_output_head_v1::Request::Release = request {
            head.finished();
        }
    }
}

impl<D> Dispatch<ZwlrOutputModeV1, (), D> for OutputManagementManagerState
where
    D: Dispatch<ZwlrOutputModeV1, ()>,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        mode: &ZwlrOutputModeV1,
        request: zwlr_output_mode_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let zwlr_output_mode_v1::Request::Release = request {
            mode.finished();
        }
    }
}

// ── Configuration dispatch ───────────────────────────────────────────────────

impl<D> Dispatch<ZwlrOutputConfigurationV1, u32, D> for OutputManagementManagerState
where
    D: Dispatch<ZwlrOutputConfigurationV1, u32>,
    D: Dispatch<ZwlrOutputConfigurationHeadV1, OutputConfigurationHeadState>,
    D: OutputManagementHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        cfg: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        _serial: &u32,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let client_id = match cfg.client() {
            Some(c) => c.id(),
            None => return,
        };
        let mgr_state = state.output_management_state();
        let client_data = match mgr_state.clients.get_mut(&client_id) {
            Some(c) => c,
            None => return,
        };

        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let name = match head.data::<String>() {
                    Some(n) => n.clone(),
                    None => return,
                };
                // Two protocol-level reasons to refuse this head and
                // cancel the whole configuration:
                //   1. The configuration was already submitted via
                //      `apply` / `test` (state == Finished) — adding
                //      heads to a sealed config is a misuse error.
                //   2. The same output has already been added to
                //      this configuration (`enable_head` or
                //      `disable_head` referencing it). The protocol
                //      requires us to mark the config doomed.
                let already_present = matches!(
                    client_data.configs.get(cfg),
                    Some(ConfigurationState::Ongoing(map)) if map.contains_key(&name)
                );
                let finished = matches!(
                    client_data.configs.get(cfg),
                    Some(ConfigurationState::Finished)
                );
                if finished || already_present {
                    data_init.init(id, OutputConfigurationHeadState::Cancelled);
                    cfg.cancelled();
                    client_data
                        .configs
                        .insert(cfg.clone(), ConfigurationState::Finished);
                    return;
                }
                data_init.init(
                    id,
                    OutputConfigurationHeadState::Ok(name.clone(), cfg.clone()),
                );
                if let Some(ConfigurationState::Ongoing(map)) = client_data.configs.get_mut(cfg) {
                    map.insert(
                        name,
                        PendingHeadConfig {
                            enabled: true,
                            ..Default::default()
                        },
                    );
                }
            }
            zwlr_output_configuration_v1::Request::DisableHead { head } => {
                let name = match head.data::<String>() {
                    Some(n) => n.clone(),
                    None => return,
                };
                // Same protocol gate as EnableHead. We don't
                // construct a head resource for `disable_head` (the
                // request doesn't pass a `new_id`), so cancellation
                // here means firing `cancelled()` on the parent
                // configuration and recording it as Finished.
                let already_present = matches!(
                    client_data.configs.get(cfg),
                    Some(ConfigurationState::Ongoing(map)) if map.contains_key(&name)
                );
                let finished = matches!(
                    client_data.configs.get(cfg),
                    Some(ConfigurationState::Finished)
                );
                if finished || already_present {
                    cfg.cancelled();
                    client_data
                        .configs
                        .insert(cfg.clone(), ConfigurationState::Finished);
                    return;
                }
                if let Some(ConfigurationState::Ongoing(map)) = client_data.configs.get_mut(cfg) {
                    map.insert(
                        name,
                        PendingHeadConfig {
                            enabled: false,
                            ..Default::default()
                        },
                    );
                }
            }
            zwlr_output_configuration_v1::Request::Apply => {
                let pending = match client_data.configs.remove(cfg) {
                    Some(ConfigurationState::Ongoing(p)) => p,
                    _ => return,
                };
                client_data.configs.insert(cfg.clone(), ConfigurationState::Finished);
                let ok = state.apply_output_pending(pending);
                if ok {
                    cfg.succeeded();
                } else {
                    cfg.failed();
                }
            }
            zwlr_output_configuration_v1::Request::Test => {
                // We don't actually run a side-effect-free
                // simulation; declare success only if the request
                // doesn't touch features we don't support
                // (mode/enable). Anything else: reject.
                if let Some(ConfigurationState::Ongoing(map)) = client_data.configs.get(cfg) {
                    let supportable = map
                        .values()
                        .all(|p| p.enabled && p.mode.is_none());
                    if supportable {
                        cfg.succeeded();
                    } else {
                        cfg.failed();
                    }
                }
                client_data.configs.insert(cfg.clone(), ConfigurationState::Finished);
            }
            zwlr_output_configuration_v1::Request::Destroy => {
                client_data.configs.remove(cfg);
            }
            _ => {}
        }
    }
}

impl<D> Dispatch<ZwlrOutputConfigurationHeadV1, OutputConfigurationHeadState, D>
    for OutputManagementManagerState
where
    D: Dispatch<ZwlrOutputConfigurationHeadV1, OutputConfigurationHeadState>,
    D: OutputManagementHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &OutputConfigurationHeadState,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let (name, cfg) = match data {
            OutputConfigurationHeadState::Ok(n, c) => (n, c),
            OutputConfigurationHeadState::Cancelled => return,
        };
        let client_id = match cfg.client() {
            Some(c) => c.id(),
            None => return,
        };
        let mgr_state = state.output_management_state();
        let client_data = match mgr_state.clients.get_mut(&client_id) {
            Some(c) => c,
            None => return,
        };
        let pending = match client_data.configs.get_mut(cfg) {
            Some(ConfigurationState::Ongoing(map)) => match map.get_mut(name) {
                Some(p) => p,
                None => return,
            },
            _ => return,
        };

        match request {
            zwlr_output_configuration_head_v1::Request::SetScale { scale } => {
                pending.scale = Some(scale);
            }
            zwlr_output_configuration_head_v1::Request::SetTransform { transform } => {
                if let WEnum::Value(t) = transform {
                    pending.transform = Some(t);
                }
            }
            zwlr_output_configuration_head_v1::Request::SetPosition { x, y } => {
                pending.position = Some((x, y));
            }
            zwlr_output_configuration_head_v1::Request::SetMode { mode } => {
                if let Some(m) = mode.data::<()>() {
                    let _ = m;
                }
                // We honour the request slot but apply will reject
                // it. The client gets a `failed` then, not
                // `succeeded`.
                pending.mode = Some((0, 0, 0));
            }
            _ => {}
        }
    }
}

// ── Pending-head accessors used by the apply handler ─────────────────────────

impl PendingHeadConfig {
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn scale(&self) -> Option<f64> {
        self.scale
    }
    pub fn transform(&self) -> Option<WlTransform> {
        self.transform
    }
    pub fn position(&self) -> Option<(i32, i32)> {
        self.position
    }
    pub fn requests_mode_change(&self) -> bool {
        self.mode.is_some()
    }
}

// ── Helper: build snapshot from a smithay Output ─────────────────────────────

pub fn snapshot_from_output(
    output: &Output,
    enabled: bool,
    pos: (i32, i32),
) -> OutputSnapshot {
    let phys = output.physical_properties();
    let modes = output.modes();
    let current_mode = output.current_mode();
    let preferred_mode = output.preferred_mode();
    let modes_snap: Vec<ModeSnapshot> = modes
        .iter()
        .map(|m| ModeSnapshot {
            width: m.size.w,
            height: m.size.h,
            refresh: m.refresh,
        })
        .collect();
    let current_idx = current_mode.and_then(|cm| {
        modes.iter().position(|m| {
            m.size == cm.size && m.refresh == cm.refresh
        })
    });
    let preferred_idx = preferred_mode.and_then(|pm| {
        modes.iter().position(|m| {
            m.size == pm.size && m.refresh == pm.refresh
        })
    });
    OutputSnapshot {
        name: output.name(),
        description: output.description(),
        make: phys.make.clone(),
        model: phys.model.clone(),
        serial_number: phys.serial_number.clone(),
        physical_size: (phys.size.w, phys.size.h),
        pos,
        scale: output.current_scale().fractional_scale(),
        transform: output.current_transform().into(),
        modes: modes_snap,
        current_mode: current_idx,
        preferred_mode: preferred_idx,
        enabled,
    }
}

// ── delegate macro ───────────────────────────────────────────────────────────

#[macro_export]
macro_rules! delegate_output_management {
    ($ty:ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!(
            $ty: [
                smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::ZwlrOutputManagerV1: $crate::protocols::output_management::OutputManagementManagerGlobalData
            ] => $crate::protocols::output_management::OutputManagementManagerState
        );
        smithay::reexports::wayland_server::delegate_dispatch!(
            $ty: [
                smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::ZwlrOutputManagerV1: ()
            ] => $crate::protocols::output_management::OutputManagementManagerState
        );
        smithay::reexports::wayland_server::delegate_dispatch!(
            $ty: [
                smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_head_v1::ZwlrOutputHeadV1: String
            ] => $crate::protocols::output_management::OutputManagementManagerState
        );
        smithay::reexports::wayland_server::delegate_dispatch!(
            $ty: [
                smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_mode_v1::ZwlrOutputModeV1: ()
            ] => $crate::protocols::output_management::OutputManagementManagerState
        );
        smithay::reexports::wayland_server::delegate_dispatch!(
            $ty: [
                smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_v1::ZwlrOutputConfigurationV1: u32
            ] => $crate::protocols::output_management::OutputManagementManagerState
        );
        smithay::reexports::wayland_server::delegate_dispatch!(
            $ty: [
                smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1: $crate::protocols::output_management::OutputConfigurationHeadState
            ] => $crate::protocols::output_management::OutputManagementManagerState
        );
    };
}
