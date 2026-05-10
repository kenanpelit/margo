//! Small standalone helpers extracted from `udev/mod.rs` during the
//! W4.1 split. No state, no MargoState reach — pure utilities that
//! every udev sub-module can depend on.

use smithay::{
    output::Output,
    reexports::drm::control::{connector, crtc},
    backend::drm::DrmDeviceFd,
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

/// Monotonic clock as a `Duration` since process start. Used as the
/// `now` argument for `OutputPresentationFeedback::presented` so
/// every cast/screencopy/feedback path agrees on the same epoch.
pub(super) fn monotonic_now() -> std::time::Duration {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}
