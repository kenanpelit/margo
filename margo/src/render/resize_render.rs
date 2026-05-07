//! Niri-style resize animation render element.
//!
//! When the layout slot for a window changes size, we capture the
//! window's current rendering as a `GlesTexture` (via
//! [`crate::render::window_capture::capture_window`]) and store it
//! alongside the source size. While the move animation interpolates
//! the slot from the old rect to the new rect, this render element
//! draws the captured texture scaled to whatever the slot is *now*.
//!
//! When the animation finishes (or the client commits a fresh buffer
//! matching the new size, whichever comes first) we drop the snapshot
//! and go back to drawing the live surface.
//!
//! This is the bare-bones port of niri's `ResizeRenderElement`. It
//! doesn't yet do the prev/next crossfade niri does — only the
//! single-texture scaling, which already removes the visible "buffer
//! is the wrong size for its slot" jitter that survives our
//! size-snap fix.

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::Texture;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Transform};

#[derive(Debug)]
pub struct ResizeRenderElement {
    /// Stable identity, derived from the source surface so smithay's
    /// damage tracker can match the element across frames.
    id: Id,
    /// The captured window content. Scales to fit `geometry`.
    texture: GlesTexture,
    /// Where to render the texture (logical coordinates), updated each
    /// frame from the live (interpolated) layout slot.
    geometry: Rectangle<i32, Logical>,
    /// Output scale, propagated to physical conversions inside
    /// `Element::geometry`.
    scale: Scale<f64>,
    /// Bumped every time `geometry` changes so smithay re-damages.
    commit: CommitCounter,
    /// Per-frame opacity. 1.0 during the resize animation; the caller
    /// can fade it out at the end if they want a crossfade with the
    /// live surface.
    alpha: f32,
}

impl ResizeRenderElement {
    pub fn new(
        id: Id,
        texture: GlesTexture,
        geometry: Rectangle<i32, Logical>,
        scale: Scale<f64>,
        alpha: f32,
        commit: CommitCounter,
    ) -> Self {
        Self {
            id,
            texture,
            geometry,
            scale,
            commit,
            alpha,
        }
    }
}

impl Element for ResizeRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        let size = self.texture.size();
        Rectangle::new((0.0, 0.0).into(), (size.w as f64, size.h as f64).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(self.scale)
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit != Some(self.commit) {
            DamageSet::from_slice(&[Rectangle::new(
                Point::default(),
                self.geometry(scale).size,
            )])
        } else {
            DamageSet::default()
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // The captured texture has alpha (we ask for Abgr8888) and the
        // resize animation uses fractional alpha; treat as fully
        // translucent so the compositor doesn't skip drawing what's
        // behind us.
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for ResizeRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // Smithay's `render_texture_from_to` scales the texture
        // sample-rect (`src`, in buffer coords) into the destination
        // rect (`dst`, in physical coords) — exactly what we need: the
        // captured snapshot at its native pixel size, drawn into the
        // current animated slot in physical screen coords.
        smithay::backend::renderer::Frame::render_texture_from_to(
            frame,
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha,
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // No matching wl_buffer — the texture lives entirely on the
        // GPU side. This disables direct-scanout for resize-animated
        // windows, which is fine: the animation is short and the
        // snapshot is mid-flight by definition.
        None
    }
}
