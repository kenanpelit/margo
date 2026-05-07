#![allow(dead_code)]
//! udev/DRM/KMS backend — real hardware via libseat + libinput + GBM/EGL.

use std::{
    cell::RefCell,
    collections::HashMap,
    num::NonZeroU64,
    os::unix::io::{AsFd, FromRawFd, IntoRawFd},
    rc::Rc,
    sync::Mutex,
    time::Duration,
};

use anyhow::{Context, Result};
use drm_fourcc::DrmFourcc;
use smithay::{
    backend::{
        allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        drm::{
            compositor::{DrmCompositor, FrameFlags},
            exporter::gbm::GbmFramebufferExporter,
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType,
        },
        egl::{EGLContext, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{
                AsRenderElements, Wrap,
                memory::MemoryRenderBufferRenderElement,
                render_elements,
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                Kind,
            },
            gles::{GlesRenderer, GlesTexProgram},
            ImportDma, ImportEgl,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{UdevBackend, UdevEvent, primary_gpu},
    },
    desktop::{layer_map_for_output, space::SpaceRenderElements, PopupManager, WindowSurface},
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{timer::{TimeoutAction, Timer}, EventLoop},
        drm::control::{
            property, Device as DrmDeviceTrait, Mode as DrmMode, ModeTypeFlags, connector, crtc,
        },
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, Logical, Physical, Point, Rectangle, Scale, Transform},
    wayland::{
        compositor::with_states,
        dmabuf::DmabufFeedbackBuilder,
    },
    wayland::shell::wlr_layer::Layer as WlrLayer,
};
use tracing::{error, info, warn};

use crate::{input_handler::handle_input, state::MargoState};

const REPAINT_INTERVAL_MS: u64 = 16;
render_elements! {
    MargoRenderElement<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
    WaylandSurface=WaylandSurfaceRenderElement<GlesRenderer>,
    Border=crate::render::rounded_border::RoundedBorderElement,
    Clipped=crate::render::clipped_surface::ClippedSurfaceRenderElement,
}

// ── Type aliases ──────────────────────────────────────────────────────────────

type GbmDrmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (),
    DrmDeviceFd,
>;

struct OutputDevice {
    output: Output,
    compositor: GbmDrmCompositor,
    render_count: u64,
    queued_count: u64,
    empty_count: u64,
    queue_error_count: u64,
    /// Per-CRTC GAMMA_LUT property handles, populated when the connector is
    /// bound. `None` if the kernel/driver doesn't expose GAMMA_LUT (in which
    /// case sunsetr / gammastep silently skip the output).
    gamma: Option<GammaProps>,
    /// Connector handle this CRTC is driving. Needed during hotplug so we
    /// can re-check whether the *specific* connector for this output is
    /// still connected — the previous code asked "is anything still
    /// connected on this card?" which gave wrong answers in multi-monitor
    /// setups.
    connector: connector::Handle,
}

// ── DRM gamma properties ──────────────────────────────────────────────────────
//
// Adapted from niri's `src/backend/tty.rs` GammaProps.

struct GammaProps {
    crtc: crtc::Handle,
    gamma_lut: property::Handle,
    gamma_lut_size: property::Handle,
    /// Currently-active LUT blob id, so we can free it when replacing.
    previous_blob: Option<NonZeroU64>,
}

impl GammaProps {
    fn discover(device: &DrmDevice, crtc: crtc::Handle) -> Option<Self> {
        let props = device.get_properties(crtc).ok()?;
        let mut gamma_lut = None;
        let mut gamma_lut_size = None;
        for (prop, _) in props {
            let Ok(info) = device.get_property(prop) else { continue };
            let Ok(name) = info.name().to_str() else { continue };
            match name {
                "GAMMA_LUT" => {
                    if matches!(info.value_type(), property::ValueType::Blob) {
                        gamma_lut = Some(prop);
                    }
                }
                "GAMMA_LUT_SIZE" => {
                    if matches!(info.value_type(), property::ValueType::UnsignedRange(_, _)) {
                        gamma_lut_size = Some(prop);
                    }
                }
                _ => (),
            }
        }
        Some(Self {
            crtc,
            gamma_lut: gamma_lut?,
            gamma_lut_size: gamma_lut_size?,
            previous_blob: None,
        })
    }

    fn gamma_size(&self, device: &DrmDevice) -> Option<u32> {
        let props = device.get_properties(self.crtc).ok()?;
        for (prop, value) in props {
            if prop == self.gamma_lut_size {
                // value is the raw u64 property value.
                return Some(value as u32);
            }
        }
        None
    }

