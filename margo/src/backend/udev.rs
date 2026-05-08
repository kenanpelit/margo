#![allow(dead_code)]
//! udev/DRM/KMS backend — real hardware via libseat + libinput + GBM/EGL.

use std::{
    cell::RefCell,
    collections::HashMap,
    num::NonZeroU64,
    os::unix::io::{AsFd, FromRawFd, IntoRawFd},
    rc::Rc,
    sync::Mutex,
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
        calloop::{ping::make_ping, EventLoop},
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
        seat::WaylandFocus,
    },
    wayland::shell::wlr_layer::Layer as WlrLayer,
};
use tracing::{error, info, warn};

use crate::{input_handler::handle_input, state::MargoState};

render_elements! {
    MargoRenderElement<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
    WaylandSurface=WaylandSurfaceRenderElement<GlesRenderer>,
    Border=crate::render::rounded_border::RoundedBorderElement,
    Clipped=crate::render::clipped_surface::ClippedSurfaceRenderElement,
    Resize=crate::render::resize_render::ResizeRenderElement,
    OpenClose=crate::render::open_close::OpenCloseRenderElement,
    Solid=smithay::backend::renderer::element::solid::SolidColorRenderElement,
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
    /// Allocator + framebuffer-exporter dependencies needed to construct
    /// new `DrmCompositor`s on hotplug. Captured once at startup; everything
    /// here is cheap to clone.
    gbm: GbmDevice<DrmDeviceFd>,
    primary_node: DrmNode,
    renderer_formats: smithay::backend::allocator::format::FormatSet,
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

    // Hardware cursor plane diagnostics. Reports what the driver advertises
    // so we can confirm cursor placement fits and we're actually getting
    // atomic cursor-plane updates instead of compositing the cursor into
    // the primary plane on every motion event.
    {
        let cs = drm.cursor_size();
        info!("DRM hardware cursor plane: {}×{} (advertised by driver)", cs.w, cs.h);
    }

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

    // ── 4b. linux-drm-syncobj-v1 (explicit sync) ────────────────────────────
    //
    // Modern hardware-accelerated clients (Chromium 100+, Firefox with
    // dmabuf textures, native Wayland games via DXVK / VKD3D-Proton)
    // tile their frame pacing on a real GPU timeline rather than the
    // implicit fence wait baked into the dmabuf protocol. Exposing this
    // global is what lets them attach acquire / release fences to a
    // surface commit and let smithay's compositor handle the GPU
    // synchronisation for us. We gate on `supports_syncobj_eventfd` so
    // older kernels (< 5.18) and devices without
    // `DRM_CAP_SYNCOBJ_TIMELINE` don't see a global advertised at all
    // — same contract niri / sway / mutter follow.
    if smithay::wayland::drm_syncobj::supports_syncobj_eventfd(&drm_fd) {
        let syncobj_state = smithay::wayland::drm_syncobj::DrmSyncobjState::new::<MargoState>(
            &state.display_handle,
            drm_fd.clone(),
        );
        state.drm_syncobj_state = Some(syncobj_state);
        info!("linux-drm-syncobj-v1 enabled (explicit sync available to clients)");
    } else {
        info!(
            "linux-drm-syncobj-v1 NOT advertised — kernel / driver lacks syncobj_eventfd; \
             clients will fall back to implicit dmabuf sync"
        );
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

        // Create per-output DRM compositor.
        //
        // `cursor_size` is the maximum hardware-cursor buffer size the
        // DRM device can scan out on its cursor plane. Querying it from
        // the device (instead of the old hardcoded 64×64) lets smithay
        // place larger cursors directly on the cursor plane on GPUs
        // that support 128² or 256² (most modern AMD/Intel/NVIDIA);
        // anything that fits gets atomic plane updates and never
        // touches the primary swapchain. Falling back to 64×64 only
        // when the driver doesn't report a size (very old drivers).
        let cursor_size = {
            let s = drm.cursor_size();
            if s.w == 0 || s.h == 0 { (64u32, 64u32).into() } else { s }
        };
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
            cursor_size,
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
        gbm: gbm.clone(),
        primary_node,
        renderer_formats: renderer_formats.clone(),
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
            move |event, _, state: &mut MargoState| match event {
                DrmEvent::VBlank(crtc) => {
                    let mut bd = backend_data.borrow_mut();
                    if let Some(od) = bd.outputs.get_mut(&crtc) {
                        // Acknowledge the previous flip; without this, queue_frame
                        // for the next frame will fail and the render loop stalls.
                        if let Err(e) = od.compositor.frame_submitted() {
                            warn!("frame_submitted: {e:?}");
                        }
                    }
                    // Drop the in-flight count and, if the scene is still
                    // dirty (animation, deferred input, late commit), let
                    // the redraw scheduler ping itself for the next frame.
                    // This is what gives us continuous animation playback
                    // now that the global 16 ms timer is gone.
                    state.note_vblank();
                }
                DrmEvent::Error(e) => error!("DRM error: {:?}", e),
            }
        })
        .map_err(|e| anyhow::anyhow!("DRM event source: {e}"))?;

    // ── On-demand redraw scheduler ────────────────────────────────────────────
    //
    // Replaces the old 16 ms polling timer. The flow now is:
    //
    //   * Anything that dirties the scene calls `state.request_repaint()`,
    //     which sets the dirty flag *and* pings this source.
    //   * Calloop wakes, runs the closure below once (no matter how many
    //     pings landed since the last dispatch — eventfd coalesces them).
    //   * If the dirty flag is set, render every output. `queue_frame`
    //     schedules a page-flip; the resulting `DrmEvent::VBlank` will
    //     wake the loop again, at which point `main.rs`'s post-dispatch
    //     callback ticks animations and may re-arm a redraw.
    //
    // Idle behaviour: no events → no pings → no wake-ups. CPU/GPU drop to
    // zero while the user does nothing, instead of paying a 60 Hz poll
    // tax for a flag that's almost always false.
    let (repaint_ping, repaint_source) =
        make_ping().map_err(|e| anyhow::anyhow!("create repaint ping: {e}"))?;
    event_loop
        .handle()
        .insert_source(repaint_source, {
            let backend_data = backend_data.clone();
            move |(), _, state: &mut MargoState| {
                if state.take_repaint_request() {
                    let mut bd = backend_data.borrow_mut();
                    let BackendData { renderer, outputs, drm, .. } = &mut *bd;
                    render_all_outputs(renderer, outputs, drm, state, "repaint");
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("repaint ping source: {e}"))?;
    state.set_repaint_ping(repaint_ping);
    // Initial ping to drain the dirty flag we set in MargoState::new (the
    // first frame must run; no other event has fired yet).
    state.request_repaint();

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
        let BackendData { renderer, outputs, drm, .. } = &mut *bd;
        render_all_outputs(renderer, outputs, drm, state, "initial");
    }

    // Now that all outputs have been discovered and registered via
    // setup_connector, publish the topology to wlr-output-management
    // clients so anything that bound the global before us (`kanshi`,
    // `wlr-randr` queries) sees the full list.
    state.publish_output_topology();

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
// 3. Add new outputs for connectors that *just* came up. Walks every
//    connector reported by `resource_handles().connectors()`, picks the
//    ones in `Connected` state that don't already have an OutputDevice,
//    and runs them through `setup_connector()`. The freshly-built
//    `DrmCompositor` gets an initial `render_frame` + `queue_frame` so
//    the new monitor lights up without waiting for the next repaint
//    timer tick.

fn rescan_outputs(
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

    let removed_outputs: Vec<Output> = to_remove
        .into_iter()
        .filter_map(|crtc| outputs.remove(&crtc).map(|od| od.output))
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
        // Already driving this connector? Don't double-bind.
        if bd.outputs.values().any(|od| od.connector == *conn_handle) {
            continue;
        }
        let Ok(conn_info) = bd.drm.get_connector(*conn_handle, false) else {
            continue;
        };
        if conn_info.state() != connector::State::Connected {
            continue;
        }
        // Borrow split: setup_connector needs &mut DrmDevice + &mut MargoState
        // simultaneously, so peel everything we need off `bd` first.
        let BackendData {
            drm,
            gbm,
            primary_node,
            renderer_formats,
            ..
        } = &mut *bd;

        if let Some((crtc, od)) = setup_connector(
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
            current_used.insert(crtc);
            new_outputs.push((crtc, od));
            added_any = true;
        }
    }

    for (crtc, mut od) in new_outputs {
        // Kick the swapchain so the freshly-built compositor schedules a
        // first vblank — otherwise the new monitor stays blank until the
        // global repaint timer happens to tick *and* something on the
        // existing outputs marks itself dirty.
        let elements = build_render_elements(&mut bd.renderer, &od, state);
        if let Err(e) = od.compositor.render_frame(
            &mut bd.renderer,
            &elements,
            [0.1, 0.1, 0.1, 1.0],
            FrameFlags::DEFAULT,
        ) {
            tracing::warn!("hotplug initial render failed for {}: {e:?}", od.output.name());
        } else {
            let _ = od.compositor.queue_frame(());
        }
        bd.outputs.insert(crtc, od);
    }
    drop(bd);

    if !removed_outputs.is_empty() || added_any {
        state.arrange_all();
        state.request_repaint();
    }
    // Always re-publish output topology after a rescan so kanshi /
    // wlr-randr see the new layout. snapshot_changed is cheap when
    // nothing actually changed.
    state.publish_output_topology();
}

/// Build the OutputDevice + associated MargoMonitor for a single
/// connected connector. Mirrors the inline init loop so that hotplug
/// goes through exactly the same code path as startup.
#[allow(clippy::too_many_arguments)]
fn setup_connector(
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
    let crtc = find_crtc(&drm.device_fd().clone(), conn_info, resources, used_crtcs)?;

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
        "hotplug add: {} {}x{}@{} pos={:?} scale={}",
        output_name,
        wl_mode.size.w,
        wl_mode.size.h,
        wl_mode.refresh / 1000,
        position,
        scale
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

    let drm_surface = match drm.create_surface(crtc, drm_mode, &[conn_handle]) {
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
        if s.w == 0 || s.h == 0 { (64u32, 64u32).into() } else { s }
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
    });
    state.apply_tag_rules_to_monitor(state.monitors.len() - 1);

    let mut gamma_props = GammaProps::discover(drm, crtc);
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
        crtc,
        OutputDevice {
            output,
            compositor,
            render_count: 0,
            queued_count: 0,
            empty_count: 0,
            queue_error_count: 0,
            gamma: gamma_props,
            connector: conn_handle,
        },
    ))
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

