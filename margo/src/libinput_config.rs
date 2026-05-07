use margo_config::{
    AccelProfile as ConfigAccelProfile, ClickMethod as ConfigClickMethod, Config,
    ScrollMethod as ConfigScrollMethod,
};
use smithay::reexports::input::{
    AccelProfile, ClickMethod, Device, DeviceCapability, DeviceConfigError, DeviceConfigResult,
    DragLockState, ScrollMethod, SendEventsMode,
};
use tracing::{debug, warn};

pub fn apply_to_device(device: &mut Device, config: &Config) {
    let name = device.name().into_owned();
    let sysname = device.sysname().to_string();
    let is_pointer = device.has_capability(DeviceCapability::Pointer);
    let is_touchpad = device.config_tap_finger_count() > 0;

    if config.disable_trackpad && is_touchpad {
        log_config_result(
            &name,
            &sysname,
            "send_events_mode",
            device.config_send_events_set_mode(SendEventsMode::DISABLED),
        );
    } else {
        let mode = SendEventsMode::from_bits_truncate(config.send_events_mode);
        log_config_result(
            &name,
            &sysname,
            "send_events_mode",
            device.config_send_events_set_mode(mode),
        );
    }

    if is_pointer && device.config_accel_is_available() {
        if let Some(profile) = map_accel_profile(config.accel_profile) {
            if device.config_accel_profiles().contains(&profile) {
                log_config_result(
                    &name,
                    &sysname,
                    "accel_profile",
                    device.config_accel_set_profile(profile),
                );
            }
        }

        log_config_result(
            &name,
            &sysname,
            "accel_speed",
            device.config_accel_set_speed(config.accel_speed.clamp(-1.0, 1.0)),
        );
    }

    if is_touchpad {
        log_config_result(
            &name,
            &sysname,
            "tap_to_click",
            device.config_tap_set_enabled(config.tap_to_click),
        );
        log_config_result(
            &name,
            &sysname,
            "tap_and_drag",
            device.config_tap_set_drag_enabled(config.tap_and_drag),
        );
        log_config_result(
            &name,
            &sysname,
            "drag_lock",
            device.config_tap_set_drag_lock_enabled(if config.drag_lock {
                DragLockState::EnabledTimeout
            } else {
                DragLockState::Disabled
            }),
        );

        if device.config_dwt_is_available() {
            log_config_result(
                &name,
                &sysname,
                "disable_while_typing",
                device.config_dwt_set_enabled(config.disable_while_typing),
            );
        }
    }

    if device.config_scroll_has_natural_scroll() {
        let natural_scroll = if is_touchpad {
            config.trackpad_natural_scrolling
        } else {
            config.mouse_natural_scrolling
        };
        log_config_result(
            &name,
            &sysname,
            "natural_scrolling",
            device.config_scroll_set_natural_scroll_enabled(natural_scroll),
        );
    }

    let scroll_method = map_scroll_method(config.scroll_method);
    if device.config_scroll_methods().contains(&scroll_method) {
        log_config_result(
            &name,
            &sysname,
            "scroll_method",
            device.config_scroll_set_method(scroll_method),
        );
    }

    if config.scroll_method == ConfigScrollMethod::OnButtonDown {
        log_config_result(
            &name,
            &sysname,
            "scroll_button",
            device.config_scroll_set_button(config.scroll_button),
        );
    }

    if let Some(click_method) = map_click_method(config.click_method) {
        if device.config_click_methods().contains(&click_method) {
            log_config_result(
                &name,
                &sysname,
                "click_method",
                device.config_click_set_method(click_method),
            );
        }
    }

    if device.config_left_handed_is_available() {
        log_config_result(
            &name,
            &sysname,
            "left_handed",
            device.config_left_handed_set(config.left_handed),
        );
    }

    if device.config_middle_emulation_is_available() {
        log_config_result(
            &name,
            &sysname,
            "middle_button_emulation",
            device.config_middle_emulation_set_enabled(config.middle_button_emulation),
        );
    }

    debug!(
        device = %name,
        sysname = %sysname,
        accel_speed = config.accel_speed,
        is_touchpad,
        "applied libinput config"
    );
}

fn map_accel_profile(profile: ConfigAccelProfile) -> Option<AccelProfile> {
    match profile {
        ConfigAccelProfile::None => None,
        ConfigAccelProfile::Flat => Some(AccelProfile::Flat),
        ConfigAccelProfile::Adaptive => Some(AccelProfile::Adaptive),
    }
}

fn map_scroll_method(method: ConfigScrollMethod) -> ScrollMethod {
    match method {
        ConfigScrollMethod::NoScroll => ScrollMethod::NoScroll,
        ConfigScrollMethod::TwoFinger => ScrollMethod::TwoFinger,
        ConfigScrollMethod::Edge => ScrollMethod::Edge,
        ConfigScrollMethod::OnButtonDown => ScrollMethod::OnButtonDown,
    }
}

fn map_click_method(method: ConfigClickMethod) -> Option<ClickMethod> {
    match method {
        ConfigClickMethod::None => None,
        ConfigClickMethod::ButtonAreas => Some(ClickMethod::ButtonAreas),
        ConfigClickMethod::Clickfinger => Some(ClickMethod::Clickfinger),
    }
}

fn log_config_result(device: &str, sysname: &str, option: &str, result: DeviceConfigResult) {
    match result {
        Ok(()) => {}
        Err(DeviceConfigError::Unsupported) => {
            debug!(device, sysname, option, "libinput config unsupported");
        }
        Err(DeviceConfigError::Invalid) => {
            warn!(device, sysname, option, "libinput config invalid");
        }
    }
}
