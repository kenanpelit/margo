//! Hotplug rescan path.
//!
//! Triggered from `UdevEvent::Changed`. Three responsibilities:
//!   1. Drop every `OutputDevice` whose specific connector is no longer
//!      `Connected` (laptop dock unplug, monitor cable pulled).
//!   2. Migrate clients that lived on the removed monitor to a
//!      surviving one — without this they keep `c.monitor = <stale>`
//!      and disappear from the layout.
//!   3. Add new `OutputDevice`s for connectors that just came up.
//!      Each fresh device gets an initial `render_frame` + `queue_frame`
//!      so the new monitor lights up without waiting for the next
//!      repaint timer tick.

use std::{cell::RefCell, rc::Rc};

use drm_fourcc::DrmFourcc;
use smithay::{
    backend::{
        allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        drm::{
            compositor::{DrmCompositor, FrameFlags},
            exporter::gbm::GbmFramebufferExporter,
            DrmDevice, DrmDeviceFd, DrmNode,
        },
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::drm::control::{connector, crtc, Device as DrmDeviceTrait},
};
use tracing::{info, warn};

use super::{
    build_render_elements,
    helpers::{find_crtc, smithay_transform},
    mode::select_drm_mode,
    BackendData, GammaProps, OutputDevice,
};
use crate::state::MargoState;

pub(super) fn rescan_outputs(
    backend_data: &Rc<RefCell<BackendData>>,
    state: &mut MargoState,
) {
    // Phase 1: remove disconnected outputs.
    let mut bd = backend_data.borrow_mut();
    let BackendData {
        renderer: _,
        outputs,
        drm,
        gbm: _,
        primary_node: _,
        renderer_formats: _,
    } = &mut *bd;

    let mut to_remove: Vec<crtc::Handle> = Vec::new();
    for (crtc_h, od) in outputs.iter() {
        let still_connected = drm
            .get_connector(od.connector, false)
            .map(|c| c.state() == connector::State::Connected)
            .unwrap_or(false);
        if !still_connected {
            tracing::info!(
                output = %od.output.name(),
                crtc = ?crtc_h,
                "output disconnected",
            );
            to_remove.push(*crtc_h);
        }
    }

    let removed_outputs: Vec<Output> = to_remove
        .into_iter()
        .filter_map(|crtc_h| outputs.remove(&crtc_h).map(|od| od.output))
        .collect();
    drop(bd);

    for output in &removed_outputs {
        migrate_clients_off_output(state, output);
        state.remove_output(output);
    }

    // Phase 2: add newly-connected outputs.
    let mut added_any = false;
    let mut bd = backend_data.borrow_mut();
    let used_crtcs: std::collections::HashSet<crtc::Handle> =
        bd.outputs.keys().copied().collect();
    let resources = match bd.drm.resource_handles() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("rescan: resource_handles failed: {e}");
            drop(bd);
            if !removed_outputs.is_empty() {
                state.arrange_all();
                state.request_repaint();
            }
            return;
        }
    };

    let mut current_used = used_crtcs.clone();
    let mut new_outputs: Vec<(crtc::Handle, OutputDevice)> = Vec::new();
    for conn_handle in resources.connectors() {
        if bd.outputs.values().any(|od| od.connector == *conn_handle) {
            continue;
        }
        let Ok(conn_info) = bd.drm.get_connector(*conn_handle, false) else {
            continue;
        };
        if conn_info.state() != connector::State::Connected {
            continue;
        }
        let BackendData {
            drm,
            gbm,
            primary_node,
            renderer_formats,
            ..
        } = &mut *bd;

        if let Some((crtc_h, od)) = setup_connector(
            drm,
            *conn_handle,
            &conn_info,
            &resources,
            &current_used,
            state,
            gbm,
            *primary_node,
            renderer_formats,
        ) {
            current_used.insert(crtc_h);
            new_outputs.push((crtc_h, od));
            added_any = true;
        }
    }

    for (crtc_h, mut od) in new_outputs {
        // Kick the swapchain so the freshly-built compositor schedules a
        // first vblank — otherwise the new monitor stays blank until the
        // global repaint timer happens to tick *and* something on the
        // existing outputs marks itself dirty.
        let elements = build_render_elements(&mut bd.renderer, &od, state);
        match od.compositor.render_frame(
            &mut bd.renderer,
            &elements,
            [0.1, 0.1, 0.1, 1.0],
            FrameFlags::DEFAULT,
        ) { Err(e) => {
            tracing::warn!("hotplug initial render failed for {}: {e:?}", od.output.name());
        } _ => {
            let _ = od.compositor.queue_frame(());
        }}
        bd.outputs.insert(crtc_h, od);
    }
    drop(bd);

    if !removed_outputs.is_empty() || added_any {
        state.arrange_all();
        state.request_repaint();
    }
    state.publish_output_topology();
}