    /// Apply a gamma ramp (R, G, B planes concatenated, each `gamma_size`
    /// u16 entries). Pass `None` to clear/restore the default identity LUT.
    fn set_gamma(&mut self, device: &DrmDevice, gamma: Option<&[u16]>) -> Result<()> {
        #[allow(non_camel_case_types)]
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct drm_color_lut {
            red: u16,
            green: u16,
            blue: u16,
            reserved: u16,
        }

        let blob_id: Option<NonZeroU64> = if let Some(gamma) = gamma {
            let n = self.gamma_size(device).context("gamma_size unreadable")? as usize;
            anyhow::ensure!(
                gamma.len() == n * 3,
                "wrong gamma length: got {}, expected {}",
                gamma.len(),
                n * 3
            );
            // wlr-gamma-control orders the ramp as R, G, B planes (per niri's
            // observation; some implementations swap G/B but this matches what
            // sunsetr / gammastep send).
            let (red, rest) = gamma.split_at(n);
            let (green, blue) = rest.split_at(n);
            let mut data: Vec<drm_color_lut> = red
                .iter()
                .zip(green.iter())
                .zip(blue.iter())
                .map(|((&r, &g), &b)| drm_color_lut {
                    red: r,
                    green: g,
                    blue: b,
                    reserved: 0,
                })
                .collect();
            let bytes = bytemuck::cast_slice_mut::<drm_color_lut, u8>(&mut data);
            let blob = drm_ffi::mode::create_property_blob(device.as_fd(), bytes)
                .map_err(|e| anyhow::anyhow!("create_property_blob: {e}"))?;
            NonZeroU64::new(u64::from(blob.blob_id))
        } else {
            None
        };

        let blob_value = blob_id.map(NonZeroU64::get).unwrap_or(0);
        let raw: property::RawValue = property::Value::Blob(blob_value).into();
        device
            .set_property(self.crtc, self.gamma_lut, raw)
            .map_err(|e| anyhow::anyhow!("set GAMMA_LUT property: {e}"))?;

        if let Some(old) = std::mem::replace(&mut self.previous_blob, blob_id) {
            let _ = device.destroy_property_blob(old.get());
        }
        Ok(())
    }
}

