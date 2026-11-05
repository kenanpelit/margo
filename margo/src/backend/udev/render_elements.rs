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
use crate::state::DecorationKind;

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

/// An output's full display scene: [`build_render_elements`] plus the
/// compositor wallpaper at the bottom of the z-order (skipped while the
/// session is locked, unless the lock surface shows the desktop).
pub(super) fn build_output_scene(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
) -> Vec<MargoRenderElement> {
    let mut elements = build_render_elements(renderer, od, state);
    // Wallpaper sits at the *bottom* of the z-order: pushed last so
    // it's at the tail of the slice. smithay's `render_frame`
    // iterates element[0] = topmost, so trailing entries are drawn
    // first / occluded by everything above. Skip while session is
    // locked — mlock paints its own backdrop over the entire output.
    if (!state.session_locked || state.lock_shows_desktop(&od.output))
        && let Some(ref wp) = state.wallpaper
        && let Some(output_geo) = state.space.output_geometry(&od.output)
        && let Some(wp_elem) = wp.render_element(
            renderer,
            output_geo.size,
            state.config.wallpaper_fit,
            od.output.current_scale().fractional_scale(),
        )
    {
        elements.push(MargoRenderElement::Wallpaper(wp_elem));
    }
    elements
}

/// An output's mode size after its transform: the extent of the space
/// its render elements live in.
fn transformed_mode_size(output: &Output) -> Option<(i32, i32)> {
    let mode = output.current_mode()?;
    let size = output.current_transform().transform_size(mode.size);
    Some((size.w, size.h))
}

/// The frame for the active mirror's target: the source output's scene,
/// scaled and letterboxed into the target. `source_scene` is the scene the
/// source just rendered from in this batch, if it did; otherwise the
/// scene is built here. `None` when nothing is mirrored, or either end has
/// no device / mode right now.
pub(super) fn build_mirror_frame(
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
pub(super) fn mirror_source_of<'a>(
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
    if !state.session_locked
        && let Some(ref wp) = state.wallpaper
        && let Some(output_geo) = state.space.output_geometry(&od.output)
        && let Some(wp_elem) = wp.render_element(
            renderer,
            output_geo.size,
            state.config.wallpaper_fit,
            od.output.current_scale().fractional_scale(),
        )
    {
        elements.push(MargoRenderElement::Wallpaper(wp_elem));
    }
    elements
}

