#![allow(dead_code)]
//! udev/DRM/KMS backend — real hardware via libseat + libinput + GBM/EGL.

use std::{
    cell::RefCell,
    collections::HashMap,
    os::unix::io::{FromRawFd, IntoRawFd},
    rc::Rc,
    sync::Mutex,
};

use anyhow::{Context, Result};
use drm_fourcc::DrmFourcc;
use smithay::{
    backend::{
        allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmEventTime, DrmNode, NodeType,
            compositor::DrmCompositor, exporter::gbm::GbmFramebufferExporter,
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
        drm::control::{Device as DrmDeviceTrait, connector, crtc},
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

mod capture;
mod frame;
mod gamma;
mod helpers;
mod hotplug;
mod lease;
mod mirror_element;
mod mode;
mod render_elements;
mod repaint;
mod session;
// Render-element builders live in `render_elements` now; a plain `use` pulls
// them into the udev-module namespace so `run` / `serve_screencopies` (here)
// and the sibling `frame` module (`use super::build_render_elements`, a child
// reaching an ancestor's private item) resolve unchanged.
pub(super) use capture::{
    serve_screencopies, take_mru_thumbnails, take_pending_open_close_captures,
    take_pending_snapshots,
};
use frame::{
    apply_pending_gamma, flush_presentation_feedback, render_all_outputs, render_due_outputs,
};
pub(super) use gamma::GammaProps;
use helpers::{connector_is_non_desktop, find_crtc, monotonic_now, smithay_transform};
use hotplug::rescan_outputs;
use mode::{apply_pending_mode_changes, select_drm_mode};
#[cfg(feature = "xdp-gnome-screencast")]
use render_elements::drain_active_cast_frames;
use render_elements::{build_render_elements, capture_scene, drain_image_copy_frames};
use repaint::repaint_tick;

render_elements! {
    pub MargoRenderElement<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    // Cursor images and text labels (memory buffers).
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
    // Compositor wallpaper and the overview backdrop image (one uploaded
    // texture, see `crate::wallpaper`). Pushed to the *bottom* of the
    // element vec, so it z-orders behind everything else.
    Wallpaper=smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
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
    // A mirror target's frame: the source output's scene, each element
    // scaled + letterboxed (see `crate::mirror`).
    Mirrored=mirror_element::MirroredElement,
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
    /// Length of this output's last display element list: the next
    /// frame's capacity, so the Vec of large render-element enums is not
    /// regrown on every frame (a per-client estimate under-counts popups,
    /// subsurfaces and decorations).
    pub(super) last_element_count: std::cell::Cell<usize>,
    pub(super) compositor: GbmDrmCompositor,
    pub(super) render_count: u64,
    pub(super) queued_count: u64,
    pub(super) empty_count: u64,
    pub(super) queue_error_count: u64,
    /// `render_frame` returned `Err` (distinct from `queue_error_count`, which
    /// counts a *successful* render whose page-flip submit failed). Both are a
    /// no-present, but a render error needs its own clock recovery — see the
    /// `Err` arm in `frame.rs`.
    pub(super) render_error_count: u64,
    /// Consecutive queue/render failures since the last empty or queued frame.
    /// Drives bounded exponential retry backoff; lifetime telemetry remains in
    /// the two counters above.
    pub(super) render_failure_streak: u32,
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
    /// `output_pending.dpms (output, true)` re-renders + re-enables it.
    pub(super) dpms_off: bool,
    /// A page flip was queued and its vblank hasn't arrived yet. Gamma
    /// changes wait for it (see `apply_pending_gamma`).
    pub(super) flip_in_flight: bool,
    /// `wp_presentation_feedback` builders that have been collected at
    /// submit time but not yet signalled. We hold them here until the
    /// matching `DrmEvent::VBlank` fires, then call `.presented(time,
    /// refresh, seq, Vsync)` with the event's kernel page-flip timestamp
    /// instead of "when we queued the flip", which is off by one frame
    /// interval. Stored as a Vec because in
    /// pathological cases (smithay reports back-to-back VBlanks)
    /// we'd otherwise drop feedbacks; in practice it's almost always
    /// 0 or 1 entry.
    pub(super) pending_presentation: Vec<smithay::desktop::utils::OutputPresentationFeedback>,
    /// Monotonically-increasing per-output VBlank sequence number,
    /// advanced every time `DrmEvent::VBlank(crtc)` fires for this
    /// output's CRTC. Published as the `seq` field of every
    /// `wp_presentation_feedback.presented` event. Follows the kernel's
    /// `drm_event_vblank.sequence` from the event metadata, so clients
    /// can count skipped vblanks; steps by one when the metadata is
    /// missing or the counter would go backwards (see
    /// [`crate::frame_clock::next_vblank_seq`]). Advanced at the head of
    /// the VBlank handler so the presentation-feedback flush sees the
    /// post-flip value.
    pub(super) vblank_seq: u64,
}

pub(crate) struct BackendData {
    pub(super) renderer: GlesRenderer,
    pub(super) outputs: HashMap<crtc::Handle, OutputDevice>,
    /// Non-desktop connectors (VR headsets etc.). Never driven as
    /// outputs; offered to clients through `wp_drm_lease_device_v1`.
    pub(super) non_desktop_connectors: Vec<connector::Handle>,
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
    /// libseat session handle, kept for `chvt` (Ctrl+Alt+F<n>) VT switches.
    pub(super) session: LibSeatSession,
    /// Per-output clock: the mirror target skipped a source frame because
    /// its own flip was in flight, and owes it. Its vblank wakes a repaint
    /// that draws it (see `frame::render_due_outputs`).
    pub(super) mirror_frame_pending: bool,
}

impl BackendData {
    /// Ask logind/seatd to switch to virtual terminal `vt`. The session
    /// then pauses and resumes through the normal libseat event path.
    pub(crate) fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            warn!("chvt {vt} failed: {e}");
        }
    }
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
        // SAFETY: `into_raw_fd` hands over sole ownership of a freshly opened, valid DRM fd.
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
    // SAFETY: `gbm` wraps the open DRM device and is kept alive by the display it is moved into.
    let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }.context("EGLDisplay::new")?;
    let egl_context = EGLContext::new(&egl_display).context("EGLContext::new")?;
    // SAFETY: the EGL context was created on this thread for this renderer and is not shared.
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
                .protocols
                .dmabuf
                .create_global_with_default_feedback::<MargoState>(
                    &state.display_handle,
                    &feedback,
                );
            state.protocols.dmabuf_global = Some(global);
            info!("linux-dmabuf v5 enabled with default feedback");
        }
        Err(err) => {
            warn!("failed to build dmabuf feedback, falling back to v3: {err:?}");
            let global = state
                .protocols
                .dmabuf
                .create_global::<MargoState>(&state.display_handle, dmabuf_formats);
            state.protocols.dmabuf_global = Some(global);
        }
    }

    // ── 4b. linux-drm-syncobj-v1 (explicit sync) — opt-in `syncobj_enable` ──
    //
    // Off by default. An older smithay only shipped the protocol handler
    // without fence integration; advertising the global then made GTK4's
    // Vulkan renderer (mshell-frame) attach release points nobody
    // signalled, Mesa WSI dropped margo's late wl_buffer.release events
    // and the bar flickered (16537081). Current smithay signals release
    // points when the last Buffer reference drops, and the pre-commit
    // hook (`state/handlers/compositor.rs`) now waits on acquire points,
    // so it can be turned on — but it stays opt-in until verified on
    // real hardware. Read once at start-up (the global can't be added or
    // withdrawn on reload), and only when the kernel supports syncobj
    // eventfds.
    if state.config.syncobj_enable {
        if smithay::wayland::drm_syncobj::supports_syncobj_eventfd(&drm_fd) {
            state.protocols.drm_syncobj =
                Some(smithay::wayland::drm_syncobj::DrmSyncobjState::new::<
                    MargoState,
                >(&state.display_handle, drm_fd.clone()));
            info!("linux-drm-syncobj-v1 advertised (syncobj_enable = 1)");
        } else {
            warn!("syncobj_enable = 1 but the DRM device lacks syncobj eventfd support");
        }
    } else {
        info!("linux-drm-syncobj-v1 not advertised (syncobj_enable = 0)");
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
    let mut used_crtcs: std::collections::HashSet<crtc::Handle> = std::collections::HashSet::new();

    let mut backend_outputs: HashMap<crtc::Handle, OutputDevice> = HashMap::new();
    let mut non_desktop_connectors: Vec<connector::Handle> = Vec::new();

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

        // Non-desktop connectors (VR headsets) are never driven as
        // outputs — they get offered to clients via DRM lease below.
        if connector_is_non_desktop(&drm_fd, *conn_handle) {
            info!(
                "connector {}-{} is non-desktop; offering for DRM lease",
                conn_info.interface().as_str(),
                conn_info.interface_id()
            );
            non_desktop_connectors.push(*conn_handle);
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
        let mut wl_mode = OutputMode::from(drm_mode);

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
        state.advertise_output(&output);

        output.change_current_state(
            Some(wl_mode),
            Some(transform),
            Some(smithay::output::Scale::Fractional(scale as f64)),
            Some(position.into()),
        );
        mode::advertise_connector_modes(&output, conn_info.modes(), wl_mode);
        state.space.map_output(&output, position);

        // Create DRM surface for this output
        let created = drm
            .create_surface(crtc, drm_mode, &[*conn_handle])
            .or_else(|e| {
                // A `custom:1` mode the driver won't take: fall back to the
                // advertised mode the rule would otherwise pick.
                let Some(fallback) = mode::fallback_for_custom(&conn_info, rule.as_ref()) else {
                    return Err(e);
                };
                warn!("custom mode for {output_name} rejected ({e}); using an advertised mode");
                let surface = drm.create_surface(crtc, fallback, &[*conn_handle])?;
                wl_mode = OutputMode::from(fallback);
                output.change_current_state(Some(wl_mode), None, None, None);
                Ok(surface)
            });
        let drm_surface = match created {
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
        // Layout-space rect: the final mode (after any custom-mode fallback)
        // with the rule's transform and scale applied, as the Space sees it.
        let monitor_area = crate::state::logical_output_rect(&output, position.0, position.1);
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
            minimap_visible: false,
            enabled: true,
            gamma_size: 0, // backfilled by GAMMA_LUT discovery below
            focus_history: std::collections::VecDeque::new(),
            visible_clients: 0,
            visible_tiled: 0,
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
                last_element_count: Default::default(),
                output,
                compositor,
                render_count: 0,
                queued_count: 0,
                empty_count: 0,
                queue_error_count: 0,
                render_error_count: 0,
                render_failure_streak: 0,
                gamma: gamma_props,
                connector: *conn_handle,
                dpms_off: false,
                flip_in_flight: false,
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
        non_desktop_connectors,
        drm,
        gbm: gbm.clone(),
        primary_node,
        renderer_formats: renderer_formats.clone(),
        session: session.clone(),
        mirror_frame_pending: false,
    }));

    // ── wp_drm_lease_device_v1 ────────────────────────────────────────────────
    // The synchronous `lease_request` path reaches the live DrmDevice
    // through this Rc handle — the "small backend-access change" from
    // docs/protocol-comparison.md. Non-desktop connectors found above
    // are offered; lease CRTC/plane selection happens at request time.
    state.udev_backend = Some(backend_data.clone());
    match smithay::wayland::drm_lease::DrmLeaseState::new::<MargoState>(
        &state.display_handle,
        &primary_node,
    ) {
        Ok(mut lease_state) => {
            let bd = backend_data.borrow();
            for conn in &bd.non_desktop_connectors {
                if let Ok(info) = bd.drm.get_connector(*conn, false) {
                    let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
                    lease_state.add_connector::<MargoState>(
                        *conn,
                        name,
                        "non-desktop connector".to_string(),
                    );
                }
            }
            drop(bd);
            state.protocols.drm_lease = Some(lease_state);
        }
        Err(e) => warn!("wp_drm_lease_device_v1 unavailable: {e}"),
    }
    // `apply_pending_mode_changes` drains wlr-output-management mode
    // changes on this backend.
    state.output_pending.mode_changes_supported = true;
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
            move |event, meta, state: &mut MargoState| match event {
                DrmEvent::VBlank(crtc) => {
                    // The kernel's page-flip timestamp and vblank counter.
                    // The timestamp is only usable when the device reports
                    // CLOCK_MONOTONIC (the clock wp_presentation advertises);
                    // otherwise fall back to "now" in the handler.
                    let flip_monotonic = match meta.as_ref().map(|m| m.time) {
                        Some(DrmEventTime::Monotonic(t)) => Some(t),
                        _ => None,
                    };
                    let kernel_seq = meta.as_ref().map(|m| m.sequence);
                    let now_monotonic = monotonic_now();
                    let presented_monotonic = flip_monotonic.unwrap_or(now_monotonic);
                    let presented_at = flip_monotonic.map(|t| {
                        crate::frame_clock::instant_at_monotonic(
                            t,
                            now_monotonic,
                            std::time::Instant::now(),
                        )
                    });
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
                    let mut gamma_waiting = false;
                    let mut mirror_owed = false;
                    {
                        let mut bd = backend_data.borrow_mut();
                        let mirror_pending = bd.mirror_frame_pending;
                        if let Some(od) = bd.outputs.get_mut(&crtc) {
                            // Advance the per-output VBlank counter
                            // BEFORE building the feedback tuples so
                            // every drained feedback sees the
                            // post-flip seq the protocol promises.
                            od.vblank_seq =
                                crate::frame_clock::next_vblank_seq(od.vblank_seq, kernel_seq);
                            // Without this, queue_frame for the next
                            // frame will fail and the render loop
                            // stalls.
                            if let Err(e) = od.compositor.frame_submitted() {
                                warn!("frame_submitted: {e:?}");
                            }
                            od.flip_in_flight = false;
                            // A gamma ramp parked behind this flip can go
                            // in now.
                            gamma_waiting = state
                                .output_pending
                                .gamma
                                .iter()
                                .any(|(o, _)| *o == od.output);
                            // The mirror target skipped a frame while this
                            // flip was in flight: draw it now.
                            mirror_owed = mirror_pending && state.mirror.is_target(&od.output_name);
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
                    // frame we queued earlier, stamped with the
                    // kernel's flip time (not the submit moment, nor
                    // when this handler got to run). In steady state
                    // there's exactly one entry; the Vec covers the
                    // rare case of back-to-back VBlanks queued before
                    // we drained the previous one.
                    for (output, feedback, seq) in to_flush {
                        flush_presentation_feedback(&output, feedback, presented_monotonic, seq);
                    }
                    // Drop the in-flight count and, if the scene is still
                    // dirty (animation, deferred input, late commit), let
                    // the redraw scheduler ping itself for the next frame.
                    // Also bumps the per-output frame_callback_sequence
                    // + sends frame callbacks — see `state::note_vblank`.
                    if gamma_waiting || mirror_owed {
                        state.wake_repaint_backend();
                    }
                    if let Some(out) = flipped_output {
                        // Select by the gate that queued this specific frame,
                        // not by the current config value: config reload may
                        // flip the frame-clock mode while a page-flip is in
                        // flight.
                        state.note_backend_vblank(&out, presented_at);
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
            move |(), _, state: &mut MargoState| repaint_tick(&backend_data, state)
        })
        .map_err(|e| anyhow::anyhow!("repaint ping source: {e}"))?;
    // A firing present timer runs the same body directly (one wake-up per
    // frame less than pinging this source).
    state.set_repaint_hook(Rc::new({
        let backend_data = backend_data.clone();
        move |state: &mut MargoState| repaint_tick(&backend_data, state)
    }));
    state.set_repaint_ping(repaint_ping);
    // Initial ping to drain the dirty flag we set in MargoState::new (the
    // first frame must run; no other event has fired yet).
    state.request_repaint();

    // ── 8. Session event source ───────────────────────────────────────────────
    // (libinput will be inserted into state below; the closure reads it from state)
    event_loop
        .handle()
        .insert_source(session_notifier, {
            let backend_data = backend_data.clone();
            move |event, _, state: &mut MargoState| match event {
                SessionEvent::PauseSession => {
                    info!("session paused");
                    if let Some(li) = state.libinput.as_mut() {
                        li.suspend();
                    }
                    if let Some(ls) = state.protocols.drm_lease.as_mut() {
                        ls.suspend();
                    }
                    state.libinput_devices.clear();
                    // Release DRM master and stop committing: repaint_tick
                    // and the hotplug rescan skip KMS until the resume.
                    session::pause_drm(&mut backend_data.borrow_mut().drm);
                    // Nothing renders until the resume: answer captures
                    // now instead of after the switch back.
                    state.protocols.screencopy.set_session_paused(true);
                }
                SessionEvent::ActivateSession => {
                    info!("session activated");
                    state.protocols.screencopy.set_session_paused(false);
                    {
                        let mut bd = backend_data.borrow_mut();
                        session::activate_drm(&mut bd.drm);
                        // Another DRM master may have reprogrammed the CRTCs
                        // while we were away: make every compositor re-read
                        // the hardware state and render its next frame in
                        // full (niri does the same). Failures from the paused
                        // stretch must not leave a retry backoff behind.
                        for od in bd.outputs.values_mut() {
                            if let Err(err) = od.compositor.reset_state() {
                                warn!(output = %od.output_name, "session resume: DrmCompositor reset failed: {err:?}");
                            }
                            od.compositor.reset_buffers();
                            od.render_failure_streak = 0;
                            state.clear_output_render_retry(&od.output);
                        }
                    }
                    // Monitors plugged or unplugged while paused: the udev
                    // rescan skipped them (no KMS access), catch up now.
                    rescan_outputs(&backend_data, state);
                    if let Some(li) = state.libinput.as_mut() {
                        if li.resume().is_err() {
                            warn!("libinput resume failed");
                        }
                    }
                    if let Some(ls) = state.protocols.drm_lease.as_mut() {
                        ls.resume::<MargoState>();
                    }
                    state.arrange_all();
                    // Guaranteed DPMS recovery: a VT-switch back ALWAYS wakes
                    // every panel, so a stuck DPMS-off can never survive a
                    // Ctrl+Alt+F-key round-trip. Harmless when nothing is off.
                    state.request_dpms(Some(true), None);
                    // Another DRM master may have changed the LUTs while we
                    // were away, and a flip queued before the switch never
                    // got its vblank: re-commit every output's last ramp
                    // (unless a newer one is already queued).
                    let mut bd = backend_data.borrow_mut();
                    for od in bd.outputs.values_mut() {
                        od.flip_in_flight = false;
                        let Some(ramp) = od.gamma.as_mut().and_then(GammaProps::forget_applied)
                        else {
                            continue;
                        };
                        if !state
                            .output_pending
                            .gamma
                            .iter()
                            .any(|(o, _)| *o == od.output)
                        {
                            state.output_pending.gamma.push((od.output.clone(), ramp));
                        }
                    }
                    drop(bd);
                    state.wake_repaint_backend();
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
    // ActivateSession does a full re-enumeration via resume(), and pause the
    // DRM device the same way.
    if !session.is_active() {
        info!("session not active at startup, suspending libinput and DRM");
        libinput.suspend();
        // Same as a PauseSession: no KMS commits until ActivateSession, which
        // re-takes master and resets every output's state.
        session::pause_drm(&mut backend_data.borrow_mut().drm);
        state.protocols.screencopy.set_session_paused(true);
    }

    state.libinput = Some(libinput.clone());

    event_loop
        .handle()
        .insert_source(
            LibinputInputBackend::new(libinput),
            |mut event, _, state: &mut MargoState| {
                match &mut event {
                    InputEvent::DeviceAdded { device } => state.libinput_device_added(device),
                    InputEvent::DeviceRemoved { device } => state.libinput_device_removed(device),
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
                    state.output_pending.hotplug_last_event_at = Some(std::time::Instant::now());
                    if state.output_pending.hotplug_rescan_pending {
                        // Burst still in flight, existing timer will
                        // pick up the freshly-updated timestamp.
                        return;
                    }
                    state.output_pending.hotplug_rescan_pending = true;
                    let timer = calloop::timer::Timer::from_duration(
                        std::time::Duration::from_millis(50),
                    );
                    let backend_data_for_timer = backend_data.clone();
                    if let Err(e) = loop_handle_for_udev.insert_source(
                        timer,
                        move |_, _, state: &mut MargoState| {
                            let now = std::time::Instant::now();
                            let stale = state
                                .output_pending.hotplug_last_event_at
                                .map(|t| {
                                    now.duration_since(t)
                                        >= std::time::Duration::from_millis(50)
                                })
                                .unwrap_or(true);
                            if stale {
                                info!("udev hotplug burst settled, rescanning outputs");
                                rescan_outputs(&backend_data_for_timer, state);
                                state.output_pending.hotplug_rescan_pending = false;
                                state.output_pending.hotplug_last_event_at = None;
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
                        state.output_pending.hotplug_rescan_pending = false;
                        state.output_pending.hotplug_last_event_at = None;
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
            mirror_frame_pending,
            ..
        } = &mut *bd;
        render_all_outputs(
            renderer,
            outputs,
            drm,
            state,
            mirror_frame_pending,
            "initial",
        );
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
