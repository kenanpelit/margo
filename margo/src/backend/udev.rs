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
    Shadow=crate::render::shadow::ShadowRenderElement,
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
    /// `wp_presentation_feedback` builders that have been collected at
    /// submit time but not yet signalled. We hold them here until the
    /// matching `DrmEvent::VBlank` fires, then call `.presented(now,
    /// refresh, 0, Vsync)` at that point — the timestamp is the actual
    /// page-flip moment instead of "when we queued the flip", which is
    /// off by one frame interval. Smithay 0.7's `DrmEvent::VBlank`
    /// doesn't expose the kernel's page-flip seq, so the seq value
    /// stays 0; clients that need true monotonic seq will see this as
    /// a future-iteration upgrade. Stored as a Vec because in pathological
    /// cases (smithay reports back-to-back VBlanks) we'd otherwise drop
    /// feedbacks; in practice it's almost always 0 or 1 entry.
    pending_presentation: Vec<smithay::desktop::utils::OutputPresentationFeedback>,
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
                pending_presentation: Vec::new(),
            },
        );
    }

    if state.monitors.is_empty() {
        return Err(anyhow::anyhow!("no connected outputs found"));
    }

    // Hand a clone of GBM + render-format set to MargoState so the
    // screencast / D-Bus thread can read them without crossing the
    // udev backend's RefCell borrow. Fresh snapshot — formats
    // change rarely (only on renderer reset, which margo doesn't
    // currently do at runtime).
    state.cast_gbm = Some(gbm.clone());
    state.cast_render_formats = renderer_formats.clone();

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
                    // Acknowledge the previous flip and drain any
                    // pending presentation feedback that was queued
                    // when we submitted that frame. We do this in a
                    // tight scope so the `RefMut` is released before
                    // we signal `presented(...)` — the per-surface
                    // callbacks may end up taking their own borrows
                    // on backend_data via wayland-server dispatch.
                    let mut to_flush: Vec<(Output, smithay::desktop::utils::OutputPresentationFeedback)> = Vec::new();
                    {
                        let mut bd = backend_data.borrow_mut();
                        if let Some(od) = bd.outputs.get_mut(&crtc) {
                            // Without this, queue_frame for the next
                            // frame will fail and the render loop
                            // stalls.
                            if let Err(e) = od.compositor.frame_submitted() {
                                warn!("frame_submitted: {e:?}");
                            }
                            for feedback in od.pending_presentation.drain(..) {
                                to_flush.push((od.output.clone(), feedback));
                            }
                        }
                    }
                    // Now that the page flip has actually landed,
                    // signal `wp_presentation_feedback.presented`
                    // for every surface that contributed to the
                    // frame we queued earlier. The timestamp is
                    // taken inside `flush_presentation_feedback` so
                    // it matches the real flip moment instead of
                    // the submit moment. In steady state there's
                    // exactly one entry; the Vec covers the rare
                    // case of back-to-back VBlanks queued before
                    // we drained the previous one.
                    for (output, feedback) in to_flush {
                        flush_presentation_feedback(&output, feedback);
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
                // Apply any wlr_output_management mode changes
                // before rendering. The handler that accepted them
                // ran on MargoState (no backend handle), so it
                // queued requests onto state.pending_output_mode_changes;
                // we drain here where DrmCompositor::use_mode is
                // reachable. A successful apply re-arranges and
                // re-pings repaint internally.
                if !state.pending_output_mode_changes.is_empty() {
                    let mut bd = backend_data.borrow_mut();
                    apply_pending_mode_changes(&mut bd, state);
                }
                if state.take_repaint_request() {
                    let mut bd = backend_data.borrow_mut();
                    let BackendData { renderer, outputs, drm, .. } = &mut *bd;
                    render_all_outputs(renderer, outputs, drm, state, "repaint");
                }
                // ext-image-copy-capture: drain pending frames
                // queued by `ImageCopyCaptureHandler::frame()` and
                // render each into its client buffer. Done after
                // the live render so the renderer is warm + the
                // scene state is the same one the user just saw.
                if !state.pending_image_copy_frames.is_empty() {
                    let mut bd = backend_data.borrow_mut();
                    let BackendData { renderer, outputs, .. } = &mut *bd;
                    drain_image_copy_frames(renderer, outputs, state);
                }
                // PipeWire screencast: render every active cast on
                // every repaint. niri's output render path tags casts
                // with `check_time_and_schedule` for inter-frame
                // pacing; until that lands here, we lean on
                // PipeWire's own `dequeue_available_buffer` to drop
                // frames when the consumer hasn't returned a buffer
                // yet — buffer-bounded backpressure.
                //
                // We only need ONE active cast (or one pending
                // PipeWire redraw msg) to bother running the drain.
                let needs_drain = !state.pending_cast_redraws.is_empty()
                    || state
                        .screencasting
                        .as_ref()
                        .is_some_and(|s| !s.casts.is_empty());
                if needs_drain {
                    let mut bd = backend_data.borrow_mut();
                    let BackendData { renderer, outputs, .. } = &mut *bd;
                    drain_pending_cast_frames(renderer, outputs, state);
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
            pending_presentation: Vec::new(),
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

/// Drain `MargoState::pending_image_copy_frames` and render each
/// frame into its client buffer. Step 2 of the per-window
/// screencast story — output capture today, toplevel capture
/// pending Step 2.5.
///
/// Called once per repaint after the live render so the
/// renderer is already warm and the scene state is identical
/// to what just landed on screen. Each frame is rendered into
/// an offscreen Xrgb8888 renderbuffer, then `copy_framebuffer`
/// reads pixels back and we memcpy into the client's SHM buffer
/// — exactly the same shape as `serve_screencopies`'s SHM arm,
/// just driven by a different list of consumers.
///
/// DMA-BUF transport is Step 2.1 — for now SHM is the only
/// allocation path the handler advertises, so every frame here
/// is SHM-backed.
fn drain_image_copy_frames(
    renderer: &mut GlesRenderer,
    outputs: &mut std::collections::HashMap<crtc::Handle, OutputDevice>,
    state: &mut MargoState,
) {
    use smithay::backend::renderer::damage::OutputDamageTracker as DamageTracker;
    use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
    use smithay::wayland::image_copy_capture::CaptureFailureReason;

    let drained: Vec<_> = state.pending_image_copy_frames.drain(..).collect();
    if drained.is_empty() {
        return;
    }

    for mut pending in drained {
        let frame = match pending.frame.take() {
            Some(f) => f,
            None => continue,
        };

        // Two source kinds — output (Screen tab) and toplevel
        // (Window tab). Both end up rendering into the same
        // shape of GLES renderbuffer + SHM memcpy; the only
        // difference is which scene subset we render.
        //
        // We pre-compute (output_size, scale, render_elements)
        // for each kind, then the shared bind/render/copy block
        // below handles the rest.
        let (buf_size, scale, elements_owned): (
            smithay::utils::Size<i32, smithay::utils::Buffer>,
            f64,
            Vec<MargoRenderElement>,
        ) = match &pending.source {
            crate::PendingImageCopySource::Output(name) => {
                let od = match outputs
                    .iter_mut()
                    .find(|(_, od)| od.output.name() == *name)
                {
                    Some((_, od)) => od,
                    None => {
                        frame.fail(CaptureFailureReason::Stopped);
                        continue;
                    }
                };
                let output_size = od
                    .output
                    .current_mode()
                    .map(|m| m.size)
                    .unwrap_or_default();
                if output_size.w == 0 || output_size.h == 0 {
                    frame.fail(CaptureFailureReason::Stopped);
                    continue;
                }
                let scale = od.output.current_scale().fractional_scale();
                let buf_size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from(
                    (output_size.w, output_size.h),
                );
                let elements = build_render_elements_inner(renderer, od, state, false, true);
                (buf_size, scale, elements)
            }
            crate::PendingImageCopySource::Toplevel(window) => {
                use smithay::backend::renderer::element::AsRenderElements;

                // Find the live MargoClient backing this Window
                // so we can read its current geometry. Window is
                // Arc-backed; even if the client got dropped
                // from `state.clients`, the Window itself is
                // still alive enough to render its surface tree
                // — but if the underlying wl_surface destroyed,
                // render_elements returns empty + we'd send a
                // black frame, which is worse than failing.
                let client = state
                    .clients
                    .iter()
                    .find(|c| &c.window == window);
                let geom = match client {
                    Some(c) => c.geom,
                    None => {
                        // Toplevel went away.
                        frame.fail(CaptureFailureReason::Stopped);
                        continue;
                    }
                };
                if geom.width <= 0 || geom.height <= 0 {
                    frame.fail(CaptureFailureReason::BufferConstraints);
                    continue;
                }
                // Render the window into a buffer sized to its
                // own geometry. Scale 1.0: the window's render
                // tree is already in physical pixels for the
                // monitor it lives on; we don't fractional-scale
                // the capture (clients pick a target resolution
                // via their own framework).
                let scale = smithay::utils::Scale::from(1.0);
                // Element location (0, 0) so the window's top-
                // left lines up with the buffer's origin —
                // capture is the window itself, not the screen
                // it's positioned on.
                let elements: Vec<smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>> =
                    AsRenderElements::<GlesRenderer>::render_elements(
                        window,
                        renderer,
                        smithay::utils::Point::from((0, 0)),
                        scale,
                        1.0,
                    );
                // Wrap each surface element in MargoRenderElement
                // so the existing render_output dispatch works.
                let wrapped: Vec<MargoRenderElement> = elements
                    .into_iter()
                    .map(MargoRenderElement::WaylandSurface)
                    .collect();
                let buf_size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from(
                    (geom.width, geom.height),
                );
                (buf_size, 1.0, wrapped)
            }
        };

        let elements_refs: Vec<&MargoRenderElement> = elements_owned.iter().collect();
        let output_size = smithay::utils::Size::<i32, smithay::utils::Physical>::from(
            (buf_size.w, buf_size.h),
        );

        // Allocate an offscreen renderbuffer and render the scene
        // into it. Identical shape to the SHM arm of
        // `serve_screencopies` — see that function for context.
        let mut renderbuffer = match <GlesRenderer as Offscreen<
            smithay::backend::renderer::gles::GlesRenderbuffer,
        >>::create_buffer(
            renderer,
            drm_fourcc::DrmFourcc::Xrgb8888,
            buf_size,
        ) {
            Ok(rb) => rb,
            Err(e) => {
                warn!("image_copy_capture: create_buffer failed: {e:?}");
                frame.fail(CaptureFailureReason::Unknown);
                continue;
            }
        };
        let mut target = match renderer.bind(&mut renderbuffer) {
            Ok(t) => t,
            Err(e) => {
                warn!("image_copy_capture: bind renderbuffer failed: {e:?}");
                frame.fail(CaptureFailureReason::Unknown);
                continue;
            }
        };
        let mut tracker = DamageTracker::new(output_size, scale, Transform::Normal);
        if let Err(e) = tracker.render_output(
            renderer,
            &mut target,
            0,
            &elements_refs,
            [0.0, 0.0, 0.0, 1.0],
        ) {
            warn!("image_copy_capture: render_output failed: {e:?}");
            frame.fail(CaptureFailureReason::Unknown);
            continue;
        }
        // Pull pixels back from GL into a CPU-side mapping, then
        // memcpy into the client SHM buffer.
        let region = smithay::utils::Rectangle::new(
            smithay::utils::Point::<i32, smithay::utils::Buffer>::from((0, 0)),
            buf_size,
        );
        let mapping = match renderer.copy_framebuffer(
            &target,
            region,
            drm_fourcc::DrmFourcc::Xrgb8888,
        ) {
            Ok(m) => m,
            Err(e) => {
                warn!("image_copy_capture: copy_framebuffer failed: {e:?}");
                frame.fail(CaptureFailureReason::Unknown);
                continue;
            }
        };
        drop(target);
        let pixels = match renderer.map_texture(&mapping) {
            Ok(p) => p,
            Err(e) => {
                warn!("image_copy_capture: map_texture failed: {e:?}");
                frame.fail(CaptureFailureReason::Unknown);
                continue;
            }
        };

        // Write into the client's wl_buffer (SHM only — DMA-BUF
        // is Step 2.1).
        let buffer = frame.buffer();
        let need = (buf_size.w as usize)
            .saturating_mul(4)
            .saturating_mul(buf_size.h as usize);
        let copy_result = smithay::wayland::shm::with_buffer_contents_mut(
            &buffer,
            |dst_ptr, dst_len, _meta| {
                let n = need.min(dst_len).min(pixels.len());
                // SAFETY: dst_ptr/dst_len come from a validated
                // wl_shm wl_buffer; we never read more than n
                // bytes from `pixels` (whose length is bounded by
                // dst_len above). Both regions are non-overlapping
                // (CPU map vs renderer mapping).
                unsafe {
                    std::ptr::copy_nonoverlapping(pixels.as_ptr(), dst_ptr, n);
                }
                n > 0
            },
        );
        match copy_result {
            Ok(true) => {
                // Success — present the frame with the current
                // monotonic time. damage = None means "everything
                // changed" which is the right answer for a fresh
                // capture.
                frame.success(
                    Transform::Normal,
                    None,
                    monotonic_now(),
                );
            }
            Ok(false) | Err(_) => {
                frame.fail(CaptureFailureReason::BufferConstraints);
            }
        }
    }
}

/// Drain `MargoState::pending_cast_redraws` and render each pending
/// cast frame into its PipeWire dmabuf. The third leg of the
/// screencast story (alongside the live display path and
/// `drain_image_copy_frames`).
///
/// PipeWire callbacks (`on_param_changed`, frame ready) push a
/// `CastStreamId` onto the pending list via `MargoState::on_pw_msg`'s
/// `Redraw` arm. Each entry resolves to one `Cast` whose target
/// (Output / Window / Nothing) selects which subset of the scene
/// gets rendered into the cast's queued PipeWire buffer.
///
/// Direct port of niri's `redraw_cast` (Window arm) and
/// `render_for_screen_cast` / `render_windows_for_screen_cast`
/// fused into a single drain pass — margo doesn't have niri's
/// per-output redraw scheduler, so we run all pending casts on
/// every repaint tick they were enqueued for.
fn drain_pending_cast_frames(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    state: &mut MargoState,
) {
    use crate::screencasting::pw_utils::{CastSizeChange, CursorData};
    use crate::screencasting::CastTarget;
    use smithay::utils::Size;

    // Drain the PipeWire-driven request list — we don't actually
    // route off it (we render every active cast on every repaint),
    // but draining keeps it from growing unbounded.
    state.pending_cast_redraws.clear();

    // Take the casts out so we can mutate each cast while still
    // reading from `state.clients` / `state.monitors`. niri uses
    // the same `mem::take` trick — Vec layout means re-inserting
    // unchanged is essentially free.
    let mut casts = match state.screencasting.as_mut() {
        Some(s) => std::mem::take(&mut s.casts),
        None => return,
    };

    let mut to_stop = Vec::new();
    let now = crate::utils::get_monotonic_time();

    // niri's output render path iterates every active cast each
    // frame. We do the same — PipeWire's `dequeue_available_buffer`
    // returns None when the consumer hasn't returned a buffer yet,
    // so frame production self-throttles to whatever the WebRTC
    // consumer can chew through.
    for cast in casts.iter_mut() {
        if !cast.is_active() {
            continue;
        }

        // Clone the target up front so we drop the borrow on `cast`
        // while we read state.* — then re-borrow `cast` for the
        // actual render call below.
        let target = cast.target.clone();
        match target {
            CastTarget::Nothing => {
                if cast.dequeue_buffer_and_clear(renderer) {
                    cast.last_frame_time = now;
                }
            }
            CastTarget::Window { id } => {
                let client = state
                    .clients
                    .iter()
                    .find(|c| std::ptr::addr_of!(**c) as u64 == id);
                let Some(client) = client else { continue };
                let geom = client.geom;
                if geom.width <= 0 || geom.height <= 0 {
                    continue;
                }
                let size = Size::<i32, Physical>::from((geom.width, geom.height));
                match cast.ensure_size(size) {
                    Ok(CastSizeChange::Ready) => (),
                    Ok(CastSizeChange::Pending) => continue,
                    Err(err) => {
                        warn!("cast ensure_size: {err:?}");
                        to_stop.push(cast.session_id);
                        continue;
                    }
                }
                let scale = Scale::from(1.0_f64);
                // Window's surface tree at (0,0) — the cast buffer
                // *is* the window, so the window's top-left = the
                // buffer's origin.
                let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    AsRenderElements::<GlesRenderer>::render_elements(
                        &client.window,
                        renderer,
                        Point::from((0, 0)),
                        scale,
                        1.0,
                    );
                let cursor_data =
                    CursorData::compute(&elements, 0, Point::default(), scale);
                if cast.dequeue_buffer_and_render(
                    renderer,
                    &elements,
                    &cursor_data,
                    size,
                    scale,
                ) {
                    cast.last_frame_time = now;
                }
            }
            CastTarget::Output { name, .. } => {
                let Some((_, od)) = outputs
                    .iter()
                    .find(|(_, od)| od.output.name() == name)
                else {
                    continue;
                };
                let Some(mode) = od.output.current_mode() else {
                    continue;
                };
                let size = mode.size;
                if size.w <= 0 || size.h <= 0 {
                    continue;
                }
                let scale =
                    Scale::from(od.output.current_scale().fractional_scale());

                match cast.ensure_size(size) {
                    Ok(CastSizeChange::Ready) => (),
                    Ok(CastSizeChange::Pending) => continue,
                    Err(err) => {
                        warn!("cast ensure_size: {err:?}");
                        to_stop.push(cast.session_id);
                        continue;
                    }
                }

                let mon_idx =
                    state.monitors.iter().position(|m| m.name == name);
                let Some(mon_idx) = mon_idx else { continue };
                let mon = &state.monitors[mon_idx];
                let mon_loc_x = mon.monitor_area.x;
                let mon_loc_y = mon.monitor_area.y;
                let visible_tags = mon.current_tagset();

                // Render each visible client on this monitor at its
                // monitor-local position. Surface elements only —
                // borders/shadows live in the display path's
                // `MargoRenderElement` enum, not in the cast feed
                // (the cast type alias is
                // `WaylandSurfaceRenderElement<R>`).
                let mut elements: Vec<
                    WaylandSurfaceRenderElement<GlesRenderer>,
                > = Vec::new();
                for client in &state.clients {
                    if client.monitor != mon_idx {
                        continue;
                    }
                    if (client.tags & visible_tags) == 0 {
                        continue;
                    }
                    if client.is_minimized {
                        continue;
                    }
                    let pos = Point::<i32, Physical>::from((
                        client.geom.x - mon_loc_x,
                        client.geom.y - mon_loc_y,
                    ));
                    let elems = AsRenderElements::<GlesRenderer>::render_elements::<
                        WaylandSurfaceRenderElement<GlesRenderer>,
                    >(
                        &client.window, renderer, pos, scale, 1.0
                    );
                    elements.extend(elems);
                }
                let cursor_data =
                    CursorData::compute(&elements, 0, Point::default(), scale);
                if cast.dequeue_buffer_and_render(
                    renderer,
                    &elements,
                    &cursor_data,
                    size,
                    scale,
                ) {
                    cast.last_frame_time = now;
                }
            }
        }
    }

    let any_active = casts.iter().any(|c| c.is_active());
    if let Some(s) = state.screencasting.as_mut() {
        s.casts = casts;
    }
    for id in to_stop {
        state.stop_cast(id);
    }
    // Keep the repaint chain ticking while a cast is active.
    // Without this, after the first frame the repaint scheduler
    // goes idle (no input/animation = no dirty), and the cast
    // freezes. We re-arm via request_repaint(); the VBlank handler
    // will re-fire the ping when the queued frame lands, giving us
    // ~refresh-rate cast frames.
    if any_active {
        state.request_repaint();
    }
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

                // Drop shadow under floating windows when
                // `Config::shadows` is on, the client doesn't have
                // `no_shadow:1` from a windowrule, and the global
                // `shadow_only_floating` policy lets it through.
                // Shadow goes BENEATH the surface (later in
                // `elements` Vec = lower scene layer) so the window
                // bites into its own shadow naturally. Skipped on
                // fullscreen / overlay / tagged scratchpad clients
                // where a shadow would just bleed past edges that
                // are supposed to feel locked to the screen.
                if let Some(client) = client {
                    if state.config.shadows
                        && !client.no_shadow
                        && !client.is_fullscreen
                        && !client.is_in_scratchpad
                        && (client.is_floating || !state.config.shadow_only_floating)
                    {
                        if let Some(program) = crate::render::shadow::shader(renderer) {
                            let win_rect = smithay::utils::Rectangle::new(
                                (
                                    client.geom.x - output_geo.loc.x,
                                    client.geom.y - output_geo.loc.y,
                                )
                                    .into(),
                                (
                                    client.geom.width.max(1),
                                    client.geom.height.max(1),
                                )
                                    .into(),
                            );
                            let shadow = crate::render::shadow::ShadowRenderElement::new(
                                smithay::backend::renderer::element::Id::new(),
                                win_rect,
                                state.config.border_radius.max(0) as f32,
                                state.config.shadows_size as f32,
                                state.config.shadows_blur,
                                (
                                    state.config.shadows_position_x,
                                    state.config.shadows_position_y,
                                ),
                                state.config.shadowscolor.0,
                                scale,
                                program.0,
                            );
                            elements.push(MargoRenderElement::Shadow(shadow));
                        }
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

/// Build an `OutputPresentationFeedback` collecting every surface that
/// contributed a render element to the frame just queued. The result
/// holds per-surface feedback callbacks ready for `.presented(...)`,
/// but we deliberately do **not** call `.presented()` here — the page
/// flip hasn't actually landed yet, only been queued. Storing the
/// builder on the `OutputDevice` and signalling at `DrmEvent::VBlank`
/// time gives clients a timestamp that matches the real scan-out
/// moment, not "queue + ~half-a-frame-of-latency".
///
/// Mirrors anvil's `take_presentation_feedback` shape; the difference
/// is this returns the builder instead of consuming it.
fn build_presentation_feedback(
    output: &Output,
    state: &mut MargoState,
    render_states: &smithay::backend::renderer::element::RenderElementStates,
) -> smithay::desktop::utils::OutputPresentationFeedback {
    use smithay::desktop::layer_map_for_output;
    use smithay::desktop::utils::{
        surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
        OutputPresentationFeedback,
    };

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

    feedback
}

/// Convert an `Output`'s current mode refresh rate to a per-frame
/// `Duration`. Falls back to 60 Hz when the mode isn't known yet
/// (winit nested mode, hotplug-in-progress, kernel reporting 0 mHz).
fn output_refresh_duration(output: &Output) -> std::time::Duration {
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

/// Signal `presented(now, refresh, 0, Vsync)` on a feedback builder
/// previously stashed on the OutputDevice. Called from the
/// `DrmEvent::VBlank` handler, so `now` reflects the actual
/// page-flip moment — not the submit time, which is the cheap
/// approximation we used to do.
///
/// `seq` is left at 0 because smithay 0.7's `DrmEvent::VBlank(crtc)`
/// doesn't carry the kernel's page-flip sequence number; pulling it
/// from the kernel directly needs bypassing smithay's compositor and
/// is tracked as a future iteration. Clients that care about seq
/// see this as "no monotonic seq available"; clients that care
/// about the timestamp (the common case — kitty / mpv frame pacing)
/// now get a meaningful one.
fn flush_presentation_feedback(
    output: &Output,
    feedback: smithay::desktop::utils::OutputPresentationFeedback,
) {
    use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;

    let mut feedback = feedback;
    let now = monotonic_now();
    let refresh = output_refresh_duration(output);
    feedback.presented::<_, smithay::utils::Monotonic>(
        now,
        smithay::wayland::presentation::Refresh::fixed(refresh),
        0,
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
                    // wp_presentation: collect every surface's
                    // feedback callback now while we still have the
                    // RenderElementStates, but defer the actual
                    // `presented(...)` signal until the matching
                    // `DrmEvent::VBlank` fires. The flip hasn't
                    // actually landed yet — calling presented() here
                    // would publish a timestamp that's a frame-or-so
                    // earlier than reality. The VBlank handler picks
                    // this up and signals with a clock value taken
                    // at the real page-flip moment.
                    let feedback = build_presentation_feedback(&od.output, state, &result.states);
                    od.pending_presentation.push(feedback);
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

/// Drain `state.pending_output_mode_changes` and apply each via
/// `DrmCompositor::use_mode`, then update the smithay `Output` so
/// wl_output mode events reach clients (kanshi, status bar).
///
/// This runs at the top of the repaint handler (before rendering)
/// so a kanshi profile flip lands within one frame instead of
/// being delayed to the next event.
///
/// Failure modes — each just skips the entry with a warning:
///   * Output name not in `state.monitors` → output went away.
///   * Connector info read fails → DRM in a weird state, retry next frame.
///   * No DRM mode matches the (w, h, refresh) triple → kanshi
///     asked for a mode the panel doesn't actually advertise.
///   * `compositor.use_mode` fails → atomic test failed (the kernel
///     refused the modeset, e.g. CRTC pixel-clock limit).
fn apply_pending_mode_changes(bd: &mut BackendData, state: &mut MargoState) {
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
        // active. delete_mode/add_mode handles the case where the
        // new mode wasn't in the previously-advertised list (rare
        // but possible if the connector probe race with kanshi).
        let new_wl_mode = OutputMode::from(drm_mode);
        od.output.change_current_state(
            Some(new_wl_mode),
            None,
            None,
            None,
        );
        // Prefer the new mode for the next preferred-mode query
        // too, so a later `wlr-randr` without a `--mode` argument
        // sticks with what the user just picked.
        od.output.set_preferred(new_wl_mode);

        tracing::info!(
            "output_management: applied mode {}x{}@{}.{:03}Hz on {}",
            change.width,
            change.height,
            change.refresh_mhz / 1000,
            change.refresh_mhz % 1000,
            change.output_name,
        );

        // Logical work area (in compositor coords) follows the new
        // mode size — the layout reflow already fired in
        // apply_output_pending, but we run it once more after the
        // real DRM size is known so client-side widgets see the
        // correct geometry from the very first post-modeset frame.
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
    // Prefer KMS PREFERRED mode if multiple match.
    candidates.sort_by_key(|m| {
        if m.mode_type().contains(ModeTypeFlags::PREFERRED) {
            0
        } else {
            1
        }
    });
    Some(*candidates[0])
}

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
