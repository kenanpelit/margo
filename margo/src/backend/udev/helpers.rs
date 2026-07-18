//! Small standalone helpers extracted from `udev/mod.rs` during the
//! W4.1 split. No state, no MargoState reach — pure utilities that
//! every udev sub-module can depend on.

use smithay::{
    backend::drm::DrmDeviceFd,
    output::Output,
    reexports::drm::control::{connector, crtc},
    utils::Transform,
};

/// Map margo's integer transform-id (parsed from the config file —
/// matches the dwl convention) to smithay's `Transform` enum.
pub(super) fn smithay_transform(n: i32) -> Transform {
    match n {
        1 => Transform::_90,
        2 => Transform::_180,
        3 => Transform::_270,
        4 => Transform::Flipped,
        5 => Transform::Flipped90,
        6 => Transform::Flipped180,
        7 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

/// Pick the first CRTC compatible with `conn` that isn't already in
/// `used_crtcs`. Returns `None` when every encoder's CRTC bitmask
/// collides with the in-use set — caller treats that as a hotplug
/// race and skips the connector for this pass.
pub(super) fn find_crtc(
    drm: &DrmDeviceFd,
    conn: &connector::Info,
    resources: &smithay::reexports::drm::control::ResourceHandles,
    used_crtcs: &std::collections::HashSet<crtc::Handle>,
) -> Option<crtc::Handle> {
    use smithay::reexports::drm::control::Device as _;
    for enc_handle in conn.encoders() {
        let Ok(enc) = drm.get_encoder(*enc_handle) else {
            continue;
        };
        for c in resources.filter_crtcs(enc.possible_crtcs()) {
            if !used_crtcs.contains(&c) {
                return Some(c);
            }
        }
    }
    None
}

/// Whether a connector carries the DRM "non-desktop" property (VR
/// headsets and similar) — margo never drives those as outputs and
/// offers them for `wp_drm_lease_device_v1` instead.
pub(super) fn connector_is_non_desktop(drm: &DrmDeviceFd, conn: connector::Handle) -> bool {
    use smithay::reexports::drm::control::Device as _;
    let Ok(props) = drm.get_properties(conn) else {
        return false;
    };
    for (prop, value) in props.iter() {
        let Ok(info) = drm.get_property(*prop) else {
            continue;
        };
        if info.name().to_str() == Ok("non-desktop") {
            return info
                .value_type()
                .convert_value(*value)
                .as_boolean()
                .unwrap_or(false);
        }
    }
    false
}

/// Frame interval derived from the output's current mode. Falls
/// back to 60 Hz when the mode is unset or reports a zero refresh
/// rate (some virtual outputs).
pub(super) fn output_refresh_duration(output: &Output) -> std::time::Duration {
    output
        .current_mode()
        .map(|m| {
            // mode.refresh is in mHz.
            let hz = (m.refresh as f64) / 1000.0;
            if hz > 0.0 {
                std::time::Duration::from_secs_f64(1.0 / hz)
            } else {
                std::time::Duration::from_secs_f64(1.0 / 60.0)
            }
        })
        .unwrap_or_else(|| std::time::Duration::from_secs_f64(1.0 / 60.0))
}

/// Real CLOCK_MONOTONIC time as a `Duration`. Used as the `now`
/// argument for `wp_presentation_feedback.presented` and wlr-screencopy
/// `ready`. These timestamps MUST be in CLOCK_MONOTONIC — the same clock
/// margo advertises (`PresentationState::new`) and that clients read via
/// `clock_gettime` — or A/V-sync consumers (mpv --vo=gpu-next) see the
/// presentation time offset by the machine's uptime-at-launch. The old
/// process-start `Instant` epoch broke that; delegate to the shared
/// clock helper pw_utils already uses.
pub(super) fn monotonic_now() -> std::time::Duration {
    crate::utils::get_monotonic_time()
}
