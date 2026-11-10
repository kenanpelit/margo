//! Push the model's state onto the tile widgets: visibility (config +
//! edit mode) for every tile, then each stateful tile's subtitle, icon
//! and active tint.

use super::ControlCenterTilesModel;
use super::grid::ControlCenterTilesWidgets;
use super::registry::TileId;
use mshell_config::schema::themes::MatugenMode;
use mshell_utils::battery::{get_battery_icon, get_charging_battery_icon};
use mshell_utils::power_profile::get_power_profile_label;
use relm4::gtk::prelude::WidgetExt;
use wayle_battery::types::DeviceState;
use wayle_power_profiles::types::profile::PowerProfile;

pub(super) fn apply_visuals(model: &ControlCenterTilesModel, w: &ControlCenterTilesWidgets) {
    // A tile's overlay is always visible in edit mode; in normal mode only
    // enabled tiles show, and the Battery tile additionally needs a battery.
    for id in TileId::all() {
        let enabled = (id.config().visible)();
        let available = match id {
            TileId::Battery => model.battery.present,
            TileId::ThinkpadLed => model.led.available,
            _ => true,
        };
        w.update_visibility(id, model.edit_mode, enabled, available);
    }
    w.repack_dashboard();

    // Wi-Fi — active when connected to a network (has an SSID)
    let wifi = w.tile(TileId::Wifi);
    wifi.set_subtitle(&model.wifi_subtitle);
    wifi.set_active(model.wifi_connected);

    // Bluetooth — active when a device is connected
    let bluetooth = w.tile(TileId::Bluetooth);
    bluetooth.set_subtitle(&model.bt_subtitle);
    bluetooth.set_active(model.bt_connected);

    // Audio Out
    w.tile(TileId::AudioOut)
        .set_subtitle(&model.audio_out_subtitle);

    // Mic
    w.tile(TileId::Mic).set_subtitle(&model.mic_subtitle);

    // VPN — active when VPN is connected
    let vpn = w.tile(TileId::Vpn);
    vpn.set_subtitle(&model.vpn_subtitle);
    vpn.set_active(model.vpn_connected);

    // Valent — active when a device is reachable + paired
    let valent = w.tile(TileId::Valent);
    valent.set_subtitle(&model.valent_subtitle);
    valent.set_active(model.valent_connected);

    // Firewall (UFW) — active (accent) when enabled; icon reflects state.
    let ufw = w.tile(TileId::Ufw);
    ufw.set_subtitle(&model.ufw_subtitle);
    ufw.set_active(model.ufw_active);
    ufw.set_icon(if model.ufw_active {
        "shield-check-symbolic"
    } else {
        "firewall-symbolic"
    });

    // Podman — active (accent) when a machine is running / containers up.
    let podman = w.tile(TileId::Podman);
    podman.set_subtitle(&model.podman_subtitle);
    podman.set_active(model.podman_active);

    // Mirror — active (accent) while an output is being mirrored.
    let mirror = w.tile(TileId::Mirror);
    mirror.set_subtitle(&model.mirror_subtitle);
    mirror.set_active(model.mirror_active);

    // Keep Awake
    let keep_awake = w.tile(TileId::KeepAwake);
    keep_awake.set_active(model.keep_awake);
    keep_awake.set_subtitle(if model.keep_awake { "On" } else { "Off" });
    keep_awake.set_icon(if model.keep_awake {
        "eye-symbolic"
    } else {
        "eye-off-symbolic"
    });

    // DND
    let dnd = w.tile(TileId::Dnd);
    dnd.set_active(model.dnd);
    dnd.set_subtitle(if model.dnd {
        "Silenced"
    } else {
        "All notifications"
    });
    dnd.set_icon(if model.dnd {
        "notification-disabled-symbolic"
    } else {
        "notification-symbolic"
    });

    // Dark Mode — now a full labeled tile
    let dark_mode = w.tile(TileId::DarkMode);
    let is_dark = model.dark == MatugenMode::Dark;
    dark_mode.set_active(is_dark);
    dark_mode.set_subtitle(if is_dark { "On" } else { "Off" });
    dark_mode.set_icon(match model.dark {
        MatugenMode::Dark => "weather-clear-symbolic",
        MatugenMode::Light => "weather-clear-night-symbolic",
    });

    // Twilight — expandable tile; subtitle shows the active profile
    // (e.g. "Schedule · 3500K"), active tint when the filter is on.
    let night_light = w.tile(TileId::NightLight);
    night_light.set_active(model.twilight_enabled);
    night_light.set_subtitle(&model.twilight_subtitle);
    night_light.set_icon(if model.twilight_enabled {
        "night-light-symbolic"
    } else {
        "night-light-disabled-symbolic"
    });

    // Airplane Mode
    let airplane = w.tile(TileId::AirplaneMode);
    airplane.set_active(model.airplane_mode);
    airplane.set_subtitle(if model.airplane_mode { "On" } else { "Off" });

    // Hotspot
    let hotspot = w.tile(TileId::Hotspot);
    hotspot.set_active(model.hotspot.is_active());
    hotspot.set_subtitle(&if model.hotspot_busy {
        "…".to_string()
    } else {
        model.hotspot.subtitle()
    });
    hotspot
        .button
        .set_sensitive(model.hotspot.available && !model.hotspot_busy);

    // Noise suppression
    let denoise = w.tile(TileId::MicDenoise);
    denoise.set_active(model.denoise.enabled);
    denoise.set_subtitle(&if model.denoise_busy {
        "…".to_string()
    } else {
        model.denoise.subtitle()
    });
    denoise
        .button
        .set_sensitive(model.denoise.available() && !model.denoise_busy);

    // ThinkPad lid logo LED — active (accent) while lit.
    let led = w.tile(TileId::ThinkpadLed);
    led.set_active(model.led.on);
    led.set_subtitle(&model.led.subtitle());
    led.button
        .set_sensitive(model.led.available && !model.led.busy);

    // Custom shortcut — the user's label / icon; active per its state command.
    let custom = w.tile(TileId::CustomShortcut);
    if !model.custom.label.is_empty() {
        custom.set_title(&model.custom.label);
        custom.set_icon(&model.custom.icon);
        custom.set_subtitle(&model.custom.subtitle);
    }
    custom.set_active(model.custom.active);

    // Disk
    w.tile(TileId::Disk).set_subtitle(&model.disk.format());

    // Battery content update (when present) — include active power profile
    let bat = &model.battery;
    if bat.present {
        let battery = w.tile(TileId::Battery);
        let on_ac = bat.on_ac
            || bat.state == DeviceState::Charging
            || bat.state == DeviceState::FullyCharged;
        let icon = if on_ac {
            get_charging_battery_icon(bat.percent as f64)
        } else {
            get_battery_icon(bat.percent as f64)
        };
        battery.set_icon(icon);
        battery.set_subtitle(&battery_subtitle(
            bat.percent,
            bat.state,
            &model.power_profile,
        ));
    }
}

