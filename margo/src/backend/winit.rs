#![allow(dead_code)]
//! Winit backend — for nested Wayland/X11 testing (no real hardware needed).

use anyhow::Result;
use smithay::{
    backend::{
        egl::EGLDevice,
        renderer::{
            damage::OutputDamageTracker,
            element::{
                render_elements,
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                Kind,
            },
            gles::GlesRenderer,
            ImportDma, ImportEgl,
        },
        winit::{self, WinitEvent},
    },
    desktop::{space::render_output, Window},
    input::pointer::CursorImageStatus,
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Transform, Physical, Point},
    wayland::dmabuf::DmabufFeedbackBuilder,
};

render_elements! {
    /// Composite type for the extra (non-Space) render elements drawn on
    /// top of the winit space — currently the cursor and the rounded
    /// borders. Using `render_elements!` instead of two separate slices
    /// keeps `render_output<_, CE, _, _>`'s single-CE constraint happy.
    WinitExtra<=GlesRenderer>;
    WaylandSurface=WaylandSurfaceRenderElement<GlesRenderer>,
    Border=crate::render::rounded_border::RoundedBorderElement,
}
use tracing::{error, info, warn};

use crate::state::MargoState;

const REFRESH_HZ: u32 = 60;

pub fn run(
    state: &mut MargoState,
    event_loop: &mut EventLoop<'static, MargoState>,
) -> Result<()> {
    let (mut backend, winit_evt) = winit::init::<GlesRenderer>()
        .map_err(|e| anyhow::anyhow!("winit init failed: {e}"))?;

    let dmabuf_formats = backend.renderer().dmabuf_formats();
    match EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .and_then(|device| device.try_get_render_node())
    {
        Ok(Some(node)) => {
            match DmabufFeedbackBuilder::new(node.dev_id(), dmabuf_formats.clone()).build() {
                Ok(feedback) => {
                    let global = state
                        .dmabuf_state
                        .create_global_with_default_feedback::<MargoState>(
                            &state.display_handle,
                            &feedback,
                        );
                    state.dmabuf_global = Some(global);
                    info!("nested linux-dmabuf v5 enabled with default feedback");
                }
                Err(err) => {
                    warn!("failed to build nested dmabuf feedback, falling back to v3: {err:?}");
                    let global = state
                        .dmabuf_state
                        .create_global::<MargoState>(&state.display_handle, dmabuf_formats);
                    state.dmabuf_global = Some(global);
                }
            }
        }
        Ok(None) => {
            warn!("failed to query nested render node, falling back to linux-dmabuf v3");
            let global = state
                .dmabuf_state
                .create_global::<MargoState>(&state.display_handle, dmabuf_formats);
            state.dmabuf_global = Some(global);
        }
        Err(err) => {
            warn!("failed to query nested EGL device, falling back to linux-dmabuf v3: {err:?}");
            let global = state
                .dmabuf_state
                .create_global::<MargoState>(&state.display_handle, dmabuf_formats);
            state.dmabuf_global = Some(global);
        }
    }

    match backend.renderer().bind_wl_display(&state.display_handle) {
        Ok(()) => info!("nested EGL Wayland hardware-acceleration enabled"),
        Err(err) => warn!("failed to bind nested EGL Wayland display: {err:?}"),
    }

    let mode = OutputMode {
        size: backend.window_size(),
        refresh: REFRESH_HZ as i32 * 1000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "winit".into(),
            serial_number: "Unknown".into(),
        },
    );
    let _global = output.create_global::<MargoState>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    let monitor_area = crate::layout::Rect {
        x: 0,
        y: 0,
        width: mode.size.w,
        height: mode.size.h,
    };
    let pertag = crate::layout::Pertag::new(
        state.default_layout(),
        state.config.default_mfact,
        state.config.default_nmaster,
    );
    state.monitors.push(crate::state::MargoMonitor {
        name: "winit".to_string(),
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
            focus_history: std::collections::VecDeque::new(), // winit/nested: no DRM gamma plane
    });
    state.apply_tag_rules_to_monitor(state.monitors.len() - 1);

    let mut output_size = mode.size;
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut force_full_redraw = 2usize;

    // Kick off the first redraw; subsequent redraws are requested at the end of each frame
    backend.window().request_redraw();

    // Winit event source
    event_loop
        .handle()
        .insert_source(winit_evt, move |event, _, state: &mut MargoState| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    if size.w <= 0 || size.h <= 0 {
                        return;
                    }
                    output_size = size;
                    force_full_redraw = 2;
                    let new_mode = OutputMode { size, refresh: REFRESH_HZ as i32 * 1000 };
                    output.change_current_state(Some(new_mode), None, None, None);
                    output.set_preferred(new_mode);
                    state.space.map_output(&output, (0, 0));
                    // Update monitor geometry to match new window size
                    let new_area = crate::layout::Rect {
                        x: 0, y: 0, width: size.w, height: size.h,
                    };
                    if let Some(mon) = state.monitors.iter_mut().find(|m| m.output == output) {
                        mon.monitor_area = new_area;
                        mon.work_area = new_area;
                    }
                    info!("winit resized to {}x{}", size.w, size.h);
                    state.arrange_all();
                }
                WinitEvent::Input(input_event) => {
                    crate::input_handler::handle_input(state, input_event);
                }
                WinitEvent::CloseRequested => {
                    info!("window close requested, quitting");
                    state.should_quit = true;
                }
                WinitEvent::Redraw => {
                    // Clamp pointer to output bounds
                    let (out_w, out_h) = (output_size.w.max(1) as f64, output_size.h.max(1) as f64);
                    state.input_pointer.x = state.input_pointer.x.clamp(0.0, out_w - 1.0);
                    state.input_pointer.y = state.input_pointer.y.clamp(0.0, out_h - 1.0);

                    // In nested (winit) mode: show OS cursor when no client sets its own
                    let show_os_cursor = !matches!(state.cursor_status, CursorImageStatus::Surface(_) | CursorImageStatus::Hidden);
                    backend.window().set_cursor_visible(show_os_cursor);

                    // Snapshot cursor surface for rendering
                    let cursor_surface = match &state.cursor_status {
                        CursorImageStatus::Surface(s) => Some(s.clone()),
                        _ => None,
                    };
                    let ptr_pos = (state.input_pointer.x as i32, state.input_pointer.y as i32);

                    let space = &state.space;
                    let age = if force_full_redraw > 0 {
                        force_full_redraw -= 1;
                        0
                    } else {
                        backend.buffer_age().unwrap_or(0)
                    };
                    let damage = {
                        let (renderer, mut fb) = match backend.bind() {
                            Ok(x) => x,
                            Err(e) => {
                                warn!("bind failed: {e}");
                                return;
                            }
                        };

                        if let Some((_, lock_surface)) = state.lock_surfaces.iter().find(|(o, _)| o == &output) {
                            let mut extras: Vec<WinitExtra> = Vec::new();
                            let cursor_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                                cursor_surface
                                    .as_ref()
                                    .map(|s| render_elements_from_surface_tree(
                                        renderer, s, ptr_pos, 1.0, 1.0, Kind::Cursor,
                                    ))
                                    .unwrap_or_default();
                            for e in cursor_elements { extras.push(WinitExtra::WaylandSurface(e)); }

                            let lock_elements = render_elements_from_surface_tree(
                                renderer, lock_surface.wl_surface(), Point::<i32, Physical>::from((0, 0)), 1.0, 1.0, Kind::Unspecified,
                            );
                            for e in lock_elements { extras.push(WinitExtra::WaylandSurface(e)); }
                            let _ = render_output::<GlesRenderer, WinitExtra, Window, _>(
                                &output, renderer, &mut fb, 1.0, age, &[], &extras, &mut damage_tracker, [0.0, 0.0, 0.0, 1.0],
                            );
                            return;
                        }

                        let cursor_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            cursor_surface
                                .as_ref()
                                .map(|s| render_elements_from_surface_tree(
                                    renderer, s, ptr_pos, 1.0, 1.0, Kind::Cursor,
                                ))
                                .unwrap_or_default();

                        // Borders render via custom GLES SDF shader. Compile
                        // once on first frame; if compile fails (driver
                        // limitation) silently fall back to no borders.
                        let border_elements: Vec<_> =
                            match crate::render::rounded_border::shader(renderer) { Some(prog) => {
                                crate::border::render_elements(
                                    state,
                                    smithay::utils::Point::from((0, 0)),
                                    smithay::utils::Scale::from(1.0_f64),
                                    prog.0,
                                )
                            } _ => {
                                Vec::new()
                            }};

                        let mut extras: Vec<WinitExtra> =
                            Vec::with_capacity(cursor_elements.len() + border_elements.len());
                        for e in cursor_elements {
                            extras.push(WinitExtra::WaylandSurface(e));
                        }
                        for e in border_elements {
                            extras.push(WinitExtra::Border(e));
                        }

                        match render_output::<GlesRenderer, WinitExtra, Window, _>(
                            &output,
                            renderer,
                            &mut fb,
                            1.0,
                            age,
                            std::slice::from_ref(space),
                            &extras,
                            &mut damage_tracker,
                            [0.1, 0.1, 0.1, 1.0],
                        ) {
                            Ok(result) => result.damage,
                            Err(e) => {
                                error!("render: {e}");
                                None
                            }
                        }
                    };
                    match backend.submit(damage.map(|d| d.as_slice())) {
                        Ok(()) => {
                            // winit has no hardware vblank — treat the
                            // post-submit moment as the cycle boundary.
                            // Bump sequence + send frame callbacks here,
                            // mirroring what `note_vblank` does on udev.
                            let entry = state
                                .frame_callback_sequence
                                .entry(output.name())
                                .or_insert(0);
                            *entry = entry.wrapping_add(1);
                            state.send_frame_callbacks(&output, state.clock.now());
                            state.post_repaint(&output, state.clock.now());
                            state.display_handle.flush_clients().ok();
                        }
                        Err(e) => warn!("submit failed: {e}"),
                    }
                    backend.window().request_redraw();
                }
                WinitEvent::Focus(_) => {}
            }
        })
        .map_err(|e| anyhow::anyhow!("winit source insert: {e}"))?;

    info!("winit backend ready ({}x{})", mode.size.w, mode.size.h);
    Ok(())
}
