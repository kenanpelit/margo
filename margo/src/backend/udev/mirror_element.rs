//! The render element a mirror target is drawn with: one element of the
//! source output's scene, scaled by the letterbox fit and moved into the
//! letterbox (see [`crate::mirror::letterbox`]).
//!
//! It wraps a [`MargoRenderElement`] and is itself a variant of it, so the
//! mirror frame runs through the very same `render_output` path (damage
//! tracking, page flip, presentation feedback). That recursion is why the
//! inner element is boxed.

use smithay::backend::renderer::element::utils::{
    Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform};

use super::MargoRenderElement;
use crate::mirror::Letterbox;

type Inner = RelocateRenderElement<RescaleRenderElement<MargoRenderElement>>;

pub struct MirroredElement(Box<Inner>, bool);

impl std::fmt::Debug for MirroredElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MirroredElement").finish()
    }
}

impl MirroredElement {
    /// `element` from the source scene, placed into `fit` on the target.
    pub fn new(element: MargoRenderElement, fit: Letterbox) -> Self {
        let blur = matches!(element, MargoRenderElement::Blur(_));
        let scaled = RescaleRenderElement::from_element(element, Point::from((0, 0)), fit.scale);
        let placed = RelocateRenderElement::from_element(
            scaled,
            Point::<i32, Physical>::from(fit.offset),
            Relocate::Relative,
        );
        Self(Box::new(placed), blur)
    }

    /// The wrapped element is a blur (samples the framebuffer behind it).
    pub fn is_blur(&self) -> bool {
        self.1
    }
}

impl Element for MirroredElement {
    fn id(&self) -> &Id {
        self.0.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.0.current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.0.location(scale)
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.0.src()
    }
    fn transform(&self) -> Transform {
        self.0.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.0.geometry(scale)
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.0.damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.0.opaque_regions(scale)
    }
    fn alpha(&self) -> f32 {
        self.0.alpha()
    }
    fn kind(&self) -> Kind {
        self.0.kind()
    }
    fn is_framebuffer_effect(&self) -> bool {
        self.0.is_framebuffer_effect()
    }
}

impl RenderElement<GlesRenderer> for MirroredElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        self.0.draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.0.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        self.0.capture_framebuffer(frame, src, dst, cache)
    }
}
