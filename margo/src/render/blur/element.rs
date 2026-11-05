//! The blur render element: the backdrop cell it shares with its
//! surface, its commit/damage bookkeeping, and the `Element` /
//! `RenderElement` impls that capture the framebuffer and draw the blur.

use std::cell::RefCell;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{Frame, FrameContext, Offscreen};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform};

use margo_config::BlurParams;

use super::passes::{BlurTarget, SavedGl, blur_backdrop, composite_onto_output};
use super::{CACHED, framebuffer_rect};

/// The blurred backdrop of one blur element, kept between frames in the
/// damage tracker's per-element effect cache (the `cache` smithay hands to
/// [`RenderElement::capture_framebuffer`] and [`RenderElement::draw`]).
///
/// It is a pure function of what lies behind the element, so it stays
/// valid for every swapchain buffer until the tracker reports a change
/// under the element and calls `capture_framebuffer` again.
struct Backdrop {
    /// Exactly region-sized, so the composite samples it with UV scale 1.
    tex: GlesTexture,
    size: (i32, i32),
    /// The last capture produced a blurred image (false when the region
    /// is too small for one pass or the programs are missing).
    ready: bool,
}

type BackdropCell = RefCell<Option<Backdrop>>;

/// Commit counter for a blur element's look: equal inputs give equal
/// counters, so a stable-id blur reports damage (and gets re-captured)
/// exactly when a reload changed its parameters or its corner radius.
pub fn blur_commit(params: &BlurParams, corner_radius: f32) -> CommitCounter {
    let words = [
        params.num_passes as u32,
        params.radius as u32,
        params.noise.to_bits(),
        params.brightness.to_bits(),
        params.contrast.to_bits(),
        params.saturation.to_bits(),
        corner_radius.to_bits(),
    ];
    CommitCounter::from(words.iter().fold(0usize, |acc, w| {
        acc.wrapping_mul(31).wrapping_add(*w as usize)
    }))
}

/// The parts of `fb_rect` that `damage` (element-relative rects in output
/// space, as smithay passes them to `draw`) covers, in the same GL window
/// coordinates as `fb_rect`.
pub fn damage_in_framebuffer(
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    transform: Transform,
    fb_size: Size<i32, Physical>,
    fb_rect: Rectangle<i32, Physical>,
) -> Vec<Rectangle<i32, Physical>> {
    damage
        .iter()
        .filter_map(|d| {
            framebuffer_rect(Rectangle::new(dst.loc + d.loc, d.size), transform, fb_size)
                .intersection(fb_rect)
        })
        .collect()
}

/// The rounded-corner radius in framebuffer pixels for an element whose
/// own physical size is `own` but which is drawn at `drawn` (smaller when a
/// mirror target shows it scaled by the letterbox fit).
pub fn drawn_corner_px(
    corner_radius: f32,
    scale: f64,
    own: Size<i32, Physical>,
    drawn: Size<i32, Physical>,
) -> f32 {
    let px = corner_radius * scale as f32;
    if own.w > 0 && drawn.w > 0 {
        px * drawn.w as f32 / own.w as f32
    } else {
        px
    }
}

/// A blur draw under one translucent surface.
///
/// A smithay *framebuffer effect*: the damage tracker calls
/// [`RenderElement::capture_framebuffer`] whenever something behind the
/// element changed this frame, and then redraws everything behind the
/// whole element rect first, so the capture always reads a clean, current
/// backdrop. The blurred result is cached per element and output; a frame
/// that only changes what lies *above* the blur (a caret, the window's own
/// content) composites the cached image over just the damaged rects. That
/// keeps age-based damage tracking (and the swapchain) intact while blur is
/// on, and a static blurred screen renders nothing.
#[derive(Debug)]
pub struct BlurRenderElement {
    id: Id,
    /// The surface rect in logical coords (output-relative origin is
    /// applied by the caller before constructing this).
    geometry: Rectangle<i32, Logical>,
    /// Rounded-corner radius (logical px).
    corner_radius: f32,
    params: BlurParams,
    scale: Scale<f64>,
    commit: CommitCounter,
}