struct BackendData {
    renderer: GlesRenderer,
    outputs: HashMap<crtc::Handle, OutputDevice>,
    /// DRM device shared by all outputs on this card. Used for late-binding
    /// operations (gamma LUT updates, output power management) that need to
    /// poke properties outside the per-CRTC `DrmCompositor`.
    drm: DrmDevice,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(
    state: &mut MargoState,
    event_loop: &mut EventLoop<'static, MargoState>,
) -> Result<()> {
    // ── 1. Open libseat session ───────────────────────────────────────────────
    let (mut session, session_notifier) = LibSeatSession::new()
        .map_err(|e| anyhow::anyhow!("libseat session failed: {e}"))?;
    let seat_name = session.seat();
    info!("libseat session on seat: {seat_name}");

    // ── 2. Discover primary GPU ───────────────────────────────────────────────
    let primary_gpu_path = primary_gpu(&seat_name)
        .ok()
        .flatten()
        .context("no primary GPU found")?;
    let primary_node = DrmNode::from_path(&primary_gpu_path)
        .ok()
        .and_then(|n| n.node_with_type(NodeType::Primary).and_then(|r| r.ok()))
        .context("DrmNode from primary GPU path failed")?;
    info!("primary GPU: {:?}", primary_gpu_path);

    // ── 3. Open DRM device ────────────────────────────────────────────────────
    let drm_fd = {
        let owned_fd = session
            .open(
                &primary_gpu_path,
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
            )
            .map_err(|e| anyhow::anyhow!("open DRM device: {e}"))?;
        DrmDeviceFd::new(unsafe { DeviceFd::from_raw_fd(owned_fd.into_raw_fd()) })
    };

    let (mut drm, drm_notifier) =
        DrmDevice::new(drm_fd.clone(), false).context("DrmDevice::new")?;

    // ── 4. GBM + EGL + GLES ──────────────────────────────────────────────────
    let gbm = GbmDevice::new(drm_fd.clone()).context("GbmDevice::new")?;
    let egl_display =
        unsafe { EGLDisplay::new(gbm.clone()) }.context("EGLDisplay::new")?;
    let egl_context = EGLContext::new(&egl_display).context("EGLContext::new")?;
    let mut renderer =
        unsafe { GlesRenderer::new(egl_context) }.context("GlesRenderer::new")?;

    match renderer.bind_wl_display(&state.display_handle) {
        Ok(()) => info!("EGL Wayland hardware-acceleration enabled"),
        Err(err) => warn!("failed to bind EGL Wayland display: {err:?}"),
    }

    let dmabuf_formats = renderer.dmabuf_formats();
    match DmabufFeedbackBuilder::new(primary_node.dev_id(), dmabuf_formats.clone()).build() {
        Ok(feedback) => {
            let global = state
                .dmabuf_state
                .create_global_with_default_feedback::<MargoState>(
                    &state.display_handle,
                    &feedback,
                );
            state.dmabuf_global = Some(global);
            info!("linux-dmabuf v5 enabled with default feedback");
        }
        Err(err) => {
            warn!("failed to build dmabuf feedback, falling back to v3: {err:?}");
            let global = state
                .dmabuf_state
                .create_global::<MargoState>(&state.display_handle, dmabuf_formats);
            state.dmabuf_global = Some(global);
        }
    }

    // ── 5. Get renderer formats for DRM compositor ───────────────────────────
    let renderer_formats = renderer
        .egl_context()
        .display()
        .dmabuf_render_formats()
        .clone();

    let color_formats = [DrmFourcc::Xrgb8888, DrmFourcc::Argb8888];

    // ── 6. Enumerate connected connectors and create outputs + compositors ───
    let resources = drm_fd.resource_handles().context("DRM resource_handles")?;
    let mut used_crtcs: std::collections::HashSet<crtc::Handle> =
        std::collections::HashSet::new();

    let mut backend_outputs: HashMap<crtc::Handle, OutputDevice> = HashMap::new();

    for conn_handle in resources.connectors() {
        let conn_info = match drm_fd.get_connector(*conn_handle, false) {
            Ok(c) => c,
            Err(e) => {
                warn!("get_connector: {e}");
                continue;
            }
        };

        if conn_info.state() != connector::State::Connected {
            continue;
        }

        let Some(crtc) = find_crtc(&drm_fd, &conn_info, &resources, &used_crtcs) else {
            warn!("no CRTC for connector {:?}", conn_info.interface());
            continue;
        };
        used_crtcs.insert(crtc);

        let (phys_w, phys_h) = conn_info.size().unwrap_or((0, 0));
        let output_name = format!(
            "{}-{}",
            conn_info.interface().as_str(),
            conn_info.interface_id()
        );

        // Match against monitorrule config
        let rule = state.config.monitor_rules.iter().find(|r| {
            r.name.as_deref().map(|n| n == output_name).unwrap_or(true)
        }).cloned();

        // Select DRM mode: prefer rule-specified w×h@refresh, else preferred flag
        let drm_mode = select_drm_mode(&conn_info, rule.as_ref());
        let Some(drm_mode) = drm_mode else {
            warn!("no suitable mode for {output_name}");
            continue;
        };
        let wl_mode = OutputMode::from(drm_mode);

        let scale = rule.as_ref().map(|r| r.scale).unwrap_or(1.0);
        let transform = smithay_transform(rule.as_ref().map(|r| r.transform).unwrap_or(0));

        // Position: use rule if set, otherwise auto side-by-side
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

        info!("output: {} {}x{}@{} pos={:?} scale={}", output_name,
            wl_mode.size.w, wl_mode.size.h, wl_mode.refresh / 1000, position, scale);

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

        // Create DRM surface for this output
        let drm_surface = match drm.create_surface(crtc, drm_mode, &[*conn_handle]) {
            Ok(s) => s,
            Err(e) => {
                warn!("create_surface for {output_name}: {e}");
                continue;
            }
        };

        // Create per-output DRM compositor
        let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING);
        let exporter = GbmFramebufferExporter::new(gbm.clone(), primary_node.into());
        let compositor = match DrmCompositor::new(
            &output,
            drm_surface,
            None,
            allocator,
            exporter,
            color_formats.iter().copied(),
            renderer_formats.clone(),
            (64u32, 64u32).into(),
            Some(gbm.clone()),
        ) {
            Ok(c) => c,
            Err(e) => {
                warn!("DrmCompositor::new for {output_name}: {e:?}");
                continue;
            }
        };

        // Register monitor
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
            name: output_name,
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
            gamma_size: 0, // backfilled below
        });
        state.apply_tag_rules_to_monitor(state.monitors.len() - 1);

        // Best-effort GAMMA_LUT discovery. If the connector/driver doesn't
        // expose it (some VC4/Mali-DP setups), `gamma` stays None and
        // wlr_gamma_control clients get a `failed` event for this output.
        let mut gamma_props = GammaProps::discover(&drm, crtc);
        if let Some(gamma) = gamma_props.as_mut() {
            // Match niri: reset any stale LUT left by a previous compositor or
            // crashed night-light client before accepting new gamma-control ramps.
            if let Err(err) = gamma.set_gamma(&drm, None) {
                tracing::debug!("couldn't reset gamma on {}: {err:?}", output.name());
            }
        }
        let gamma_size = gamma_props
            .as_ref()
            .and_then(|g| g.gamma_size(&drm))
            .unwrap_or(0);
        let mon_idx = state.monitors.len() - 1;
        state.monitors[mon_idx].gamma_size = gamma_size;
        if gamma_size > 0 {
            info!("output {} gamma_size = {}", state.monitors[mon_idx].name, gamma_size);
        }

        backend_outputs.insert(
            crtc,
            OutputDevice {
                output,
                compositor,
                render_count: 0,
                queued_count: 0,
                empty_count: 0,
                queue_error_count: 0,
                gamma: gamma_props,
                connector: *conn_handle,
            },
        );
    }

    if state.monitors.is_empty() {
        return Err(anyhow::anyhow!("no connected outputs found"));
    }

    let backend_data = Rc::new(RefCell::new(BackendData {
        renderer,
        outputs: backend_outputs,
        drm,
    }));
    state.dmabuf_import_hook = Some(Rc::new(RefCell::new({
        let backend_data = backend_data.clone();
        move |dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf| {
            backend_data
                .borrow_mut()
                .renderer
                .import_dmabuf(dmabuf, None)
                .is_ok()
        }
    })));

    // ── 7. DRM VBlank event source ────────────────────────────────────────────
    event_loop
        .handle()
        .insert_source(drm_notifier, {
            let backend_data = backend_data.clone();
            move |event, _, _state: &mut MargoState| match event {
                DrmEvent::VBlank(crtc) => {
                    let mut bd = backend_data.borrow_mut();
                    if let Some(od) = bd.outputs.get_mut(&crtc) {
                        // Acknowledge the previous flip; without this, queue_frame
                        // for the next frame will fail and the render loop stalls.
                        if let Err(e) = od.compositor.frame_submitted() {
                            warn!("frame_submitted: {e:?}");
                        }
                    }
                }
                DrmEvent::Error(e) => error!("DRM error: {:?}", e),
            }
        })
        .map_err(|e| anyhow::anyhow!("DRM event source: {e}"))?;

    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(REPAINT_INTERVAL_MS)), {
            let backend_data = backend_data.clone();
            move |_, _, state: &mut MargoState| {
                if state.take_repaint_request() {
                    let mut bd = backend_data.borrow_mut();
                    let BackendData { renderer, outputs, drm } = &mut *bd;
                    render_all_outputs(renderer, outputs, drm, state, "repaint");
                }
                TimeoutAction::ToDuration(Duration::from_millis(REPAINT_INTERVAL_MS))
            }
        })
        .map_err(|e| anyhow::anyhow!("repaint timer source: {e}"))?;

    // ── 8. Session event source ───────────────────────────────────────────────
    // (libinput will be inserted into state below; the closure reads it from state)
    event_loop
        .handle()
        .insert_source(session_notifier, |event, _, state: &mut MargoState| {
            match event {
                SessionEvent::PauseSession => {
                    info!("session paused");
                    if let Some(li) = state.libinput.as_mut() {
                        li.suspend();
                    }
                    state.libinput_devices.clear();
                }
                SessionEvent::ActivateSession => {
                    info!("session activated");
                    if let Some(li) = state.libinput.as_mut() {
                        if li.resume().is_err() {
                            warn!("libinput resume failed");
                        }
                    }
                    state.arrange_all();
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("session event source: {e}"))?;

    // ── 9. libinput ───────────────────────────────────────────────────────────
    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|_| anyhow::anyhow!("libinput assign seat failed"))?;

    // If the session isn't active yet (e.g. launched from a display manager that
    // hasn't switched VTs to us yet), suspend libinput so the eventual
    // ActivateSession does a full re-enumeration via resume().
    if !session.is_active() {
        info!("session not active at startup, suspending libinput");
        libinput.suspend();
    }

    state.libinput = Some(libinput.clone());

    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput), |mut event, _, state: &mut MargoState| {
            match &mut event {
                InputEvent::DeviceAdded { device } => {
                    crate::libinput_config::apply_to_device(device, &state.config);
                    state.libinput_devices.retain(|known| known != device);
                    state.libinput_devices.push(device.clone());
                }
                InputEvent::DeviceRemoved { device } => {
                    state.libinput_devices.retain(|known| known != device);
                }
                _ => {}
            }
            handle_input(state, event);
        })
        .map_err(|e| anyhow::anyhow!("libinput source: {e}"))?;

    // ── 10. Udev hotplug source ───────────────────────────────────────────────
    let udev_backend = UdevBackend::new(&seat_name)
        .map_err(|e| anyhow::anyhow!("UdevBackend::new: {e}"))?;
    event_loop
        .handle()
        .insert_source(udev_backend, {
            let backend_data = backend_data.clone();
            move |event, _, state: &mut MargoState| match event {
                UdevEvent::Added { device_id: _, path } => {
                    info!("udev added: {:?}", path);
                }
                UdevEvent::Changed { device_id: _ } => {
                    info!("udev device changed, rescanning outputs");
                    rescan_outputs(&backend_data, state);
                }
                UdevEvent::Removed { device_id: _ } => {}
            }
        })
        .map_err(|e| anyhow::anyhow!("udev source: {e}"))?;

    // ── 11. Initial render pass ───────────────────────────────────────────────
    {
        let mut bd = backend_data.borrow_mut();
        let BackendData { renderer, outputs, drm } = &mut *bd;
        render_all_outputs(renderer, outputs, drm, state, "initial");
    }

    info!("udev backend ready ({} outputs)", state.monitors.len());
    Ok(())
}

