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
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformValue,
};
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
    /// Corner radius applied to the rendered texture (logical px).
    /// Reuses the `clipped_surface` GLES texture shader to mask the
    /// snapshot's corners so they match the live surface's rounded
    /// corners during the crossfade — without this the snapshot's
    /// sharp corners are visible until alpha fades out, which the
    /// user perceives as a 90°-corner-flash mid-resize.
    radius: f32,
    /// Optional override texture-program: when set, the snapshot is
    /// drawn through it instead of the default tex program. Used to
    /// inject the corner-clipping shader.
    program: Option<GlesTexProgram>,
}

impl ResizeRenderElement {
    pub fn new(
        id: Id,
        texture: GlesTexture,
        geometry: Rectangle<i32, Logical>,
        scale: Scale<f64>,
        alpha: f32,
        commit: CommitCounter,
        radius: f32,
        program: Option<GlesTexProgram>,
    ) -> Self {
        Self {
            id,
            texture,
            geometry,
            scale,
            commit,
            alpha,
            radius,
            program,
        }
    }

    fn rounded_clip_uniforms(&self, dst: Rectangle<i32, Physical>) -> Vec<Uniform<'static>> {
        // The clipped_surface fragment shader expects three uniforms:
        //   * `geo_size` — the destination rect's size in physical px.
        //     Used inside `rounded_rect_alpha(p, size, radius)` to
        //     compute the corner mask.
        //   * `corner_radius` — same units as `geo_size`, scaled to
        //     the output's fractional scale.
        //   * `input_to_geo` — 3×3 matrix mapping the texture's UV
        //     space (`v_coords`, [0,1]²) into "geometry-relative
        //     position" space (also [0,1]²). For the resize-snapshot
        //     case the destination *is* the geometry rect, so the
        //     mapping is the identity.
        const MAT_IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

        let geo_size = (dst.size.w.max(1) as f32, dst.size.h.max(1) as f32);
        let radius_phys = self.radius * (self.scale.x as f32);

        vec![
            Uniform::new("geo_size", geo_size),
            Uniform::new("corner_radius", radius_phys),
            Uniform {
                name: "input_to_geo".into(),
                value: UniformValue::Matrix3x3 {
                    matrices: vec![MAT_IDENTITY],
                    transpose: false,
                },
            },
        ]
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
        // Inject the corner-clipping shader if we have one. This is
        // the same `compile_custom_texture_shader`-style pattern used
        // by `crate::render::clipped_surface::ClippedSurfaceRenderElement`:
        // override the renderer's default texture program for the
        // duration of *this* draw call so `render_texture_from_to`
        // routes its texture sample through our rounded-mask GLSL,
        // then clear the override so the next render element gets
        // the default path back.
        let overridden = if let Some(program) = self.program.as_ref().filter(|_| self.radius > 0.0)
        {
            frame.override_default_tex_program(program.clone(), self.rounded_clip_uniforms(dst));
            true
        } else {
            false
        };

        let result = smithay::backend::renderer::Frame::render_texture_from_to(
            frame,
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha,
        );

        if overridden {
            frame.clear_tex_program_override();
        }

        result
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // No matching wl_buffer — the texture lives entirely on the
        // GPU side. This disables direct-scanout for resize-animated
        // windows, which is fine: the animation is short and the
        // snapshot is mid-flight by definition.
        None
    }
}
