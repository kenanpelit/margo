//! Capture paths: mirror frames, the capture-scene owner of a mirrored
//! output, image-copy (ext-image-copy-capture) and screencast frame
//! drains, and per-window screencast elements.

#[cfg(feature = "xdp-gnome-screencast")]
use super::clients::push_client_elements;
#[cfg(feature = "xdp-gnome-screencast")]
use super::cursor::{build_cursor_elements_for_output, push_cursor_elements};
use super::*;

/// The frame for the active mirror's target: the source output's scene,
/// scaled and letterboxed into the target. `source_scene` is the scene the
/// source just rendered from in this batch, if it did; otherwise the
/// scene is built here. `None` when nothing is mirrored, or either end has
/// no device / mode right now.
pub(in crate::backend::udev) fn build_mirror_frame(
    renderer: &mut GlesRenderer,
    outputs: &HashMap<crtc::Handle, OutputDevice>,
    state: &MargoState,
    source_scene: Option<super::mirror_element::SharedScene>,
) -> Option<(Output, Vec<MargoRenderElement>)> {
    let session = state.mirror.active()?;
    let src = outputs
        .values()
        .find(|od| od.output_name == session.source)?;
    let dst = outputs
        .values()
        .find(|od| od.output_name == session.target)?;
    let fit = mirror_fit(src, dst)?;
    let scene = match source_scene {
        Some(scene) => scene,
        None => std::rc::Rc::new(build_output_scene(renderer, src, state)),
    };
    let elements = super::mirror_element::MirroredElement::frame(&scene, fit);
    Some((dst.output.clone(), elements))
}

/// Which output's scene a capture of `output` shows. A mirror target is
/// out of the layout, so its own scene is empty: a screenshot, image copy
/// or output cast of it shows its source's scene, like the screen does.
fn capture_scene_owner<'a>(
    output: &'a str,
    mirror: Option<&'a crate::mirror::MirrorSession>,
) -> &'a str {
    match mirror {
        Some(m) if m.target == output => &m.source,
        _ => output,
    }
}

/// The active mirror's source device when `od` is its target.
pub(in crate::backend::udev) fn mirror_source_of<'a>(
    outputs: &'a HashMap<crtc::Handle, OutputDevice>,
    od: &OutputDevice,
    state: &MargoState,
) -> Option<&'a OutputDevice> {
    let owner = capture_scene_owner(&od.output_name, state.mirror.active());
    if owner == od.output_name {
        return None;
    }
    outputs.values().find(|o| o.output_name == owner)
}

/// How the source's picture is fitted into the target: the same letterbox
/// for the on-screen mirror frame and for captures of the target.
fn mirror_fit(src: &OutputDevice, dst: &OutputDevice) -> Option<crate::mirror::Letterbox> {
    crate::mirror::letterbox(
        transformed_mode_size(&src.output)?,
        transformed_mode_size(&dst.output)?,
    )
}

/// An output's own capture scene: the screencast variant of its scene
/// (blackout filter applied, cursor per the client's request) plus the
/// compositor wallpaper the display path adds below it.
fn own_capture_scene(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    include_cursor: bool,
) -> Vec<MargoRenderElement> {
    let mut elements = build_render_elements_inner(
        renderer,
        od,
        state,
        RenderTarget::Screencast { include_cursor },
    );
    if !state.session_locked {
        let wallpaper = crate::wallpaper::runtime::scene_elements(renderer, state, &od.output);
        elements.extend(wallpaper.into_iter().map(Into::into));
    }
    elements
}

