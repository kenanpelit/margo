//! The render element a mirror target is drawn with: one element of the
//! source output's scene, scaled by the letterbox fit and moved into the
//! letterbox (see [`crate::mirror::letterbox`]).
//!
//! It is itself a variant of [`MargoRenderElement`], so the mirror frame
//! runs through the very same `render_output` path (damage tracking, page
//! flip, presentation feedback). It refers to its source element by index
//! into the source scene, which the source output rendered from in the same
//! batch and both outputs share through an `Rc` ([`SharedScene`]): the
//! scene is built once per batch, and nothing is cloned or boxed per
//! element.

use std::rc::Rc;

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

/// One output's built scene, shared between its own render and the mirror
/// target's.
pub type SharedScene = Rc<Vec<MargoRenderElement>>;

/// Element `idx` of a [`SharedScene`].
pub struct SceneElement {
    scene: SharedScene,
    idx: usize,
}

impl SceneElement {
    fn get(&self) -> &MargoRenderElement {
        &self.scene[self.idx]
    }
}

impl Element for SceneElement {
    fn id(&self) -> &Id {
        self.get().id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.get().current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.get().location(scale)
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.get().src()
    }
    fn transform(&self) -> Transform {
        self.get().transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.get().geometry(scale)
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.get().damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.get().opaque_regions(scale)
    }
    fn alpha(&self) -> f32 {
        self.get().alpha()
    }
    fn kind(&self) -> Kind {
        self.get().kind()
    }
    fn is_framebuffer_effect(&self) -> bool {
        self.get().is_framebuffer_effect()
    }
}

impl RenderElement<GlesRenderer> for SceneElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        self.get()
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.get().underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        self.get().capture_framebuffer(frame, src, dst, cache)
    }
}

type Inner = RelocateRenderElement<RescaleRenderElement<SceneElement>>;

pub struct MirroredElement(Inner);

impl std::fmt::Debug for MirroredElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MirroredElement").finish()
    }
}

impl MirroredElement {
    /// The whole source `scene`, placed into `fit` on the target: one
    /// element per source element, all sharing the scene.
    pub fn frame(scene: &SharedScene, fit: Letterbox) -> Vec<MargoRenderElement> {
        (0..scene.len())
            .map(|idx| {
                let element = SceneElement {
                    scene: Rc::clone(scene),
                    idx,
                };
                let scaled =
                    RescaleRenderElement::from_element(element, Point::from((0, 0)), fit.scale);
                let placed = RelocateRenderElement::from_element(
                    scaled,
                    Point::<i32, Physical>::from(fit.offset),
                    Relocate::Relative,
                );
                MargoRenderElement::Mirrored(Self(placed))
            })
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::renderer::element::solid::SolidColorRenderElement;

    fn solid(x: i32, y: i32, w: i32, h: i32) -> MargoRenderElement {
        MargoRenderElement::Solid(SolidColorRenderElement::new(
            Id::new(),
            Rectangle::new((x, y).into(), (w, h).into()),
            CommitCounter::default(),
            [1.0, 0.0, 0.0, 1.0],
            Kind::Unspecified,
        ))
    }

    /// The mirror frame wraps the source scene the source rendered from
    /// (shared, not rebuilt or copied), keeps its ids and order, and places
    /// every element through the letterbox fit.
    #[test]
    fn the_mirror_frame_shares_the_source_scene() {
        let scene: SharedScene = Rc::new(vec![solid(0, 0, 1920, 40), solid(100, 200, 800, 600)]);
        // 1920x1200 laptop onto a 2560x1440 monitor: 1.2x, pillarboxed by 128.
        let fit = crate::mirror::letterbox((1920, 1200), (2560, 1440)).expect("fit");
        let frame = MirroredElement::frame(&scene, fit);
        assert_eq!(frame.len(), 2);
        assert_eq!(Rc::strong_count(&scene), 3, "one shared scene, no copies");
        let one = Scale::from(1.0);
        for (m, src) in frame.iter().zip(scene.iter()) {
            assert_eq!(m.id(), src.id());
        }
        assert_eq!(
            frame[0].geometry(one),
            Rectangle::new((128, 0).into(), (2304, 48).into())
        );
        assert_eq!(
            frame[1].geometry(one),
            Rectangle::new((248, 240).into(), (960, 720).into())
        );
        drop(frame);
        assert_eq!(Rc::strong_count(&scene), 1);
    }
}
