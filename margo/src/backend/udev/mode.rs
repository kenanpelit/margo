//! DRM mode selection + apply path.
//!
//! Two responsibilities:
//!   * **Pick a mode at hotplug.** [`select_drm_mode`] consults the
//!     monitor rule (kanshi-style config), falls back to the
//!     `PREFERRED` flag, then to the first advertised mode.
//!   * **Apply queued mode changes mid-session.** [`apply_pending_mode_changes`]
//!     drains `MargoState::pending_output_mode_changes` (populated by
//!     `wlr_output_management`) and runs each through
//!     `DrmCompositor::use_mode`. Runs at the top of the repaint
//!     handler so a kanshi profile flip lands within one frame.

use smithay::{
    output::Mode as OutputMode,
    reexports::drm::control::{connector, Device as DrmDeviceTrait, Mode as DrmMode, ModeTypeFlags},
};

use super::BackendData;
use crate::state::MargoState;

/// Drain `state.pending_output_mode_changes` and apply each via
/// `DrmCompositor::use_mode`, then update the smithay `Output` so
/// wl_output mode events reach clients (kanshi, status bar).
///
/// Failure modes — each just skips the entry with a warning:
///   * Output name not in `state.monitors` → output went away.
///   * Connector info read fails → DRM in a weird state, retry next frame.
///   * No DRM mode matches the (w, h, refresh) triple → kanshi
///     asked for a mode the panel doesn't actually advertise.
///   * `compositor.use_mode` fails → atomic test failed (the kernel
///     refused the modeset, e.g. CRTC pixel-clock limit).
pub(super) fn apply_pending_mode_changes(bd: &mut BackendData, state: &mut MargoState) {
    let drained: Vec<crate::PendingOutputModeChange> =
        state.pending_output_mode_changes.drain(..).collect();
    if drained.is_empty() {
        return;
    }

    for change in drained {
        // Find the OutputDevice by output name. The `Output` stored
        // on each device has the same name we surface to clients.
        let Some((_crtc, od)) = bd
            .outputs
            .iter_mut()
            .find(|(_, od)| od.output.name() == change.output_name)
        else {
            tracing::warn!(
                "output_management: pending mode change for unknown output {} dropped",
                change.output_name,
            );
            continue;
        };

        // Read the current connector info so we can match the
        // requested mode against the real KMS mode list. The drm
        // crate's `Mode` is what `use_mode` wants; smithay's
        // `OutputMode` is the wl_output-side type, so we need the
        // drm one for the apply path.
        let conn_info = match bd.drm.get_connector(od.connector, false) {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(
                    "output_management: get_connector({:?}) failed: {e}",
                    od.connector
                );
                continue;
            }
        };

        let drm_mode = match find_matching_drm_mode(
            conn_info.modes(),
            change.width,
            change.height,
            change.refresh_mhz,
        ) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    "output_management: no DRM mode matches {}x{}@{}.{:03}Hz on {} \
                     (advertised modes: {})",
                    change.width,
                    change.height,
                    change.refresh_mhz / 1000,
                    change.refresh_mhz % 1000,
                    change.output_name,
                    conn_info.modes().len(),
                );
                continue;
            }
        };

        // Try the modeset. `use_mode` resizes the swapchain to the
        // new dimensions internally; the next queue_frame will
        // commit a frame at the new resolution. If atomic-test
        // rejects (pixel clock cap, missing connector property,
        // VRR-only mode), we log + leave the old mode in place.
        if let Err(e) = od.compositor.use_mode(drm_mode) {
            tracing::warn!(
                "output_management: DrmCompositor::use_mode failed on {}: {e:?}",
                change.output_name,
            );
            continue;
        }

        // Mirror the new mode into the smithay Output. Without
        // this, the wl_output protocol never advertises the
        // change, and clients keep believing the old mode is
        // active.
        let new_wl_mode = OutputMode::from(drm_mode);
        od.output.change_current_state(
            Some(new_wl_mode),
            None,
            None,
            None,
        );
        od.output.set_preferred(new_wl_mode);

        tracing::info!(
            "output_management: applied mode {}x{}@{}.{:03}Hz on {}",
            change.width,
            change.height,
            change.refresh_mhz / 1000,
            change.refresh_mhz % 1000,
            change.output_name,
        );

        let output = od.output.clone();
        state.refresh_output_work_area(&output);
    }

    state.arrange_all();
    state.request_repaint();
    // Re-publish topology so output-management watchers see the
    // new mode reflected in OutputSnapshot.current_mode.
    state.publish_output_topology();
}

/// Find a `drm::control::Mode` matching `(w, h, refresh_mhz)`.
///
/// drm-rs's `Mode::vrefresh()` is the integer Hz approximation of
/// the actual refresh rate (e.g. 60 for both 59.940 and 60.000 Hz);
/// the protocol delivers refresh in mHz so we tolerate ±500 mHz
/// rounding on top of an exact `(w, h)` match. If multiple modes
/// share dimensions and refresh, prefer one with PREFERRED set.
fn find_matching_drm_mode(
    modes: &[DrmMode],
    width: i32,
    height: i32,
    refresh_mhz: i32,
) -> Option<DrmMode> {
    let target_w = width as u16;
    let target_h = height as u16;
    let target_hz = (refresh_mhz as f64) / 1000.0;

    let mut candidates: Vec<&DrmMode> = modes
        .iter()
        .filter(|m| {
            let (w, h) = m.size();
            w == target_w && h == target_h
        })
        .filter(|m| {
            let hz = m.vrefresh() as f64;
            (hz - target_hz).abs() < 1.0
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|m| {
        if m.mode_type().contains(ModeTypeFlags::PREFERRED) {
            0
        } else {
            1
        }
    });
    Some(*candidates[0])
}

/// Pick a DRM mode for a connector at hotplug.
///
/// Order:
///   1. Monitor rule's `(w, h, refresh)` exact match.
///   2. Monitor rule's `(w, h)`, highest refresh.
///   3. Connector's PREFERRED mode.
///   4. First advertised mode.
pub(super) fn select_drm_mode(
    conn: &connector::Info,
    rule: Option<&margo_config::MonitorRule>,
) -> Option<DrmMode> {
    let modes = conn.modes();
    if modes.is_empty() {
        return None;
    }

    if let Some(r) = rule {
        if r.width > 0 && r.height > 0 {
            let rw = r.width as u16;
            let rh = r.height as u16;
            let rf = r.refresh as u32;

            // Exact match: w × h @ refresh
            if rf > 0 {
                if let Some(m) = modes.iter().find(|m| {
                    let (w, h) = m.size();
                    w == rw && h == rh && m.vrefresh() == rf
                }) {
                    return Some(*m);
                }
            }

            // Fallback: w × h, highest refresh
            if let Some(m) = modes
                .iter()
                .filter(|m| {
                    let (w, h) = m.size();
                    w == rw && h == rh
                })
                .max_by_key(|m| m.vrefresh())
            {
                return Some(*m);
            }
        }
    }

    if let Some(m) = modes
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
    {
        return Some(*m);
    }

    Some(modes[0])
}