/// The scene a capture (screencopy, image copy, output cast) of `od`
/// records. `mirror_source` ([`mirror_source_of`]) is set when `od` is the
/// mirror target: the capture then shows the source's capture scene,
/// letterboxed like the screen, so block-out still applies.
pub(in crate::backend::udev) fn capture_scene(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    mirror_source: Option<&OutputDevice>,
    state: &MargoState,
    include_cursor: bool,
) -> Vec<MargoRenderElement> {
    if let Some(src) = mirror_source
        && let Some(fit) = mirror_fit(src, od)
    {
        let scene = std::rc::Rc::new(own_capture_scene(renderer, src, state, include_cursor));
        return super::mirror_element::MirroredElement::frame(&scene, fit);
    }
    own_capture_scene(renderer, od, state, include_cursor)
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
pub(in crate::backend::udev) fn drain_image_copy_frames(
    renderer: &mut GlesRenderer,
    outputs: &mut std::collections::HashMap<crtc::Handle, OutputDevice>,
    state: &mut MargoState,
) {
    use smithay::backend::renderer::damage::OutputDamageTracker as DamageTracker;
    use smithay::backend::renderer::{Bind, ExportMem};
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
                let outputs: &HashMap<_, _> = outputs;
                let od = match outputs.iter().find(|(_, od)| od.output.name() == *name) {
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
                let mirror_source = mirror_source_of(outputs, od, state);
                let elements = capture_scene(renderer, od, mirror_source, state, false);
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

        // Borrow a pooled offscreen renderbuffer (one allocation per
        // capture size, not per frame) and render the scene into it.
        // Same shape as the SHM arm of `serve_screencopies`.
        let mut renderbuffer =
            match crate::render::offscreen_pool::take_capture_renderbuffer(renderer, buf_size) {
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
        crate::render::offscreen_pool::return_capture_renderbuffer(buf_size, renderbuffer);

        // Write into the client's wl_buffer (SHM only — DMA-BUF
        // is Step 2.1).
        let buffer = frame.buffer();
        let copy_result =
            smithay::wayland::shm::with_buffer_contents_mut(&buffer, |pool_ptr, pool_len, meta| {
                // SAFETY: pool_ptr/pool_len are the client's validated
                // wl_shm pool mapping, valid for this callback; the helper
                // writes only inside this buffer's own bytes (`meta.offset`,
                // `meta.stride`). `pixels` is our own readback, so the
                // regions don't overlap.
                unsafe {
                    super::helpers::copy_frame_into_shm_pool(
                        pool_ptr,
                        pool_len,
                        &meta,
                        (buf_size.w.max(0) as usize).saturating_mul(4),
                        buf_size.h.max(0) as usize,
                        &pixels,
                    )
                }
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
pub(in crate::backend::udev) fn drain_active_cast_frames(
    renderer: &mut GlesRenderer,
    outputs: &mut HashMap<crtc::Handle, OutputDevice>,
    state: &mut MargoState,
) {
    use crate::screencasting::pw_utils::{CastFrameOutcome, CastSizeChange, CursorData};
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
    // What this tick did for each cast, by index; decides below whether the
    // cast needs its own wake-up.
    let mut outcomes = vec![CastFrameOutcome::Skipped; casts.len()];

    for (cast, outcome) in casts.iter_mut().zip(outcomes.iter_mut()) {
        if !cast.is_active() {
            continue;
        }

        // Clone the target up front so we drop the borrow on
        // `cast` while we read state.* — then re-borrow `cast`
        // mutably for the render call.
        let target = cast.target.clone();
        match target {
            CastTarget::Nothing => {
                *outcome = cast.dequeue_buffer_and_clear(renderer);
                if *outcome == CastFrameOutcome::Rendered {
                    cast.last_frame_time = now;
                }
            }
            CastTarget::Window { id } => {
                let Some(client_idx) = state.clients.iter().position(|c| c.id == id) else {
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
                    *outcome = CastFrameOutcome::Throttled;
                    continue;
                }

                match cast.ensure_size(size) {
                    Ok(CastSizeChange::Ready) => (),
                    Ok(CastSizeChange::Pending) => {
                        *outcome = CastFrameOutcome::SizePending;
                        continue;
                    }
                    Err(err) => {
                        warn!("cast ensure_size: {err:?}");
                        to_stop.push(cast.session_id);
                        continue;
                    }
                }

                let Some((_, od)) = outputs.iter().find(|(_, od)| od.output == mon.output) else {
                    continue;
                };

                // Build only the target window's elements (surface,
                // popups, border, shadow, block-out) plus the cursor,
                // and shift them so the window's top-left lands at
                // (0, 0) in the cast buffer.
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

                let window = client.window.clone();
                let output_elems =
                    build_window_cast_elements(renderer, od, state, &window, include_cursor);
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
                *outcome =
                    cast.dequeue_buffer_and_render(renderer, &elements, &cursor_data, size, scale);
                if *outcome == CastFrameOutcome::Rendered {
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
                    *outcome = CastFrameOutcome::Throttled;
                    continue;
                }

                match cast.ensure_size(size) {
                    Ok(CastSizeChange::Ready) => (),
                    Ok(CastSizeChange::Pending) => {
                        *outcome = CastFrameOutcome::SizePending;
                        continue;
                    }
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

                let mirror_source = mirror_source_of(outputs, od, state);
                let output_elems =
                    capture_scene(renderer, od, mirror_source, state, include_cursor);
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
                *outcome =
                    cast.dequeue_buffer_and_render(renderer, &elements, &cursor_data, size, scale);
                if *outcome == CastFrameOutcome::Rendered {
                    cast.last_frame_time = now;
                }
            }
        }
    }

    // Scene damage pings the repaint source, which runs this function for
    // every cast, so most casts need no wake of their own. The old code
    // armed one for every active cast at `last_frame_time + min`; once the
    // scene went static that target lay in the past, became a zero-length
    // timer, and re-ran the repaint back to back forever. Now only a frame
    // that is owed but blocked on PipeWire (no free buffer, resize)
    // retries, at a future time; a cast the pacing layer throttled keeps
    // the timer `check_time_and_schedule` armed.
    for (cast, outcome) in casts.iter_mut().zip(outcomes) {
        if !cast.is_active() {
            continue;
        }
        let output = match &cast.target {
            CastTarget::Output { name, .. } => state
                .monitors
                .iter()
                .find(|m| m.output.name() == *name)
                .map(|m| m.output.clone()),
            CastTarget::Window { id } => state
                .clients
                .iter()
                .find(|c| c.id == *id)
                .and_then(|c| state.monitors.get(c.monitor))
                .map(|m| m.output.clone()),
            CastTarget::Nothing => state.monitors.first().map(|m| m.output.clone()),
        };
        if let Some(output) = output {
            cast.schedule_after_tick(output, outcome);
        }
    }

    if let Some(s) = state.screencasting.as_mut() {
        s.casts = casts;
    }
    for id in to_stop {
        state.stop_cast(id);
    }
}

/// Elements for a single-window cast: only `window` itself (surface,
/// popups, border, shadow) plus the cursor — never other windows, layer
/// surfaces (notifications, bars) or compositor overlays that happen to
/// overlap it. Output-relative coordinates, like the full builder.
#[cfg(feature = "xdp-gnome-screencast")]
pub(in crate::backend::udev) fn build_window_cast_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    window: &smithay::desktop::Window,
    include_cursor: bool,
) -> Vec<MargoRenderElement> {
    if state.session_locked {
        return Vec::new();
    }
    let Some(output_geo) = state.space.output_geometry(&od.output) else {
        return Vec::new();
    };
    let output_scale = od.output.current_scale().fractional_scale();
    let border_program = crate::render::rounded_border::shader(renderer).map(|program| program.0);
    let clipped_surface_program =
        crate::render::clipped_surface::shader(renderer).map(|program| program.0);
    let mut elements = Vec::new();
    if include_cursor {
        push_cursor_elements(renderer, state, output_geo, output_scale, &mut elements);
    }
    push_client_elements(
        renderer,
        state,
        &od.output,
        output_geo,
        output_scale,
        border_program,
        clipped_surface_program,
        true,
        Some(window),
        &mut elements,
    );
    elements
}

#[cfg(test)]
mod capture_owner_tests {
    use super::capture_scene_owner;
    use crate::mirror::MirrorSession;

    #[test]
    fn a_capture_of_the_mirror_target_shows_the_source() {
        let m = MirrorSession {
            source: "eDP-1".into(),
            target: "DP-3".into(),
        };
        // `grim -o DP-3` while presenting: the projector's picture.
        assert_eq!(capture_scene_owner("DP-3", Some(&m)), "eDP-1");
        assert_eq!(capture_scene_owner("eDP-1", Some(&m)), "eDP-1");
        assert_eq!(capture_scene_owner("HDMI-A-1", Some(&m)), "HDMI-A-1");
        assert_eq!(capture_scene_owner("DP-3", None), "DP-3");
    }
}
