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
            compositor::DrmCompositor,
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
            property, Device as DrmDeviceTrait, connector, crtc,
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

mod frame;
mod helpers;
mod hotplug;
mod mode;
use frame::{flush_presentation_feedback, render_all_outputs};
use helpers::{find_crtc, monotonic_now, smithay_transform};
use hotplug::rescan_outputs;
use mode::{apply_pending_mode_changes, select_drm_mode};

render_elements! {
    pub MargoRenderElement<=GlesRenderer>;
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

pub struct OutputDevice {
    pub output: Output,
    pub(super) compositor: GbmDrmCompositor,
    pub(super) render_count: u64,
    pub(super) queued_count: u64,
    pub(super) empty_count: u64,
    pub(super) queue_error_count: u64,
    /// Per-CRTC GAMMA_LUT property handles, populated when the connector is
    /// bound. `None` if the kernel/driver doesn't expose GAMMA_LUT (in which
    /// case sunsetr / gammastep silently skip the output).
    pub(super) gamma: Option<GammaProps>,
    /// Connector handle this CRTC is driving. Needed during hotplug so we
    /// can re-check whether the *specific* connector for this output is
    /// still connected — the previous code asked "is anything still
    /// connected on this card?" which gave wrong answers in multi-monitor
    /// setups.
    pub(super) connector: connector::Handle,
    /// `wp_presentation_feedback` builders that have been collected at
    /// submit time but not yet signalled. We hold them here until the
    /// matching `DrmEvent::VBlank` fires, then call `.presented(now,
    /// refresh, seq, Vsync)` at that point — the timestamp is the
    /// actual page-flip moment instead of "when we queued the flip",
    /// which is off by one frame interval. Stored as a Vec because in
    /// pathological cases (smithay reports back-to-back VBlanks)
    /// we'd otherwise drop feedbacks; in practice it's almost always
    /// 0 or 1 entry.
    pub(super) pending_presentation: Vec<smithay::desktop::utils::OutputPresentationFeedback>,
    /// Monotonically-increasing per-output VBlank sequence number,
    /// incremented every time `DrmEvent::VBlank(crtc)` fires for
    /// this output's CRTC. Published as the `seq` field of every
    /// `wp_presentation_feedback.presented` event. The protocol asks
    /// for "an implementation-defined monotonic counter" — the kernel
    /// `drm_event_vblank.sequence` would also satisfy this contract
    /// but smithay 0.7's `DrmEvent::VBlank(crtc)` doesn't surface it,
    /// and a per-output counter is observably equivalent for the
    /// frame-pacing-sensitive consumers (mpv `--vo=gpu-next`,
    /// kitty's render loop, gnome-shell's `getRefreshRate` polling).
    /// Increments at the head of the VBlank handler so the
    /// presentation-feedback flush sees the post-flip value.
    pub(super) vblank_seq: u64,
}

// ── DRM gamma properties ──────────────────────────────────────────────────────
//
// Adapted from niri's `src/backend/tty.rs` GammaProps.

pub(super) struct GammaProps {
    crtc: crtc::Handle,
    gamma_lut: property::Handle,
    gamma_lut_size: property::Handle,
    /// Currently-active LUT blob id, so we can free it when replacing.
    previous_blob: Option<NonZeroU64>,
}

impl GammaProps {
    pub(super) fn discover(device: &DrmDevice, crtc: crtc::Handle) -> Option<Self> {
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

    pub(super) fn gamma_size(&self, device: &DrmDevice) -> Option<u32> {
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
    pub(super) fn set_gamma(&mut self, device: &DrmDevice, gamma: Option<&[u16]>) -> Result<()> {
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

pub(super) struct BackendData {
    pub(super) renderer: GlesRenderer,
    pub(super) outputs: HashMap<crtc::Handle, OutputDevice>,
    /// DRM device shared by all outputs on this card. Used for late-binding
    /// operations (gamma LUT updates, output power management) that need to
    /// poke properties outside the per-CRTC `DrmCompositor`.
    pub(super) drm: DrmDevice,
    /// Allocator + framebuffer-exporter dependencies needed to construct
    /// new `DrmCompositor`s on hotplug. Captured once at startup; everything
    /// here is cheap to clone.
    pub(super) gbm: GbmDevice<DrmDeviceFd>,
    pub(super) primary_node: DrmNode,
    pub(super) renderer_formats: smithay::backend::allocator::format::FormatSet,
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
            gamma_size: 0, // backfilled by GAMMA_LUT discovery below
            focus_history: std::collections::VecDeque::new(),
        });
        state.apply_tag_rules_to_monitor(state.monitors.len() - 1);
        // Hotplug-in (initial-setup path): refresh the shared
        // ipc_outputs snapshot so DisplayConfig + ScreenCast see
        // the new monitor immediately.
        state.refresh_ipc_outputs();

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
                vblank_seq: 0,
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
                    let mut to_flush: Vec<(Output, smithay::desktop::utils::OutputPresentationFeedback, u64)> = Vec::new();
                    {
                        let mut bd = backend_data.borrow_mut();
                        if let Some(od) = bd.outputs.get_mut(&crtc) {
                            // Bump the per-output VBlank counter
                            // BEFORE building the feedback tuples so
                            // every drained feedback sees the
                            // post-flip seq the protocol promises.
                            od.vblank_seq = od.vblank_seq.wrapping_add(1);
                            // Without this, queue_frame for the next
                            // frame will fail and the render loop
                            // stalls.
                            if let Err(e) = od.compositor.frame_submitted() {
                                warn!("frame_submitted: {e:?}");
                            }
                            let seq = od.vblank_seq;
                            for feedback in od.pending_presentation.drain(..) {
                                to_flush.push((od.output.clone(), feedback, seq));
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
                    for (output, feedback, seq) in to_flush {
                        flush_presentation_feedback(&output, feedback, seq);
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
                // every repaint. niri's output render path tags
                // casts with `check_time_and_schedule` for inter-
                // frame pacing; until that lands here, we lean on
                // PipeWire's own `dequeue_available_buffer` to drop
                // frames when the consumer hasn't returned a buffer
                // yet — buffer-bounded backpressure.
                #[cfg(feature = "xdp-gnome-screencast")]
                {
                    let has_casts = state
                        .screencasting
                        .as_ref()
                        .is_some_and(|s| !s.casts.is_empty());
                    if has_casts {
                        let mut bd = backend_data.borrow_mut();
                        let BackendData { renderer, outputs, .. } = &mut *bd;
                        drain_active_cast_frames(renderer, outputs, state);
                    }
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
    // Seed the runtime state file so `mctl clients` / `mctl outputs`
    // work from the very first frame. Without this, the file only
    // shows up after the first `arrange_all` (typically a tag toggle
    // or window mapping), which means a stale-margo session has no
    // way to query its own state.
    state.write_state_file();

    info!("udev backend ready ({} outputs)", state.monitors.len());
    Ok(())
}

// ── Hotplug rescan ──────────────────────────────────────────────────────────
//
// Implementation extracted to `hotplug.rs` (W4.1).

// ── Per-frame render ──────────────────────────────────────────────────────────

/// Drain `snapshot_pending` flags by capturing the live surface tree
/// of each affected client into a `GlesTexture`, stored in
/// `client.resize_snapshot`. Called once per frame before the render
/// element collection runs, so the rest of the frame can read the
/// snapshot from `state.clients` immutably.
pub(super) fn take_pending_snapshots(
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
pub(super) fn take_pending_open_close_captures(
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

/// What kind of frame the caller is building. Replaces the previous
/// `(include_cursor: bool, for_screencast: bool)` two-bool parameter
/// pair on [`build_render_elements_inner`] — same data, but the
/// callsites read as intent (`RenderTarget::Display`) instead of
/// (`true, false`).
#[derive(Debug, Clone, Copy)]
pub(super) enum RenderTarget {
    /// Live display path: cursor sprite drawn, no screencast blackout filter.
    Display,
    /// Display path with cursor suppressed. Used by callers that
    /// composite the cursor separately (region-selector overlay).
    DisplayNoCursor,
    /// Screencast / screencopy capture: `block_out_from_screencast`
    /// clients are substituted with solid black; cursor inclusion is
    /// driven by the capture client's request (`overlay_cursor` /
    /// metadata-mode cursor sidecar).
    Screencast { include_cursor: bool },
}

impl RenderTarget {
    fn flags(self) -> (bool, bool) {
        // (include_cursor, for_screencast)
        match self {
            RenderTarget::Display => (true, false),
            RenderTarget::DisplayNoCursor => (false, false),
            RenderTarget::Screencast { include_cursor } => (include_cursor, true),
        }
    }
}

pub(super) fn build_render_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
) -> Vec<MargoRenderElement> {
    build_render_elements_inner(renderer, od, state, RenderTarget::Display)
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
                let elements = build_render_elements_inner(
                    renderer,
                    od,
                    state,
                    RenderTarget::Screencast { include_cursor: false },
                );
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

/// Render every active screencast into its queued PipeWire dmabuf.
/// The third leg of the screencast story (alongside the live
/// display path and `drain_image_copy_frames`).
///
/// Each `Cast` carries a target (Output / Window / Nothing) that
/// selects which subset of the scene gets rendered into the cast's
/// PipeWire buffer. Casts ride the live render — we reuse
/// `build_render_elements_inner` to produce a `Vec<MargoRenderElement>`
/// with full decorations (border, shadow, clipped surface, open /
/// close / resize animations, solid block-out, cursor) and feed
/// that list straight into the cast pipeline.
///
/// Three optimisations layered on top:
///
///   1. **Pacing**: `Cast::check_time_and_schedule` skips a cast
///      this tick if `now < last_frame_time + min_time_between_frames`
///      and re-arms a timer-driven redraw at the proper interval.
///      Saves ~50% of GLES element-build work for static scenes.
///   2. **Damage**: `Cast::dequeue_buffer_and_render` already runs
///      a per-cast `OutputDamageTracker` and short-circuits the
///      whole render+queue path when no element changed. Static
///      scenes produce zero PipeWire buffers ⇒ encoder bandwidth
///      drops to keyframe-only.
///   3. **Cursor**: `include_cursor = true` on the
///      `build_render_elements_inner` call — the live cursor is
///      part of the element list. For window casts the cursor is
///      relocated along with the rest of the output via
///      `CastRenderElement::Relocated`.
#[cfg(feature = "xdp-gnome-screencast")]
fn drain_active_cast_frames(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    state: &mut MargoState,
) {
    use crate::screencasting::pw_utils::{CastSizeChange, CursorData};
    use crate::screencasting::{CastRenderElement, CastTarget};
    use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
    use smithay::utils::Size;

    // Take the casts out so we can mutate each cast while still
    // reading from `state.clients` / `state.monitors` / `outputs`.
    // niri uses the same `mem::take` trick — Vec layout means
    // re-inserting unchanged is essentially free.
    let mut casts = match state.screencasting.as_mut() {
        Some(s) => std::mem::take(&mut s.casts),
        None => return,
    };

    let mut to_stop = Vec::new();
    let now = crate::utils::get_monotonic_time();

    for cast in casts.iter_mut() {
        if !cast.is_active() {
            continue;
        }

        // Clone the target up front so we drop the borrow on
        // `cast` while we read state.* — then re-borrow `cast`
        // mutably for the render call.
        let target = cast.target.clone();
        match target {
            CastTarget::Nothing => {
                if cast.dequeue_buffer_and_clear(renderer) {
                    cast.last_frame_time = now;
                }
            }
            CastTarget::Window { id } => {
                let Some(client_idx) = state
                    .clients
                    .iter()
                    .position(|c| std::ptr::addr_of!(*c) as u64 == id)
                else {
                    continue;
                };
                let client = &state.clients[client_idx];
                let geom = client.geom;
                if geom.width <= 0 || geom.height <= 0 {
                    continue;
                }
                let mon_idx = client.monitor;
                let Some(mon) = state.monitors.get(mon_idx) else {
                    continue;
                };
                let scale_f = mon.output.current_scale().fractional_scale();
                let scale = Scale::from(scale_f);

                // Cast buffer = window-sized in physical pixels.
                // Margo client.geom is in logical-output coordinates;
                // multiply by the monitor's fractional scale for the
                // physical extent the cast buffer needs.
                let size = Size::<i32, Physical>::from((
                    (geom.width as f64 * scale_f).round() as i32,
                    (geom.height as f64 * scale_f).round() as i32,
                ));
                if size.w <= 0 || size.h <= 0 {
                    continue;
                }

                if cast.check_time_and_schedule(&mon.output, now) {
                    continue;
                }

                match cast.ensure_size(size) {
                    Ok(CastSizeChange::Ready) => (),
                    Ok(CastSizeChange::Pending) => continue,
                    Err(err) => {
                        warn!("cast ensure_size: {err:?}");
                        to_stop.push(cast.session_id);
                        continue;
                    }
                }

                let Some((_, od)) = outputs
                    .iter()
                    .find(|(_, od)| od.output == mon.output)
                else {
                    continue;
                };

                // Build the FULL output element list (decorations,
                // cursor, block-out, popups, animations) and shift
                // each element so the target window's top-left
                // lands at (0, 0) in the cast buffer.
                //
                // Relocate offset: cast wants the window at origin,
                // so we translate by -(window_pos_relative_to_output).
                // Margo's client.geom is logical (matches output_geo
                // origin); convert to physical with the output scale.
                let win_off_x =
                    -((geom.x - mon.monitor_area.x) as f64 * scale_f).round() as i32;
                let win_off_y =
                    -((geom.y - mon.monitor_area.y) as f64 * scale_f).round() as i32;
                let win_off = Point::<i32, Physical>::from((win_off_x, win_off_y));

                let cursor_mode = cast.cursor_mode();
                let include_cursor = matches!(
                    cursor_mode,
                    crate::dbus::mutter_screen_cast::CursorMode::Embedded
                );
                let want_metadata_cursor = matches!(
                    cursor_mode,
                    crate::dbus::mutter_screen_cast::CursorMode::Metadata
                );

                let output_elems = build_render_elements_inner(
                    renderer,
                    od,
                    state,
                    RenderTarget::Screencast { include_cursor },
                );
                // Pointer-only sidecar elements for Metadata mode.
                // Same shape as Embedded but lifted out of `elements`
                // so pw_utils strips them from the main damage pass
                // and renders them into the spa cursor bitmap.
                let (cursor_elems_vec, cursor_loc) = if want_metadata_cursor {
                    let (e, loc) = build_cursor_elements_for_output(renderer, od, state);
                    let v: Vec<CastRenderElement> = e
                        .into_iter()
                        .map(|e| {
                            CastRenderElement::Relocated(
                                RelocateRenderElement::from_element(
                                    e,
                                    win_off,
                                    Relocate::Relative,
                                ),
                            )
                        })
                        .collect();
                    (v, loc)
                } else {
                    (Vec::new(), Point::default())
                };
                let cursor_count = cursor_elems_vec.len();

                let main_elems: Vec<CastRenderElement> = output_elems
                    .into_iter()
                    .map(|e| {
                        CastRenderElement::Relocated(
                            RelocateRenderElement::from_element(
                                e,
                                win_off,
                                Relocate::Relative,
                            ),
                        )
                    })
                    .collect();
                // Pointer elements come FIRST so CursorData::compute
                // grabs them via `&elements[..elem_count]`.
                let mut elements: Vec<CastRenderElement> =
                    Vec::with_capacity(cursor_count + main_elems.len());
                elements.extend(cursor_elems_vec);
                elements.extend(main_elems);

                let cursor_data: CursorData<CastRenderElement> =
                    CursorData::compute(&elements, cursor_count, cursor_loc, scale);
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
                let output = od.output.clone();

                if cast.check_time_and_schedule(&output, now) {
                    continue;
                }

                match cast.ensure_size(size) {
                    Ok(CastSizeChange::Ready) => (),
                    Ok(CastSizeChange::Pending) => continue,
                    Err(err) => {
                        warn!("cast ensure_size: {err:?}");
                        to_stop.push(cast.session_id);
                        continue;
                    }
                }

                let cursor_mode = cast.cursor_mode();
                let include_cursor = matches!(
                    cursor_mode,
                    crate::dbus::mutter_screen_cast::CursorMode::Embedded
                );
                let want_metadata_cursor = matches!(
                    cursor_mode,
                    crate::dbus::mutter_screen_cast::CursorMode::Metadata
                );

                let output_elems = build_render_elements_inner(
                    renderer,
                    od,
                    state,
                    RenderTarget::Screencast { include_cursor },
                );
                let (cursor_elems_vec, cursor_loc) = if want_metadata_cursor {
                    let (e, loc) = build_cursor_elements_for_output(renderer, od, state);
                    let v: Vec<CastRenderElement> =
                        e.into_iter().map(CastRenderElement::Direct).collect();
                    (v, loc)
                } else {
                    (Vec::new(), Point::default())
                };
                let cursor_count = cursor_elems_vec.len();

                let main_elems: Vec<CastRenderElement> = output_elems
                    .into_iter()
                    .map(CastRenderElement::Direct)
                    .collect();
                let mut elements: Vec<CastRenderElement> =
                    Vec::with_capacity(cursor_count + main_elems.len());
                elements.extend(cursor_elems_vec);
                elements.extend(main_elems);

                let cursor_data: CursorData<CastRenderElement> =
                    CursorData::compute(&elements, cursor_count, cursor_loc, scale);
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
    // freezes. The pacing layer (`check_time_and_schedule`) above
    // ensures we don't burn frames on static scenes — that runs
    // before render and bails early when too soon.
    if any_active {
        state.request_repaint();
    }
}

/// Build just the cursor sprite render elements for a given output
/// without any of the surrounding scene (no clients, no layers, no
/// borders). Used by the screencast Metadata cursor path: xdp-gnome's
/// CursorMode::Metadata sends the cursor as a sidecar bitmap to the
/// PipeWire consumer rather than embedding it in the frame, so the
/// consumer can composite the cursor sharply at low cast resolutions.
/// We need the same elements the embedded path would produce, but
/// extracted from the main scene so `CursorData::compute` can wrap
/// them, `add_cursor_metadata` can render them to a side bitmap,
/// and the main render runs without them.
///
/// Returns `(elements, cursor_logical_loc)`. Empty vec when the
/// pointer is off this output, hidden, or a non-renderable image.
pub fn build_cursor_elements_for_output(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
) -> (Vec<MargoRenderElement>, Point<f64, Logical>) {
    let output_scale = od.output.current_scale().fractional_scale();
    let Some(output_geo) = state.space.output_geometry(&od.output) else {
        return (Vec::new(), Point::default());
    };
    let ptr_global = Point::<f64, _>::from((state.input_pointer.x, state.input_pointer.y));
    if !output_geo.to_f64().contains(ptr_global) {
        return (Vec::new(), Point::default());
    }
    let ptr_pos = ptr_global - output_geo.loc.to_f64();
    let mut elements = Vec::new();
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
    (elements, ptr_pos)
}

/// Like `build_render_elements`, but optionally omits the cursor sprite
/// and/or substitutes blocked-out (`block_out_from_screencast = 1`) clients
/// with solid black rectangles. The cursor flag is honoured by every
/// caller (display render passes `true`, screencopy with `overlay_cursor`
/// off passes `false`); the screencast flag is set ONLY by
/// `serve_screencopies` so the regular display render still shows
/// password managers / private-browsing tabs / 2FA codes intact while
/// any wlr-screencopy client recording the output sees them blacked out.
pub(super) fn build_render_elements_inner(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    target: RenderTarget,
) -> Vec<MargoRenderElement> {
    let _span = tracy_client::span!("build_render_elements");
    let (include_cursor, for_screencast) = target.flags();
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
    // Exclusive fullscreen suppresses every layer-shell surface on the
    // affected output — the focused window literally covers the
    // panel, bar pixels included. WorkArea fullscreen leaves the bar
    // visible and merely sizes the window to `work_area`.
    let suppress_layers = state
        .monitors
        .iter()
        .position(|m| m.output == od.output)
        .map(|mon_idx| state.monitor_has_exclusive_fullscreen(mon_idx))
        .unwrap_or(false);
    let upper_layers: Vec<_> = if suppress_layers {
        Vec::new()
    } else {
        layer_map
            .layers()
            .rev()
            .filter(|surface| surface.layer() == WlrLayer::Overlay)
            .chain(
                layer_map
                    .layers()
                    .rev()
                    .filter(|surface| surface.layer() == WlrLayer::Top),
            )
            .collect()
    };
    let lower_layers: Vec<_> = if suppress_layers {
        Vec::new()
    } else {
        layer_map
            .layers()
            .rev()
            .filter(|surface| surface.layer() == WlrLayer::Bottom)
            .chain(
                layer_map
                    .layers()
                    .rev()
                    .filter(|surface| surface.layer() == WlrLayer::Background),
            )
            .collect()
    };
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
pub(super) fn serve_screencopies(
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
            RenderTarget::Screencast {
                include_cursor: screencopy.overlay_cursor(),
            },
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

// ── CRTC helper ───────────────────────────────────────────────────────────────

