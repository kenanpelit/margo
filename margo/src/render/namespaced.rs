//! `NamespacedElement` — wraps a render element and overrides only its
//! `Id` with a namespaced copy.
//!
//! The scroller overview draws the SAME wallpaper (background / bottom
//! layer-shell surfaces) into every tag cell. Smithay's damage tracker
//! keys element state by `Id`, so drawing one surface at several
//! positions with the same `Id` collapses them and corrupts tracking.
//! niri solves this by tagging each duplicated element with a
//! per-workspace namespace (`Id::namespaced`); we do the same. Every
//! method delegates to the inner element except `id()`.

use smithay::backend::renderer::Renderer;
use smithay::backend::renderer::element::{
    Element, Id, Kind, RenderElement, UnderlyingStorage,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer, Physical, Rectangle, Scale, user_data::UserDataMap};

/// Wraps `inner`, returning a namespaced `Id` so the same surface can be
/// drawn into multiple overview cells without damage-tracker collisions.
#[derive(Debug)]
pub struct NamespacedElement<E> {
    inner: E,
    id: Id,
}

impl<E: Element> NamespacedElement<E> {
    pub fn new(inner: E, namespace: usize) -> Self {
        let id = inner.id().namespaced(namespace);
        Self { inner, id }
    }
}

impl<E: Element> Element for NamespacedElement<E> {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn transform(&self) -> smithay::utils::Transform {
        self.inner.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn is_framebuffer_effect(&self) -> bool {
        self.inner.is_framebuffer_effect()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for NamespacedElement<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner.draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), R::Error> {
        self.inner.capture_framebuffer(frame, src, dst, cache)
    }
}
