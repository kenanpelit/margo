//! Per-client elements: window surfaces with borders, shadows, blur,
//! open/close/resize animations, group tabs, the drag-swap highlight,
//! the dim overlay and closing clients' snapshots.

use super::*;

/// Push render elements for windows in their close animation. Mirrors
/// `push_client_elements` but operates on `state.closing_clients`
/// (entries that survived `toplevel_destroyed` to play their fade-out)
/// instead of mapped clients.
pub(super) fn push_closing_clients(
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

pub(super) fn push_client_elements(
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
