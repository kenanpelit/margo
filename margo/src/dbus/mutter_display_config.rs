//! `org.gnome.Mutter.DisplayConfig` D-Bus shim.
//!
//! xdp-gnome cross-references this interface when enumerating
//! monitors for the screencast chooser dialog. Niri's full impl
//! handles `ApplyMonitorsConfig` for runtime topology changes;
//! margo doesn't expose its own monitor topology over D-Bus
//! (we have `wlr-output-management` for that), so this is a
//! **read-only stub** that:
//!
//!   * Reports the current monitor list via `GetCurrentState`
//!     (xdp-gnome's chooser uses this to populate the Entire
//!     Screen tab).
//!   * Stubs `ApplyMonitorsConfig` to return success without
//!     changing anything (so xdp-gnome doesn't error out, but
//!     real topology changes still go through wlr-output-management
//!     / `mctl reload`).
//!
//! Source provenance: niri/src/dbus/mutter_display_config.rs —
//! we keep the read shape but drop the apply-side state plumbing
//! (~280 of the 354 niri lines), which would need a full mango-
//! style monitor-config plumbing layer in margo.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tracing::debug;
use zbus::fdo::{self, RequestNameFlags};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Type};
use zbus::{interface, zvariant};

use super::ipc_output::IpcOutputMap;
use super::Start;

pub struct DisplayConfig {
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
}

#[derive(Serialize, Type)]
pub struct Monitor {
    names: (String, String, String, String),
    modes: Vec<Mode>,
    properties: HashMap<String, OwnedValue>,
}

#[derive(Serialize, Type)]
pub struct Mode {
    id: String,
    width: i32,
    height: i32,
    refresh_rate: f64,
    preferred_scale: f64,
    supported_scales: Vec<f64>,
    properties: HashMap<String, OwnedValue>,
}

#[derive(Serialize, Type)]
pub struct LogicalMonitor {
    x: i32,
    y: i32,
    scale: f64,
    transform: u32,
    is_primary: bool,
    monitors: Vec<(String, String, String, String)>,
    properties: HashMap<String, OwnedValue>,
}

#[interface(name = "org.gnome.Mutter.DisplayConfig")]
impl DisplayConfig {
    /// Read-only snapshot of the current display state. xdp-gnome's
    /// screencast chooser uses this to list available monitors.
    /// Tuple-encoded per the protocol: serial, monitors,
    /// logical_monitors, properties.
    async fn get_current_state(
        &self,
    ) -> fdo::Result<(
        u32,
        Vec<Monitor>,
        Vec<LogicalMonitor>,
        HashMap<String, OwnedValue>,
    )> {
        debug!("DisplayConfig::get_current_state");

        let ipc_outputs = self.ipc_outputs.lock().unwrap();
        let mut monitors = Vec::with_capacity(ipc_outputs.len());
        let mut logicals = Vec::with_capacity(ipc_outputs.len());

        for output in ipc_outputs.values() {
            let modes: Vec<Mode> = output
                .modes
                .iter()
                .enumerate()
                .map(|(idx, m)| Mode {
                    id: format!("{}", idx),
                    width: m.width as i32,
                    height: m.height as i32,
                    refresh_rate: f64::from(m.refresh_rate) / 1000.0,
                    preferred_scale: 1.0,
                    supported_scales: vec![1.0, 1.25, 1.5, 1.75, 2.0],
                    properties: HashMap::new(),
                })
                .collect();

            let mut props = HashMap::new();
            if let Some((w, h)) = output.physical_size {
                props.insert(
                    "width-mm".to_string(),
                    OwnedValue::try_from(zvariant::Value::from(w as i32)).unwrap(),
                );
                props.insert(
                    "height-mm".to_string(),
                    OwnedValue::try_from(zvariant::Value::from(h as i32)).unwrap(),
                );
            }

            let names = (
                output.name.clone(),
                output.make.clone(),
                output.model.clone(),
                output.serial.clone().unwrap_or_default(),
            );
            monitors.push(Monitor {
                names: names.clone(),
                modes,
                properties: props,
            });

            if let Some(logical) = &output.logical {
                logicals.push(LogicalMonitor {
                    x: logical.x,
                    y: logical.y,
                    scale: logical.scale,
                    transform: logical.transform.as_u32(),
                    is_primary: false,
                    monitors: vec![names],
                    properties: HashMap::new(),
                });
            }
        }

        // Serial 0 — clients shouldn't try to ApplyMonitorsConfig
        // off this snapshot (we'd reject it anyway).
        Ok((0, monitors, logicals, HashMap::new()))
    }

    /// Stub: accept ApplyMonitorsConfig but do nothing. Real topology
    /// changes go through wlr-output-management / mctl reload.
    async fn apply_monitors_config(
        &self,
        _serial: u32,
        _method: u32,
        _logical_monitors: Vec<zvariant::Value<'_>>,
        _properties: HashMap<String, zvariant::Value<'_>>,
    ) -> fdo::Result<()> {
        debug!("DisplayConfig::apply_monitors_config (no-op)");
        Ok(())
    }

    #[zbus(signal)]
    pub async fn monitors_changed(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;
}

impl DisplayConfig {
    pub fn new(ipc_outputs: Arc<Mutex<IpcOutputMap>>) -> Self {
        Self { ipc_outputs }
    }
}

impl Start for DisplayConfig {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Mutter/DisplayConfig", self)?;
        conn.request_name_with_flags("org.gnome.Mutter.DisplayConfig", flags)?;

        Ok(conn)
    }
}
