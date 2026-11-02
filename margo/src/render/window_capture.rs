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
//! tracking. Callers that re-capture the same window repeatedly (the
//! resize crossfade's live layer, MRU thumbnails) keep a
//! [`CaptureSlot`] keyed by [`window_content_key`], so a capture only
//! re-renders when the window committed new content, and reuses the
//! previous texture's storage when the size is unchanged.

use std::hash::{Hash, Hasher};

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::gles::{GlesError, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::{CommitCounter, RendererSurfaceStateUserData};
use smithay::backend::renderer::{Bind, Color32F, Offscreen, Texture};
use smithay::desktop::Window;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer as BufferCoord, Logical, Physical, Scale, Size, Transform};
use smithay::wayland::compositor::{TraversalAction, with_surface_tree_downward};
use smithay::wayland::seat::WaylandFocus;

use drm_fourcc::DrmFourcc;

/// One cached capture, re-rendered only when its content key changes.
#[derive(Debug)]
pub struct CaptureSlot<T> {
    entry: Option<(u64, T)>,
}

impl<T> Default for CaptureSlot<T> {
    fn default() -> Self {
        Self { entry: None }
    }
}

impl<T: Clone> CaptureSlot<T> {
    /// The capture for `key`: the cached one when the key matches, else
    /// `capture(previous)` — `previous` is the stale capture, handed over
    /// so its storage can be reused. A failed capture empties the slot.
    pub fn get_or_capture<E>(
        &mut self,
        key: u64,
        capture: impl FnOnce(Option<T>) -> Result<T, E>,
    ) -> Result<T, E> {
        if let Some((cached_key, value)) = &self.entry
            && *cached_key == key
        {
            return Ok(value.clone());
        }
        let previous = self.entry.take().map(|(_, value)| value);
        let value = capture(previous)?;
        self.entry = Some((key, value.clone()));
        Ok(value)
    }

    /// Key of the cached capture, if any.
    pub fn key(&self) -> Option<u64> {
        self.entry.as_ref().map(|(key, _)| *key)
    }
}

/// Hash of everything a capture of `window` at `size` × `scale` depends
/// on: the size, the scale, and the commit counter of every surface in
/// the window's tree. Equal keys mean an identical capture.
pub fn window_content_key(window: &Window, size: Size<i32, Logical>, scale: Scale<f64>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (size.w, size.h, scale.x.to_bits(), scale.y.to_bits()).hash(&mut hasher);
    if let Some(surface) = window.wl_surface() {
        with_surface_tree_downward(
            &surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |surface, states, _| {
                surface.id().hash(&mut hasher);
                if let Some(state) = states.data_map.get::<RendererSurfaceStateUserData>()
                    && let Ok(state) = state.lock()
                {
                    // `CommitCounter` has no accessor for its value; its
                    // distance from zero is that value.
                    state
                        .current_commit()
                        .distance(Some(CommitCounter::default()))
                        .hash(&mut hasher);
                }
            },
            |_, _, _| true,
        );
    }
    hasher.finish()
}

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
    let buffer_size: Size<i32, BufferCoord> =
        (physical_size.w.max(1), physical_size.h.max(1)).into();

    let mut texture: GlesTexture = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
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
        .render_output(renderer, &mut target, 0, &elements, Color32F::TRANSPARENT)
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

/// [`capture_window`] into `reuse` when it already has the right size,
/// instead of allocating a new offscreen texture. Rendering clears the
/// whole texture first (age 0), so nothing of the old content survives.
/// GL executes commands in order, so a frame that still samples the old
/// content was submitted before this overwrite and is unaffected.
pub fn capture_window_reusing(
    renderer: &mut GlesRenderer,
    window: &Window,
    size: Size<i32, Logical>,
    scale: Scale<f64>,
    reuse: Option<GlesTexture>,
) -> Result<GlesTexture, GlesError> {
    let physical_size: Size<i32, Physical> = size.to_physical_precise_round(scale);
    let buffer_size: Size<i32, BufferCoord> =
        (physical_size.w.max(1), physical_size.h.max(1)).into();
    let Some(mut texture) = reuse.filter(|texture| texture.size() == buffer_size) else {
        return capture_window(renderer, window, size, scale);
    };
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
        .render_output(renderer, &mut target, 0, &elements, Color32F::TRANSPARENT)
        .map_err(|err| match err {
            smithay::backend::renderer::damage::Error::Rendering(e) => e,
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

    let mut texture: GlesTexture = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
        renderer,
        DrmFourcc::Abgr8888,
        buffer_size,
    )?;

    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(renderer, surface, (0, 0), scale, 1.0, Kind::Unspecified);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_slot_recaptures_only_when_the_key_changes() {
        let mut slot = CaptureSlot::<u32>::default();
        let mut calls = Vec::new();
        let mut get = |slot: &mut CaptureSlot<u32>, key: u64, value: u32| {
            slot.get_or_capture(key, |previous| {
                calls.push(previous);
                Ok::<_, ()>(value)
            })
        };
        assert_eq!(get(&mut slot, 1, 10), Ok(10));
        // Same content (e.g. the screencopy rebuild in the same frame, or a
        // static window on the next frame): served from the cache.
        assert_eq!(get(&mut slot, 1, 11), Ok(10));
        assert_eq!(get(&mut slot, 1, 12), Ok(10));
        // New commit: re-captured, handing the old texture over for reuse.
        assert_eq!(get(&mut slot, 2, 20), Ok(20));
        assert_eq!(calls, [None, Some(10)]);
        assert_eq!(slot.key(), Some(2));
    }

    #[test]
    fn failed_capture_empties_the_slot() {
        let mut slot = CaptureSlot::<u32>::default();
        assert_eq!(slot.get_or_capture(1, |_| Ok::<_, ()>(1)), Ok(1));
        assert_eq!(slot.get_or_capture(2, |_| Err::<u32, _>(())), Err(()));
        assert_eq!(slot.key(), None);
        // Next attempt with the old key captures again rather than serving
        // stale content.
        let mut called = false;
        assert_eq!(
            slot.get_or_capture(1, |previous| {
                called = true;
                assert!(previous.is_none());
                Ok::<_, ()>(5)
            }),
            Ok(5)
        );
        assert!(called);
    }
}