// ── Hotplug rescan ────────────────────────────────────────────────────────────
//
// Called from `UdevEvent::Changed` whenever the kernel notifies us that
// the DRM device's connector topology may have shifted. We:
//
// 1. Remove every OutputDevice whose specific connector is no longer
//    `Connected` (laptop dock unplug, monitor cable pulled). The previous
//    implementation answered "is *anything* still connected on this card?"
//    which gave wrong answers in multi-monitor setups.
//
// 2. Migrate any clients that lived on the removed monitor to the
//    remaining first monitor — without this they keep `c.monitor =
//    <stale index>` and disappear from the layout.
//
// 3. (TODO) Add new outputs for connectors that *just* came up. Building
//    a DrmCompositor at runtime is a chunky operation — punted to a
//    follow-up so the unplug path lands in isolation. Until then,
//    plugging in an external monitor still requires a logout.

fn rescan_outputs(
    backend_data: &Rc<RefCell<BackendData>>,
    state: &mut MargoState,
) {
    let mut bd = backend_data.borrow_mut();
    let BackendData {
        renderer: _,
        outputs,
        drm,
    } = &mut *bd;

    // Find which currently-tracked outputs have lost their connector.
    let mut to_remove: Vec<crtc::Handle> = Vec::new();
    for (crtc, od) in outputs.iter() {
        let still_connected = drm
            .get_connector(od.connector, false)
            .map(|c| c.state() == connector::State::Connected)
            .unwrap_or(false);
        if !still_connected {
            tracing::info!(
                "output {} disconnected (CRTC {:?})",
                od.output.name(),
                crtc
            );
            to_remove.push(*crtc);
        }
    }

    if to_remove.is_empty() {
        return;
    }

    let removed_outputs: Vec<Output> = to_remove
        .into_iter()
        .filter_map(|crtc| outputs.remove(&crtc).map(|od| od.output))
        .collect();
    drop(bd);

    // Migrate clients off the gone monitors before letting state forget
    // them; otherwise their per-client `monitor` indices become dangling.
    for output in &removed_outputs {
        migrate_clients_off_output(state, output);
        state.remove_output(output);
    }

    // Always re-arrange remaining outputs so the migrated clients land in
    // the new layout and any per-tag scroller proportions re-center.
    state.arrange_all();
    state.request_repaint();
}

