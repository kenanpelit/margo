//! Open/close animation render element.
//!
//! Used by both the toplevel-open animation (fade-in + scale-up around the
//! target slot's centre) and the toplevel-close animation (fade-out +
//! scale-down around the dying slot's centre, driven from a captured
//! `GlesTexture` snapshot since the live `wl_surface` is gone by the time
//! we draw the close animation).
//!
//! Why one element type covers both: the math is identical except for the
//! sign of the scale curve and the alpha curve. Caller picks a `progress`
//! in `[0, 1]` and a scaling preset, we compute the dst rect (centred on
//! `geometry`), pick the alpha, and run a single `render_texture_from_to`.
//!
//! Compared to dwl/mangowm's fadingout list, which only fades alpha, we
//! also scale around the centre — so the window doesn't just "vanish"
//! at a fixed rect, it pulls in to a point. Compared to niri's open-close
//! transition (which uses GLSL custom shaders for richer effects), we use
//! the existing `clipped_surface` rounded-corner shader so the corners
//! stay rounded throughout the transition. Niri's effect is configurable
//! per-window-rule; for now margo uses one preset set at config level.

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformValue,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::Texture;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Transform};

/// Visual flavour for open/close animations. Picked from config strings
/// `animation_type_open` / `animation_type_close`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCloseKind {
    /// Scale + fade together. Window grows from `initial_scale` to 1.0
    /// (open) or shrinks from 1.0 to `final_scale` (close), with alpha
    /// matching. Pleasing default; what most modern compositors do.
    Zoom,
    /// Pure alpha fade, no scale change. Cheap and unobtrusive — good
    /// for clients that don't tolerate visual scaling well (legacy
    /// Java apps, some SDL games on close).
    Fade,
    /// Slide in from / out to a screen edge. Direction encoded in the
    /// `direction` parameter at draw time.
    Slide(SlideDirection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideDirection {
    Up,
    Down,
    Left,
    Right,
}

impl OpenCloseKind {
    /// Parse `animation_type_open` / `animation_type_close` config
    /// strings. Recognises:
    ///
    /// * `"zoom"` (default for both) → scale + alpha around centre.
    /// * `"fade"` → pure alpha.
    /// * `"slide_in_*"` / `"slide_out_*"` for `*` ∈ `{up, down, left, right}`.
    ///
    /// Anything unrecognised falls back to `Zoom`. The `_in_/_out_`
    /// distinction in the config is just naming; the slide direction
    /// is what actually matters and gets baked here.
    pub fn parse(s: &str) -> Self {
        let s = s.trim().to_lowercase();
        match s.as_str() {
            "fade" => OpenCloseKind::Fade,
            "zoom" => OpenCloseKind::Zoom,
            "slide_in_up" | "slide_up" | "slide_out_up" => {
                OpenCloseKind::Slide(SlideDirection::Up)
            }
            "slide_in_down" | "slide_down" | "slide_out_down" => {
                OpenCloseKind::Slide(SlideDirection::Down)
            }
            "slide_in_left" | "slide_left" | "slide_out_left" => {
                OpenCloseKind::Slide(SlideDirection::Left)
            }
            "slide_in" | "slide_in_right" | "slide_right" | "slide_out" | "slide_out_right" => {
                OpenCloseKind::Slide(SlideDirection::Right)
            }
            _ => OpenCloseKind::Zoom,
        }
    }
}

#[derive(Debug)]
pub struct OpenCloseRenderElement {
    id: Id,
    /// Texture sampled across the entire transition. For open, this is
    /// captured on first buffer commit (we wait until the client has
    /// actually painted something before kicking off the animation —
    /// otherwise we'd zoom in an empty rectangle). For close, this is
    /// captured in `toplevel_destroyed` *before* the wl_surface is
    /// released, so it's the last visible frame the user saw.
    texture: GlesTexture,
    /// Target rect in logical coords. Open zooms toward this rect from
    /// a smaller centred rect; close zooms away from this rect to a
    /// smaller centred rect. Slide just translates this rect along its
    /// chosen axis.
    geometry: Rectangle<i32, Logical>,
    /// Output scale, for physical-pixel conversion.
    scale: Scale<f64>,
    /// Animation progress.
    ///   * Open:  0 = invisible (initial scale, alpha 0), 1 = fully open.
    ///   * Close: 0 = fully visible, 1 = invisible.
    /// In both cases the kind decides which direction `final_scale`
    /// refers to.
    progress: f32,
    /// Outer multiplier applied on top of the kind-derived alpha. Lets
    /// the caller layer a window-rule opacity hint over the transition.
    result_alpha: f32,
    /// Animation flavour.
    kind: OpenCloseKind,
    /// True for close (progress maps to "going away"); false for open
    /// (progress maps to "appearing"). Matters for the alpha + scale
    /// curve direction.
    is_close: bool,
    /// Initial / final scale ratio. For open, the start scale is this;
    /// for close, the end scale is this. Typically 0.5–0.8 — small
    /// enough to be visibly different from 1.0, large enough that the
    /// window doesn't visually pop out of nowhere.
    extreme_scale: f32,
    /// Bumped each frame so smithay's damage tracker re-damages the
    /// element while the transition is in flight.
    commit: CommitCounter,
    /// Optional rounded-corner clipping shader (reused from
    /// clipped_surface). When `radius > 0`, draw() installs this as the
    /// active texture program for one frame so the corners stay rounded
    /// throughout the transition.
    program: Option<GlesTexProgram>,
    /// Corner radius in logical pixels. 0 = no clipping.
    radius: f32,
}

impl OpenCloseRenderElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Id,
        texture: GlesTexture,
        geometry: Rectangle<i32, Logical>,
        scale: Scale<f64>,
        progress: f32,
        result_alpha: f32,
        kind: OpenCloseKind,
        is_close: bool,
        extreme_scale: f32,
        commit: CommitCounter,
        radius: f32,
        program: Option<GlesTexProgram>,
    ) -> Self {
        Self {
            id,
            texture,
            geometry,
            scale,
            progress: progress.clamp(0.0, 1.0),
            result_alpha,
            kind,
            is_close,
            extreme_scale: extreme_scale.clamp(0.05, 1.0),
            commit,
            program,
            radius,
        }
    }

    /// Compute the alpha at the current progress.
    fn current_alpha(&self) -> f32 {
        let p = self.progress;
        // Map progress so alpha rises (open) or falls (close) from 0 to 1.
        let visible = if self.is_close { 1.0 - p } else { p };
        // For pure-fade we want the curve linear; for zoom we slightly
        // bias the alpha to be ahead of the scale (so the window is
        // visible by the time it's near full size, instead of winking
        // in at the last moment). The bias is a single multiplication
        // — keep it cheap, the visual difference is the point.
        match self.kind {
            OpenCloseKind::Fade => visible,
            OpenCloseKind::Zoom => (visible * 1.15).min(1.0),
            OpenCloseKind::Slide(_) => visible,
        }
        .clamp(0.0, 1.0)
            * self.result_alpha
    }

    /// Compute the scale factor in [extreme_scale, 1.0] at the current
    /// progress. Open: linearly grows from extreme to 1; close: linearly
    /// shrinks from 1 to extreme.
    fn current_scale(&self) -> f32 {
        let p = self.progress;
        match self.kind {
            OpenCloseKind::Zoom => {
                let factor = if self.is_close { 1.0 - p } else { p };
                self.extreme_scale + (1.0 - self.extreme_scale) * factor
            }
            OpenCloseKind::Fade | OpenCloseKind::Slide(_) => 1.0,
        }
    }

    /// Compute a logical-pixel offset for slide animations.
    fn current_offset(&self) -> Point<i32, Logical> {
        let p = self.progress;
        // Distance to slide. We use the slot's diagonal as a generous
        // upper bound — guarantees the window starts/ends fully off-
        // screen relative to its slot.
        let dist = self.geometry.size.w.max(self.geometry.size.h);
        let frac = if self.is_close { p } else { 1.0 - p };
        let d = (dist as f32 * frac) as i32;
        match self.kind {
            OpenCloseKind::Slide(SlideDirection::Up) => Point::from((0, -d)),
            OpenCloseKind::Slide(SlideDirection::Down) => Point::from((0, d)),
            OpenCloseKind::Slide(SlideDirection::Left) => Point::from((-d, 0)),
            OpenCloseKind::Slide(SlideDirection::Right) => Point::from((d, 0)),
            _ => Point::from((0, 0)),
        }
    }

    /// Destination rect in logical coords for the current frame.
    fn current_geometry(&self) -> Rectangle<i32, Logical> {
        let scale = self.current_scale();
        let offset = self.current_offset();
        // Scale around the centre of self.geometry.
        let cx = self.geometry.loc.x + self.geometry.size.w / 2;
        let cy = self.geometry.loc.y + self.geometry.size.h / 2;
        let w = (self.geometry.size.w as f32 * scale) as i32;
        let h = (self.geometry.size.h as f32 * scale) as i32;
        Rectangle::new(
            Point::from((cx - w / 2 + offset.x, cy - h / 2 + offset.y)),
            (w, h).into(),
        )
    }

    fn rounded_clip_uniforms(&self, dst: Rectangle<i32, Physical>) -> Vec<Uniform<'static>> {
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

impl Element for OpenCloseRenderElement {
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
        self.current_geometry().to_physical_precise_round(self.scale)
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        // Always damage the *current* full rect when our commit moves,
        // otherwise nothing — smithay redraws the cleared area itself.
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
        // Transition is by definition translucent (alpha curve).
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.current_alpha()
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for OpenCloseRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let alpha = self.current_alpha();
        if alpha < 0.001 {
            return Ok(());
        }

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

        let overridden = install_override(frame);
        let size = self.texture.size();
        let src: Rectangle<f64, Buffer> =
            Rectangle::new((0.0, 0.0).into(), (size.w as f64, size.h as f64).into());

        smithay::backend::renderer::Frame::render_texture_from_to(
            frame,
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            alpha,
        )?;

        if overridden {
            frame.clear_tex_program_override();
        }
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // GPU-only texture; no associated wl_buffer → opt out of direct
        // scanout for the duration of the transition.
        None
    }
}