/// Drain `snapshot_pending` flags by capturing the live surface tree
/// of each affected client into a `GlesTexture`, stored in
/// `client.resize_snapshot`. Called once per frame before the render
/// element collection runs, so the rest of the frame can read the
/// snapshot from `state.clients` immutably.
fn take_pending_snapshots(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &mut MargoState,
) {
    let output_scale = od.output.current_scale().fractional_scale().into();

    // Collect the indices to process first to dodge the
    // iter-while-mutating dance: we mutate `state.clients[i]` after the
    // iteration so the borrow checker stays happy.
    let pending: Vec<usize> = state
        .clients
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if c.snapshot_pending && state.monitors.get(c.monitor).is_some_and(|m| m.output == od.output) {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    for idx in pending {
        let (window, size) = {
            let c = &state.clients[idx];
            // Capture at the *current* live geometry size, not the
            // animated slot size — the snapshot mirrors what the
            // client has on screen RIGHT NOW (typically still the
            // pre-resize buffer).
            let geom = c.window.geometry();
            (c.window.clone(), geom.size)
        };

        if size.w <= 0 || size.h <= 0 {
            // Window not yet mapped or zero-sized — skip this round,
            // the flag stays set and we'll try again next frame.
            continue;
        }

        match crate::render::window_capture::capture_window(
            renderer,
            &window,
            size,
            output_scale,
        ) {
            Ok(texture) => {
                state.clients[idx].resize_snapshot =
                    Some(crate::state::ResizeSnapshot {
                        texture,
                        source_size: size,
                        captured_at: std::time::Instant::now(),
                    });
                state.clients[idx].snapshot_pending = false;
                tracing::debug!(
                    "resize_snapshot: captured {} ({}x{})",
                    state.clients[idx].app_id,
                    size.w,
                    size.h,
                );
            }
            Err(e) => {
                tracing::warn!(
                    "resize_snapshot: capture failed for {}: {e:?}",
                    state.clients[idx].app_id
                );
                // Drop the flag anyway so we don't loop on an
                // un-snapshottable surface. Worst case the user sees
                // the existing pre-fix buffer/slot mismatch for this
                // animation.
                state.clients[idx].snapshot_pending = false;
            }
        }
    }
}

/// Drain `opening_capture_pending` and `ClosingClient::capture_pending`
/// flags by capturing the corresponding wl_surface tree to a
/// `GlesTexture`. Mirrors `take_pending_snapshots` but feeds the open
/// and close transitions instead of the resize snapshot. Has to run on
/// the render thread because `GlesRenderer` is the only place we can
/// turn a wl_buffer into a sampleable texture.
///
/// Capture failures are demoted to "no animation": for the open path
/// we drop `opening_animation` so the live surface gets drawn from the
/// next frame; for the close path we drop the closing entry entirely.
/// Better than rendering an empty rect.
fn take_pending_open_close_captures(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &mut MargoState,
) {
    let output_scale = od.output.current_scale().fractional_scale().into();

    // Open captures — only for clients on this output.
    let opening_idxs: Vec<usize> = state
        .clients
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if c.opening_capture_pending
                && c.opening_animation.is_some()
                && state.monitors.get(c.monitor).is_some_and(|m| m.output == od.output)
            {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    for idx in opening_idxs {
        let (window, size) = {
            let c = &state.clients[idx];
            let geom = c.window.geometry();
            (c.window.clone(), geom.size)
        };
        if size.w <= 0 || size.h <= 0 {
            // No buffer attached yet (Qt clients especially commit
            // null-buffer attaches during configure handshakes).
            // Leave the flag set so we retry next frame; the
            // animation timer will catch up once the buffer arrives.
            continue;
        }
        match crate::render::window_capture::capture_window(renderer, &window, size, output_scale)
        {
            Ok(texture) => {
                state.clients[idx].opening_texture = Some(texture);
                state.clients[idx].opening_capture_pending = false;
                tracing::debug!(
                    "open_anim: captured {} ({}x{})",
                    state.clients[idx].app_id,
                    size.w,
                    size.h,
                );
            }
            Err(e) => {
                tracing::warn!(
                    "open_anim: capture failed for {}: {e:?}",
                    state.clients[idx].app_id
                );
                state.clients[idx].opening_animation = None;
                state.clients[idx].opening_capture_pending = false;
            }
        }
    }

    // Close captures — only for entries on this output.
    let close_idxs: Vec<usize> = state
        .closing_clients
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if c.capture_pending
                && state.monitors.get(c.monitor).is_some_and(|m| m.output == od.output)
            {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    let mut to_drop: Vec<usize> = Vec::new();
    for idx in close_idxs {
        // We need a window-like handle to capture from. The closing
        // entry kept the wl_surface alive; build a temporary capture
        // from its surface tree directly.
        let Some(surface) = state.closing_clients[idx].source_surface.clone() else {
            to_drop.push(idx);
            continue;
        };
        let geom = state.closing_clients[idx].geom;
        let size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            geom.width.max(1),
            geom.height.max(1),
        ));
        match crate::render::window_capture::capture_surface(
            renderer,
            &surface,
            size,
            output_scale,
        ) {
            Ok(texture) => {
                state.closing_clients[idx].texture = Some(texture);
                state.closing_clients[idx].capture_pending = false;
                state.closing_clients[idx].source_surface = None;
                tracing::debug!(
                    "close_anim: captured wl_surface ({}x{})",
                    size.w,
                    size.h,
                );
            }
            Err(e) => {
                tracing::warn!("close_anim: capture failed: {e:?}");
                to_drop.push(idx);
            }
        }
    }

    // Drop failures (in reverse so indices stay valid).
    for idx in to_drop.into_iter().rev() {
        state.closing_clients.remove(idx);
    }

    // Layer-surface close captures. Same shape as the toplevel-close
    // path: the layer's wl_surface is still alive at `layer_destroyed`
    // time, the renderer grabs one frame of it before it goes dark.
    let pending_layer_keys: Vec<_> = state
        .layer_animations
        .iter()
        .filter_map(|(k, a)| if a.is_close && a.capture_pending { Some(k.clone()) } else { None })
        .collect();
    for key in pending_layer_keys {
        let Some(anim) = state.layer_animations.get(&key) else { continue };
        let Some(surface) = anim.source_surface.clone() else { continue };
        let geom = anim.geom;
        let size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            geom.width.max(1),
            geom.height.max(1),
        ));
        match crate::render::window_capture::capture_surface(
            renderer,
            &surface,
            size,
            output_scale,
        ) {
            Ok(texture) => {
                if let Some(a) = state.layer_animations.get_mut(&key) {
                    a.texture = Some(texture);
                    a.capture_pending = false;
                    a.source_surface = None;
                }
            }
            Err(e) => {
                tracing::warn!("layer close_anim: capture failed: {e:?}");
                state.layer_animations.remove(&key);
            }
        }
    }
}

