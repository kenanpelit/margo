#![allow(dead_code)]
//! Margo equivalents of niri's `IpcOutputMap` + `niri_ipc::Output` —
//! a structurally-compatible stand-in so the ported D-Bus shims compile
//! against the same shape they do in niri.
//!
//! These are only used as the message-payload format the
//! `mutter_screen_cast.rs` interface stores per-session and the
//! `mutter_display_config.rs` interface reports back to xdp-gnome.
//! Margo's actual outputs live as `MargoMonitor` on `MargoState`; a
//! snapshot helper builds a fresh `IpcOutputMap` whenever the caller
//! needs to hand one to a D-Bus server.

use std::collections::HashMap;

/// Stable opaque ID used as the map key. Matches niri's `OutputId`
/// shape (a u64 wrapper) so the call sites port without changes.
pub type OutputId = u64;

/// Map of every active output. Identical wire shape to niri so the
/// ported D-Bus interfaces drop in unchanged.
pub type IpcOutputMap = HashMap<OutputId, IpcOutput>;

/// Snapshot of one output, mirroring `niri_ipc::Output` field-by-field
/// for the bits screencast actually uses (`name`, `logical`).
#[derive(Debug, Clone)]
pub struct IpcOutput {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: Option<String>,
    pub physical_size: Option<(u32, u32)>,
    pub logical: Option<IpcLogicalOutput>,
    pub modes: Vec<IpcMode>,
    pub current_mode: Option<usize>,
    pub vrr_supported: bool,
    pub vrr_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct IpcLogicalOutput {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub transform: IpcTransform,
}

#[derive(Debug, Clone, Copy)]
pub struct IpcMode {
    pub width: u16,
    pub height: u16,
    pub refresh_rate: u32,
    pub is_preferred: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcTransform {
    Normal,
    _90,
    _180,
    _270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl IpcTransform {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::_90 => 1,
            Self::_180 => 2,
            Self::_270 => 3,
            Self::Flipped => 4,
            Self::Flipped90 => 5,
            Self::Flipped180 => 6,
            Self::Flipped270 => 7,
        }
    }
}

/// Build a fresh `IpcOutputMap` from the live `MargoState::monitors`.
/// xdp-gnome calls `GetCurrentState` / monitors listing on every
/// chooser dialog so taking a fresh snapshot per call is fine
/// (small N — usually 1-3 outputs).
///
/// Output IDs are stable for the duration of a margo session: we
/// hash the connector name into a u64 so the same monitor maps to
/// the same OutputId across snapshots, which is what xdp-gnome
/// expects.
pub fn snapshot(state: &crate::state::MargoState) -> IpcOutputMap {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut map = IpcOutputMap::new();
    for mon in &state.monitors {
        let mode = mon.output.current_mode();
        let phys = mon.output.physical_properties();

        let modes: Vec<IpcMode> = mon
            .output
            .modes()
            .iter()
            .map(|m| IpcMode {
                width: m.size.w as u16,
                height: m.size.h as u16,
                refresh_rate: m.refresh as u32,
                is_preferred: mon
                    .output
                    .preferred_mode()
                    .is_some_and(|p| p.size == m.size && p.refresh == m.refresh),
            })
            .collect();

        let current_mode_idx = mode.and_then(|cm| {
            mon.output
                .modes()
                .iter()
                .position(|m| m.size == cm.size && m.refresh == cm.refresh)
        });

        let logical = Some(IpcLogicalOutput {
            x: mon.monitor_area.x,
            y: mon.monitor_area.y,
            width: mon.monitor_area.width.max(0) as u32,
            height: mon.monitor_area.height.max(0) as u32,
            scale: mon.scale as f64,
            transform: IpcTransform::Normal,
        });

        let mut hasher = DefaultHasher::new();
        mon.name.hash(&mut hasher);
        let id: OutputId = hasher.finish();

        map.insert(
            id,
            IpcOutput {
                name: mon.name.clone(),
                make: phys.make.clone(),
                model: phys.model.clone(),
                serial: Some(phys.serial_number.clone()).filter(|s| !s.is_empty()),
                physical_size: Some((phys.size.w as u32, phys.size.h as u32))
                    .filter(|(w, h)| *w > 0 && *h > 0),
                logical,
                modes,
                current_mode: current_mode_idx,
                vrr_supported: false,
                vrr_enabled: false,
            },
        );
    }
    map
}