fn migrate_clients_off_output(state: &mut MargoState, removed: &Output) {
    let removed_idx = state
        .monitors
        .iter()
        .position(|m| &m.output == removed);
    let Some(removed_idx) = removed_idx else { return };

    // Pick the surviving monitor that's NOT the one being removed. If
    // there are no other monitors, the session is essentially headless;
    // arrange_all() below still runs but the windows just stay invisible
    // until something replugs.
    let target_idx = state
        .monitors
        .iter()
        .enumerate()
        .find(|(i, _)| *i != removed_idx)
        .map(|(i, _)| i);

    let Some(target_idx) = target_idx else {
        // Last monitor unplugged — nothing to migrate to. Clients keep
        // their geometry; on next plug-in they'll snap to whoever shows
        // up first.
        return;
    };

    let target_tagset = state.monitors[target_idx].current_tagset();
    let target_name = state.monitors[target_idx].name.clone();

    // Compute the post-removal index for `target_idx` once. After
    // `state.remove_output()` runs and Vec::remove(removed_idx) shifts
    // every later element down by one, this is the slot where the
    // surviving target monitor will land.
    let target_after = if target_idx > removed_idx {
        target_idx - 1
    } else {
        target_idx
    };

    let mut migrated = 0;
    for client in state.clients.iter_mut() {
        if client.monitor == removed_idx {
            client.monitor = target_after;
            // Make sure the client is on at least one tag of the target
            // monitor — otherwise it's invisible until the user toggles
            // a tag.
            if client.tags & target_tagset == 0 {
                client.tags |= target_tagset;
            }
            migrated += 1;
        } else if client.monitor > removed_idx {
            // Same Vec::remove shift applied to clients that already
            // lived on a later monitor.
            client.monitor -= 1;
        }
    }
    if migrated > 0 {
        tracing::info!(
            "migrated {migrated} clients from {} → {target_name}",
            removed.name(),
        );
    }
}

// ── Per-frame render ──────────────────────────────────────────────────────────

fn build_render_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
) -> Vec<MargoRenderElement> {
    build_render_elements_inner(renderer, od, state, true)
}

/// Like `build_render_elements`, but optionally omits the cursor sprite.
/// Used by the screencopy path so clients with `overlay_cursor=false` get
/// a cursor-free capture.
fn build_render_elements_inner(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    include_cursor: bool,
) -> Vec<MargoRenderElement> {
    let output_scale = od.output.current_scale().fractional_scale();

    let Some(output_geo) = state.space.output_geometry(&od.output) else {
        return Vec::new();
    };

    if let Some((_, lock_surface)) = state.lock_surfaces.iter().find(|(o, _)| o == &od.output) {
        let mut elements = Vec::new();
        
        // Highest priority: cursor (if inside this output)
        let ptr_global = Point::<f64, _>::from((state.input_pointer.x, state.input_pointer.y));
        if include_cursor && output_geo.to_f64().contains(ptr_global) {
            let ptr_pos = ptr_global - output_geo.loc.to_f64();
            match &state.cursor_status {
                CursorImageStatus::Surface(surface) => {
                    let hotspot = with_states(surface, |states| {
                        states
                            .data_map
                            .get::<Mutex<CursorImageAttributes>>()
                            .and_then(|attrs| attrs.lock().ok().map(|attrs| attrs.hotspot))
                            .unwrap_or_default()
                    });
                    let ptr_i = (ptr_pos - hotspot.to_f64())
                        .to_physical_precise_round::<f64, i32>(output_scale);
                    let cursor_elems = render_elements_from_surface_tree(
                        renderer, surface, ptr_i, output_scale, 1.0f32, Kind::Cursor,
                    );
                    for e in cursor_elems {
                        elements.push(MargoRenderElement::WaylandSurface(e));
                    }
                }
                CursorImageStatus::Hidden => {}
                _ => {
                    if let Some(cursor_elem) =
                        state.cursor_manager.render_element(renderer, ptr_pos, output_scale)
                    {
                        elements.push(MargoRenderElement::Cursor(cursor_elem));
                    }
                }
            }
        }

        // Lock surface
        let lock_elements = render_elements_from_surface_tree(
            renderer,
            lock_surface.wl_surface(),
            Point::<i32, Physical>::from((0, 0)), // Lock surface is always output-relative (0,0) in smithay
            output_scale,
            1.0,
            Kind::Unspecified,
        );
        for e in lock_elements {
            elements.push(MargoRenderElement::WaylandSurface(e));
        }

        return elements;
    }

    let Some(output_geo) = state.space.output_geometry(&od.output) else {
        return Vec::new();
    };

    let layer_map = layer_map_for_output(&od.output);
    let upper_layers: Vec<_> = layer_map
        .layers()
        .rev()
        .filter(|surface| surface.layer() == WlrLayer::Overlay)
        .chain(
            layer_map
                .layers()
                .rev()
                .filter(|surface| surface.layer() == WlrLayer::Top),
        )
        .collect();
    let lower_layers: Vec<_> = layer_map
        .layers()
        .rev()
        .filter(|surface| surface.layer() == WlrLayer::Bottom)
        .chain(
            layer_map
                .layers()
                .rev()
                .filter(|surface| surface.layer() == WlrLayer::Background),
        )
        .collect();
    let border_program = crate::render::rounded_border::shader(renderer).map(|program| program.0);
    let clipped_surface_program =
        crate::render::clipped_surface::shader(renderer).map(|program| program.0);

    let mut elements: Vec<MargoRenderElement> = Vec::with_capacity(
        upper_layers.len()
            + lower_layers.len()
            + state.clients.len() * 2
            + 1,
    );

    // First elements are highest z-order in the DRM compositor.
    let ptr_global = Point::<f64, _>::from((state.input_pointer.x, state.input_pointer.y));
    if include_cursor && output_geo.to_f64().contains(ptr_global) {
        let ptr_pos = ptr_global - output_geo.loc.to_f64();
        match &state.cursor_status {
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .and_then(|attrs| attrs.lock().ok().map(|attrs| attrs.hotspot))
                        .unwrap_or_default()
                });
                if hotspot.x != 0 || hotspot.y != 0 {
                    tracing::trace!(
                        "cursor hotspot=({}, {}) ptr_pos=({:.0}, {:.0})",
                        hotspot.x,
                        hotspot.y,
                        ptr_pos.x,
                        ptr_pos.y
                    );
                }
                let ptr_i = (ptr_pos - hotspot.to_f64())
                    .to_physical_precise_round::<f64, i32>(output_scale);
                let cursor_elems = render_elements_from_surface_tree(
                    renderer, surface, ptr_i, output_scale, 1.0f32, Kind::Cursor,
                );
                for e in cursor_elems {
                    elements.push(MargoRenderElement::WaylandSurface(e));
                }            }
            CursorImageStatus::Hidden => {}
            _ => {
                if let Some(cursor_elem) =
                    state.cursor_manager.render_element(renderer, ptr_pos, output_scale)
                {
                    elements.push(MargoRenderElement::Cursor(cursor_elem));
                }
            }
        }
    }

    push_layer_elements(
        renderer,
        &layer_map,
        &upper_layers,
        output_scale,
        1.0,
        &mut elements,
    );

    push_client_elements(
        renderer,
        state,
        &od.output,
        output_geo,
        output_scale,
        border_program,
        clipped_surface_program,
        &mut elements,
    );

    push_layer_elements(
        renderer,
        &layer_map,
        &lower_layers,
        output_scale,
        1.0,
        &mut elements,
    );

    elements
}