/// Build the `OutputDevice` + associated `MargoMonitor` for a single
/// connected connector. Mirrors the inline init loop so that hotplug
/// goes through exactly the same code path as startup.
#[allow(clippy::too_many_arguments)]
pub(super) fn setup_connector(
    drm: &mut DrmDevice,
    conn_handle: connector::Handle,
    conn_info: &connector::Info,
    resources: &smithay::reexports::drm::control::ResourceHandles,
    used_crtcs: &std::collections::HashSet<crtc::Handle>,
    state: &mut MargoState,
    gbm: &GbmDevice<DrmDeviceFd>,
    primary_node: DrmNode,
    renderer_formats: &smithay::backend::allocator::format::FormatSet,
) -> Option<(crtc::Handle, OutputDevice)> {
    let crtc_h = find_crtc(&drm.device_fd().clone(), conn_info, resources, used_crtcs)?;

    let (phys_w, phys_h) = conn_info.size().unwrap_or((0, 0));
    let output_name = format!(
        "{}-{}",
        conn_info.interface().as_str(),
        conn_info.interface_id()
    );

    let rule = state
        .config
        .monitor_rules
        .iter()
        .find(|r| r.name.as_deref().map(|n| n == output_name).unwrap_or(true))
        .cloned();

    let drm_mode = select_drm_mode(conn_info, rule.as_ref())?;
    let wl_mode = OutputMode::from(drm_mode);

    let scale = rule.as_ref().map(|r| r.scale).unwrap_or(1.0);
    let transform = smithay_transform(rule.as_ref().map(|r| r.transform).unwrap_or(0));

    let position = if let Some(r) = &rule {
        if r.x != i32::MAX && r.y != i32::MAX {
            (r.x, r.y)
        } else {
            let x_offset = state.space.outputs().fold(0i32, |acc, o| {
                acc + state.space.output_geometry(o).map(|g| g.size.w).unwrap_or(0)
            });
            (x_offset, 0)
        }
    } else {
        let x_offset = state.space.outputs().fold(0i32, |acc, o| {
            acc + state.space.output_geometry(o).map(|g| g.size.w).unwrap_or(0)
        });
        (x_offset, 0)
    };

    info!(
        output = %output_name,
        width = wl_mode.size.w,
        height = wl_mode.size.h,
        refresh_hz = wl_mode.refresh / 1000,
        pos_x = position.0,
        pos_y = position.1,
        scale = scale,
        "hotplug add",
    );

    let output = Output::new(
        output_name.clone(),
        PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: Subpixel::Unknown,
            make: "Unknown".into(),
            model: "Unknown".into(),
            serial_number: "Unknown".into(),
        },
    );
    let _global = output.create_global::<MargoState>(&state.display_handle);
    output.change_current_state(
        Some(wl_mode),
        Some(transform),
        Some(smithay::output::Scale::Fractional(scale as f64)),
        Some(position.into()),
    );
    output.set_preferred(wl_mode);
    state.space.map_output(&output, position);

    let drm_surface = match drm.create_surface(crtc_h, drm_mode, &[conn_handle]) {
        Ok(s) => s,
        Err(e) => {
            warn!("hotplug create_surface for {output_name}: {e}");
            return None;
        }
    };

    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), primary_node.into());
    let color_formats = [DrmFourcc::Xrgb8888, DrmFourcc::Argb8888];
    // Use device-reported cursor plane size (matches the startup path).
    let cursor_size = {
        let s = drm.cursor_size();
        if s.w == 0 || s.h == 0 {
            (64u32, 64u32).into()
        } else {
            s
        }
    };
    let compositor = match DrmCompositor::new(
        &output,
        drm_surface,
        None,
        allocator,
        exporter,
        color_formats.iter().copied(),
        renderer_formats.clone(),
        cursor_size,
        Some(gbm.clone()),
    ) {
        Ok(c) => c,
        Err(e) => {
            warn!("hotplug DrmCompositor::new for {output_name}: {e:?}");
            return None;
        }
    };

    let monitor_area = crate::layout::Rect {
        x: position.0,
        y: position.1,
        width: wl_mode.size.w,
        height: wl_mode.size.h,
    };
    let pertag = crate::layout::Pertag::new(
        state.default_layout(),
        state.config.default_mfact,
        state.config.default_nmaster,
    );
    state.monitors.push(crate::state::MargoMonitor {
        name: output_name.clone(),
        output: output.clone(),
        monitor_area,
        work_area: monitor_area,
        seltags: 0,
        tagset: [1, 1],
        gappih: state.config.gappih as i32,
        gappiv: state.config.gappiv as i32,
        gappoh: state.config.gappoh as i32,
        gappov: state.config.gappov as i32,
        pertag,
        selected: None,
        prev_selected: None,
        is_overview: false,
        overview_backup_tagset: 1,
        canvas_overview_visible: false,
        canvas_in_overview: false,
        canvas_saved_pan_x: 0.0,
        canvas_saved_pan_y: 0.0,
        canvas_saved_zoom: 1.0,
        minimap_visible: false,
        dwl_ipc: crate::protocols::dwl_ipc::DwlIpcState::new(),
        ext_workspace: crate::protocols::ext_workspace::ExtWorkspaceState::new(),
        scale: 1.0,
        transform: 0,
        enabled: true,
        gamma_size: 0,
        focus_history: std::collections::VecDeque::new(),
    });
    state.apply_tag_rules_to_monitor(state.monitors.len() - 1);
    // Hotplug-in (live-connector path): keep the shared
    // ipc_outputs snapshot in sync so xdp-gnome's chooser
    // dialog picks up the new monitor without a margo restart.
    state.refresh_ipc_outputs();

    let mut gamma_props = GammaProps::discover(drm, crtc_h);
    if let Some(gamma) = gamma_props.as_mut() {
        if let Err(err) = gamma.set_gamma(drm, None) {
            tracing::debug!("couldn't reset gamma on {output_name}: {err:?}");
        }
    }
    let gamma_size = gamma_props
        .as_ref()
        .and_then(|g| g.gamma_size(drm))
        .unwrap_or(0);
    let mon_idx = state.monitors.len() - 1;
    state.monitors[mon_idx].gamma_size = gamma_size;

    Some((
        crtc_h,
        OutputDevice {
            output,
            compositor,
            render_count: 0,
            queued_count: 0,
            empty_count: 0,
            queue_error_count: 0,
            gamma: gamma_props,
            connector: conn_handle,
            pending_presentation: Vec::new(),
            vblank_seq: 0,
        },
    ))
}

fn migrate_clients_off_output(state: &mut MargoState, removed: &Output) {
    let removed_idx = state
        .monitors
        .iter()
        .position(|m| &m.output == removed);
    let Some(removed_idx) = removed_idx else { return };

    let target_idx = state
        .monitors
        .iter()
        .enumerate()
        .find(|(i, _)| *i != removed_idx)
        .map(|(i, _)| i);

    let Some(target_idx) = target_idx else {
        return;
    };

    let target_tagset = state.monitors[target_idx].current_tagset();
    let target_name = state.monitors[target_idx].name.clone();

    let target_after = if target_idx > removed_idx {
        target_idx - 1
    } else {
        target_idx
    };

    let mut migrated = 0;
    for client in state.clients.iter_mut() {
        if client.monitor == removed_idx {
            client.monitor = target_after;
            if client.tags & target_tagset == 0 {
                client.tags |= target_tagset;
            }
            migrated += 1;
        } else if client.monitor > removed_idx {
            client.monitor -= 1;
        }
    }
    if migrated > 0 {
        tracing::info!(
            migrated = migrated,
            from = %removed.name(),
            to = %target_name,
            "migrated clients off removed output",
        );
    }
}
