//! Render-element construction for the udev (DRM/GBM) backend.
//!
//! Extracted from `backend/udev/mod.rs` (god-file split): everything that turns
//! the compositor scene into a `Vec<MargoRenderElement>` for a frame — the
//! `RenderTarget` mode, the top-level `build_render_elements[_inner]` and the
//! desktop scene, here; cursor (`cursor`), MRU-switcher / scroller-overview
//! overlays (`overview`), the per-client / per-layer push helpers (`clients`,
//! `layers`), and the mirror / screencast / image-copy paths (`capture`) in
//! submodules. Pure free functions over `&MargoState` + the GLES renderer;
//! `mod.rs` keeps backend setup (`run`), capture queueing, and
//! `serve_screencopies`.

use super::*;
use crate::state::DecorationKind;
use clients::{push_client_elements, push_closing_clients};
use cursor::push_cursor_elements;
use layers::{push_closing_layers, push_layer_elements};
use overview::{build_mru_switcher_elements, build_scroller_overview_elements};

mod capture;
mod clients;
mod cursor;
mod layers;
mod overview;

#[cfg(feature = "xdp-gnome-screencast")]
pub(super) use capture::drain_active_cast_frames;
pub(super) use capture::{
    build_mirror_frame, capture_scene, drain_image_copy_frames, mirror_source_of,
};

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
    if !state.session_locked || state.lock_shows_desktop(&od.output) {
        let wallpaper = crate::wallpaper::runtime::scene_elements(renderer, state, &od.output);
        elements.extend(wallpaper.into_iter().map(Into::into));
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