fn push_client_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    border_program: Option<smithay::backend::renderer::gles::GlesPixelProgram>,
    clipped_surface_program: Option<GlesTexProgram>,
    elements: &mut Vec<MargoRenderElement>,
) {
    let scale = Scale::from(output_scale);

    for window in state.space.elements_for_output(output).rev() {
        let Some(location) = state.space.element_location(window) else {
            continue;
        };
        let render_location = location - window.geometry().loc;
        let physical_location = (render_location - output_geo.loc).to_physical_precise_round(scale);

        let client = state.clients.iter().find(|client| client.window == *window);
        let radius = client
            .filter(|client| !client.no_radius && !client.is_fullscreen)
            .map(|_| state.config.border_radius.max(0) as f32)
            .unwrap_or(0.0);
        let clip_geometry = client.map(|client| {
            Rectangle::new(
                (
                    f64::from(client.geom.x - output_geo.loc.x),
                    f64::from(client.geom.y - output_geo.loc.y),
                )
                    .into(),
                (
                    f64::from(client.geom.width.max(1)),
                    f64::from(client.geom.height.max(1)),
                )
                    .into(),
            )
        });

        match window.underlying_surface() {
            WindowSurface::Wayland(surface) => {
                let wl_surface = surface.wl_surface();
                let popup_elements = PopupManager::popups_for_surface(wl_surface).flat_map(
                    |(popup, popup_offset)| {
                        let offset = (window.geometry().loc + popup_offset - popup.geometry().loc)
                            .to_physical_precise_round(scale);

                        render_elements_from_surface_tree::<
                            GlesRenderer,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                        >(
                            renderer,
                            popup.wl_surface(),
                            physical_location + offset,
                            scale,
                            1.0,
                            Kind::Unspecified,
                        )
                    },
                );

                for elem in popup_elements {
                    elements.push(MargoRenderElement::Space(SpaceRenderElements::Element(
                        Wrap::from(elem),
                    )));
                }

                if let (Some(client), Some(program)) = (client, border_program.as_ref()) {
                    if let Some(border) = crate::border::render_element_for_client(
                        client,
                        output_geo.loc,
                        program.clone(),
                    ) {
                        elements.push(MargoRenderElement::Border(border));
                    }
                }

                let surface_elements = render_elements_from_surface_tree::<
                    GlesRenderer,
                    WaylandSurfaceRenderElement<GlesRenderer>,
                >(
                    renderer,
                    wl_surface,
                    physical_location,
                    scale,
                    1.0,
                    Kind::Unspecified,
                );

                for elem in surface_elements {
                    if radius > 0.0 {
                        if let (Some(program), Some(clip_geometry)) =
                            (clipped_surface_program.as_ref(), clip_geometry)
                        {
                            elements.push(MargoRenderElement::Clipped(
                                crate::render::clipped_surface::ClippedSurfaceRenderElement::new(
                                    elem,
                                    scale,
                                    clip_geometry,
                                    radius,
                                    program.clone(),
                                ),
                            ));
                            continue;
                        }
                    }

                    elements.push(MargoRenderElement::Space(SpaceRenderElements::Element(
                        Wrap::from(elem),
                    )));
                }
            }
            WindowSurface::X11(_) => {
                if let (Some(client), Some(program)) = (client, border_program.as_ref()) {
                    if let Some(border) = crate::border::render_element_for_client(
                        client,
                        output_geo.loc,
                        program.clone(),
                    ) {
                        elements.push(MargoRenderElement::Border(border));
                    }
                }

                let rendered = AsRenderElements::<GlesRenderer>::render_elements::<
                    WaylandSurfaceRenderElement<GlesRenderer>,
                >(window, renderer, physical_location, scale, 1.0);
                for elem in rendered {
                    elements.push(MargoRenderElement::Space(SpaceRenderElements::Element(
                        Wrap::from(elem),
                    )));
                }
            }
        }
    }
}