/// The scene a capture (screencopy, image copy, output cast) of `od`
/// records. `mirror_source` ([`mirror_source_of`]) is set when `od` is the
/// mirror target: the capture then shows the source's capture scene,
/// letterboxed like the screen, so block-out still applies.
pub(super) fn capture_scene(
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
pub(super) fn drain_image_copy_frames(
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
pub(super) fn drain_active_cast_frames(
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
    if state.input_timers.cursor_hidden || !output_geo.to_f64().contains(ptr_global) {
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

    if state.session_locked {
        let mut elements = Vec::new();

        // Highest priority: cursor (if inside this output)
        if include_cursor {
            push_cursor_elements(renderer, state, output_geo, output_scale, &mut elements);
        }

        // If the locker has not supplied a surface for this output yet, the
        // black frame clear remains the entire scene. Never fall through to
        // desktop windows during that protocol gap.
        if let Some((_, lock_surface)) = state
            .lock_surfaces
            .iter()
            .find(|(output, _)| output == &od.output)
        {
            let lock_elements = render_elements_from_surface_tree(
                renderer,
                lock_surface.wl_surface(),
                Point::<i32, Physical>::from((0, 0)),
                output_scale,
                1.0,
                Kind::Unspecified,
            );
            for e in lock_elements {
                elements.push(MargoRenderElement::WaylandSurface(e));
            }
        }

        // `allow_lock_transparent` (mango): a translucent lock surface
        // shows the desktop beneath it — only once that output's lock
        // surface has a buffer (never during the protocol gap) and never
        // in a capture.
        if !for_screencast && state.lock_shows_desktop(&od.output) {
            elements.extend(build_desktop_elements(
                renderer,
                od,
                state,
                output_geo,
                output_scale,
                false,
                false,
            ));
        }

        return elements;
    }

    build_desktop_elements(
        renderer,
        od,
        state,
        output_geo,
        output_scale,
        include_cursor,
        for_screencast,
    )
}

/// The unlocked scene for one output: overlays, layers, windows,
/// decorations (and the cursor when `include_cursor`).
fn build_desktop_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    include_cursor: bool,
    for_screencast: bool,
) -> Vec<MargoRenderElement> {
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
    // Scroller overview returned above. On the ordinary desktop the
    // visibility decision is the same for every layer role; resolve the
    // monitor and exclusive-fullscreen scan once, not per layer.
    let layers_visible = state.layer_renders_on_output(&od.output, WlrLayer::Top);
    // Use the same role-aware predicate as commit and callback scheduling.
    // Exclusive fullscreen suppresses all layers; classic overview restores
    // them even when an exclusive client appears among its thumbnails.
    let upper_layers: Vec<_> = layer_map
        .layers()
        .rev()
        .filter(|_| layers_visible)
        .filter(|surface| matches!(surface.layer(), WlrLayer::Overlay | WlrLayer::Top))
        .collect();
    let lower_layers: Vec<_> = layer_map
        .layers()
        .rev()
        .filter(|_| layers_visible)
        .filter(|surface| matches!(surface.layer(), WlrLayer::Bottom | WlrLayer::Background))
        .collect();
    let border_program = crate::render::rounded_border::shader(renderer).map(|program| program.0);
    let clipped_surface_program =
        crate::render::clipped_surface::shader(renderer).map(|program| program.0);

    // Last frame's length (+ room for the wallpaper) beats the estimate.
    let estimate = upper_layers.len() + lower_layers.len() + state.clients.len() * 2 + 1;
    let mut elements: Vec<MargoRenderElement> =
        Vec::with_capacity(estimate.max(od.last_element_count.get() + 2));

    // First elements are highest z-order in the DRM compositor.
    if include_cursor {
        push_cursor_elements(renderer, state, output_geo, output_scale, &mut elements);
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

    push_closing_layers(
        state,
        &od.output,
        output_scale,
        clipped_surface_program.clone(),
        &[WlrLayer::Overlay, WlrLayer::Top],
        &mut elements,
    );

    // Closing-client snapshots. Each entry is a window whose toplevel
    // role was destroyed but whose close animation hasn't finished;
    // we render the captured texture scaled+faded around its last
    // known geometry. Drawn before live clients because element[0] is
    // topmost — slightly fragile if a new window mapped
    // exactly underneath, but acceptable for a sub-second transition.
    push_closing_clients(
        state,
        &od.output,
        output_geo,
        output_scale,
        clipped_surface_program.clone(),
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
        None,
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
        output_scale,
        clipped_surface_program,
        &[WlrLayer::Bottom, WlrLayer::Background],
        &mut elements,
    );
    od.last_element_count.set(elements.len());
    elements
}

/// The pointer sprite for this output (if the pointer is on it).
fn push_cursor_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    elements: &mut Vec<MargoRenderElement>,
) {
    let ptr_global = Point::<f64, _>::from((state.input_pointer.x, state.input_pointer.y));
    if state.input_timers.cursor_hidden || !output_geo.to_f64().contains(ptr_global) {
        return;
    }
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
}

/// Elements for a single-window cast: only `window` itself (surface,
/// popups, border, shadow) plus the cursor — never other windows, layer
/// surfaces (notifications, bars) or compositor overlays that happen to
/// overlap it. Output-relative coordinates, like the full builder.
#[cfg(feature = "xdp-gnome-screencast")]
pub(super) fn build_window_cast_elements(
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

/// MRU window-switcher overlay: a centred row of scaled-down window
/// thumbnails drawn on top of the normal desktop while the switcher is open,
/// the selected one ringed with the accent colour. Thumbnails are off-screen
/// captures cached per window (`take_mru_thumbnails`, re-captured only when
/// the window commits); every element keeps a stable id with a commit counter
/// that changes only with its content, so an idle switcher costs no damage.
/// Returned elements are appended above windows but below the cursor.
fn build_mru_switcher_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
) -> Vec<MargoRenderElement> {
    use crate::render::open_close::{OpenCloseKind, OpenCloseRenderElement};
    use smithay::backend::renderer::utils::CommitCounter;

    let Some(sw) = state.mru_switcher.as_ref() else {
        return Vec::new();
    };
    let scale = Scale::from(output_scale);
    let mut out: Vec<MargoRenderElement> = Vec::new();

    // Layout (logical px, output-local).
    let th = (state.config.mru_thumb_height as i32).clamp(60, 600);
    let show_labels = state.config.mru_show_labels;
    let gap = (state.config.mru_thumb_gap as i32).clamp(0, 64);
    let pad = (state.config.mru_panel_padding as i32).clamp(0, 80);
    let max = (state.config.mru_max as usize).clamp(2, 20);
    const TITLE_H: i32 = 22;
    let label_h: i32 = if show_labels { 20 } else { 0 };

    // Accent theming: tie the selection ring, scope title and selected label to
    // the matugen focus colour when enabled, else keep the neutral greys.
    let accent = state.config.mru_accent_selection;
    let fc = state.config.focuscolor.0;
    let accent_rgb: [u8; 3] = [
        (fc[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (fc[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (fc[2] * 255.0).round().clamp(0.0, 255.0) as u8,
    ];
    let ring_color = if accent {
        fc
    } else {
        state.config.group_active_color.0
    };
    let title_rgb = if accent { accent_rgb } else { [200, 200, 210] };

    // Sliding window of at most `max` candidates, centred on the
    // selection. `sw.selected` indexes the FULL candidate list (it wraps
    // modulo `sw.candidates.len()` in `mru_advance`) — a flat
    // `.take(max)` from the front left the strip frozen to the first
    // `max` entries while cycling kept advancing `selected` (and focus)
    // past what was ever rendered, so the ring landed on the wrong
    // thumbnail (or none) for anyone with more open windows than `max`.
    let window_start = mru_window_start(sw.selected, sw.candidates.len(), max);

    // (window, thumb_width, app_id). Thumb width keeps the window's aspect.
    let mut cells: Vec<(smithay::desktop::Window, i32, String)> = Vec::new();
    for win in sw.candidates.iter().skip(window_start).take(max) {
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
        acc += tw + gap;
    }
    let inner_w = acc - gap; // total row width (drop the trailing gap)
    let panel_h = TITLE_H + th + label_h + 2 * pad;
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
        title_rgb,
        title_pos.to_f64(),
    ) {
        out.push(MargoRenderElement::Cursor(el));
    }

    // ── Thumbnails (topmost) + labels + per-thumb selection ring ─────
    let row_y = oy + pad + TITLE_H;
    for (i, (win, tw, app_id)) in cells.iter().enumerate() {
        let cell_x = shift + left_edges[i];
        let cell_y = row_y;

        // Draw the pre-captured snapshot scaled into the cell. Using a static
        // OpenCloseRenderElement (Fade, progress=1, alpha=1) just blits the
        // texture into `geometry` — works for windows on other tags because
        // the snapshot was taken off-screen before render.
        if let Some(thumb) = sw.thumb(win)
            && let (Some(tex), Some(key)) = (thumb.capture.current(), thumb.capture.key())
        {
            let geom = Rectangle::<i32, Logical>::new(
                Point::from((cell_x, cell_y)),
                smithay::utils::Size::from((*tw, th)),
            );
            out.push(MargoRenderElement::OpenClose(OpenCloseRenderElement::new(
                thumb.id.clone(),
                tex.clone(),
                geom,
                scale,
                1.0,
                1.0,
                OpenCloseKind::Fade,
                false,
                1.0,
                CommitCounter::from(key as usize),
                0.0,
                None,
            )));
        }

        // app-id label under the thumbnail.
        if show_labels && !app_id.is_empty() {
            let lpos: Point<i32, Physical> = Point::<i32, Logical>::from((cell_x, cell_y + th + 2))
                .to_physical_precise_round(scale);
            let rgb = if i == sw.selected {
                if accent { accent_rgb } else { [240, 240, 255] }
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
                    sw.chrome_ids.ring.clone(),
                    ring,
                    radius,
                    ring_color,
                    p,
                )
                .with_commit(color_commit(ring_color)),
            ));
        }
    }

    // ── Title bar under the panel (mango 7a1f90ed): the selected
    //    window's title, then its app_id, centred on the output. Shown
    //    with the labels (`mru_show_labels`).
    if show_labels
        && let Some(client) = sw
            .candidates
            .get(sw.selected)
            .and_then(|w| state.clients.iter().find(|c| c.window == *w))
    {
        let text = match (client.title.trim(), client.app_id.trim()) {
            ("", app) => app.to_string(),
            (title, "") => title.to_string(),
            (title, app) => format!("{title}  —  {app}"),
        };
        const BAR_H: i32 = 34;
        let text_h = (f64::from(BAR_H) * output_scale * 0.5) as i32;
        let max_w = ((output_geo.size.w - 120).max(100) as f64 * output_scale) as i32;
        if let Some(text_w) = crate::render::text::label_width(&text, text_h, max_w) {
            let text_w_logical = (f64::from(text_w) / output_scale).ceil() as i32;
            let bar_w = text_w_logical + 2 * 18;
            let bar_x = (output_geo.size.w - bar_w) / 2;
            let bar_y = oy + panel_h + 10;
            let text_pos: Point<i32, Physical> = Point::<i32, Logical>::from((
                bar_x + 18,
                bar_y + (BAR_H - (f64::from(text_h) / output_scale).round() as i32) / 2 - 2,
            ))
            .to_physical_precise_round(scale);
            if let Some(el) = crate::render::text::label_element(
                renderer,
                &text,
                text_h,
                max_w,
                if accent { accent_rgb } else { [235, 235, 245] },
                text_pos.to_f64(),
            ) {
                out.push(MargoRenderElement::Cursor(el));
            }
            if let Some(p) = prog.clone() {
                let bar = Rectangle::<i32, Logical>::new(
                    Point::from((bar_x, bar_y)),
                    smithay::utils::Size::from((bar_w, BAR_H)),
                )
                .to_physical_precise_round(scale);
                out.push(MargoRenderElement::RoundedSolid(
                    crate::render::rounded_solid::RoundedSolidElement::new(
                        sw.chrome_ids.title_bar.clone(),
                        bar,
                        radius,
                        [0.0, 0.0, 0.0, 0.62],
                        p,
                    ),
                ));
            }
        }
    }

    // ── Backing panel: hugs the thumbnail row (preview-sized), scrolls
    //    with it via the same `shift`. Not a full-width band.
    if let Some(p) = prog {
        let panel = Rectangle::<i32, Logical>::new(
            Point::from((shift - pad, oy)),
            smithay::utils::Size::from((inner_w + 2 * pad, panel_h)),
        )
        .to_physical_precise_round(scale);
        out.push(MargoRenderElement::RoundedSolid(
            crate::render::rounded_solid::RoundedSolidElement::new(
                sw.chrome_ids.panel.clone(),
                panel,
                radius,
                [0.0, 0.0, 0.0, 0.62],
                p,
            ),
        ));
    }

    // Backdrop dim over the whole output (still in front of the windows) —
    // pushed last so it sits behind the panel + thumbnails, darkening only the
    // desktop beneath the switcher.
    let dim = state.config.mru_dim_alpha.clamp(0.0, 0.9);
    if dim > 0.0 {
        let full = Rectangle::<i32, Physical>::new(
            Point::from((0, 0)),
            smithay::utils::Size::from((output_geo.size.w, output_geo.size.h))
                .to_physical_precise_round(scale),
        );
        out.push(MargoRenderElement::Solid(
            smithay::backend::renderer::element::solid::SolidColorRenderElement::new(
                sw.chrome_ids.dim.clone(),
                full,
                color_commit([0.0, 0.0, 0.0, dim]),
                [0.0, 0.0, 0.0, dim],
                smithay::backend::renderer::element::Kind::Unspecified,
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

    // Stable across frames (see `ScrollerOverview::backdrop_id`).
    let backdrop_id = state
        .scroller_overview
        .as_ref()
        .map(|overview| overview.backdrop_id.clone())
        .unwrap_or_else(smithay::backend::renderer::element::Id::new);
    let Some(mon_idx) = state.monitors.iter().position(|m| m.output == od.output) else {
        // No monitor for this output — just dim it.
        let dst = Rectangle::<i32, Physical>::new(
            Point::from((0, 0)),
            Size::from((output_geo.size.w, output_geo.size.h)).to_physical_precise_round(scale),
        );
        elements.push(MargoRenderElement::Solid(SolidColorRenderElement::new(
            backdrop_id,
            dst,
            color_commit(state.config.overview_backdrop_color.0),
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
    let mut cells = crate::state::overview_cells(output_rect, &tags, zoom, gap, pos, loop_strip);
    // Keys stay as they are, so a cell keeps its namespace while it
    // scrolls in and out of view.
    cells.retain(|cell| overview_cell_visible(cell.rect, output_rect, gap));

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
            output_geo.size,
            margo_config::WallpaperFit::Cover,
            scale.x,
        )
    {
        elements.push(MargoRenderElement::Wallpaper(elem));
    }

    let backdrop = Rectangle::<i32, Physical>::new(
        Point::from((0, 0)),
        Size::from((output_geo.size.w, output_geo.size.h)).to_physical_precise_round(scale),
    );
    elements.push(MargoRenderElement::Solid(SolidColorRenderElement::new(
        backdrop_id,
        backdrop,
        color_commit(state.config.overview_backdrop_color.0),
        state.config.overview_backdrop_color.0,
        Kind::Unspecified,
    )));

    elements
}

/// Whether an overview cell at `cell` can show anything on `output`: its
/// rect, grown by `margin` on every side, meets the output. Off-screen
/// cells would still cost a full element build (every window of the tag
/// plus the background layers) only for the damage tracker to cull them.
fn overview_cell_visible(
    cell: crate::layout::Rect,
    output: crate::layout::Rect,
    margin: i32,
) -> bool {
    let m = margin.max(0);
    cell.x - m < output.x + output.width
        && cell.x + cell.width + m > output.x
        && cell.y - m < output.y + output.height
        && cell.y + cell.height + m > output.y
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
                cc.opacity,
                cc.kind,
                true, // is_close
                cc.extreme_scale,
                smithay::backend::renderer::utils::CommitCounter::default(),
                cc.border_radius,
                clipped_surface_program.clone(),
            )
            .with_fade(
                state.config.animation_fade_out,
                state.config.fadeout_begin_opacity,
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
    client: Option<(usize, &MargoClient)>,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    elements: &mut Vec<MargoRenderElement>,
) {
    let bar_h = state.config.group_bar_height as i32;
    if bar_h <= 0 {
        return;
    }
    let Some((active_idx, client)) = client else {
        return;
    };
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
            |member| {
                state
                    .clients
                    .get(member)
                    .and_then(|c| c.window.wl_surface())
                    .map(|s| state.decoration_element_id(&s, DecorationKind::GroupChip))
                    .unwrap_or_else(smithay::backend::renderer::element::Id::new)
            },
        ) {
            elements.push(MargoRenderElement::RoundedSolid(chip));
        }
    }
}

/// Phase 2 drag-tile-to-tile drop-target preview — see
/// `render::drag_highlight`. Called once per client alongside
/// `push_group_tabs`; no-ops unless `client` is exactly the client
/// `MoveSurfaceGrab::motion` currently has marked as the live swap
/// target.
fn push_drag_swap_highlight(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    client: Option<(usize, &MargoClient)>,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    elements: &mut Vec<MargoRenderElement>,
) {
    let Some(target_id) = state.drag_swap_target else {
        return;
    };
    let Some((client_idx, client)) = client else {
        return;
    };
    if client.id != target_id {
        return;
    }
    let Some(prog) = crate::render::rounded_solid::shader(renderer) else {
        return;
    };
    let radius = state.client_radius(client_idx) * output_scale as f32;
    elements.push(MargoRenderElement::RoundedSolid(
        crate::render::drag_highlight::render_element(
            client
                .window
                .wl_surface()
                .map(|s| {
                    crate::render::drag_highlight::highlight_id(
                        &smithay::backend::renderer::element::Id::from_wayland_resource(&*s),
                    )
                })
                .unwrap_or_else(smithay::backend::renderer::element::Id::new),
            client.geom,
            output_geo.loc,
            output_scale,
            radius,
            state.config.focuscolor.0,
            prog.0,
        ),
    ));
}

/// `dim_enable` overlay (mango a4764ccf): a translucent rounded fill over a
/// window's content, tinted by `dim_focused_color` / `dim_unfocused_color`,
/// so a window can be shaded without touching the app's own opacity. Never
/// takes input (it's a render element only). Uses a stable per-window id
/// plus a commit derived from the tint, so an unchanged dim reports zero
/// damage instead of repainting every frame.
fn push_dim_overlay(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    window: &smithay::desktop::Window,
    client: &MargoClient,
    is_focused: bool,
    clip_geometry: Option<Rectangle<f64, Logical>>,
    radius: f32,
    output_scale: f64,
    elements: &mut Vec<MargoRenderElement>,
) {
    // Overview already dims non-selected thumbnails; an opening window is
    // drawn scaled by the open animation, so a slot-sized dim would lead it.
    if !state.config.dim_enable || state.is_overview_open() || client.opening_animation.is_some() {
        return;
    }
    let color = if is_focused {
        state.config.dim_focused_color.0
    } else {
        state.config.dim_unfocused_color.0
    };
    if color[3] <= 0.0 {
        return;
    }
    let Some(geo) = clip_geometry else { return };
    let Some(program) = crate::render::rounded_solid::shader(renderer) else {
        return;
    };
    let rect = Rectangle::<i32, smithay::utils::Physical>::new(
        (
            (geo.loc.x * output_scale).round() as i32,
            (geo.loc.y * output_scale).round() as i32,
        )
            .into(),
        (
            ((geo.size.w * output_scale).round() as i32).max(1),
            ((geo.size.h * output_scale).round() as i32).max(1),
        )
            .into(),
    );
    let id = state.window_decoration_id(window, DecorationKind::Dim);
    let tint_key = color.iter().fold(0usize, |acc, c| {
        acc.wrapping_mul(31).wrapping_add(c.to_bits() as usize)
    });
    elements.push(MargoRenderElement::RoundedSolid(
        crate::render::rounded_solid::RoundedSolidElement::new(
            id,
            rect,
            radius * output_scale as f32,
            color,
            program.0,
        )
        .with_commit(smithay::backend::renderer::utils::CommitCounter::from(
            tint_key,
        )),
    ));
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
    only_window: Option<&smithay::desktop::Window>,
    elements: &mut Vec<MargoRenderElement>,
) {
    let scale = Scale::from(output_scale);

    // The keyboard-focused window, resolved once per frame (the lookup is a
    // linear scan) for the `dim_*` tint below.
    let focused_window = state
        .focused_client_idx()
        .and_then(|idx| state.clients.get(idx))
        .map(|c| &c.window);

    // Index clients by window once per output per frame. The per-window
    // body below resolved its client with a linear `clients.iter().find`,
    // making this loop O(windows × clients) every frame — quadratic in the
    // open-window count. The map makes each lookup O(1), and carries the
    // client's index so the radius / group-tab / drag-highlight helpers
    // below don't re-scan `clients` for it either.
    let client_by_window: std::collections::HashMap<_, _> = state
        .clients
        .iter()
        .enumerate()
        .map(|(idx, c)| (&c.window, (idx, c)))
        .collect();

    for window in state.space.elements_for_output(output).rev() {
        if only_window.is_some_and(|only| only != window) {
            continue;
        }
        let Some(location) = state.space.element_location(window) else {
            continue;
        };
        let render_location = location - window.geometry().loc;
        let physical_location = (render_location - output_geo.loc).to_physical_precise_round(scale);

        let client_entry = client_by_window.get(&window).copied();
        let client_idx = client_entry.map(|(idx, _)| idx);
        let client = client_entry.map(|(_, c)| c);

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
        // `focused_opacity` / `unfocused_opacity` (with the focus crossfade),
        // on top of the overview dim. A window cast shows the window itself,
        // so it stays opaque there.
        let window_alpha = match client {
            Some(c) if only_window.is_none() => {
                overview_alpha * crate::animation::client_opacity(c, focused_window == Some(window))
            }
            _ => overview_alpha,
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

        let radius = client_idx
            .map(|idx| state.client_radius(idx))
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

                push_group_tabs(
                    renderer,
                    state,
                    client_entry,
                    output_geo,
                    output_scale,
                    elements,
                );
                push_drag_swap_highlight(
                    renderer,
                    state,
                    client_entry,
                    output_geo,
                    output_scale,
                    elements,
                );

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
                        // Hide the shadow for the duration of an open
                        // transition, same as border.rs's refresh_client
                        // already does for the border and for the same
                        // reason: OpenCloseRenderElement draws the
                        // content scaled around c.geom's centre from
                        // `extreme_scale`, so a full-slot shadow around
                        // it would visibly precede the window into
                        // existence for the whole animation instead of
                        // popping in together with the settled content
                        // at the end, the way border already does.
                        && client.opening_animation.is_none()
                    {
                        if let Some(program) = crate::render::shadow::shader(renderer) {
                            // Follow the client down to its actual live
                            // size, same as the border does -- a floating
                            // window's slot is pinned by its window rule
                            // and doesn't shrink when the client itself
                            // does (mtune switching to a much smaller
                            // mini/strip skin inside a fixed float_geom
                            // was rendering a shadow sized for the full
                            // skin around visibly-smaller content).
                            // Skipped during a resize-snapshot crossfade,
                            // matching border: the snapshot is scaled to
                            // the full slot for that transition, so the
                            // shadow should track the slot too, not the
                            // not-yet-settled live buffer underneath it.
                            let snapshot_active =
                                client.resize_snapshot.is_some() || client.snapshot_pending;
                            let shadow_geom = if snapshot_active {
                                client.geom
                            } else {
                                let actual = window.geometry().size;
                                client.geom.clamped_to_actual_size(actual.w, actual.h)
                            };
                            let win_rect = smithay::utils::Rectangle::new(
                                (
                                    shadow_geom.x - output_geo.loc.x,
                                    shadow_geom.y - output_geo.loc.y,
                                )
                                    .into(),
                                (shadow_geom.width.max(1), shadow_geom.height.max(1)).into(),
                            );
                            // Stable id (reused across frames) so an
                            // unchanged shadow reports zero damage instead
                            // of re-damaging its oversized bbox every frame.
                            let shadow_id =
                                state.window_decoration_id(window, DecorationKind::Shadow);
                            let shadow = crate::render::shadow::ShadowRenderElement::new(
                                shadow_id,
                                win_rect,
                                radius,
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

                // Dim overlay: pushed ahead of every content variant below
                // (open/close, resize snapshot, live surface) so it sits
                // above all of them, but after the border/popups so those
                // stay on top of it. Skipped in screencasts — a capture
                // shouldn't come out darkened.
                if !for_screencast && let Some(client) = client {
                    push_dim_overlay(
                        renderer,
                        state,
                        window,
                        client,
                        focused_window == Some(window),
                        clip_geometry,
                        radius,
                        output_scale,
                        elements,
                    );
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
                                    window_alpha,
                                    anim.kind,
                                    false,
                                    anim.extreme_scale,
                                    smithay::backend::renderer::utils::CommitCounter::default(),
                                    radius,
                                    clipped_surface_program.clone(),
                                )
                                .with_fade(
                                    state.config.animation_fade_in,
                                    state.config.fadein_begin_opacity,
                                ),
                            ));
                        }
                        // capture_pending → emit nothing this frame; next
                        // frame the texture will be ready.
                        continue;
                    }
                }

                // Two-texture niri-style crossfade: if a snapshot
                // is active, capture the live surface tree to an
                // offscreen GlesTexture (`tex_next`), then
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

                    // Live layer → tex_next, through the same
                    // offscreen-render path as tex_prev
                    // (`capture_window`), so both textures share the
                    // same pixel-level layout. Cached on the snapshot
                    // by content key: re-rendered only when the
                    // window committed (or its size / the scale
                    // changed), into the previous texture when the
                    // size holds — not a fresh window-sized texture
                    // every frame, and not again for the screencopy /
                    // cast rebuilds of the same frame. Failure → no
                    // tex_next, ResizeRenderElement falls back to
                    // tex_prev only at full alpha.
                    let live_size = c.window.geometry().size;
                    let live_key = crate::render::window_capture::window_content_key(
                        &c.window,
                        live_size,
                        output_scale.into(),
                    );
                    let tex_next = if live_size.w > 0 && live_size.h > 0 {
                        match snapshot
                            .live
                            .borrow_mut()
                            .get_or_capture(live_key, |reuse| {
                                crate::render::window_capture::capture_window_reusing(
                                    renderer,
                                    &c.window,
                                    live_size,
                                    output_scale.into(),
                                    reuse,
                                )
                            }) {
                            Ok(t) => Some(t),
                            Err(e) => {
                                tracing::trace!("resize_next: live capture failed: {e:?}");
                                None
                            }
                        }
                    } else {
                        None
                    };
                    // The element keeps one id for the whole transition,
                    // so what changes in place — live content and the
                    // crossfade progress — goes into its commit counter.
                    let resize_commit = {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        (live_key, progress.to_bits()).hash(&mut h);
                        smithay::backend::renderer::utils::CommitCounter::from(h.finish() as usize)
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
                            window_alpha,
                            resize_commit,
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
                    window_alpha,
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
                // Only where it can show (see `window_blur_wanted`):
                // never under an opaque window, and never for `no_blur` /
                // scratchpad clients (a blur there would bleed past edges
                // that should feel locked to the screen).
                if let Some(client) = client {
                    if window_blur_wanted(
                        state.config.blur,
                        client.no_blur,
                        client.is_in_scratchpad,
                        window_alpha,
                        || {
                            window.wl_surface().is_some_and(|s| {
                                surface_hides_backdrop(&s, Some(window.geometry()))
                            })
                        },
                    ) {
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
                            let blur_id = state.window_decoration_id(window, DecorationKind::Blur);
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

                push_group_tabs(
                    renderer,
                    state,
                    client_entry,
                    output_geo,
                    output_scale,
                    elements,
                );
                push_drag_swap_highlight(
                    renderer,
                    state,
                    client_entry,
                    output_geo,
                    output_scale,
                    elements,
                );

                if !for_screencast && let Some(client) = client {
                    push_dim_overlay(
                        renderer,
                        state,
                        window,
                        client,
                        focused_window == Some(window),
                        clip_geometry,
                        radius,
                        output_scale,
                        elements,
                    );
                }

                let rendered =
                    AsRenderElements::<GlesRenderer>::render_elements::<
                        WaylandSurfaceRenderElement<GlesRenderer>,
                    >(window, renderer, physical_location, scale, window_alpha);
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
            Some(anim) if !anim.is_close => {
                let p = anim.progress.clamp(0.0, 1.0);
                let faded = if state.config.animation_fade_in {
                    let begin = state.config.fadein_begin_opacity.clamp(0.0, 1.0);
                    begin + (1.0 - begin) * p
                } else {
                    1.0
                };
                alpha * faded
            }
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
        // layer is mid open-animation and under a fully opaque layer
        // (see `layer_blur_wanted`).
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
            if layer_blur_wanted(true, no_blur, mid_anim, layer_alpha, || {
                surface_hides_backdrop(surface.wl_surface(), None)
            }) && crate::render::blur::shader(renderer).is_some()
            {
                let rect = Rectangle::<i32, Logical>::new(
                    (geo.loc.x, geo.loc.y).into(),
                    (geo.size.w.max(1), geo.size.h.max(1)).into(),
                );
                // Stable id (reused across frames) so unchanged layer blur
                // reports zero damage rather than re-damaging every frame.
                let blur_id = state.decoration_element_id(
                    surface.layer_surface().wl_surface(),
                    DecorationKind::Blur,
                );
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
    output_scale: f64,
    clipped_surface_program: Option<GlesTexProgram>,
    roles: &[WlrLayer],
    elements: &mut Vec<MargoRenderElement>,
) {
    let scale = Scale::from(output_scale);
    let target_mon_idx = state.monitors.iter().position(|m| m.output == *output);
    let Some(_target_mon_idx) = target_mon_idx else {
        return;
    };
    let visibility = state.layer_visibility_on_output(output);
    for (_id, anim) in state.layer_animations.iter() {
        if !anim.is_close
            || anim.output != *output
            || !roles.contains(&anim.layer)
            || !visibility.renders(anim.layer)
        {
            continue;
        }
        let Some(texture) = anim.texture.as_ref() else {
            continue;
        };
        let dst = smithay::utils::Rectangle::new(
            (anim.geom.x, anim.geom.y).into(),
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
            )
            .with_fade(
                state.config.animation_fade_out,
                state.config.fadeout_begin_opacity,
            ),
        ));
    }
}

/// Commit counter for an element that keeps one id across frames and can
/// only change in place by colour: equal colours give equal counters, so the
/// damage tracker repaints the element exactly when its colour changed.
fn color_commit(color: [f32; 4]) -> smithay::backend::renderer::utils::CommitCounter {
    let key = color.iter().fold(0usize, |acc, c| {
        acc.wrapping_mul(31).wrapping_add(c.to_bits() as usize)
    });
    smithay::backend::renderer::utils::CommitCounter::from(key)
}

/// First index of the `max`-wide slice of the full `total`-long candidate
/// list to render, so that `selected` always falls inside it (centred when
/// there's room, clamped at either end otherwise). Pure so it's testable
/// without a renderer — see [`build_mru_switcher_elements`]'s call site.
/// Whether `surface` hides everything beneath `rect` (surface-local
/// logical coords): no `wp_alpha_modifier_v1` multiplier below 1, and a
/// committed buffer whose opaque region covers all of `rect` (`None`: the
/// whole surface). An unknown state (no buffer yet) counts as see-through.
fn surface_hides_backdrop(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    rect: Option<Rectangle<i32, Logical>>,
) -> bool {
    use smithay::wayland::alpha_modifier::AlphaModifierSurfaceCachedState;
    let faded = smithay::wayland::compositor::with_states(surface, |states| {
        states
            .cached_state
            .get::<AlphaModifierSurfaceCachedState>()
            .current()
            .multiplier()
            .is_some_and(|m| m < u32::MAX)
    });
    if faded {
        return false;
    }
    smithay::backend::renderer::utils::with_renderer_surface_state(surface, |rs| {
        let Some(size) = rs.surface_size() else {
            return false;
        };
        let rect = rect.unwrap_or_else(|| Rectangle::from_size(size));
        rs.opaque_regions().is_some_and(|regions| {
            Rectangle::subtract_rects_many([rect], regions.iter().copied()).is_empty()
        })
    })
    .unwrap_or(false)
}

/// Whether a window gets a blur element under it. The blur only shows
/// where the window lets the backdrop through: a window drawn below full
/// opacity, or one whose surface is not opaque over its whole geometry
/// (`hides_backdrop`, probed only when the alpha doesn't already decide).
/// Under an opaque window the blur would be invisible yet still re-run its
/// whole capture + Kawase chain whenever something behind the window
/// changes (its rounded clip keeps the window's opaque region from letting
/// the damage tracker cull it). Holds for fullscreen windows too: a
/// translucent fullscreen terminal keeps its blur (mango a30fa7e4), an
/// opaque one keeps the direct-scanout path.
fn window_blur_wanted(
    blur_on: bool,
    no_blur: bool,
    in_scratchpad: bool,
    window_alpha: f32,
    hides_backdrop: impl FnOnce() -> bool,
) -> bool {
    blur_on && !no_blur && !in_scratchpad && (window_alpha < 1.0 || !hides_backdrop())
}

/// Whether a layer surface gets a blur element under it: `blur_layer` on,
/// no `layerrule = noblur:1`, not mid open-animation (the blur would flash
/// in ahead of the fading surface), and the layer lets the backdrop
/// through somewhere (drawn below full alpha, or not opaque everywhere).
fn layer_blur_wanted(
    blur_layer_on: bool,
    no_blur_rule: bool,
    mid_open_anim: bool,
    layer_alpha: f32,
    hides_backdrop: impl FnOnce() -> bool,
) -> bool {
    blur_layer_on && !no_blur_rule && !mid_open_anim && (layer_alpha < 1.0 || !hides_backdrop())
}

fn mru_window_start(selected: usize, total: usize, max: usize) -> usize {
    if total <= max {
        0
    } else {
        selected.saturating_sub(max / 2).min(total - max)
    }
}

#[cfg(test)]
mod mru_window_tests {
    use super::mru_window_start;

    #[test]
    fn fits_entirely_when_under_the_cap() {
        assert_eq!(mru_window_start(0, 5, 10), 0);
        assert_eq!(mru_window_start(4, 5, 10), 0);
    }

    #[test]
    fn centres_on_selection_away_from_either_edge() {
        // 50 candidates, cap 10: selecting #25 should show roughly #20..#30.
        let start = mru_window_start(25, 50, 10);
        assert_eq!(start, 20);
        assert!((start..start + 10).contains(&25));
    }

    #[test]
    fn clamps_at_the_start_without_underflow() {
        assert_eq!(mru_window_start(0, 50, 10), 0);
        assert_eq!(mru_window_start(2, 50, 10), 0);
    }

    #[test]
    fn clamps_at_the_end_without_running_past_total() {
        // Selecting the very last candidate must still show it — this is
        // exactly the case the old flat `.take(max)` got wrong: `selected`
        // (49) landed far past the rendered window (0..10).
        let start = mru_window_start(49, 50, 10);
        assert_eq!(start, 40);
        assert!((start..start + 10).contains(&49));
    }

    #[test]
    fn selection_always_lands_inside_the_window_across_a_full_cycle() {
        let (total, max) = (37usize, 6usize);
        for selected in 0..total {
            let start = mru_window_start(selected, total, max);
            assert!(
                (start..start + max).contains(&selected),
                "selected={selected} start={start} max={max}"
            );
            assert!(start + max <= total, "window must not run past `total`");
        }
    }
}

#[cfg(test)]
mod overview_cull_tests {
    use super::overview_cell_visible;
    use crate::layout::Rect;
    use crate::state::overview_cells;

    const OUT: Rect = Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1440,
    };

    #[test]
    fn only_the_cells_on_screen_are_built() {
        // 9 occupied tags at zoom 0.3 (432 px cells, 40 px gap), the 5th
        // centred: about three cells are on screen.
        let tags: Vec<usize> = (1..=9).collect();
        let cells = overview_cells(OUT, &tags, 0.3, 40, 4.0, false);
        assert_eq!(cells.len(), 9);
        let mut shown = cells.clone();
        shown.retain(|c| overview_cell_visible(c.rect, OUT, 40));
        assert!(shown.len() < cells.len());
        for c in &cells {
            // On screen, or within one gap of it.
            let on_screen =
                c.rect.y + c.rect.height + 40 > OUT.y && c.rect.y - 40 < OUT.y + OUT.height;
            assert_eq!(
                shown.iter().any(|s| s == c),
                on_screen,
                "cell for tag {} at y={}",
                c.tag,
                c.rect.y
            );
        }
        // The centred tag and its neighbours, keys untouched.
        assert!(shown.iter().any(|c| c.tag == 5 && c.key == 4));
    }

    #[test]
    fn a_cell_peeking_in_by_less_than_the_margin_is_kept() {
        let tags = [1, 2, 3];
        // 720 px cells, 760 px stride: cell 1's top sits 20 px below the
        // screen edge (inside one 40 px gap), cell 2 is far off.
        let cells = overview_cells(OUT, &tags, 0.5, 40, 1.0 - 1100.0 / 760.0, false);
        assert_eq!(cells[1].rect.y, 1460);
        let mut shown = cells.clone();
        shown.retain(|c| overview_cell_visible(c.rect, OUT, 40));
        assert_eq!(shown, cells[..2]);
    }

    #[test]
    fn the_wrap_around_window_is_cut_to_the_screen_too() {
        let tags: Vec<usize> = (1..=9).collect();
        let cells = overview_cells(OUT, &tags, 0.3, 40, 0.0, true);
        let mut shown = cells.clone();
        shown.retain(|c| overview_cell_visible(c.rect, OUT, 40));
        assert!(
            shown.len() < cells.len(),
            "{} of {}",
            shown.len(),
            cells.len()
        );
        assert!(
            shown
                .iter()
                .all(|c| c.rect.y + c.rect.height + 40 > 0 && c.rect.y - 40 < 1440)
        );
    }
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

#[cfg(test)]
mod blur_gate_tests {
    use super::{layer_blur_wanted, window_blur_wanted};

    fn never() -> bool {
        panic!("opacity probed when the answer was already known")
    }

    #[test]
    fn an_opaque_window_gets_no_blur() {
        // A focused, fully opaque browser: the blur would be invisible
        // and re-run its whole Kawase chain every time the page changes.
        assert!(!window_blur_wanted(true, false, false, 1.0, || true));
    }

    #[test]
    fn a_translucent_or_faded_window_keeps_its_blur() {
        // Alpha buffer (terminal with a transparent background).
        assert!(window_blur_wanted(true, false, false, 1.0, || false));
        // Opaque buffer drawn at `unfocused_opacity` / overview dim.
        assert!(window_blur_wanted(true, false, false, 0.8, never));
    }

    #[test]
    fn rules_and_the_global_switch_win() {
        assert!(!window_blur_wanted(false, false, false, 0.5, never));
        assert!(!window_blur_wanted(true, true, false, 0.5, never));
        assert!(!window_blur_wanted(true, false, true, 0.5, never));
    }

    #[test]
    fn an_opaque_layer_gets_no_blur() {
        assert!(!layer_blur_wanted(true, false, false, 1.0, || true));
        assert!(layer_blur_wanted(true, false, false, 1.0, || false));
        // An opaque bar faded by the caller's alpha shows the backdrop.
        assert!(layer_blur_wanted(true, false, false, 0.7, never));
        assert!(!layer_blur_wanted(true, true, false, 0.7, never));
        assert!(!layer_blur_wanted(true, false, true, 0.7, never));
        assert!(!layer_blur_wanted(false, false, false, 0.7, never));
    }
}

#[cfg(test)]
mod overlay_commit_tests {
    use super::color_commit;

    #[test]
    fn color_commit_changes_only_with_the_colour() {
        let a = [0.0, 0.0, 0.0, 0.5];
        // Same colour across frames → same counter → no damage for a
        // stable-id element.
        assert_eq!(color_commit(a), color_commit(a));
        assert_ne!(color_commit(a), color_commit([0.0, 0.0, 0.0, 0.6]));
        assert_ne!(color_commit(a), color_commit([0.1, 0.0, 0.0, 0.5]));
    }
}
