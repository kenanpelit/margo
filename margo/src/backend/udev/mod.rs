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
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType, compositor::DrmCompositor,
            exporter::gbm::GbmFramebufferExporter,
        },
        egl::{EGLContext, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma, ImportEgl,
            element::{
                AsRenderElements, Kind, Wrap,
                memory::MemoryRenderBufferRenderElement,
                render_elements,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::{GlesRenderer, GlesTexProgram},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, primary_gpu},
    },
    desktop::{PopupManager, WindowSurface, layer_map_for_output, space::SpaceRenderElements},
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{EventLoop, ping::make_ping},
        drm::control::{Device as DrmDeviceTrait, connector, crtc, property},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, Logical, Physical, Point, Rectangle, Scale, Transform},
    wayland::shell::wlr_layer::Layer as WlrLayer,
    wayland::{compositor::with_states, dmabuf::DmabufFeedbackBuilder, seat::WaylandFocus},
};
use tracing::{debug, error, info, warn};

use crate::{
    input_handler::handle_input,
    state::{MargoClient, MargoState},
};

mod frame;
mod helpers;
mod hotplug;
mod mode;
mod render_elements;
// Render-element builders live in `render_elements` now; a plain `use` pulls
// them into the udev-module namespace so `run` / `serve_screencopies` (here)
// and the sibling `frame` module (`use super::build_render_elements`, a child
// reaching an ancestor's private item) resolve unchanged.
use frame::{flush_presentation_feedback, render_all_outputs, render_due_outputs};
use helpers::{find_crtc, monotonic_now, smithay_transform};
use hotplug::rescan_outputs;
use mode::{apply_pending_mode_changes, select_drm_mode};
use render_elements::{
    RenderTarget, build_render_elements, build_render_elements_inner, drain_active_cast_frames,
    drain_image_copy_frames,
};

render_elements! {
    pub MargoRenderElement<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    // Wallpaper elements ride this same variant — both Cursor and
    // Wallpaper use `MemoryRenderBufferRenderElement<GlesRenderer>`,
    // and `render_elements!` rejects two variants of the same
    // underlying type (conflicting `From` impls). The wallpaper
    // element is pushed to the *bottom* of the element vec by the
    // render loop, so it z-orders behind windows / layers / shadows
    // / cursor regardless of sharing the variant.
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
    WaylandSurface=WaylandSurfaceRenderElement<GlesRenderer>,
    Border=crate::render::rounded_border::RoundedBorderElement,
    Shadow=crate::render::shadow::ShadowRenderElement,
    Blur=crate::render::blur::BlurRenderElement,
    Clipped=crate::render::clipped_surface::ClippedSurfaceRenderElement,
    Resize=crate::render::resize_render::ResizeRenderElement,
    OpenClose=crate::render::open_close::OpenCloseRenderElement,
    Solid=smithay::backend::renderer::element::solid::SolidColorRenderElement,
    RoundedSolid=crate::render::rounded_solid::RoundedSolidElement,
    // Scroller-overview thumbnail: a window surface scaled down (Rescale)
    // and placed into its tag's cell (Relocate). See
    // `build_scroller_overview_elements`.
    // Scroller-overview cell content (windows + wallpaper), scaled into a
    // cell and given a per-cell namespaced Id so a tag repeated by the
    // wrap-around loop doesn't collide in the damage tracker. See
    // `crate::render::namespaced`.
    NamespacedSurface=smithay::backend::renderer::element::utils::RelocateRenderElement<smithay::backend::renderer::element::utils::RescaleRenderElement<crate::render::namespaced::NamespacedElement<WaylandSurfaceRenderElement<GlesRenderer>>>>,
}

// ── Type aliases ──────────────────────────────────────────────────────────────

type GbmDrmCompositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

pub struct OutputDevice {
    pub output: Output,
    /// The output's connector name (e.g. `eDP-1`), cached at creation.
    /// `Output::name()` locks the output's inner mutex and clones a fresh
    /// `String` on every call, so the per-frame render path keys
    /// `perf_counters` / `frame_callback_sequence` off this stable copy
    /// instead — no allocation on the steady-state hot path.
    pub(super) output_name: String,
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
    /// `true` while this output is DPMS-off (panel powered down via
    /// `DrmCompositor::clear()`). The render loop skips it until woken; a
    /// `pending_dpms (output, true)` re-renders + re-enables it.
    pub(super) dpms_off: bool,
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
            let Ok(info) = device.get_property(prop) else {
                continue;
            };
            let Ok(name) = info.name().to_str() else {
                continue;
            };
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

pub fn run(state: &mut MargoState, event_loop: &mut EventLoop<'static, MargoState>) -> Result<()> {
    // ── 1. Open libseat session ───────────────────────────────────────────────
    let (mut session, session_notifier) =
        LibSeatSession::new().map_err(|e| anyhow::anyhow!("libseat session failed: {e}"))?;
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
        info!(
            "DRM hardware cursor plane: {}×{} (advertised by driver)",
            cs.w, cs.h
        );
    }