fn push_layer_elements(
    renderer: &mut GlesRenderer,
    layer_map: &smithay::desktop::LayerMap,
    layers: &[&smithay::desktop::LayerSurface],
    output_scale: f64,
    alpha: f32,
    elements: &mut Vec<MargoRenderElement>,
) {
    for surface in layers {
        let Some(geo) = layer_map.layer_geometry(surface) else {
            continue;
        };
        let rendered =
            AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<
                GlesRenderer,
            >>(
                *surface,
                renderer,
                geo.loc.to_physical_precise_round(output_scale),
                Scale::from(output_scale),
                alpha,
            );
        elements.extend(
            rendered
                .into_iter()
                .map(|elem| MargoRenderElement::Space(SpaceRenderElements::Surface(elem))),
        );
    }
}

/// Drain any wlr-screencopy frames pending for `od.output` and write the
/// rendered pixels into the client's buffer. Currently supports SHM
/// targets only — dmabuf clients fall through and time out (their `Drop`
/// emits `failed`). Renders into a transient `GlesRenderbuffer` matching
/// the client's requested `buffer_size` so we honour scaling/transform.
fn serve_screencopies(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &mut MargoState,
    elements: &[MargoRenderElement],
) {
    use smithay::backend::renderer::{
        damage::OutputDamageTracker as DamageTracker, Bind, ExportMem, Offscreen,
    };

    let now = monotonic_now();
    let output = od.output.clone();

    // Collect screencopies destined for THIS output across all queues.
    let mut to_serve: Vec<crate::protocols::screencopy::Screencopy> = Vec::new();
    state.screencopy_state.with_queues_mut(|queue| {
        // niri keeps damage tracking per-queue; we just drain everything
        // that targets our output. Region-based capture and damage events
        // are best-effort here — full-frame ready/damage is good enough
        // for grim/wf-recorder/screen-rec.
        let drained: Vec<_> = (0..)
            .map_while(|_| {
                let head_matches = queue
                    .split()
                    .1
                    .map(|s| s.output() == &output)
                    .unwrap_or(false);
                if head_matches {
                    Some(queue.pop())
                } else {
                    None
                }
            })
            .collect();
        to_serve.extend(drained);
    });
    if to_serve.is_empty() {
        return;
    }

    // The output's full physical mode size — we render this into the
    // offscreen target, then `copy_framebuffer` reads back only the
    // client-requested sub-region (region_loc + buffer_size).
    let output_size = match od.output.current_mode().map(|m| m.size) {
        Some(s) => s,
        None => {
            for s in to_serve {
                drop(s); // → fires `failed` via Drop
            }
            return;
        }
    };

    for screencopy in to_serve {
        let size = screencopy.buffer_size();
        let region_loc = screencopy.region_loc();
        if size.w <= 0 || size.h <= 0 {
            continue;
        }
        let scale = od.output.current_scale().fractional_scale();

        // Render the FULL output (not just the region). We crop on read-back
        // via `copy_framebuffer`. Renderbuffer matches the output mode.
        let buf_size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from(
            (output_size.w, output_size.h),
        );

        let mut renderbuffer =
            match <GlesRenderer as Offscreen<smithay::backend::renderer::gles::GlesRenderbuffer>>::create_buffer(
                renderer,
                drm_fourcc::DrmFourcc::Xrgb8888,
                buf_size,
            ) {
                Ok(rb) => rb,
                Err(e) => {
                    warn!("screencopy: create_buffer failed: {e:?}");
                    continue;
                }
            };

        let mut target = match renderer.bind(&mut renderbuffer) {
            Ok(t) => t,
            Err(e) => {
                warn!("screencopy: bind renderbuffer failed: {e:?}");
                continue;
            }
        };

        // Re-build the element list honouring the client's `overlay_cursor`
        // preference. When false (the screenshot default), we skip the
        // cursor sprite so the captured image has no overlay.
        let owned_elements: Vec<MargoRenderElement>;
        let elements_to_render: &[MargoRenderElement] = if screencopy.overlay_cursor() {
            elements
        } else {
            owned_elements = build_render_elements_inner(renderer, od, state, false);
            &owned_elements
        };

        let mut tracker = DamageTracker::new(output_size, scale, Transform::Normal);
        let render_result = tracker.render_output(
            renderer,
            &mut target,
            0,
            elements_to_render,
            [0.0, 0.0, 0.0, 1.0],
        );
        let damage = match render_result {
            Ok(res) => res.damage,
            Err(e) => {
                warn!("screencopy: render_output failed: {e:?}");
                continue;
            }
        };

        // Crop on copy: pull out the requested sub-region (or the full
        // output if region_loc=(0,0) and size==output_size).
        let region = smithay::utils::Rectangle::new(
            smithay::utils::Point::<i32, smithay::utils::Buffer>::from(
                (region_loc.x, region_loc.y),
            ),
            smithay::utils::Size::<i32, smithay::utils::Buffer>::from((size.w, size.h)),
        );
        let mapping = match renderer.copy_framebuffer(&target, region, drm_fourcc::DrmFourcc::Xrgb8888) {
            Ok(m) => m,
            Err(e) => {
                warn!("screencopy: copy_framebuffer failed: {e:?}");
                continue;
            }
        };
        // Drop the bind so map_texture can re-bind the GL state.
        drop(target);
        let pixels = match renderer.map_texture(&mapping) {
            Ok(p) => p,
            Err(e) => {
                warn!("screencopy: map_texture failed: {e:?}");
                continue;
            }
        };

        // Write into the SHM buffer (dmabuf branch is a TODO).
        let copied = match screencopy.buffer() {
            crate::protocols::screencopy::ScreencopyBuffer::Shm(buf) => {
                let need = (size.w as usize).saturating_mul(4).saturating_mul(size.h as usize);
                let copied_n = match smithay::wayland::shm::with_buffer_contents_mut(
                    buf,
                    |dst_ptr, dst_len, _meta| {
                        let n = need.min(dst_len).min(pixels.len());
                        // SAFETY: dst_ptr+dst_len come from a validated wl_shm
                        // wl_buffer; we never read more than n bytes from
                        // pixels (whose size is dst_len-clamped).
                        unsafe {
                            std::ptr::copy_nonoverlapping(pixels.as_ptr(), dst_ptr, n);
                        }
                        n
                    },
                ) {
                    Ok(n) => n,
                    Err(e) => {
                        warn!("screencopy: shm map failed: {e:?}");
                        0
                    }
                };
                copied_n > 0
            }
            crate::protocols::screencopy::ScreencopyBuffer::Dmabuf(_) => {
                warn!("screencopy: dmabuf target not yet implemented");
                false
            }
        };

        if !copied {
            // Drop without submit → frame.failed() fires from `impl Drop`.
            continue;
        }

        // Optional: announce damage for with_damage clients.
        if screencopy.with_damage() {
            if let Some(damage_rects) = damage.as_ref() {
                screencopy.damage(damage_rects.iter().map(|r| {
                    smithay::utils::Rectangle::new(
                        smithay::utils::Point::from((r.loc.x, r.loc.y)),
                        smithay::utils::Size::from((r.size.w, r.size.h)),
                    )
                }));
            }
        }
        // SHM: synchronous, no GPU sync needed before submit.
        screencopy.submit_now(false, now);
    }
}

