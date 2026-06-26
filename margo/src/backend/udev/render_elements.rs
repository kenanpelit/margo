//! Render-element construction for the udev (DRM/GBM) backend.
//!
//! Extracted from `backend/udev/mod.rs` (god-file split): everything that turns
//! the compositor scene into a `Vec<MargoRenderElement>` for a frame — the
//! `RenderTarget` mode, the top-level `build_render_elements[_inner]`, cursor /
//! MRU-switcher / scroller-overview overlays, the per-client / per-layer push
//! helpers, and the screencast/image-copy frame drains. Pure free functions
//! over `&MargoState` + the GLES renderer; `mod.rs` keeps backend setup
//! (`run`), capture queueing, and `serve_screencopies`.

use super::*;

/// What kind of frame the caller is building. Replaces the previous
/// `(include_cursor: bool, for_screencast: bool)` two-bool parameter pair on
/// [`build_render_elements_inner`] — same data, but callsites read as intent
/// (`RenderTarget::Display`) instead of (`true, false`).
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
pub(super) fn drain_image_copy_frames(
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
                let od = match outputs.iter_mut().find(|(_, od)| od.output.name() == *name) {
                    Some((_, od)) => od,
                    None => {
                        frame.fail(CaptureFailureReason::Stopped);
                        continue;
                    }
                };
                let output_size = od.output.current_mode().map(|m| m.size).unwrap_or_default();
                if output_size.w == 0 || output_size.h == 0 {
                    frame.fail(CaptureFailureReason::Stopped);
                    continue;
                }
                let scale = od.output.current_scale().fractional_scale();
                let buf_size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((
                    output_size.w,
                    output_size.h,
                ));
                let elements = build_render_elements_inner(
                    renderer,
                    od,
                    state,
                    RenderTarget::Screencast {
                        include_cursor: false,
                    },
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
                let client = state.clients.iter().find(|c| &c.window == window);
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
                let elements: Vec<
                    smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<
                        GlesRenderer,
                    >,
                > = AsRenderElements::<GlesRenderer>::render_elements(
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
                let buf_size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((
                    geom.width,
                    geom.height,
                ));
                (buf_size, 1.0, wrapped)
            }
        };

        let elements_refs: Vec<&MargoRenderElement> = elements_owned.iter().collect();
        let output_size =
            smithay::utils::Size::<i32, smithay::utils::Physical>::from((buf_size.w, buf_size.h));

        // Allocate an offscreen renderbuffer and render the scene
        // into it. Identical shape to the SHM arm of
        // `serve_screencopies` — see that function for context.
        let mut renderbuffer = match <GlesRenderer as Offscreen<
            smithay::backend::renderer::gles::GlesRenderbuffer,
        >>::create_buffer(
            renderer, drm_fourcc::DrmFourcc::Xrgb8888, buf_size
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
        let mapping =
            match renderer.copy_framebuffer(&target, region, drm_fourcc::DrmFourcc::Xrgb8888) {
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
        let copy_result =
            smithay::wayland::shm::with_buffer_contents_mut(&buffer, |dst_ptr, dst_len, _meta| {
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
            });
        match copy_result {
            Ok(true) => {
                // Success — present the frame with the current
                // monotonic time. damage = None means "everything
                // changed" which is the right answer for a fresh
                // capture.
                frame.success(Transform::Normal, None, monotonic_now());
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
pub(super) fn drain_active_cast_frames(
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

                let Some((_, od)) = outputs.iter().find(|(_, od)| od.output == mon.output) else {
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
                let win_off_x = -((geom.x - mon.monitor_area.x) as f64 * scale_f).round() as i32;
                let win_off_y = -((geom.y - mon.monitor_area.y) as f64 * scale_f).round() as i32;
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
                            CastRenderElement::Relocated(RelocateRenderElement::from_element(
                                e,
                                win_off,
                                Relocate::Relative,
                            ))
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
                        CastRenderElement::Relocated(RelocateRenderElement::from_element(
                            e,
                            win_off,
                            Relocate::Relative,
                        ))
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
                if cast.dequeue_buffer_and_render(renderer, &elements, &cursor_data, size, scale) {
                    cast.last_frame_time = now;
                }
            }
            CastTarget::Output { name, .. } => {
                let Some((_, od)) = outputs.iter().find(|(_, od)| od.output.name() == name) else {
                    continue;
                };
                let Some(mode) = od.output.current_mode() else {
                    continue;
                };
                let size = mode.size;
                if size.w <= 0 || size.h <= 0 {
                    continue;
                }
                let scale = Scale::from(od.output.current_scale().fractional_scale());
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
                if cast.dequeue_buffer_and_render(renderer, &elements, &cursor_data, size, scale) {
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
            let ptr_i =
                (ptr_pos - hotspot.to_f64()).to_physical_precise_round::<f64, i32>(output_scale);
            let cursor_elems = render_elements_from_surface_tree(
                renderer,
                surface,
                ptr_i,
                output_scale,
                1.0f32,
                Kind::Cursor,
            );
            for e in cursor_elems {
                elements.push(MargoRenderElement::WaylandSurface(e));
            }
        }
        CursorImageStatus::Hidden => {}
        _ => {
            if let Some(cursor_elem) =
                state
                    .cursor_manager
                    .render_element(renderer, ptr_pos, output_scale)
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
                        renderer,
                        surface,
                        ptr_i,
                        output_scale,
                        1.0f32,
                        Kind::Cursor,
                    );
                    for e in cursor_elems {
                        elements.push(MargoRenderElement::WaylandSurface(e));
                    }
                }
                CursorImageStatus::Hidden => {}
                _ => {
                    if let Some(cursor_elem) =
                        state
                            .cursor_manager
                            .render_element(renderer, ptr_pos, output_scale)
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

    // Scroller overview takes over the whole output render when open:
    // a dark backdrop with each tag's windows scaled into a vertical
    // strip of cells. Replaces the normal window/layer compositing
    // (lock screen above still wins). See P2 in `state/scroller_overview.rs`.
    if state.scroller_overview.is_some() {
        return build_scroller_overview_elements(
            renderer,
            od,
            state,
            output_geo,
            output_scale,
            include_cursor,
        );
    }

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

    let mut elements: Vec<MargoRenderElement> =
        Vec::with_capacity(upper_layers.len() + lower_layers.len() + state.clients.len() * 2 + 1);

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
                    renderer,
                    surface,
                    ptr_i,
                    output_scale,
                    1.0f32,
                    Kind::Cursor,
                );
                for e in cursor_elems {
                    elements.push(MargoRenderElement::WaylandSurface(e));
                }
            }
            CursorImageStatus::Hidden => {}
            _ => {
                if let Some(cursor_elem) =
                    state
                        .cursor_manager
                        .render_element(renderer, ptr_pos, output_scale)
                {
                    elements.push(MargoRenderElement::Cursor(cursor_elem));
                }
            }
        }
    }

    // Config-error overlay — niri-style red-bordered banner pinned
    // to the top-right of every output while the deadline set by
    // `MargoState::reload_config` is in the future. Sits below the
    // cursor (which we pushed first → highest z-order) but above
    // every window and layer surface that follow. Shown for ~10 s
    // after a reload that the validator rejected; cleared by
    // `tick_animations` once the deadline passes.
    if let Some(until) = state.config_error_overlay_until {
        if std::time::Instant::now() < until {
            let origin = (output_geo.loc.x, output_geo.loc.y);
            let size = (output_geo.size.w, output_geo.size.h);
            for solid in state
                .config_error_overlay
                .render_elements(origin, size, output_scale)
            {
                elements.push(MargoRenderElement::Solid(solid));
            }
        }
    }

    // MRU window switcher overlay (Super+Tab) — above windows, below cursor.
    if state.is_mru_open() {
        for e in build_mru_switcher_elements(renderer, state, output_geo, output_scale) {
            elements.push(e);
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

/// Build the render-element list for an output while the **scroller
/// overview** is open: a niri-style grey backdrop with every tag's
/// windows scaled down live (via `RescaleRenderElement` — no window
/// resize) into a vertical strip of per-tag cells, the selected tag
/// centered. Cursor stays on top. Replaces the normal window/layer
/// compositing for this output.
/// MRU window-switcher overlay: a centred row of live, scaled-down window
/// thumbnails drawn on top of the normal desktop while the switcher is open,
/// the selected one ringed with the accent colour. Reuses the scroller
/// overview's live-surface Rescale+Relocate path (no off-screen capture), so
/// it's damage-tracked and cheap. Returned elements are appended above windows
/// but below the cursor.
fn build_mru_switcher_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
) -> Vec<MargoRenderElement> {
    use crate::render::open_close::{OpenCloseKind, OpenCloseRenderElement};
    use smithay::backend::renderer::element::Id;
    use smithay::backend::renderer::utils::CommitCounter;

    let Some(sw) = state.mru_switcher.as_ref() else {
        return Vec::new();
    };
    let scale = Scale::from(output_scale);
    let mut out: Vec<MargoRenderElement> = Vec::new();

    // Layout (logical px, output-local).
    let th = (state.config.mru_thumb_height as i32).clamp(60, 600);
    let show_labels = state.config.mru_show_labels;
    const GAP: i32 = 16;
    const PAD: i32 = 22;
    const MAX: usize = 8;
    const TITLE_H: i32 = 22;
    let label_h: i32 = if show_labels { 20 } else { 0 };

    // (window, thumb_width, app_id). Thumb width keeps the window's aspect.
    let mut cells: Vec<(smithay::desktop::Window, i32, String)> = Vec::new();
    for win in sw.candidates.iter().take(MAX) {
        let g = win.geometry().size;
        let (gw, gh) = (g.w.max(1), g.h.max(1));
        let tw = ((f64::from(gw) * f64::from(th) / f64::from(gh)).round() as i32).clamp(60, th * 2);
        let app_id = state
            .clients
            .iter()
            .find(|c| c.window == *win)
            .map(|c| c.app_id.clone())
            .unwrap_or_default();
        cells.push((win.clone(), tw, app_id));
    }
    if cells.is_empty() {
        return out;
    }

    // Row-relative left edge of each thumbnail (cumulative).
    let mut left_edges: Vec<i32> = Vec::with_capacity(cells.len());
    let mut acc = 0;
    for (_, tw, _) in &cells {
        left_edges.push(acc);
        acc += tw + GAP;
    }
    let inner_w = acc - GAP; // total row width (drop the trailing gap)
    let panel_h = TITLE_H + th + label_h + 2 * PAD;
    let oy = (output_geo.size.h - panel_h) / 2;
    // Carousel: scroll the row so the SELECTED thumbnail is centred on the
    // output. Thumbnails left/right of it slide off the edges.
    let sel = sw.selected.min(cells.len() - 1);
    let shift = output_geo.size.w / 2 - (left_edges[sel] + cells[sel].1 / 2);

    let prog = crate::render::rounded_solid::shader(renderer).map(|p| p.0);
    let radius = (14.0 * output_scale) as f32;

    // Scope title, centred at the band top (niri shows the active scope).
    let scope_txt = match sw.scope {
        crate::state::mru_switcher::MruScope::All => "All windows",
        crate::state::mru_switcher::MruScope::Output => "This output",
        crate::state::mru_switcher::MruScope::Workspace => "This workspace",
    };
    let title_pos: Point<i32, Physical> =
        Point::<i32, Logical>::from((output_geo.size.w / 2 - 80, oy + 4))
            .to_physical_precise_round(scale);
    if let Some(el) = crate::render::text::label_element(
        renderer,
        scope_txt,
        (f64::from(TITLE_H) * output_scale * 0.78) as i32,
        (320.0 * output_scale) as i32,
        [200, 200, 210],
        title_pos.to_f64(),
    ) {
        out.push(MargoRenderElement::Cursor(el));
    }

    // ── Thumbnails (topmost) + labels + per-thumb selection ring ─────
    let row_y = oy + PAD + TITLE_H;
    for (i, (win, tw, app_id)) in cells.iter().enumerate() {
        let cell_x = shift + left_edges[i];
        let cell_y = row_y;

        // Draw the pre-captured snapshot scaled into the cell. Using a static
        // OpenCloseRenderElement (Fade, progress=1, alpha=1) just blits the
        // texture into `geometry` — works for windows on other tags because
        // the snapshot was taken off-screen before render.
        if let Some((_, tex)) = sw.thumbs.iter().find(|(w, _)| w == win) {
            let geom = Rectangle::<i32, Logical>::new(
                Point::from((cell_x, cell_y)),
                smithay::utils::Size::from((*tw, th)),
            );
            out.push(MargoRenderElement::OpenClose(OpenCloseRenderElement::new(
                Id::new(),
                tex.clone(),
                geom,
                scale,
                1.0,
                1.0,
                OpenCloseKind::Fade,
                false,
                1.0,
                CommitCounter::default(),
                0.0,
                None,
            )));
        }

        // app-id label under the thumbnail.
        if show_labels && !app_id.is_empty() {
            let lpos: Point<i32, Physical> = Point::<i32, Logical>::from((cell_x, cell_y + th + 2))
                .to_physical_precise_round(scale);
            let rgb = if i == sw.selected {
                [240, 240, 255]
            } else {
                [165, 165, 175]
            };
            if let Some(el) = crate::render::text::label_element(
                renderer,
                app_id,
                (f64::from(label_h) * output_scale * 0.85) as i32,
                (f64::from(*tw) * output_scale) as i32,
                rgb,
                lpos.to_f64(),
            ) {
                out.push(MargoRenderElement::Cursor(el));
            }
        }

        // Selection ring behind the selected thumbnail (a slightly larger
        // rounded fill; the thumb covers the centre, leaving a border).
        if i == sw.selected
            && let Some(p) = prog.clone()
        {
            let ring = Rectangle::<i32, Logical>::new(
                Point::from((cell_x - 5, cell_y - 5)),
                smithay::utils::Size::from((tw + 10, th + 10)),
            )
            .to_physical_precise_round(scale);
            out.push(MargoRenderElement::RoundedSolid(
                crate::render::rounded_solid::RoundedSolidElement::new(
                    Id::new(),
                    ring,
                    radius,
                    state.config.group_active_color.0,
                    p,
                ),
            ));
        }
    }

    // ── Backing panel: hugs the thumbnail row (preview-sized), scrolls
    //    with it via the same `shift`. Not a full-width band.
    if let Some(p) = prog {
        let panel = Rectangle::<i32, Logical>::new(
            Point::from((shift - PAD, oy)),
            smithay::utils::Size::from((inner_w + 2 * PAD, panel_h)),
        )
        .to_physical_precise_round(scale);
        out.push(MargoRenderElement::RoundedSolid(
            crate::render::rounded_solid::RoundedSolidElement::new(
                Id::new(),
                panel,
                radius,
                [0.0, 0.0, 0.0, 0.62],
                p,
            ),
        ));
    }

    out
}

fn build_scroller_overview_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    include_cursor: bool,
) -> Vec<MargoRenderElement> {
    use smithay::backend::renderer::element::Kind;
    use smithay::backend::renderer::element::solid::SolidColorRenderElement;
    use smithay::backend::renderer::element::utils::{
        Relocate, RelocateRenderElement, RescaleRenderElement,
    };
    use smithay::backend::renderer::utils::CommitCounter;
    use smithay::utils::{Physical, Size};

    let scale = Scale::from(output_scale);
    let mut elements: Vec<MargoRenderElement> = Vec::new();

    // Cursor on top (mirrors the normal path).
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
                for e in render_elements_from_surface_tree(
                    renderer,
                    surface,
                    ptr_i,
                    output_scale,
                    1.0f32,
                    Kind::Cursor,
                ) {
                    elements.push(MargoRenderElement::WaylandSurface(e));
                }
            }
            CursorImageStatus::Hidden => {}
            _ => {
                if let Some(cursor_elem) =
                    state
                        .cursor_manager
                        .render_element(renderer, ptr_pos, output_scale)
                {
                    elements.push(MargoRenderElement::Cursor(cursor_elem));
                }
            }
        }
    }

    let Some(mon_idx) = state.monitors.iter().position(|m| m.output == od.output) else {
        // No monitor for this output — just dim it.
        let dst = Rectangle::<i32, Physical>::new(
            Point::from((0, 0)),
            Size::from((output_geo.size.w, output_geo.size.h)).to_physical_precise_round(scale),
        );
        elements.push(MargoRenderElement::Solid(SolidColorRenderElement::new(
            smithay::backend::renderer::element::Id::new(),
            dst,
            CommitCounter::default(),
            state.config.overview_backdrop_color.0,
            Kind::Unspecified,
        )));
        return elements;
    };

    let tags = state.scroller_overview_tags(mon_idx);
    // This monitor's continuous scroll position (centred cell).
    let pos = state
        .scroller_overview
        .as_ref()
        .and_then(|ov| ov.mon.get(mon_idx))
        .map(|m| m.pos)
        .unwrap_or(0.0);
    // Interpolate the effective zoom from the open/close animation
    // progress (niri's formula): progress 0 → zoom 1.0 (centred tag
    // full-screen), progress 1 → the configured zoom (full strip). The
    // gap grows with progress too, so cells start flush and fan out.
    let config_zoom = f64::from(state.config.scroller_overview_zoom.clamp(0.1, 1.0));
    let progress = state
        .scroller_overview
        .as_ref()
        .map(|o| o.progress)
        .unwrap_or(1.0);
    let zoom = 1.0 - progress * (1.0 - config_zoom);
    let gap = (f64::from(state.config.scroller_overview_gap.max(0)) * progress) as i32;
    let output_rect = crate::layout::Rect::new(
        output_geo.loc.x,
        output_geo.loc.y,
        output_geo.size.w,
        output_geo.size.h,
    );
    let loop_strip = state.config.scroller_overview_loop;
    let cells = crate::state::overview_cells(output_rect, &tags, zoom, gap, pos, loop_strip);

    // The output's wallpaper (background + bottom layer-shell surfaces),
    // drawn into every cell behind the windows — niri zooms these with
    // each workspace. Collected once; each cell gets a namespaced copy.
    let layer_map = layer_map_for_output(&od.output);
    let bg_layers: Vec<&smithay::desktop::LayerSurface> = layer_map
        .layers()
        .filter(|s| matches!(s.layer(), WlrLayer::Background | WlrLayer::Bottom))
        .collect();

    for cell in &cells {
        let cell_w = output_geo.size.w.max(1);
        let cell_scale = f64::from(cell.rect.width) / f64::from(cell_w);
        let cell_origin_phys: Point<i32, Physical> = Point::from((
            cell.rect.x - output_geo.loc.x,
            cell.rect.y - output_geo.loc.y,
        ))
        .to_physical_precise_round(scale);

        // Each window of this tag, scaled into the cell.
        for client in state.clients.iter().filter(|c| {
            c.monitor == mon_idx
                && (c.tags & (1 << (cell.tag - 1))) != 0
                && !c.is_initial_map_pending
                && !c.is_minimized
                && !c.is_killing
                && !c.is_in_scratchpad
        }) {
            let Some(surface) = client.window.wl_surface() else {
                continue;
            };
            let geo_loc = client.window.geometry().loc;
            let render_location = Point::<i32, smithay::utils::Logical>::from((
                client.geom.x - geo_loc.x,
                client.geom.y - geo_loc.y,
            ));
            let physical_location =
                (render_location - output_geo.loc).to_physical_precise_round(scale);
            let surf_elems = render_elements_from_surface_tree::<
                GlesRenderer,
                WaylandSurfaceRenderElement<GlesRenderer>,
            >(
                renderer,
                &surface,
                physical_location,
                output_scale,
                1.0,
                Kind::Unspecified,
            );
            for e in surf_elems {
                // Namespace by cell key so a tag repeated by the wrap-around
                // loop doesn't collide with its other copy in the tracker.
                let ns = crate::render::namespaced::NamespacedElement::new(e, cell.key);
                let scaled =
                    RescaleRenderElement::from_element(ns, Point::from((0, 0)), cell_scale);
                let placed = RelocateRenderElement::from_element(
                    scaled,
                    cell_origin_phys,
                    Relocate::Relative,
                );
                elements.push(MargoRenderElement::NamespacedSurface(placed));
            }
        }

        // Wallpaper behind the windows, namespaced per cell.
        for layer in &bg_layers {
            let Some(lgeo) = layer_map.layer_geometry(layer) else {
                continue;
            };
            let loc_phys = lgeo.loc.to_physical_precise_round(scale);
            let surf_elems = render_elements_from_surface_tree::<
                GlesRenderer,
                WaylandSurfaceRenderElement<GlesRenderer>,
            >(
                renderer,
                layer.wl_surface(),
                loc_phys,
                output_scale,
                1.0,
                Kind::Unspecified,
            );
            for e in surf_elems {
                let ns = crate::render::namespaced::NamespacedElement::new(e, cell.key);
                let scaled =
                    RescaleRenderElement::from_element(ns, Point::from((0, 0)), cell_scale);
                let placed = RelocateRenderElement::from_element(
                    scaled,
                    cell_origin_phys,
                    Relocate::Relative,
                );
                elements.push(MargoRenderElement::NamespacedSurface(placed));
            }
        }
    }

    // Backdrop behind every cell. An optional `overview_backdrop_image`
    // (cover-fit) sits at the bottom of the visible stack; the solid
    // `overview_backdrop_color` goes below it as the base clear (shows
    // when no image is set, and as a fallback if the image element fails
    // to build). Elements[0] is topmost, so trailing pushes draw first.
    if let Some(bg) = state.overview_backdrop.as_ref()
        && let Some(elem) = bg.render_element(
            renderer,
            output_geo.loc.to_f64(),
            output_geo.size,
            output_scale,
        )
    {
        elements.push(MargoRenderElement::Cursor(elem));
    }

    let backdrop = Rectangle::<i32, Physical>::new(
        Point::from((0, 0)),
        Size::from((output_geo.size.w, output_geo.size.h)).to_physical_precise_round(scale),
    );
    elements.push(MargoRenderElement::Solid(SolidColorRenderElement::new(
        smithay::backend::renderer::element::Id::new(),
        backdrop,
        CommitCounter::default(),
        state.config.overview_backdrop_color.0,
        Kind::Unspecified,
    )));

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
    let target_mon_idx = state.monitors.iter().position(|m| m.output == *output);
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

/// Push the tab-strip chrome for a grouped, active window. No-op for
/// ungrouped windows, hidden group members, or when
/// `group_bar_height == 0`. The strip sits above the tile's top edge,
/// one solid chip per group member (active highlighted). Minimal flat
/// chrome — see `render::group_tabs`.
fn push_group_tabs(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    client: Option<&MargoClient>,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    elements: &mut Vec<MargoRenderElement>,
) {
    let bar_h = state.config.group_bar_height as i32;
    if bar_h <= 0 {
        return;
    }
    let Some(client) = client else { return };
    // Only the visible (active) member carries the strip; hidden
    // members aren't rendered at all.
    let Some(gid) = client.group_id else { return };
    if !client.group_active {
        return;
    }
    let members: Vec<usize> = state
        .clients
        .iter()
        .enumerate()
        .filter(|(_, c)| c.group_id == Some(gid))
        .map(|(i, _)| i)
        .collect();
    if members.len() < 2 {
        return;
    }
    let active_idx = state
        .clients
        .iter()
        .position(|c| std::ptr::eq(c, client))
        .unwrap_or(members[0]);

    // App-name label on each chip (fontdue → MemoryRenderBuffer). Pushed
    // BEFORE the solid chips so it sits at a lower index → drawn on top of
    // its chip. Skipped silently when no font is available; the coloured
    // chips still render.
    let gap = state.config.group_bar_gap as i32;
    for chip in crate::render::group_tabs::chip_rects(client.geom, &members, active_idx, bar_h, gap)
    {
        let Some(c) = state.clients.get(chip.client_idx) else {
            continue;
        };
        let label = if !c.title.is_empty() {
            c.title.as_str()
        } else {
            c.app_id.as_str()
        };
        if label.is_empty() {
            continue;
        }
        // Chip rect → output-local physical.
        let phys_x = ((chip.rect.x - output_geo.loc.x) as f64) * output_scale;
        let phys_y = ((chip.rect.y - output_geo.loc.y) as f64) * output_scale;
        let phys_w = (chip.rect.width as f64) * output_scale;
        let phys_h = (chip.rect.height as f64) * output_scale;
        let text_h = (phys_h * 0.6).round() as i32;
        let pad_x = (phys_h * 0.3).round();
        let max_w = (phys_w - 2.0 * pad_x).round() as i32;
        if text_h <= 2 || max_w <= 2 {
            continue;
        }
        // Contrast: dark text on a light chip, light text on a dark one.
        let bg = if chip.active {
            state.config.group_active_color.0
        } else {
            state.config.group_inactive_color.0
        };
        let lum = 0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2];
        let rgb = if lum > 0.55 {
            [0u8, 0, 0]
        } else {
            [255u8, 255, 255]
        };
        // Vertically centre the (≈1.2×text_h tall) label within the chip.
        let label_h = (text_h as f64) * 1.2;
        let pos_y = phys_y + ((phys_h - label_h) / 2.0).max(0.0);
        let pos =
            smithay::utils::Point::<f64, smithay::utils::Physical>::from((phys_x + pad_x, pos_y));
        if let Some(el) =
            crate::render::text::label_element(renderer, label, text_h, max_w, rgb, pos)
        {
            elements.push(MargoRenderElement::Cursor(el));
        }
    }

    // Rounded chips: reuse the analytic rounded-rect shader. If it fails to
    // compile, fall back to flat solid quads so the strip still draws.
    if let Some(prog) = crate::render::rounded_solid::shader(renderer) {
        // Corner radius scales with the strip height (capped) so the chips
        // read as rounded pills regardless of group_bar_height.
        let radius = (bar_h as f32 * 0.4).min(10.0);
        for chip in crate::render::group_tabs::render_elements(
            client.geom,
            &members,
            active_idx,
            bar_h,
            state.config.group_bar_gap as i32,
            state.config.group_active_color.0,
            state.config.group_inactive_color.0,
            output_geo.loc,
            output_scale,
            radius,
            prog.0,
        ) {
            elements.push(MargoRenderElement::RoundedSolid(chip));
        }
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

    // Index clients by window once per output per frame. The per-window
    // body below resolved its client with a linear `clients.iter().find`,
    // making this loop O(windows × clients) every frame — quadratic in the
    // open-window count. The map makes each lookup O(1).
    let client_by_window: std::collections::HashMap<_, _> =
        state.clients.iter().map(|c| (&c.window, c)).collect();

    for window in state.space.elements_for_output(output).rev() {
        let Some(location) = state.space.element_location(window) else {
            continue;
        };
        let render_location = location - window.geometry().loc;
        let physical_location = (render_location - output_geo.loc).to_physical_precise_round(scale);

        let client = client_by_window.get(&window).copied();

        // Overview alpha: while overview is open, every non-selected
        // thumbnail renders dimmed (config `overview_dim_alpha`,
        // default 0.6) so the focuscolor-bordered selection reads as
        // a spotlight. This is the cinematic feel niri/Hypr ship by
        // default. Selected thumbnail (`is_overview_hovered`, set by
        // either pointer hover or keyboard cycle) stays at full
        // opacity. Outside overview every window renders at 1.0.
        // The factor is multiplied into `render_elements_from_surface_tree`
        // alpha and into the X11/Resize/OpenClose paths below.
        let overview_alpha: f32 = if state.is_overview_open() {
            match client {
                Some(c) if c.is_overview_hovered => 1.0,
                Some(_) => state.config.overview_dim_alpha.clamp(0.1, 1.0),
                None => 1.0,
            }
        } else {
            1.0
        };

        // Screencast blackout: when we're building the element list
        // for a wlr-screencopy capture (`for_screencast = true`) and
        // this window has the windowrule's `block_out_from_screencast
        // = 1` flag set, replace its surface render with a solid
        // black rectangle. The on-screen render path doesn't go
        // through this branch (it passes `for_screencast = false`)
        // so the user still sees their password manager / private-
        // browsing tab / 2FA app — only the captured output is
        // censored.
        if for_screencast && client.is_some_and(|c| c.block_out_from_screencast) {
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
                    Some(s) => smithay::backend::renderer::element::Id::from_wayland_resource(&*s),
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
            let snapshot_active = client.resize_snapshot.is_some() || client.snapshot_pending;
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

                push_group_tabs(renderer, state, client, output_geo, output_scale, elements);

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
                                (client.geom.width.max(1), client.geom.height.max(1)).into(),
                            );
                            // Stable id (reused across frames) so an
                            // unchanged shadow reports zero damage instead
                            // of re-damaging its oversized bbox every frame.
                            let shadow_id = match window.wl_surface() {
                                Some(s) => state.decoration_element_ids(&s).0,
                                None => smithay::backend::renderer::element::Id::new(),
                            };
                            let shadow = crate::render::shadow::ShadowRenderElement::new(
                                shadow_id,
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
                                (c.geom.x - output_geo.loc.x, c.geom.y - output_geo.loc.y).into(),
                                (c.geom.width.max(1), c.geom.height.max(1)).into(),
                            );
                            let id = smithay::backend::renderer::element::Id::from_wayland_resource(
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
                        (c.geom.x - output_geo.loc.x, c.geom.y - output_geo.loc.y).into(),
                        (c.geom.width.max(1), c.geom.height.max(1)).into(),
                    );
                    let id =
                        smithay::backend::renderer::element::Id::from_wayland_resource(wl_surface);

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
                                tracing::trace!("resize_next: live capture failed: {e:?}");
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
                    overview_alpha,
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

                // Background blur UNDER the surface. Pushed AFTER the
                // surface elements so it sits at a higher index in the
                // Vec → drawn earlier (painter's algorithm) → beneath
                // the translucent window, which then composites over it.
                // Where the surface is opaque the blur is fully hidden
                // (wasted GPU only); where it's translucent the blur
                // shows through — Hyprland's `blur` policy. Gated on
                // `Config::blur`, excluded for `no_blur` / fullscreen /
                // scratchpad clients (a blur there would bleed past
                // edges that should feel locked to the screen).
                if let Some(client) = client {
                    if state.config.blur
                        && !client.no_blur
                        && !client.is_fullscreen
                        && !client.is_in_scratchpad
                    {
                        if let (Some(geo), true) = (
                            clip_geometry,
                            crate::render::blur::shader(renderer).is_some(),
                        ) {
                            let rect = Rectangle::<i32, Logical>::new(
                                (geo.loc.x.round() as i32, geo.loc.y.round() as i32).into(),
                                (
                                    (geo.size.w.round() as i32).max(1),
                                    (geo.size.h.round() as i32).max(1),
                                )
                                    .into(),
                            );
                            // Stable id (reused across frames) so unchanged
                            // window blur reports zero damage rather than
                            // re-damaging its rect every frame.
                            let blur_id = match window.wl_surface() {
                                Some(s) => state.decoration_element_ids(&s).1,
                                None => smithay::backend::renderer::element::Id::new(),
                            };
                            elements.push(MargoRenderElement::Blur(
                                crate::render::blur::BlurRenderElement::new(
                                    blur_id,
                                    rect,
                                    radius,
                                    state.config.blur_params,
                                    scale,
                                ),
                            ));
                        }
                    }
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

                push_group_tabs(renderer, state, client, output_geo, output_scale, elements);

                let rendered = AsRenderElements::<GlesRenderer>::render_elements::<
                    WaylandSurfaceRenderElement<GlesRenderer>,
                >(
                    window, renderer, physical_location, scale, overview_alpha
                );
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
        if state
            .layer_animations
            .get(&key)
            .map(|a| a.is_close)
            .unwrap_or(false)
        {
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

        // Use `render_elements_from_surface_tree` directly with
        // `Kind::ScanoutCandidate` so smithay's DrmCompositor is
        // *allowed* to assign the layer to a DRM overlay plane —
        // page-flips with overlay-plane assignments update atomically
        // with the primary plane on VBlank, so the bar pixels can't
        // tear or partially flip. Without `ScanoutCandidate` (which
        // is what smithay's stock `LayerSurface::render_elements` /
        // `AsRenderElements` impl produces) the bar always composites
        // through GL into the primary swapchain — slower, and on
        // Intel MTL we observed visible flicker when GTK4 commits a
        // new revealer frame mid-render. niri uses the same
        // ScanoutCandidate path in `niri/src/layer/mapped.rs:227`,
        // which is the single biggest reason `mshell-on-niri` is
        // smooth where `mshell-on-margo` flickers.
        let scale = Scale::from(output_scale);
        let location = geo.loc.to_physical_precise_round(scale);
        let popup_iter = smithay::desktop::PopupManager::popups_for_surface(surface.wl_surface())
            .flat_map(|(popup, popup_offset)| {
                let offset = (popup_offset - popup.geometry().loc)
                    .to_f64()
                    .to_physical(scale)
                    .to_i32_round();
                render_elements_from_surface_tree::<
                    GlesRenderer,
                    WaylandSurfaceRenderElement<GlesRenderer>,
                >(
                    renderer,
                    popup.wl_surface(),
                    location + offset,
                    scale,
                    layer_alpha,
                    Kind::Unspecified,
                )
            });
        for elem in popup_iter {
            elements.push(MargoRenderElement::Space(SpaceRenderElements::Surface(
                elem,
            )));
        }
        let surface_elems = render_elements_from_surface_tree::<
            GlesRenderer,
            WaylandSurfaceRenderElement<GlesRenderer>,
        >(
            renderer,
            surface.wl_surface(),
            location,
            scale,
            layer_alpha,
            Kind::ScanoutCandidate,
        );
        for elem in surface_elems {
            elements.push(MargoRenderElement::Space(SpaceRenderElements::Surface(
                elem,
            )));
        }

        // Background blur UNDER a layer surface when `Config::blur_layer`
        // is on and no matching `layerrule = noblur:1` excludes it.
        // Pushed AFTER the surface elements → drawn beneath it (same
        // painter-order reasoning as the window path). Skipped while the
        // layer is mid open-animation (alpha < full) to avoid a blur
        // flashing in ahead of the surface.
        if state.config.blur_layer {
            let namespace = surface.namespace();
            let no_blur = state
                .config
                .layer_rules
                .iter()
                .filter(|r| crate::state::matches_layer_name(r, namespace))
                .any(|r| r.no_blur);
            let mid_anim = state
                .layer_animations
                .get(&key)
                .map(|a| !a.is_close && a.progress < 1.0)
                .unwrap_or(false);
            if !no_blur && !mid_anim && crate::render::blur::shader(renderer).is_some() {
                let rect = Rectangle::<i32, Logical>::new(
                    (geo.loc.x, geo.loc.y).into(),
                    (geo.size.w.max(1), geo.size.h.max(1)).into(),
                );
                // Stable id (reused across frames) so unchanged layer blur
                // reports zero damage rather than re-damaging every frame.
                let blur_id = state
                    .decoration_element_ids(surface.layer_surface().wl_surface())
                    .1;
                elements.push(MargoRenderElement::Blur(
                    crate::render::blur::BlurRenderElement::new(
                        blur_id,
                        rect,
                        0.0,
                        state.config.blur_params,
                        scale,
                    ),
                ));
            }
        }
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
            (
                anim.geom.x - output_geo.loc.x,
                anim.geom.y - output_geo.loc.y,
            )
                .into(),
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