impl BlurRenderElement {
    pub fn new(
        id: Id,
        rect: Rectangle<i32, Logical>,
        corner_radius: f32,
        params: BlurParams,
        scale: Scale<f64>,
    ) -> Self {
        Self {
            id,
            geometry: rect,
            corner_radius,
            params,
            scale,
            commit: blur_commit(&params, corner_radius),
        }
    }

    /// Where `dst` lands in the bound framebuffer, and the region size.
    fn framebuffer_region(
        frame: &GlesFrame<'_, '_>,
        dst: Rectangle<i32, Physical>,
    ) -> (
        Transform,
        Size<i32, Physical>,
        Rectangle<i32, Physical>,
        (i32, i32),
    ) {
        // `draw_blur` addresses the framebuffer directly: CopyTexSubImage2D
        // for the capture and a quad placed in GL window coordinates for the
        // composite. `dst` is in output space; map it to where smithay's
        // projection put this element in the framebuffer (on a rotated or
        // flipped output those differ). The blur is computed in framebuffer
        // orientation, which is fine: the Kawase filter and the rounded-
        // rect mask are both symmetric under the 8 output transforms.
        let transform = frame.transformation();
        let fb_size = transform.transform_size(frame.output_size());
        let fb_rect = framebuffer_rect(dst, transform, fb_size);
        let region = (fb_rect.size.w.max(1), fb_rect.size.h.max(1));
        (transform, fb_size, fb_rect, region)
    }
}