fn build_render_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
) -> Vec<MargoRenderElement> {
    build_render_elements_inner(renderer, od, state, true, false)
}

/// Like `build_render_elements`, but optionally omits the cursor sprite
/// and/or substitutes blocked-out (`block_out_from_screencast = 1`) clients
/// with solid black rectangles. The cursor flag is honoured by every
/// caller (display render passes `true`, screencopy with `overlay_cursor`
/// off passes `false`); the screencast flag is set ONLY by
/// `serve_screencopies` so the regular display render still shows
/// password managers / private-browsing tabs / 2FA codes intact while
/// any wlr-screencopy client recording the output sees them blacked out.
fn build_render_elements_inner(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    include_cursor: bool,
    for_screencast: bool,
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
        state,
        &mut elements,
    );

    push_client_elements(
        renderer,
        state,
        &od.output,
        output_geo,
        output_scale,
        border_program.clone(),
        clipped_surface_program.clone(),
        for_screencast,
        &mut elements,
    );

    // Closing-client snapshots. Each entry is a window whose toplevel
    // role was destroyed but whose close animation hasn't finished;
    // we render the captured texture scaled+faded around its last
    // known geometry. Drawn AFTER the live clients so it's on top of
    // its old layer band — slightly fragile if a new window mapped
    // exactly underneath, but acceptable for a sub-second transition.
    push_closing_clients(
        state,
        &od.output,
        output_geo,
        output_scale,
        clipped_surface_program.clone(),
        &mut elements,
    );

    push_layer_elements(
        renderer,
        &layer_map,
        &lower_layers,
        output_scale,
        1.0,
        state,
        &mut elements,
    );

    push_closing_layers(
        state,
        &od.output,
        output_geo,
        output_scale,
        clipped_surface_program,
        &mut elements,
    );

    elements
}

