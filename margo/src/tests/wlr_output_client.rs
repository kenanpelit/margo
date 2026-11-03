//! Client side of `zwlr_output_manager_v1` for the fixture: what
//! `wlr-randr` / `kanshi` do — read the heads and their modes, then build
//! and apply a configuration.

use std::collections::BTreeMap;

use smithay::reexports::wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};

use super::client::{Client, ClientState};

/// The outcome the compositor sent for a configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// One advertised mode: the head that announced it and its
/// `(width, height, refresh_mhz)`.
#[derive(Debug, Clone)]
pub struct AdvertisedMode {
    pub mode: ZwlrOutputModeV1,
    pub head: ZwlrOutputHeadV1,
    pub size_refresh: (i32, i32, i32),
}

#[derive(Debug, Default)]
pub struct OutputManagementEvents {
    /// Serial of the last `done`.
    pub serial: Option<u32>,
    /// Every head announced so far, with its name.
    pub heads: Vec<(ZwlrOutputHeadV1, String)>,
    pub modes: Vec<AdvertisedMode>,
    /// Outcome per configuration (keyed by protocol id).
    pub outcomes: BTreeMap<u32, ConfigOutcome>,
}

impl OutputManagementEvents {
    /// The most recently announced head named `name`.
    pub fn head(&self, name: &str) -> ZwlrOutputHeadV1 {
        self.heads
            .iter()
            .rev()
            .find(|(_, n)| n == name)
            .map(|(h, _)| h.clone())
            .unwrap_or_else(|| panic!("no head {name}"))
    }

    /// The mode `head` advertised as `(width, height, refresh_mhz)`.
    pub fn mode(&self, head: &ZwlrOutputHeadV1, size_refresh: (i32, i32, i32)) -> ZwlrOutputModeV1 {
        self.modes
            .iter()
            .find(|m| m.head == *head && m.size_refresh == size_refresh)
            .map(|m| m.mode.clone())
            .unwrap_or_else(|| panic!("{head:?} advertised no mode {size_refresh:?}"))
    }

    pub fn outcome(&self, cfg: &ZwlrOutputConfigurationV1) -> Option<ConfigOutcome> {
        self.outcomes.get(&cfg.id().protocol_id()).copied()
    }
}

impl Client {
    pub fn bind_output_manager(&mut self) -> ZwlrOutputManagerV1 {
        let manager = self.bind_global(4);
        self.connection.flush().expect("client flush");
        manager
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                state.output_management.heads.push((head, String::new()));
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.output_management.serial = Some(serial);
            }
            _ => {}
        }
    }

    event_created_child!(ClientState, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let om = &mut state.output_management;
        match event {
            zwlr_output_head_v1::Event::Name { name } => {
                if let Some(entry) = om.heads.iter_mut().find(|(h, _)| h == head) {
                    entry.1 = name;
                }
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                om.modes.push(AdvertisedMode {
                    mode,
                    head: head.clone(),
                    size_refresh: (0, 0, 0),
                });
            }
            _ => {}
        }
    }

    event_created_child!(ClientState, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(m) = state
            .output_management
            .modes
            .iter_mut()
            .find(|m| m.mode == *mode)
        else {
            return;
        };
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                m.size_refresh.0 = width;
                m.size_refresh.1 = height;
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => m.size_refresh.2 = refresh,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        cfg: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let outcome = match event {
            zwlr_output_configuration_v1::Event::Succeeded => ConfigOutcome::Succeeded,
            zwlr_output_configuration_v1::Event::Failed => ConfigOutcome::Failed,
            zwlr_output_configuration_v1::Event::Cancelled => ConfigOutcome::Cancelled,
            _ => return,
        };
        state
            .output_management
            .outcomes
            .insert(cfg.id().protocol_id(), outcome);
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: <ZwlrOutputConfigurationHeadV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
