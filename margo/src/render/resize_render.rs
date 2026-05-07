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
    /// "Previous" texture — the snapshot of the window's content
    /// captured at the moment the resize animation started. Held for
    /// the entire duration of the animation; rendered with
    /// `1.0 - progress` alpha so it fades out as the transition
    /// completes.
    tex_prev: GlesTexture,
    /// "Next" texture — the live window content re-captured into an
    /// offscreen GlesTexture every frame. By going through the SAME
    /// `render_texture_from_to` path as `tex_prev` (instead of
    /// the live `WaylandSurfaceRenderElement` tree), both layers
    /// share the same pixel-snapping, the same rounded-clip shader,
    /// and the same draw transform — eliminating the residual
    /// "oynama" the user kept seeing when the live surface and the
    /// snapshot were composited via separate code paths.
    ///
    /// `None` for the first ~1 frame of the animation while the
    /// next-texture capture catches up; in that case we just render
    /// `tex_prev` opaque, which is exactly what we want at
    /// progress = 0.
    tex_next: Option<GlesTexture>,
    /// Where to render the texture (logical coordinates), updated each
    /// frame from the live (interpolated) layout slot.
    geometry: Rectangle<i32, Logical>,
    /// Output scale, propagated to physical conversions inside
    /// `Element::geometry`.
    scale: Scale<f64>,
    /// Bumped every time `geometry` changes so smithay re-damages.
    commit: CommitCounter,
    /// Animation progress in [0, 1]. Drives the crossfade alpha for
    /// both layers: `tex_prev` at `1 - progress`, `tex_next` at
    /// `progress`. Multiplied by `result_alpha` to support an outer
    /// fade-in/out of the entire transition if needed.
    progress: f32,
    /// Outer alpha (currently always 1.0). Reserved for the layer's
    /// own opacity if we ever want to fade the whole transition.
    result_alpha: f32,
    /// Corner radius applied to BOTH rendered textures (logical px).
    /// Reuses the `clipped_surface` GLES texture shader to mask
    /// corners so the crossfade preserves the rounded look.
    radius: f32,
    /// Optional corner-clipping texture program.
    program: Option<GlesTexProgram>,
}

impl ResizeRenderElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Id,
        tex_prev: GlesTexture,
        tex_next: Option<GlesTexture>,
        geometry: Rectangle<i32, Logical>,
        scale: Scale<f64>,
        progress: f32,
        result_alpha: f32,
        commit: CommitCounter,
        radius: f32,
        program: Option<GlesTexProgram>,
    ) -> Self {
        Self {
            id,
            tex_prev,
            tex_next,
            geometry,
            scale,
            commit,
            progress: progress.clamp(0.0, 1.0),
            result_alpha,
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
        // Both textures are sampled across their full extent. The
        // `Element::src` value is the source rect in buffer coords
        // for the tex_prev texture (caller treats it as the "main"
        // texture for damage tracking). tex_next, if present, is
        // sampled likewise inside `draw`.
        let size = self.tex_prev.size();
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
        // We composite both layers ourselves at progress-derived
        // alphas inside `draw`; report the outer "result alpha" so
        // smithay's damage tracker treats the element as opaque-
        // ish only when result_alpha is 1.0.
        self.result_alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for ResizeRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // Two-pass crossfade. Both passes share:
        //   * The same destination rect (`dst`).
        //   * The same rounded-corner clipping shader (overridden
        //     before the call, cleared after).
        //   * The same `render_texture_from_to` code path.
        //
        // → the prev and next layers can't drift relative to each
        //   other for any reason — pixel snapping, matrix rounding,
        //   subsurface composition — because they're literally the
        //   same renderer call with different source textures and
        //   alphas. That kills the residual "minor oynama" the
        //   single-snapshot + live-WaylandSurfaceRenderElement
        //   composite kept producing.

        let alpha_prev = (1.0 - self.progress).clamp(0.0, 1.0) * self.result_alpha;
        let alpha_next = self.progress.clamp(0.0, 1.0) * self.result_alpha;

        let install_override = |frame: &mut GlesFrame<'_, '_>| {
            if let Some(program) = self.program.as_ref().filter(|_| self.radius > 0.0) {
                frame.override_default_tex_program(
                    program.clone(),
                    self.rounded_clip_uniforms(dst),
                );
                true
            } else {
                false
            }
        };

        // Pass 1: tex_prev at alpha = 1 - progress.
        if alpha_prev > 0.001 {
            let overridden = install_override(frame);
            let prev_size = self.tex_prev.size();
            let prev_src: Rectangle<f64, Buffer> = Rectangle::new(
                (0.0, 0.0).into(),
                (prev_size.w as f64, prev_size.h as f64).into(),
            );
            smithay::backend::renderer::Frame::render_texture_from_to(
                frame,
                &self.tex_prev,
                prev_src,
                dst,
                damage,
                &[],
                Transform::Normal,
                alpha_prev,
            )?;
            if overridden {
                frame.clear_tex_program_override();
            }
        }

        // Pass 2: tex_next at alpha = progress (if we have it). The
        // first ~1 frame of the animation typically has no next
        // texture yet — the offscreen capture runs once we know the
        // animation is in flight, so the very first frame is just
        // tex_prev opaque, which is what we'd render anyway.
        if let Some(tex_next) = self.tex_next.as_ref() {
            if alpha_next > 0.001 {
                let overridden = install_override(frame);
                let next_size = tex_next.size();
                let next_src: Rectangle<f64, Buffer> = Rectangle::new(
                    (0.0, 0.0).into(),
                    (next_size.w as f64, next_size.h as f64).into(),
                );
                smithay::backend::renderer::Frame::render_texture_from_to(
                    frame,
                    tex_next,
                    next_src,
                    dst,
                    damage,
                    &[],
                    Transform::Normal,
                    alpha_next,
                )?;
                if overridden {
                    frame.clear_tex_program_override();
                }
            }
        }

        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // No matching wl_buffer — the texture lives entirely on the
        // GPU side. This disables direct-scanout for resize-animated
        // windows, which is fine: the animation is short and the
        // snapshot is mid-flight by definition.
        None
    }
}