/// The Battery tile's subtitle, most useful part first so an ellipsized
/// narrow tile still shows it: "76% · Discharging · Balanced". An unknown
/// state or power profile is left out rather than printed as "Unknown".
pub(super) fn battery_subtitle(percent: u8, state: DeviceState, profile: &PowerProfile) -> String {
    let status = match state {
        DeviceState::Charging => "Charging",
        DeviceState::Discharging => "Discharging",
        DeviceState::FullyCharged => "Full",
        DeviceState::Empty => "Empty",
        DeviceState::PendingCharge | DeviceState::PendingDischarge => "Not charging",
        DeviceState::Unknown => "",
    };
    let profile = match profile {
        PowerProfile::Unknown => "",
        p => get_power_profile_label(p),
    };
    let percent = format!("{percent}%");
    [percent.as_str(), status, profile]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_subtitle_leads_with_the_charge() {
        assert_eq!(
            battery_subtitle(76, DeviceState::Discharging, &PowerProfile::Balanced),
            "76% · Discharging · Balanced"
        );
        assert_eq!(
            battery_subtitle(76, DeviceState::Discharging, &PowerProfile::Unknown),
            "76% · Discharging"
        );
        assert_eq!(
            battery_subtitle(100, DeviceState::Unknown, &PowerProfile::Unknown),
            "100%"
        );
    }
}