/// Push render elements for windows in their close animation. Mirrors
/// `push_client_elements` but operates on `state.closing_clients`
/// (entries that survived `toplevel_destroyed` to play their fade-out)
/// instead of mapped clients.
fn push_closing_clients(
    state: &MargoState,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    clipped_surface_program: Option<GlesTexProgram>,
    elements: &mut Vec<MargoRenderElement>,
) {
    let scale = Scale::from(output_scale);
    let target_mon_idx = state
        .monitors
        .iter()
        .position(|m| m.output == *output);
    let Some(target_mon_idx) = target_mon_idx else {
        return;
    };
    let tagset = if state.monitors[target_mon_idx].is_overview {
        !0
    } else {
        state.monitors[target_mon_idx].current_tagset()
    };

    for cc in state.closing_clients.iter() {
        if cc.monitor != target_mon_idx {
            continue;
        }
        if (cc.tags & tagset) == 0 {
            continue;
        }
        let Some(texture) = cc.texture.as_ref() else {
            continue;
        };
        let dst = smithay::utils::Rectangle::new(
            (cc.geom.x - output_geo.loc.x, cc.geom.y - output_geo.loc.y).into(),
            (cc.geom.width.max(1), cc.geom.height.max(1)).into(),
        );
        elements.push(MargoRenderElement::OpenClose(
            crate::render::open_close::OpenCloseRenderElement::new(
                cc.id.clone(),
                texture.clone(),
                dst,
                scale,
                cc.progress,
                1.0,
                cc.kind,
                true, // is_close
                cc.extreme_scale,
                smithay::backend::renderer::utils::CommitCounter::default(),
                cc.border_radius,
                clipped_surface_program.clone(),
            ),
        ));
    }
}

