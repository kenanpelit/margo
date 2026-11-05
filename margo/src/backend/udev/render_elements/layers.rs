//! Layer-shell elements: layer surfaces with their blur/shadow/animation
//! and the snapshots of closing layers.

use super::*;

pub(super) fn push_layer_elements(
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
pub(super) fn push_closing_layers(
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