fn monotonic_now() -> std::time::Duration {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

fn render_all_outputs(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    drm: &DrmDevice,
    state: &mut MargoState,
    reason: &'static str,
) {
    // Apply any gamma ramp updates queued by wlr_gamma_control clients.
    if !state.pending_gamma.is_empty() {
        let pending = std::mem::take(&mut state.pending_gamma);
        for (output, ramp) in pending {
            let target = outputs.values_mut().find(|od| od.output == output);
            let Some(od) = target else { continue };
            let Some(g) = od.gamma.as_mut() else {
                tracing::debug!("gamma: skip {} (no GAMMA_LUT)", od.output.name());
                continue;
            };
            match g.set_gamma(drm, ramp.as_deref()) {
                Ok(()) => tracing::debug!(
                    "gamma applied output={} ramp={}",
                    od.output.name(),
                    if ramp.is_some() { "client" } else { "default" }
                ),
                Err(e) => warn!("gamma set failed on {}: {e:?}", od.output.name()),
            }
        }
    }

    for od in outputs.values_mut() {
        render_output(renderer, od, state, reason);
    }
}

fn render_output(
    renderer: &mut GlesRenderer,
    od: &mut OutputDevice,
    state: &mut MargoState,
    reason: &'static str,
) {
    let elements = build_render_elements(renderer, od, state);
    // Serve any pending wlr-screencopy frames for this output BEFORE the
    // main render. We re-use `elements` so the captured image matches what
    // the user sees on the next frame. Returns early on success — the
    // screencopy clients get the same pixels we're about to scan out.
    serve_screencopies(renderer, od, state, &elements);
    let clear_color = if state.session_locked {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [0.1, 0.1, 0.1, 1.0]
    };
    match od
        .compositor
        .render_frame(renderer, &elements, clear_color, FrameFlags::DEFAULT)
    {
        Ok(result) => {
            od.render_count += 1;
            if result.is_empty {
                od.empty_count += 1;
                if od.empty_count <= 5 || od.empty_count % 120 == 0 {
                    info!(
                        "render empty output={} reason={} renders={} elements={}",
                        od.output.name(),
                        reason,
                        od.render_count,
                        elements.len()
                    );
                }
                return;
            }

            match od.compositor.queue_frame(()) {
                Ok(()) => {
                    od.queued_count += 1;
                    if od.queued_count <= 10 || od.queued_count % 300 == 0 {
                        info!(
                            "queued frame output={} reason={} queued={} renders={} elements={}",
                            od.output.name(),
                            reason,
                            od.queued_count,
                            od.render_count,
                            elements.len()
                        );
                    }
                    state.post_repaint(&od.output, state.clock.now());
                    state.display_handle.flush_clients().ok();
                }
                Err(e) => {
                    od.queue_error_count += 1;
                    state.request_repaint();
                    if od.queue_error_count <= 10 || od.queue_error_count % 300 == 0 {
                        warn!(
                            "queue_frame output={} reason={} errors={} elements={} error={e:?}",
                            od.output.name(),
                            reason,
                            od.queue_error_count,
                            elements.len()
                        );
                    }
                }
            }
        }
        Err(e) => error!(
            "render_frame output={} reason={} elements={} error={e:?}",
            od.output.name(),
            reason,
            elements.len()
        ),
    }
}

// ── CRTC helper ───────────────────────────────────────────────────────────────

// ── Mode selection ────────────────────────────────────────────────────────────

fn select_drm_mode(
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

    // Preferred flag
    if let Some(m) = modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)) {
        return Some(*m);
    }

    Some(modes[0])
}

// ── Transform helper ──────────────────────────────────────────────────────────

fn smithay_transform(n: i32) -> Transform {
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

// ── CRTC helper ───────────────────────────────────────────────────────────────

fn find_crtc(
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
