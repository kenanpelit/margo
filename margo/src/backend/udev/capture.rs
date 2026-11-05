//! Offscreen captures the frame loop serves between repaints: window
//! snapshots for open/close/resize animations, MRU switcher thumbnails and
//! wlr-screencopy requests.

use smithay::{
    backend::renderer::gles::GlesRenderer,
    utils::{Physical, Point, Transform},
};
use tracing::warn;

use super::{MargoRenderElement, OutputDevice, capture_scene, helpers, monotonic_now};
use crate::state::MargoState;

// ── Hotplug rescan ──────────────────────────────────────────────────────────
//
// Implementation extracted to `hotplug.rs` (W4.1).

// ── Per-frame render ──────────────────────────────────────────────────────────

/// Drain `snapshot_pending` flags by capturing the live surface tree
/// of each affected client into a `GlesTexture`, stored in
/// `client.resize_snapshot`. Called once per frame before the render
/// element collection runs, so the rest of the frame can read the
/// snapshot from `state.clients` immutably.
pub(in crate::backend) fn take_pending_snapshots(
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
                    live: Default::default(),
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
pub(in crate::backend) fn take_pending_open_close_captures(
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
            if a.is_close && a.capture_pending && a.output == od.output {
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
/// is why they used to draw blank until you cycled onto them). Capturing here
/// (not mid-element-build) keeps the renderer binding sane — same call site as
/// `take_pending_snapshots`.
///
/// Runs on every rendered frame of every output while the switcher is open,
/// so each thumbnail is cached by content key and re-captured only when its
/// window committed (or resized). Captures use the largest enabled output
/// scale, not this output's, so outputs with different scales share one
/// capture instead of re-capturing in turn.
pub(in crate::backend) fn take_mru_thumbnails(
    renderer: &mut GlesRenderer,
    _od: &OutputDevice,
    state: &mut MargoState,
) {
    if state.mru_switcher.is_none() {
        return;
    }
    let capture_scale: smithay::utils::Scale<f64> = state
        .monitors
        .iter()
        .filter(|monitor| monitor.enabled)
        .map(|monitor| monitor.output.current_scale().fractional_scale())
        .fold(1.0, f64::max)
        .into();
    let Some(sw) = state.mru_switcher.as_mut() else {
        return;
    };
    for win in sw.candidates.clone() {
        let size = win.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            continue;
        }
        let key = crate::render::window_capture::window_content_key(&win, size, capture_scale);
        let thumb = sw.thumb_mut(&win);
        if thumb.capture.key() == Some(key) {
            continue;
        }
        if let Err(e) = thumb.capture.get_or_capture(key, |reuse| {
            crate::render::window_capture::capture_window_reusing(
                renderer,
                &win,
                size,
                capture_scale,
                reuse,
            )
        }) {
            tracing::debug!("mru thumbnail capture failed: {e:?}");
        }
    }
}

/// Drain the wlr-screencopy frames pending for `od.output` and write the
/// rendered pixels into each client's buffer (dmabuf zero-copy or SHM
/// read-back).
///
/// The capture element list is built once per cursor variant per frame
/// (and the display list is reused outright when a cursor capture needs
/// no screencast blackout). Each queue keeps a persistent damage
/// tracker, so `copy_with_damage` clients get the region that actually
/// changed since their previous capture — and wait, as the protocol
/// intends, while nothing changed — instead of a full-frame damage
/// every frame that made encoders re-encode the whole screen.
pub(in crate::backend) fn serve_screencopies(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    mirror_source: Option<&OutputDevice>,
    state: &mut MargoState,
    elements: &[MargoRenderElement],
) {
    use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};

    let now = monotonic_now();
    let output = od.output.clone();

    let (want_cursor, want_plain) = state.protocols.screencopy.wanted_for(&output);
    if !want_cursor && !want_plain {
        return;
    }

    // An output without a mode can't be captured: fail its queued frames.
    if od.output.current_mode().is_none() {
        state.protocols.screencopy.remove_output(&output); // → `failed`
        return;
    }
    let scale = od.output.current_scale().fractional_scale();

    // `block_out_from_screencast` windows must be blacked out in every
    // capture, so the display list (which shows them) is only reusable
    // for cursor captures when no such window exists.
    let any_blocked = state.clients.iter().any(|c| c.block_out_from_screencast);
    let cursor_elements = (want_cursor && any_blocked)
        .then(|| capture_scene(renderer, od, mirror_source, state, true));
    let plain_elements =
        want_plain.then(|| capture_scene(renderer, od, mirror_source, state, false));
    let elements_for = |overlay_cursor: bool| -> &[MargoRenderElement] {
        let built = if overlay_cursor {
            cursor_elements.as_deref()
        } else {
            plain_elements.as_deref()
        };
        built.unwrap_or(elements)
    };

    let ready = state.protocols.screencopy.take_ready(&output, scale, |sc| {
        let region_loc = sc.region_loc();
        let translate = Point::<i32, Physical>::from((-region_loc.x, -region_loc.y));
        elements_for(sc.overlay_cursor())
            .iter()
            .map(|e| RelocateRenderElement::from_element(e, translate, Relocate::Relative))
            .collect()
    });

    for (screencopy, damage) in ready {
        let elements = elements_for(screencopy.overlay_cursor());
        render_screencopy(renderer, screencopy, elements, scale, &damage, now);
    }
}

/// Render one screencopy into its buffer, report `damage` (buffer
/// coordinates) when the client asked for it, and submit.
fn render_screencopy(
    renderer: &mut GlesRenderer,
    screencopy: crate::protocols::screencopy::Screencopy,
    elements_to_render: &[MargoRenderElement],
    scale: f64,
    damage: &[smithay::utils::Rectangle<i32, Physical>],
    now: std::time::Duration,
) {
    use smithay::backend::renderer::{
        Bind, ExportMem, damage::OutputDamageTracker as DamageTracker,
    };
    let size = screencopy.buffer_size();
    let region_loc = screencopy.region_loc();
    let report_damage = |screencopy: &crate::protocols::screencopy::Screencopy| {
        if screencopy.with_damage() {
            screencopy.damage(damage.iter().map(|r| {
                smithay::utils::Rectangle::new(
                    smithay::utils::Point::from((r.loc.x, r.loc.y)),
                    smithay::utils::Size::from((r.size.w, r.size.h)),
                )
            }));
        }
    };

    // ── DMA-BUF zero-copy fast path ─────────────────────────────
    // OBS / Discord / xdg-desktop-portal-wlr negotiate dmabuf via
    // `frame.linux_dmabuf(...)` and submit a GBM-allocated wl_buffer
    // sized to the full output. That's the cheap case: bind the
    // dmabuf as a render target and let smithay's damage tracker
    // paint the elements straight into it. No CPU readback, no SHM
    // upload — the screencast pipeline gets a buffer the GPU has
    // already touched.
    if let crate::protocols::screencopy::ScreencopyBuffer::Dmabuf(dmabuf) = screencopy.buffer() {
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
                    .map(|_| ());
                drop(target);
                res
            }
            Err(e) => Err(smithay::backend::renderer::damage::Error::Rendering(e)),
        };

        match render_result {
            Ok(()) => {
                report_damage(&screencopy);
                // Implicit-sync dmabuf submit. Every consumer
                // we've tested (xdg-desktop-portal-wlr, OBS,
                // wf-recorder, grim) is happy with this; the
                // explicit-sync path comes back when DRM
                // syncobj fences are wired into screencopy.
                screencopy.submit_now(false, now);
            }
            Err(e) => warn!("screencopy: dmabuf render failed: {e:?}"),
        }
        return;
    }

    // ── SHM path (renderbuffer + read-back + memcpy) ─────────────
    // Render only the requested region, the same way as the dmabuf path:
    // shift the world by -region_loc and run the damage tracker at the
    // client's buffer size, so a region capture doesn't pay for a
    // full-output render. The renderbuffer comes from a size-keyed pool
    // instead of being allocated per captured frame.
    use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
    let buf_size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from((size.w, size.h));
    let translate: smithay::utils::Point<i32, smithay::utils::Physical> =
        (-region_loc.x, -region_loc.y).into();
    let relocated: Vec<RelocateRenderElement<&MargoRenderElement>> = elements_to_render
        .iter()
        .map(|e| RelocateRenderElement::from_element(e, translate, Relocate::Relative))
        .collect();

    let mut renderbuffer =
        match crate::render::offscreen_pool::take_capture_renderbuffer(renderer, buf_size) {
            Ok(rb) => rb,
            Err(e) => {
                warn!("screencopy: create_buffer failed: {e:?}");
                return;
            }
        };

    let mut target = match renderer.bind(&mut renderbuffer) {
        Ok(t) => t,
        Err(e) => {
            warn!("screencopy: bind renderbuffer failed: {e:?}");
            return;
        }
    };

    let mut tracker = DamageTracker::new(size, scale, Transform::Normal);
    let render_result =
        tracker.render_output(renderer, &mut target, 0, &relocated, [0.0, 0.0, 0.0, 1.0]);
    if let Err(e) = render_result {
        warn!("screencopy: render_output failed: {e:?}");
        return;
    }

    let region = smithay::utils::Rectangle::new(
        smithay::utils::Point::<i32, smithay::utils::Buffer>::from((0, 0)),
        buf_size,
    );
    let mapping = match renderer.copy_framebuffer(&target, region, drm_fourcc::DrmFourcc::Xrgb8888)
    {
        Ok(m) => m,
        Err(e) => {
            warn!("screencopy: copy_framebuffer failed: {e:?}");
            return;
        }
    };
    // Drop the bind so map_texture can re-bind the GL state.
    drop(target);
    let pixels = match renderer.map_texture(&mapping) {
        Ok(p) => p,
        Err(e) => {
            warn!("screencopy: map_texture failed: {e:?}");
            return;
        }
    };
    // The read-back is complete; the renderbuffer can serve the next capture.
    crate::render::offscreen_pool::return_capture_renderbuffer(buf_size, renderbuffer);

    // Write into the SHM buffer. The dmabuf branch is handled
    // earlier in the loop with a zero-copy bind+render path; by
    // the time we reach this match the buffer must be SHM. The
    // Dmabuf arm is unreachable but kept exhaustive so adding a
    // future ScreencopyBuffer variant fails to compile here.
    let copied = match screencopy.buffer() {
        crate::protocols::screencopy::ScreencopyBuffer::Shm(buf) => {
            match smithay::wayland::shm::with_buffer_contents_mut(
                buf,
                |pool_ptr, pool_len, meta| {
                    // SAFETY: pool_ptr/pool_len are the client's validated wl_shm
                    // pool mapping, valid for this callback; the helper writes
                    // only inside this buffer's own bytes (`meta.offset`,
                    // `meta.stride`) and `pixels` is our own readback.
                    unsafe {
                        helpers::copy_frame_into_shm_pool(
                            pool_ptr,
                            pool_len,
                            &meta,
                            (size.w.max(0) as usize).saturating_mul(4),
                            size.h.max(0) as usize,
                            &pixels,
                        )
                    }
                },
            ) {
                Ok(copied) => copied,
                Err(e) => {
                    warn!("screencopy: shm map failed: {e:?}");
                    false
                }
            }
        }
        crate::protocols::screencopy::ScreencopyBuffer::Dmabuf(_) => {
            // Should not happen: handled earlier and returned.
            unreachable!("screencopy dmabuf path took the SHM branch");
        }
    };

    if !copied {
        // Drop without submit → frame.failed() fires from `impl Drop`.
        return;
    }

    report_damage(&screencopy);
    // SHM: synchronous, no GPU sync needed before submit.
    screencopy.submit_now(false, now);
}