    // ── 4. GBM + EGL + GLES ──────────────────────────────────────────────────
    let gbm = GbmDevice::new(drm_fd.clone()).context("GbmDevice::new")?;
    let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }.context("EGLDisplay::new")?;
    let egl_context = EGLContext::new(&egl_display).context("EGLContext::new")?;
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }.context("GlesRenderer::new")?;

    match renderer.bind_wl_display(&state.display_handle) {
        Ok(()) => info!("EGL Wayland hardware-acceleration enabled (legacy wl_drm binding)"),
        // `EGL_WL_bind_wayland_display` is the legacy wl_drm buffer-sharing
        // path. Modern Mesa drivers drop it in favour of linux-dmabuf (set up
        // just below), so its absence is expected and harmless — clients still
        // get zero-copy hardware buffers via dmabuf. Not worth a WARN.
        Err(smithay::backend::egl::Error::EglExtensionNotSupported(_)) => {
            debug!(
                "EGL wl_drm binding unavailable (EGL_WL_bind_wayland_display); using linux-dmabuf instead"
            );
        }
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

    // ── 4b. linux-drm-syncobj-v1 (explicit sync) — INTENTIONALLY NOT WIRED ──
    //
    // The previous version of this block advertised the
    // `wp_linux_drm_syncobj_manager_v1` global "to follow the
    // niri / sway / mutter contract." That comment was wrong — niri
    // does NOT advertise this global at all (verified by
    // `grep -rn syncobj` on the niri tree returning zero hits) and
    // for good reason: smithay only ships the protocol handler, not
    // the actual fence-signalling integration with the DRM
    // compositor / GBM allocator. Advertising the global without the
    // fence-signal half means clients (notably GTK4 4.20 with the
    // GSK Vulkan renderer, which mshell-frame uses) attach
    // `set_acquire_point` / `set_release_point` to every commit on
    // the assumption that the compositor will signal those fences at
    // present time — and when the compositor doesn't, Mesa Vulkan
    // WSI starts discarding the `wl_buffer.release` events margo
    // sends late, the swapchain pool drains, and the bar visibly
    // flickers.
    //
    // The user's WAYLAND_DEBUG trace confirmed this: 93/93 of margo's
    // `wl_buffer.release` events were marked `discarded` by the
    // `mesa vk display queue` (the Mesa Vulkan WSI thread), and the
    // 110 `wp_linux_drm_syncobj_surface_v1.set_release_point` calls
    // mshell made were going to a sink that never signalled them.
    // `GSK_RENDERER=gl` made the flicker disappear because the GTK
    // GL renderer falls back to implicit-fence dmabuf (no syncobj
    // dance at all). Dropping the global advertisement here forces
    // the same fallback for Vulkan, which is the path
    // niri / Hyprland have always used and which gives smooth
    // mshell on every other compositor.
    //
    // When (if ever) smithay grows full fence-signal integration in
    // its DrmCompositor + drm_syncobj_state pair, re-enable this
    // block. For now: leave `state.drm_syncobj_state` as `None` so
    // the dispatch refuses to bind and the global stays invisible.
    let _ = &drm_fd;
    info!(
        "linux-drm-syncobj-v1 intentionally NOT advertised (niri-style) — \
         clients (GTK4 Vulkan / Chromium / Firefox dmabuf) will use the \
         implicit-fence dmabuf sync path, which is the only one smithay \
         fence-signals correctly in this revision"
    );

    // ── 5. Get renderer formats for DRM compositor ───────────────────────────
    let renderer_formats = renderer
        .egl_context()
        .display()
        .dmabuf_render_formats()
        .clone();

    let color_formats = [DrmFourcc::Xrgb8888, DrmFourcc::Argb8888];

    // ── 6. Enumerate connected connectors and create outputs + compositors ───
    let resources = drm_fd.resource_handles().context("DRM resource_handles")?;
    let mut used_crtcs: std::collections::HashSet<crtc::Handle> = std::collections::HashSet::new();

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
        let rule = state
            .config
            .monitor_rules
            .iter()
            .find(|r| r.name.as_deref().map(|n| n == output_name).unwrap_or(true))
            .cloned();

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
                    acc + state
                        .space
                        .output_geometry(o)
                        .map(|g| g.size.w)
                        .unwrap_or(0)
                });
                (x_offset, 0)
            }
        } else {
            let x_offset = state.space.outputs().fold(0i32, |acc, o| {
                acc + state
                    .space
                    .output_geometry(o)
                    .map(|g| g.size.w)
                    .unwrap_or(0)
            });
            (x_offset, 0)
        };

        info!(
            "output: {} {}x{}@{} pos={:?} scale={}",
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
            if s.w == 0 || s.h == 0 {
                (64u32, 64u32).into()
            } else {
                s
            }
        };
        // Both RENDERING and SCANOUT flags so the GBM buffer can be used
        // as a direct scanout source for DRM page-flips without an
        // intermediate copy. Missing SCANOUT (margo had only RENDERING
        // for a long time) forces smithay's DrmCompositor to allocate a
        // separate scanout buffer and blit on every page-flip — that
        // extra blit races with the VBlank, which is what made
        // gtk4-layer-shell clients (mshell, noctalia) visibly flicker
        // while the same clients stayed smooth on Hyprland and niri.
        // niri uses these exact flags in `backend/tty.rs:892`.
        let allocator = GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
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
        let mut pertag = crate::layout::Pertag::new(
            state.default_layout(),
            state.config.default_mfact,
            state.config.default_nmaster,
        );
        pertag.seed_taglayouts(&state.config.taglayouts);
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
            info!(
                "output {} gamma_size = {}",
                state.monitors[mon_idx].name, gamma_size
            );
        }

        backend_outputs.insert(
            crtc,
            OutputDevice {
                output_name: output.name(),
                output,
                compositor,
                render_count: 0,
                queued_count: 0,
                empty_count: 0,
                queue_error_count: 0,
                gamma: gamma_props,
                connector: *conn_handle,
                dpms_off: false,
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
                    let mut to_flush: Vec<(
                        Output,
                        smithay::desktop::utils::OutputPresentationFeedback,
                        u64,
                    )> = Vec::new();
                    let mut flipped_output: Option<Output> = None;
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
                            flipped_output = Some(od.output.clone());
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
                    // Also bumps the per-output frame_callback_sequence
                    // + sends frame callbacks — see `state::note_vblank`.
                    if let Some(out) = flipped_output {
                        if state.per_output_frame_clock_enabled() {
                            // Opt-in path: clear this output's in-flight
                            // gate, stamp last_present, re-arm its timer.
                            state.note_vblank_per_output(&out);
                        } else {
                            state.note_vblank(&out);
                        }
                    }
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
                // DPMS changes MUST go through the all-outputs path: a
                // DPMS-off output stops producing vblanks, so its per-output
                // clock stalls and it never becomes "due" — meaning a wake
                // queued in `pending_dpms` would never drain on the per-output
                // path and the panel would stay dark. Forcing the global path
                // whenever `pending_dpms` is non-empty guarantees the queue is
                // serviced (and is the recovery path the VT-switch relies on).
                let force_all_for_dpms = !state.pending_dpms.is_empty();
                if state.per_output_frame_clock_enabled() && !force_all_for_dpms {
                    // Opt-in per-output path: render ONLY the outputs
                    // whose present timer has come due (dirty + refresh
                    // interval elapsed + not awaiting a vblank). Each
                    // due output's clock is flipped in-flight by
                    // `take_due_outputs`; its vblank re-arms the next
                    // tick. The global dirty flag is still drained so it
                    // doesn't leak into the global path on a later
                    // config reload that turns the flag back off.
                    state.take_repaint_request();
                    let due = state.take_due_outputs();
                    if !due.is_empty() {
                        let mut bd = backend_data.borrow_mut();
                        let BackendData {
                            renderer,
                            outputs,
                            drm,
                            ..
                        } = &mut *bd;
                        render_due_outputs(
                            renderer,
                            outputs,
                            drm,
                            state,
                            &due,
                            "repaint-per-output",
                        );
                    }
                } else if state.take_repaint_request() || force_all_for_dpms {
                    let mut bd = backend_data.borrow_mut();
                    let BackendData {
                        renderer,
                        outputs,
                        drm,
                        ..
                    } = &mut *bd;
                    render_all_outputs(renderer, outputs, drm, state, "repaint");
                }
                // ext-image-copy-capture: drain pending frames
                // queued by `ImageCopyCaptureHandler::frame()` and
                // render each into its client buffer. Done after
                // the live render so the renderer is warm + the
                // scene state is the same one the user just saw.
                if !state.pending_image_copy_frames.is_empty() {
                    let mut bd = backend_data.borrow_mut();
                    let BackendData {
                        renderer, outputs, ..
                    } = &mut *bd;
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
                        let BackendData {
                            renderer, outputs, ..
                        } = &mut *bd;
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
        .insert_source(
            session_notifier,
            |event, _, state: &mut MargoState| match event {
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
                    // Guaranteed DPMS recovery: a VT-switch back ALWAYS wakes
                    // every panel, so a stuck DPMS-off can never survive a
                    // Ctrl+Alt+F-key round-trip. Harmless when nothing is off.
                    state.request_dpms(Some(true), None);
                }
            },
        )
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
        .insert_source(
            LibinputInputBackend::new(libinput),
            |mut event, _, state: &mut MargoState| {
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
            },
        )
        .map_err(|e| anyhow::anyhow!("libinput source: {e}"))?;

    // ── 10. Udev hotplug source ───────────────────────────────────────────────
    let udev_backend =
        UdevBackend::new(&seat_name).map_err(|e| anyhow::anyhow!("UdevBackend::new: {e}"))?;
    let loop_handle_for_udev = event_loop.handle();
    event_loop
        .handle()
        .insert_source(udev_backend, {
            let backend_data = backend_data.clone();
            move |event, _, state: &mut MargoState| match event {
                UdevEvent::Added { device_id: _, path } => {
                    info!("udev added: {:?}", path);
                }
                UdevEvent::Changed { device_id: _ } => {
                    // 50 ms sliding-window debounce. udev fires `Changed`
                    // events in bursts — a gamma-daemon retune, a kernel
                    // property tweak, a hotplug ack chain — and the
                    // previous "rescan per event" path could spend
                    // ~50 ms in `rescan_outputs` while a hundred more
                    // events queued up behind it. We arm one timer per
                    // burst; it re-checks the last-event timestamp and
                    // either runs `rescan_outputs` once or re-arms for
                    // another 50 ms if more events arrived in the
                    // window.
                    state.hotplug_last_event_at = Some(std::time::Instant::now());
                    if state.hotplug_rescan_pending {
                        // Burst still in flight, existing timer will
                        // pick up the freshly-updated timestamp.
                        return;
                    }
                    state.hotplug_rescan_pending = true;
                    let timer = calloop::timer::Timer::from_duration(
                        std::time::Duration::from_millis(50),
                    );
                    let backend_data_for_timer = backend_data.clone();
                    if let Err(e) = loop_handle_for_udev.insert_source(
                        timer,
                        move |_, _, state: &mut MargoState| {
                            let now = std::time::Instant::now();
                            let stale = state
                                .hotplug_last_event_at
                                .map(|t| {
                                    now.duration_since(t)
                                        >= std::time::Duration::from_millis(50)
                                })
                                .unwrap_or(true);
                            if stale {
                                info!("udev hotplug burst settled, rescanning outputs");
                                rescan_outputs(&backend_data_for_timer, state);
                                state.hotplug_rescan_pending = false;
                                state.hotplug_last_event_at = None;
                                // Fire on_output_change so Rhai scripts can
                                // react to topology / mode / scale changes.
                                // Empty arg for now: the udev event only
                                // gives us a device id, not a specific
                                // connector — Rhai handlers should walk
                                // monitor_count() / output_geometry() to
                                // see what landed.
                                crate::scripting::fire_output_change(state, "");
                                calloop::timer::TimeoutAction::Drop
                            } else {
                                // Slide the window — another event arrived
                                // within the debounce, keep waiting.
                                calloop::timer::TimeoutAction::ToDuration(
                                    std::time::Duration::from_millis(50),
                                )
                            }
                        },
                    ) {
                        // Timer insertion failed (e.g. loop shutting
                        // down). Fall back to the immediate rescan so
                        // we don't drop the event entirely.
                        tracing::warn!(
                            "hotplug coalescer: timer insert failed ({e:?}); running rescan immediately"
                        );
                        rescan_outputs(&backend_data, state);
                        state.hotplug_rescan_pending = false;
                        state.hotplug_last_event_at = None;
                    }
                }
                UdevEvent::Removed { device_id: _ } => {}
            }
        })
        .map_err(|e| anyhow::anyhow!("udev source: {e}"))?;

    // ── 11. Initial render pass ───────────────────────────────────────────────
    {
        let mut bd = backend_data.borrow_mut();
        let BackendData {
            renderer,
            outputs,
            drm,
            ..
        } = &mut *bd;
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
    state.mark_state_dirty();

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
            if c.snapshot_pending
                && state
                    .monitors
                    .get(c.monitor)
                    .is_some_and(|m| m.output == od.output)
            {
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

        match crate::render::window_capture::capture_window(renderer, &window, size, output_scale) {
            Ok(texture) => {
                state.clients[idx].resize_snapshot = Some(crate::state::ResizeSnapshot {
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
                && state
                    .monitors
                    .get(c.monitor)
                    .is_some_and(|m| m.output == od.output)
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
        match crate::render::window_capture::capture_window(renderer, &window, size, output_scale) {
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
                && state
                    .monitors
                    .get(c.monitor)
                    .is_some_and(|m| m.output == od.output)
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
        match crate::render::window_capture::capture_surface(renderer, &surface, size, output_scale)
        {
            Ok(texture) => {
                state.closing_clients[idx].texture = Some(texture);
                state.closing_clients[idx].capture_pending = false;
                state.closing_clients[idx].source_surface = None;
                tracing::debug!("close_anim: captured wl_surface ({}x{})", size.w, size.h,);
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
        .filter_map(|(k, a)| {
            if a.is_close && a.capture_pending {
                Some(k.clone())
            } else {
                None
            }
        })
        .collect();
    for key in pending_layer_keys {
        let Some(anim) = state.layer_animations.get(&key) else {
            continue;
        };
        let Some(surface) = anim.source_surface.clone() else {
            continue;
        };
        let geom = anim.geom;
        let size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            geom.width.max(1),
            geom.height.max(1),
        ));
        match crate::render::window_capture::capture_surface(renderer, &surface, size, output_scale)
        {
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

/// Capture an off-screen snapshot of each MRU-switcher candidate BEFORE render,
/// so the thumbnail overlay shows a real preview of every window from the first
/// frame — including windows on other tags (a live surface element can't, which
/// is why they used to draw blank until you cycled onto them). Cheap: only runs
/// while the switcher is open (≤8 windows) and the switcher only repaints on a
/// selection change. Capturing here (not mid-element-build) keeps the renderer
/// binding sane — same call site as `take_pending_snapshots`.
pub(super) fn take_mru_thumbnails(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &mut MargoState,
) {
    let Some(cands) = state.mru_switcher.as_ref().map(|s| s.candidates.clone()) else {
        return;
    };
    let output_scale = od.output.current_scale().fractional_scale().into();
    let mut thumbs = Vec::with_capacity(cands.len());
    for win in cands {
        let size = win.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            continue;
        }
        if let Ok(tex) =
            crate::render::window_capture::capture_window(renderer, &win, size, output_scale)
        {
            thumbs.push((win, tex));
        }
    }
    if let Some(sw) = state.mru_switcher.as_mut() {
        sw.thumbs = thumbs;
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
        Bind, ExportMem, Offscreen, damage::OutputDamageTracker as DamageTracker,
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
        if let crate::protocols::screencopy::ScreencopyBuffer::Dmabuf(dmabuf) = screencopy.buffer()
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
            use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
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
                        .render_output(renderer, &mut target, 0, &relocated, [0.0, 0.0, 0.0, 1.0])
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
        let buf_size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((
            output_size.w,
            output_size.h,
        ));

        let mut renderbuffer = match <GlesRenderer as Offscreen<
            smithay::backend::renderer::gles::GlesRenderbuffer,
        >>::create_buffer(
            renderer, drm_fourcc::DrmFourcc::Xrgb8888, buf_size
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
            smithay::utils::Point::<i32, smithay::utils::Buffer>::from((
                region_loc.x,
                region_loc.y,
            )),
            smithay::utils::Size::<i32, smithay::utils::Buffer>::from((size.w, size.h)),
        );
        let mapping =
            match renderer.copy_framebuffer(&target, region, drm_fourcc::DrmFourcc::Xrgb8888) {
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
                let need = (size.w as usize)
                    .saturating_mul(4)
                    .saturating_mul(size.h as usize);
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