impl Element for BlurRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::new(
            (0.0, 0.0).into(),
            (self.geometry.size.w as f64, self.geometry.size.h as f64).into(),
        )
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
        // The blur output is a pure function of the framebuffer content
        // *within our own geometry* (the capture copies exactly that
        // slice). Changes behind us are the damage tracker's job: as a
        // framebuffer effect it re-captures us whenever damage below
        // overlaps our rect. Our own damage is only a changed look (blur
        // parameters / corner radius on `mctl reload`, see `blur_commit`);
        // geometry changes are caught by the tracker's per-id diff.
        if commit != Some(self.commit) {
            DamageSet::from_slice(&[Rectangle::new(
                Default::default(),
                self.geometry(scale).size,
            )])
        } else {
            DamageSet::default()
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // Rounded + translucent: never claim opacity, or the
        // compositor would skip drawing the background we sample.
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for BlurRenderElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        let (_, _, fb_rect, region) = Self::framebuffer_region(frame, dst);
        let cell = cache.get_or_insert::<BackdropCell, _>(|| RefCell::new(None));
        let mut backdrop = cell.borrow_mut();
        let stale = backdrop.as_ref().is_none_or(|b| b.size != region);
        if stale {
            let tex: GlesTexture = Offscreen::<GlesTexture>::create_buffer(
                frame.renderer().as_mut(),
                Fourcc::Abgr8888,
                Size::<i32, Buffer>::from(region),
            )?;
            *backdrop = Some(Backdrop {
                tex,
                size: region,
                ready: false,
            });
        }
        let Some(backdrop) = backdrop.as_mut() else {
            return Ok(());
        };
        let target = backdrop.tex.tex_id();
        let params = self.params;
        // SAFETY: `with_context` makes the frame's GL context current for the closure.
        backdrop.ready = frame.with_context(|gl| unsafe {
            CACHED.with(|slot| {
                let mut borrow = slot.borrow_mut();
                let Some(blur_gl) = borrow.ready_mut() else {
                    return false;
                };
                let saved = SavedGl::save(gl);
                let blurred = blur_backdrop(
                    gl,
                    blur_gl,
                    saved.fbo,
                    fb_rect,
                    region,
                    &params,
                    BlurTarget::Texture(target),
                );
                saved.restore(gl);
                blurred.is_some()
            })
        })?;
        Ok(())
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let (transform, fb_size, fb_rect, region) = Self::framebuffer_region(frame, dst);
        // Only the damaged rects may change: outside them the buffer
        // already holds this exact composite (the tracker's contract), and
        // re-blending the rounded corners there would darken them frame by
        // frame.
        let clip = damage_in_framebuffer(dst, damage, transform, fb_size, fb_rect);
        if clip.is_empty() {
            return Ok(());
        }
        let corner_px = drawn_corner_px(
            self.corner_radius,
            self.scale.x,
            self.geometry(self.scale).size,
            dst.size,
        );
        let params = self.params;
        // The backdrop the tracker had us capture. Without one (drawn by a
        // caller that renders everything from scratch, not through a
        // damage tracker) the framebuffer behind us is fresh: capture now.
        let cached = cache
            .and_then(|c| c.get::<BackdropCell>())
            .and_then(|cell| {
                cell.borrow()
                    .as_ref()
                    .filter(|b| b.ready && b.size == region)
                    .map(|b| b.tex.tex_id())
            });

        // SAFETY: `with_context` makes the frame's GL context current for the closure.
        frame.with_context(|gl| unsafe {
            CACHED.with(|slot| {
                let mut borrow = slot.borrow_mut();
                let Some(blur_gl) = borrow.ready_mut() else {
                    return;
                };
                let saved = SavedGl::save(gl);
                let blurred = match cached {
                    Some(tex) => Some((tex, (1.0, 1.0))),
                    None => blur_backdrop(
                        gl,
                        blur_gl,
                        saved.fbo,
                        fb_rect,
                        region,
                        &params,
                        BlurTarget::Pool,
                    ),
                };
                if let Some((tex, uv)) = blurred {
                    composite_onto_output(
                        gl, blur_gl, &saved, tex, uv, fb_rect, region, corner_px, &params, &clip,
                    );
                }
                saved.restore(gl);
            });
        })?;
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_radius_follows_the_drawn_size() {
        let own: Size<i32, Physical> = (400, 300).into();
        // Drawn at its own size: logical radius times the output scale.
        assert_eq!(drawn_corner_px(8.0, 1.5, own, own), 12.0);
        // A mirror target showing it at half size: half the radius, so
        // the blur's corners stay under the (also halved) window corners.
        assert_eq!(drawn_corner_px(8.0, 1.5, own, (200, 150).into()), 6.0);
        // Degenerate sizes never divide by zero.
        assert_eq!(drawn_corner_px(8.0, 1.0, (0, 0).into(), (0, 0).into()), 8.0);
    }

    #[test]
    fn blur_commit_follows_every_look_input() {
        let p = BlurParams {
            num_passes: 2,
            radius: 5,
            noise: 0.02,
            brightness: 0.9,
            contrast: 0.9,
            saturation: 1.2,
        };
        let c = blur_commit(&p, 8.0);
        // Same look on the next frame: same counter, no damage.
        assert_eq!(c, blur_commit(&p, 8.0));
        let changed = [
            BlurParams { num_passes: 3, ..p },
            BlurParams { radius: 6, ..p },
            BlurParams { noise: 0.03, ..p },
            BlurParams {
                brightness: 1.0,
                ..p
            },
            BlurParams { contrast: 1.0, ..p },
            BlurParams {
                saturation: 1.0,
                ..p
            },
        ];
        for q in changed {
            assert_ne!(c, blur_commit(&q, 8.0), "{q:?}");
        }
        assert_ne!(c, blur_commit(&p, 0.0));
    }

    #[test]
    fn damage_maps_into_the_framebuffer_and_stays_inside_the_blur() {
        let fb: Size<i32, Physical> = (1920, 1080).into();
        let dst = Rectangle::<i32, Physical>::new((100, 50).into(), (300, 200).into());
        // A caret 10 px in, plus a rect hanging past the element's edge.
        let damage = [
            Rectangle::new((10, 20).into(), (4, 16).into()),
            Rectangle::new((290, 190).into(), (40, 40).into()),
        ];
        let fb_rect = framebuffer_rect(dst, Transform::Normal, fb);
        assert_eq!(
            damage_in_framebuffer(dst, &damage, Transform::Normal, fb, fb_rect),
            vec![
                Rectangle::new((110, 70).into(), (4, 16).into()),
                Rectangle::new((390, 240).into(), (10, 10).into()),
            ]
        );
        // Upside down: the caret moves with the element.
        let fb_rect = framebuffer_rect(dst, Transform::_180, fb);
        let got = damage_in_framebuffer(dst, &damage[..1], Transform::_180, fb, fb_rect);
        assert_eq!(
            got,
            vec![Rectangle::new(
                (1920 - 114, 1080 - 86).into(),
                (4, 16).into()
            )]
        );
        // Nothing to draw when nothing is damaged.
        assert!(damage_in_framebuffer(dst, &[], Transform::Normal, fb, fb_rect).is_empty());
    }
}