fn push_client_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    border_program: Option<smithay::backend::renderer::gles::GlesPixelProgram>,
    clipped_surface_program: Option<GlesTexProgram>,
    for_screencast: bool,
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

        // Screencast blackout: when we're building the element list
        // for a wlr-screencopy capture (`for_screencast = true`) and
        // this window has the windowrule's `block_out_from_screencast
        // = 1` flag set, replace its surface render with a solid
        // black rectangle. The on-screen render path doesn't go
        // through this branch (it passes `for_screencast = false`)
        // so the user still sees their password manager / private-
        // browsing tab / 2FA app — only the captured output is
        // censored.
        if for_screencast
            && client.is_some_and(|c| c.block_out_from_screencast)
        {
            if let Some(c) = client {
                let dst = Rectangle::<i32, smithay::utils::Physical>::new(
                    smithay::utils::Point::from((
                        c.geom.x - output_geo.loc.x,
                        c.geom.y - output_geo.loc.y,
                    ))
                    .to_physical_precise_round::<f64, _>(scale),
                    smithay::utils::Size::from((c.geom.width.max(1), c.geom.height.max(1)))
                        .to_physical_precise_round::<f64, _>(scale),
                );
                let id = match window.wl_surface() {
                    Some(s) => smithay::backend::renderer::element::Id::from_wayland_resource(
                        &*s,
                    ),
                    None => smithay::backend::renderer::element::Id::new(),
                };
                elements.push(MargoRenderElement::Solid(
                    smithay::backend::renderer::element::solid::SolidColorRenderElement::new(
                        id,
                        dst,
                        smithay::backend::renderer::utils::CommitCounter::default(),
                        [0.0, 0.0, 0.0, 1.0],
                        smithay::backend::renderer::element::Kind::Unspecified,
                    ),
                ));
            }
            continue;
        }

        let radius = client
            .filter(|client| !client.no_radius && !client.is_fullscreen)
            .map(|_| state.config.border_radius.max(0) as f32)
            .unwrap_or(0.0);
        // Clip the surface tree to the same `min(geometry.size, slot)`
        // box that `border::refresh` uses for the border. The two
        // following the SAME rect is what gives a tight fit on
        // Electron clients (Spotify especially) that report a
        // declared `geometry().size` smaller than the slot we
        // requested but ALSO render a wl_buffer that's bigger than
        // their declared geometry — without intersecting the clip
        // with `geometry.size`, the surface bleeds beyond the border
        // by `buffer - geometry` pixels on the right / bottom while
        // the border stays at `geometry`. With this intersection,
        // the surface and border are guaranteed to share an outline.
        //
        // Snapshot/animation path is unaffected: when
        // `resize_snapshot` is in flight the border tracks `c.geom`
        // unmodified, so we want the clip to track `c.geom` too,
        // which is what skipping the intersection during a snapshot
        // achieves.
        let clip_geometry = client.map(|client| {
            let actual = client.window.geometry().size;
            // `snapshot_pending` mirrors the same gate used in
            // border::refresh — the clip and the border have to
            // share a rect, otherwise the resize transition's
            // snapshot (drawn at the full slot) would extend past
            // the border that already shrunk to `actual`.
            let snapshot_active =
                client.resize_snapshot.is_some() || client.snapshot_pending;
            let mut w = client.geom.width.max(1);
            let mut h = client.geom.height.max(1);
            if !snapshot_active {
                if actual.w > 0 && actual.w < w {
                    w = actual.w;
                }
                if actual.h > 0 && actual.h < h {
                    h = actual.h;
                }
            }
            Rectangle::new(
                (
                    f64::from(client.geom.x - output_geo.loc.x),
                    f64::from(client.geom.y - output_geo.loc.y),
                )
                    .into(),
                (f64::from(w), f64::from(h)).into(),
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

                // Niri-style resize transition: render BOTH the live
                // surface AND a snapshot of the pre-resize content,
                // crossfading between them as the move animation
                // progresses.
                //
                //   * The live surface goes down first (rendered as it
                //     normally would be — clipped to the slot, with
                //     rounded corners). At the start of the transition
                //     this is typically still the OLD buffer at the
                //     OLD size, the configure ack hasn't landed yet,
                //     so the live render alone would show "buffer
                //     bigger than slot, content clipped weirdly."
                //   * On TOP of that we push a `ResizeRenderElement`
                //     drawing the captured snapshot, scaled to the
                //     current animated slot, with progress-controlled
                //     alpha. The snapshot is what the user actually
                //     saw the frame BEFORE the resize started, so it
                //     hides the live render's misalignment for the
                //     first half of the transition. As the alpha
                //     fades from 1.0 → 0.0 over the animation
                //     duration, the live (by then correctly-sized)
                //     surface bleeds through.
                //
                // Net effect: the user sees a smooth crossfade from
                // the pre-resize content to the post-resize content,
                // covering the moment Helium / Spotify is busy
                // re-laying out for the new size.

                // Smithay convention: first-pushed element is
                // top-most visually. So during the resize transition
                // we push the snapshot FIRST (top, translucent,
                // fading out) and then the live surface elements
                // BELOW (fully opaque, visible through the fading
                // snapshot). Smithay's `opaque_regions()` for our
                // `ResizeRenderElement` returns empty so the live
                // render below is NOT skipped — both layers always
                // composite together for the crossfade.

                // Open animation: if this client is in the middle of
                // its open transition AND we've captured a texture
                // for it, render the snapshot through OpenClose
                // instead of the live surface tree. The live tree is
                // SKIPPED entirely for the duration of the curve so
                // the user doesn't see "instant pop, then animation"
                // — the very first frame already animates from the
                // `extreme_scale` start. Once the animation settles,
                // `tick_animations` clears `opening_animation` and
                // `opening_texture`, and the live render below picks
                // up unmodified.
                // Open animation: capture-pending OR texture-ready
                // both suppress the live surface render. The reason
                // we suppress even before capture: the live surface
                // at progress = 0 would otherwise pop in at full
                // alpha+scale for one frame before the animation
                // kicks in. Better to draw nothing for that one
                // frame than betray the transition.
                if let Some(c) = client {
                    if c.opening_animation.is_some() {
                        if let Some((anim, tex)) = c
                            .opening_animation
                            .as_ref()
                            .and_then(|a| c.opening_texture.as_ref().map(|t| (a, t)))
                        {
                            let dst = smithay::utils::Rectangle::new(
                                (c.geom.x - output_geo.loc.x, c.geom.y - output_geo.loc.y)
                                    .into(),
                                (c.geom.width.max(1), c.geom.height.max(1)).into(),
                            );
                            let id =
                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                    wl_surface,
                                );
                            elements.push(MargoRenderElement::OpenClose(
                                crate::render::open_close::OpenCloseRenderElement::new(
                                    id,
                                    tex.clone(),
                                    dst,
                                    scale,
                                    anim.progress,
                                    1.0,
                                    anim.kind,
                                    false,
                                    anim.extreme_scale,
                                    smithay::backend::renderer::utils::CommitCounter::default(),
                                    radius,
                                    clipped_surface_program.clone(),
                                ),
                            ));
                        }
                        // capture_pending → emit nothing this frame; next
                        // frame the texture will be ready.
                        continue;
                    }
                }

                // Two-texture niri-style crossfade: if a snapshot
                // is active, capture the live surface tree to a
                // *fresh* GlesTexture this frame (`tex_next`), then
                // composite tex_prev and tex_next together via a
                // single ResizeRenderElement that draws BOTH
                // through the same `render_texture_from_to` path
                // and the same rounded-clip shader. This is the
                // niri pattern: the only thing that differs between
                // the two layers in the final output is the source
                // texture and the alpha — everything else (pixel
                // snapping, clipping, transform) is byte-identical,
                // so there's nothing for the eye to lock onto as
                // "movement" between the layers.
                let mut snapshot_active = false;
                if let Some((c, snapshot)) =
                    client.and_then(|c| c.resize_snapshot.as_ref().map(|s| (c, s)))
                {
                    let dur_ms = state.config.animation_duration_move.max(1) as f32;
                    let elapsed_ms = snapshot.captured_at.elapsed().as_millis() as f32;
                    let progress = (elapsed_ms / dur_ms).clamp(0.0, 1.0);

                    let dst = smithay::utils::Rectangle::new(
                        (
                            c.geom.x - output_geo.loc.x,
                            c.geom.y - output_geo.loc.y,
                        )
                            .into(),
                        (c.geom.width.max(1), c.geom.height.max(1)).into(),
                    );
                    let id =
                        smithay::backend::renderer::element::Id::from_wayland_resource(
                            wl_surface,
                        );

                    // Capture LIVE → tex_next this frame. The
                    // capture goes through the same offscreen-
                    // render path as tex_prev (`capture_window`),
                    // so the resulting texture has the same
                    // pixel-level layout as the snapshot would have
                    // if taken right now. Failure → no tex_next,
                    // ResizeRenderElement falls back to tex_prev
                    // only at full alpha (no worse than the
                    // single-texture variant we had before).
                    let live_size = c.window.geometry().size;
                    let tex_next = if live_size.w > 0 && live_size.h > 0 {
                        match crate::render::window_capture::capture_window(
                            renderer,
                            &c.window,
                            live_size,
                            output_scale.into(),
                        ) {
                            Ok(t) => Some(t),
                            Err(e) => {
                                tracing::trace!(
                                    "resize_next: live capture failed: {e:?}"
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };

                    let resize_program = clipped_surface_program.clone();
                    elements.push(MargoRenderElement::Resize(
                        crate::render::resize_render::ResizeRenderElement::new(
                            id,
                            snapshot.texture.clone(),
                            tex_next,
                            dst,
                            scale,
                            progress,
                            1.0,
                            smithay::backend::renderer::utils::CommitCounter::default(),
                            radius,
                            resize_program,
                        ),
                    ));
                    let _ = snapshot.source_size; // for clarity
                    snapshot_active = true;
                }

                // While a resize transition is in flight we render
                // ONLY through ResizeRenderElement (which contains
                // both prev and next textures). Skipping the live
                // surface's WaylandSurfaceRenderElement tree here
                // is what guarantees the layers can't desync — they
                // *are* the same draw path now. Once the snapshot
                // expires (animation done, tick_animations clears
                // `resize_snapshot`), we drop back to the normal
                // live render below.
                if snapshot_active {
                    // Skip the live Wayland surface tree for this
                    // window; tex_next inside the ResizeRenderElement
                    // already represents its current frame. (We do
                    // NOT skip the rest of the function — other
                    // windows in the iteration still need to be
                    // rendered. Hence `continue` on the outer
                    // `for window in ...` loop, not `return`.)
                    continue;
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
                // XWayland clients route through the same
                // `clipped_surface` shader as native Wayland: without
                // this, the X11 branch pushed the rendered surface
                // straight into the scene with no rounded-clip mask
                // applied. Spotify under XWayland reports a
                // `geometry().size` larger than the slot we
                // allocate (1520×1158 vs slot 1488×1152 in the
                // user's layout), and the unclipped X11 path leaked
                // those extra 32×6 px past the border on the right
                // and bottom — exactly the "border tutarsız" the
                // user kept reporting on Spotify after every
                // semsumo-daily startup. Same wrapping logic as the
                // Wayland branch: if `radius > 0` and the shader
                // is available, wrap each rendered element in
                // `ClippedSurfaceRenderElement` with the
                // `min(actual, slot)` clip rect so border + surface
                // share an outline.
                for elem in rendered {
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
        }
    }
}

fn push_layer_elements(
    renderer: &mut GlesRenderer,
    layer_map: &smithay::desktop::LayerMap,
    layers: &[&smithay::desktop::LayerSurface],
    output_scale: f64,
    alpha: f32,
    state: &MargoState,
    elements: &mut Vec<MargoRenderElement>,
) {
    use smithay::reexports::wayland_server::Resource;
    for surface in layers {
        let Some(geo) = layer_map.layer_geometry(surface) else {
            continue;
        };

        // Skip the LIVE render entirely if this layer is in its close
        // animation — `push_closing_layers` paints it from the
        // captured texture instead. (smithay's LayerMap won't
        // actually have the layer at this point either, since
        // `unmap_layer` already ran in `layer_destroyed`; this guard
        // is just defensive.)
        let key = surface.layer_surface().wl_surface().id();
        if state.layer_animations.get(&key).map(|a| a.is_close).unwrap_or(false) {
            continue;
        }

        // Open animation: scale alpha by the curve's progress so the
        // layer fades in. We don't slide the geometry — layer surfaces
        // typically have anchor-driven layout that the user would
        // notice if we shifted, and the slide-in feel is mostly carried
        // by the alpha curve anyway.
        let layer_alpha = match state.layer_animations.get(&key) {
            Some(anim) if !anim.is_close => alpha * anim.progress.clamp(0.0, 1.0),
            _ => alpha,
        };

        let rendered =
            AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<
                GlesRenderer,
            >>(
                *surface,
                renderer,
                geo.loc.to_physical_precise_round(output_scale),
                Scale::from(output_scale),
                layer_alpha,
            );
        elements.extend(
            rendered
                .into_iter()
                .map(|elem| MargoRenderElement::Space(SpaceRenderElements::Surface(elem))),
        );
    }
}

/// Render the captured texture for any layer surface in its close
/// animation. Mirrors `push_closing_clients` but for layer surfaces;
/// drawn in the layer band so notification-style layers fade out
/// where they were instead of leaping to a different stacking
/// position.
fn push_closing_layers(
    state: &MargoState,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    clipped_surface_program: Option<GlesTexProgram>,
    elements: &mut Vec<MargoRenderElement>,
) {
    let scale = Scale::from(output_scale);
    let target_mon_idx = state.monitors.iter().position(|m| m.output == *output);
    let Some(_target_mon_idx) = target_mon_idx else {
        return;
    };
    for (_id, anim) in state.layer_animations.iter() {
        if !anim.is_close {
            continue;
        }
        let Some(texture) = anim.texture.as_ref() else {
            continue;
        };
        let dst = smithay::utils::Rectangle::new(
            (anim.geom.x - output_geo.loc.x, anim.geom.y - output_geo.loc.y).into(),
            (anim.geom.width.max(1), anim.geom.height.max(1)).into(),
        );
        // Per-frame fresh Id — the ObjectId is stable across frames
        // so we *could* derive a stable Id, but smithay's damage
        // tracker copes with new ids fine for short-lived render
        // elements like the close transition. The simpler `Id::new()`
        // avoids the Resource-vs-ObjectId type juggling at no real
        // cost (the close window is < 500 ms).
        let elem_id = smithay::backend::renderer::element::Id::new();
        elements.push(MargoRenderElement::OpenClose(
            crate::render::open_close::OpenCloseRenderElement::new(
                elem_id,
                texture.clone(),
                dst,
                scale,
                anim.progress,
                1.0,
                anim.kind,
                true,
                0.6, // layer surfaces don't carry the same zoom_end_ratio config — pick a sensible default
                smithay::backend::renderer::utils::CommitCounter::default(),
                0.0, // no rounded-corner clip on layers
                clipped_surface_program.clone(),
            ),
        ));
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

        // Re-build the element list with `for_screencast = true` so
        // any window flagged `block_out_from_screencast = 1` in the
        // user's windowrules gets substituted with a solid black
        // rect — password managers, private-browsing tabs, 2FA
        // apps, polkit prompts, …. We can't reuse the main display
        // `elements` array even when `overlay_cursor = true`,
        // because that array was built without the screencast
        // blackout filter (it's still showing those windows
        // intact, which is what the user is supposed to see on
        // screen). The cursor sprite is included if the client
        // asked for it via `overlay_cursor`.
        let owned_elements = build_render_elements_inner(
            renderer,
            od,
            state,
            screencopy.overlay_cursor(),
            true,
        );
        let elements_to_render: &[MargoRenderElement] = &owned_elements;
        let _ = elements; // main display array intentionally unused for screencast

        // ── DMA-BUF zero-copy fast path ─────────────────────────────
        // OBS / Discord / xdg-desktop-portal-wlr negotiate dmabuf via
        // `frame.linux_dmabuf(...)` and submit a GBM-allocated wl_buffer
        // sized to the full output. That's the cheap case: bind the
        // dmabuf as a render target and let smithay's damage tracker
        // paint the elements straight into it. No CPU readback, no SHM
        // upload — the screencast pipeline gets a buffer the GPU has
        // already touched. For region capture (`grim -g`) we'd need to
        // translate elements by `-region_loc` and render at
        // `buffer_size` instead of `output_size`; that's a follow-up
        // (smithay's OutputDamageTracker doesn't take a render-time
        // offset, so we'd need RelocateRenderElement wrapping). Until
        // then, region capture with a dmabuf target fails the frame —
        // grim is the only user we know of that requests one and it
        // happily falls back to SHM.
        if let crate::protocols::screencopy::ScreencopyBuffer::Dmabuf(dmabuf) =
            screencopy.buffer()
        {
            // Translate every element by `-region_loc` and render
            // to a damage-tracker sized to the *client's* requested
            // `buffer_size`. For full-output capture region_loc is
            // (0,0) and buffer_size == output_size, so the wrap is
            // a no-op and the relocate elements forward to their
            // inner draw unchanged. For region capture (`grim -g`,
            // a portal `crop` request) we shift the world by
            // -region_loc so the requested rect lands at (0,0) of
            // the dmabuf and run a damage tracker at buffer_size,
            // which clips anything outside the dmabuf for free.
            use smithay::backend::renderer::element::utils::{
                Relocate, RelocateRenderElement,
            };
            let translate: smithay::utils::Point<i32, smithay::utils::Physical> =
                (-region_loc.x, -region_loc.y).into();
            let relocated: Vec<RelocateRenderElement<&MargoRenderElement>> = elements_to_render
                .iter()
                .map(|e| RelocateRenderElement::from_element(e, translate, Relocate::Relative))
                .collect();

            let mut dmabuf = dmabuf.clone();
            let render_result = match renderer.bind(&mut dmabuf) {
                Ok(mut target) => {
                    let mut tracker = DamageTracker::new(size, scale, Transform::Normal);
                    let res = tracker
                        .render_output(
                            renderer,
                            &mut target,
                            0,
                            &relocated,
                            [0.0, 0.0, 0.0, 1.0],
                        )
                        .map(|r| r.damage.map(|d| d.to_owned()));
                    drop(target);
                    res
                }
                Err(e) => Err(smithay::backend::renderer::damage::Error::Rendering(e)),
            };

            match render_result {
                Ok(damage) => {
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
                    // Implicit-sync dmabuf submit. Every consumer
                    // we've tested (xdg-desktop-portal-wlr, OBS,
                    // wf-recorder, grim) is happy with this; the
                    // explicit-sync path comes back when DRM
                    // syncobj fences are wired into screencopy.
                    screencopy.submit_now(false, now);
                }
                Err(e) => warn!("screencopy: dmabuf render failed: {e:?}"),
            }
            continue;
        }

        // ── SHM path (renderbuffer + read-back + memcpy) ─────────────
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

        // Write into the SHM buffer. The dmabuf branch is handled
        // earlier in the loop with a zero-copy bind+render path; by
        // the time we reach this match the buffer must be SHM. The
        // Dmabuf arm is unreachable but kept exhaustive so adding a
        // future ScreencopyBuffer variant fails to compile here.
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
                // Should not happen: handled earlier and `continue`d.
                unreachable!("screencopy dmabuf path took the SHM branch");
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

/// Drain `wp_presentation_feedback` callbacks across every surface
/// that contributed a render element to the frame just queued, and
/// signal `presented(...)` on each. Mirrors anvil's
/// `take_presentation_feedback` + immediate-present pattern: we
/// don't have hardware-level page-flip timing yet (would require
/// hooking into DrmCompositor's vblank callbacks), so the timestamp
/// we publish is "right now" with a Vsync flag — same approximation
/// niri's winit backend uses. Clients that care about microsecond
/// accuracy will benefit when we plumb actual vblank timestamps in
/// a follow-up; in the meantime the protocol surface is exposed and
/// kitty / mpv stop guessing 60 Hz.
fn publish_presentation_feedback(
    output: &Output,
    state: &mut MargoState,
    render_states: &smithay::backend::renderer::element::RenderElementStates,
) {
    use smithay::desktop::layer_map_for_output;
    use smithay::desktop::utils::{
        surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
        OutputPresentationFeedback,
    };
    use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;

    let mut feedback = OutputPresentationFeedback::new(output);

    // Toplevels.
    for window in state.space.elements() {
        if state.space.outputs_for_element(window).contains(output) {
            window.take_presentation_feedback(
                &mut feedback,
                surface_primary_scanout_output,
                |surface, _| {
                    surface_presentation_feedback_flags_from_states(surface, None, render_states)
                },
            );
        }
    }
    // Layer surfaces (bar, notifications, OSD).
    let map = layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.take_presentation_feedback(
            &mut feedback,
            surface_primary_scanout_output,
            |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, None, render_states)
            },
        );
    }

    let now = monotonic_now();
    let refresh = output
        .current_mode()
        .map(|m| {
            // mode.refresh is in mHz; convert to per-frame duration.
            let hz = (m.refresh as f64) / 1000.0;
            if hz > 0.0 {
                std::time::Duration::from_secs_f64(1.0 / hz)
            } else {
                std::time::Duration::from_secs_f64(1.0 / 60.0)
            }
        })
        .unwrap_or_else(|| std::time::Duration::from_secs_f64(1.0 / 60.0));

    feedback.presented::<_, smithay::utils::Monotonic>(
        now,
        smithay::wayland::presentation::Refresh::fixed(refresh),
        0, // sequence — we don't track DRM page-flip seq yet
        wp_presentation_feedback::Kind::Vsync,
    );
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
    // Niri-style resize transition: any window whose layout slot
    // changed size since the last frame had `snapshot_pending` set by
    // `arrange_monitor`. We're now on the render thread with a live
    // `GlesRenderer`, so this is the moment we can actually allocate
    // the offscreen GlesTexture and paint the surface tree into it.
    // Subsequent frames will draw the snapshot scaled to the
    // (animated) slot until the move animation finishes — at which
    // point `tick_animations` clears `resize_snapshot` and we go back
    // to drawing the live surface.
    take_pending_snapshots(renderer, od, state);
    take_pending_open_close_captures(renderer, od, state);

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
                    // Bumps `pending_vblanks` so further repaint requests
                    // queue silently until the page-flip completes; the
                    // matching `DrmEvent::VBlank` will pop it back down.
                    state.note_frame_queued();
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
                    // wp_presentation: notify each surface that
                    // contributed a render element this frame about
                    // the actual present time + refresh interval.
                    // Clients that registered a `feedback` request
                    // get the precise timestamp they need to pace
                    // their next frame against the real display
                    // refresh, instead of guessing 60 Hz.
                    publish_presentation_feedback(&od.output, state, &result.states);
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
