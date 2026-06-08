//! Rounded-rect solid-colour render element.
//!
//! A flat `SolidColorRenderElement` draws a hard-cornered rectangle. The
//! tabbed-group tab chips want the same rounded corners as every other bar/
//! window surface, so this draws a solid fill masked by an analytic
//! rounded-rect SDF (the same one-pass GLSL approach as `shadow.rs` /
//! `rounded_border.rs` — no wl_buffer, no framebuffer ping-pong).
//!
//! Uniforms: `rect_size` (physical px), `corner_radius` (physical px),
//! `fill_color` (straight RGBA, premultiplied in the shader).

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType,
    UniformValue,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

#[derive(Debug, Clone)]
pub struct RoundedSolidProgram(pub GlesPixelProgram);

impl RoundedSolidProgram {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let program = renderer.compile_custom_pixel_shader(
            FRAG_SRC,
            &[
                UniformName::new("rect_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_1f),
                UniformName::new("fill_color", UniformType::_4f),
            ],
        )?;
        Ok(RoundedSolidProgram(program))
    }
}

thread_local! {
    static CACHED: std::cell::RefCell<Option<RoundedSolidProgram>> =
        const { std::cell::RefCell::new(None) };
}

/// Compile-once-per-renderer cache. Failures are non-fatal — callers fall
/// back to a flat `SolidColorRenderElement`.
pub fn shader(renderer: &mut GlesRenderer) -> Option<RoundedSolidProgram> {
    CACHED.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            return Some(s.clone());
        }
        match RoundedSolidProgram::compile(renderer) {
            Ok(p) => {
                *slot.borrow_mut() = Some(p.clone());
                Some(p)
            }
            Err(e) => {
                tracing::error!("rounded-solid shader compile failed: {e:?}");
                None
            }
        }
    })
}

#[derive(Debug)]
pub struct RoundedSolidElement {
    id: Id,
    /// Output-local PHYSICAL rect (scale already applied by the caller).
    geometry: Rectangle<i32, Physical>,
    /// Corner radius in physical px.
    radius: f32,
    /// Straight (non-premultiplied) RGBA; premultiplied in the shader.
    color: [f32; 4],
    commit: CommitCounter,
    program: GlesPixelProgram,
}

impl RoundedSolidElement {
    pub fn new(
        id: Id,
        geometry: Rectangle<i32, Physical>,
        radius: f32,
        color: [f32; 4],
        program: GlesPixelProgram,
    ) -> Self {
        Self {
            id,
            geometry,
            radius,
            color,
            commit: CommitCounter::default(),
            program,
        }
    }
}

impl Element for RoundedSolidElement {
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
        self.geometry
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit != Some(self.commit) {
            DamageSet::from_slice(&[Rectangle::new(Default::default(), self.geometry.size)])
        } else {
            DamageSet::default()
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.color[3]
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for RoundedSolidElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let phys_w = dst.size.w.max(1) as f32;
        let phys_h = dst.size.h.max(1) as f32;
        // Clamp the radius to half the smaller side so it can't invert.
        let radius = self.radius.min(phys_w.min(phys_h) * 0.5).max(0.0);
        let uniforms = [
            Uniform::new("rect_size", (phys_w, phys_h)),
            Uniform::new("corner_radius", radius),
            Uniform {
                name: "fill_color".into(),
                value: UniformValue::_4f(
                    self.color[0],
                    self.color[1],
                    self.color[2],
                    self.color[3],
                ),
            },
        ];
        let src: Rectangle<f64, Buffer> =
            Rectangle::new((0.0, 0.0).into(), (phys_w as f64, phys_h as f64).into());
        let size: smithay::utils::Size<i32, Buffer> = (dst.size.w, dst.size.h).into();
        frame.render_pixel_shader_to(
            &self.program,
            src,
            dst,
            size,
            Some(damage),
            self.alpha(),
            &uniforms,
        )?;
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

// No manual `#version` — `compile_custom_pixel_shader` injects it (see the
// shadow.rs note for why a manual one breaks the build).
const FRAG_SRC: &str = r#"
//_DEFINES_

precision highp float;

uniform vec2 rect_size;
uniform float corner_radius;
uniform vec4 fill_color;
uniform float alpha;
varying vec2 v_coords;

float rounded_box_sdf(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 p = v_coords * rect_size - rect_size * 0.5;
    vec2 half_size = rect_size * 0.5;
    float dist = rounded_box_sdf(p, half_size, corner_radius);
    // ~1px analytic anti-aliased edge.
    float cov = 1.0 - smoothstep(-0.5, 0.5, dist);
    // Premultiplied output (smithay's blend expects premultiplied alpha).
    float a = fill_color.a * cov * alpha;
    gl_FragColor = vec4(fill_color.rgb * a, a);
}
"#;
