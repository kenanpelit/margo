//! Capture a window's current rendered content as a `GlesTexture`.
//!
//! Used by the resize animation: at the moment the layout slot size
//! changes we snapshot the live surface tree off-screen and keep that
//! texture around. Subsequent renders draw the texture scaled to the
//! interpolated slot until the client commits a buffer at the new
//! size, at which point we drop the snapshot and go back to drawing
//! the live surface.
//!
//! Mirrors niri's `OffscreenBuffer::render` but bare-bones: no damage
//! tracking, no caching across frames — we throw the texture away when
//! the resize animation ends.

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesError, GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, Color32F, Offscreen};
use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer as BufferCoord, Logical, Physical, Scale, Size, Transform};
use smithay::wayland::seat::WaylandFocus;

use drm_fourcc::DrmFourcc;

/// Render the given `window`'s entire surface tree (the toplevel plus
/// any subsurfaces) into a fresh `GlesTexture`. The texture is sized
/// to fit the window at `size` logical pixels, scaled by `scale`.
///
/// `Err` on either offscreen-buffer creation or render failure — the
/// caller should fall back to rendering the live surface.
pub fn capture_window(
    renderer: &mut GlesRenderer,
    window: &Window,
    size: Size<i32, Logical>,
    scale: Scale<f64>,
) -> Result<GlesTexture, GlesError> {
    // Buffer-coords for the offscreen target.
    let physical_size: Size<i32, Physical> = size.to_physical_precise_round(scale);
    let buffer_size: Size<i32, BufferCoord> = (physical_size.w.max(1), physical_size.h.max(1)).into();

    let mut texture: GlesTexture =
        <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
            renderer,
            DrmFourcc::Abgr8888,
            buffer_size,
        )?;

    // Pull the surface tree's render elements. The toplevel's
    // wl_surface anchors them at (0, 0) — exactly what we want for the
    // off-screen snapshot.
    let Some(wl_surface) = window.wl_surface() else {
        return Ok(texture);
    };
    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(
            renderer,
            &wl_surface,
            (0, 0),
            scale,
            1.0,
            Kind::Unspecified,
        );

    let mut target = renderer.bind(&mut texture)?;
    let mut tracker = OutputDamageTracker::new(physical_size, scale, Transform::Normal);
    tracker
        .render_output(
            renderer,
            &mut target,
            0,
            &elements,
            Color32F::TRANSPARENT,
        )
        .map_err(|err| match err {
            smithay::backend::renderer::damage::Error::Rendering(e) => e,
            // Output damage tracker can also fail on output-not-bound
            // / output-mismatch, but we just bound above. Treat
            // anything else as a recoverable failure by returning a
            // generic GL error — the caller falls back to drawing the
            // live surface.
            _ => GlesError::UnknownPixelFormat,
        })?;
    drop(target);

    Ok(texture)
}

/// Same as [`capture_window`] but takes a raw `wl_surface` — used by
/// the close animation, where the smithay `Window` wrapper is already
/// gone (we removed the client from the layout's `clients` vec) but
/// the bare `wl_surface` is still alive long enough to grab one final
/// frame. Identical body to `capture_window` minus the `Window::wl_surface()`
/// indirection; if you change one, change the other.
pub fn capture_surface(
    renderer: &mut GlesRenderer,
    surface: &WlSurface,
    size: Size<i32, Logical>,
    scale: Scale<f64>,
) -> Result<GlesTexture, GlesError> {
    let physical_size: Size<i32, Physical> = size.to_physical_precise_round(scale);
    let buffer_size: Size<i32, BufferCoord> =
        (physical_size.w.max(1), physical_size.h.max(1)).into();

    let mut texture: GlesTexture =
        <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
            renderer,
            DrmFourcc::Abgr8888,
            buffer_size,
        )?;

    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(
            renderer,
            surface,
            (0, 0),
            scale,
            1.0,
            Kind::Unspecified,
        );

    let mut target = renderer.bind(&mut texture)?;
    let mut tracker = OutputDamageTracker::new(physical_size, scale, Transform::Normal);
    tracker
        .render_output(renderer, &mut target, 0, &elements, Color32F::TRANSPARENT)
        .map_err(|err| match err {
            smithay::backend::renderer::damage::Error::Rendering(e) => e,
            _ => GlesError::UnknownPixelFormat,
        })?;
    drop(target);

    Ok(texture)
}
