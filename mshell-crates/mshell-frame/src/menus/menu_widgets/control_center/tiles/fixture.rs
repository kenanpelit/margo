//! Render-fixture state for the tile grid: canned, realistic values pushed
//! over the live ones so a fixture screenshot never shows this machine's
//! SSID, devices or disks. Test-only.

use super::ControlCenterTilesModel;
use super::tile_probes::{BatterySnapshot, DiskUsage};
use mshell_utils::denoise::DenoiseStatus;
use mshell_utils::hotspot::HotspotStatus;
use std::path::PathBuf;
use wayle_battery::types::DeviceState;

#[derive(Debug)]
pub(crate) struct TilesFixture {
    /// Battery present (laptop) or absent (desktop).
    pub(crate) battery: bool,
    /// ThinkPad lid LED present.
    pub(crate) led: bool,
}

impl TilesFixture {
    pub(crate) fn apply(&self, m: &mut ControlCenterTilesModel) {
        m.wifi_subtitle = "Home-5G · 82%".into();
        m.wifi_connected = true;
        m.bt_subtitle = "2 connected".into();
        m.bt_connected = true;
        m.audio_out_subtitle = "Speakers · 45%".into();
        m.mic_subtitle = "Built-in mic · 60%".into();
        m.vpn_subtitle = "Off".into();
        m.vpn_connected = false;
        m.valent_subtitle = "Pixel · 71%".into();
        m.valent_connected = true;
        m.keep_awake = false;
        m.dnd = false;
        m.twilight_enabled = true;
        m.twilight_subtitle = "Schedule · 4000K".into();
        m.airplane_mode = false;
        m.airplane_available = true;
        m.ufw_subtitle = "Active".into();
        m.ufw_active = true;
        m.podman_subtitle = "Stopped".into();
        m.podman_active = false;
        m.mirror_subtitle = "Off".into();
        m.mirror_active = false;
        m.hotspot = HotspotStatus {
            available: true,
            ..HotspotStatus::default()
        };
        m.hotspot_busy = false;
        m.denoise = DenoiseStatus {
            plugin: Some(PathBuf::from("/usr/lib/ladspa/librnnoise_ladspa.so")),
            tools: true,
            running: false,
            enabled: false,
            mic: None,
        };
        m.denoise_busy = false;
        m.led.available = self.led;
        m.led.on = false;
        m.led.busy = false;
        m.disk = DiskUsage {
            used_bytes: 182 << 30,
            total_bytes: 476 << 30,
        };
        m.power_profile = wayle_power_profiles::types::profile::PowerProfile::Balanced;
        m.battery = if self.battery {
            BatterySnapshot {
                present: true,
                percent: 76,
                state: DeviceState::Discharging,
                on_ac: false,
            }
        } else {
            BatterySnapshot::default()
        };
    }
}
