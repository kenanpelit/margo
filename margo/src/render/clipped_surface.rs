//! Rounded-corner clipping for Wayland client surface content.
//!
//! The border shader rounds only the frame. This element wraps a normal
//! `WaylandSurfaceRenderElement` and temporarily replaces Smithay's default
//! texture shader so the client pixels are masked to the same rounded rect.

use crate::render::shader_cache::ShaderSlot;
use std::cell::RefCell;

use smithay::{
    backend::renderer::{
        buffer_y_inverted,
        element::{
            Element, Id, Kind, RenderElement, UnderlyingStorage,
            surface::WaylandSurfaceRenderElement,
        },
        gles::{
            GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
            UniformValue,
        },
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::user_data::UserDataMap,
    utils::{Buffer, Logical, Physical, Rectangle, Scale, Transform},
};

#[derive(Debug, Clone)]
pub struct ClippedSurfaceShader(pub GlesTexProgram);

impl ClippedSurfaceShader {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let program = renderer.compile_custom_texture_shader(
            FRAG_SRC,
            &[
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_1f),
                UniformName::new("input_to_geo", UniformType::Matrix3x3),
            ],
        )?;
        Ok(Self(program))
    }
}

thread_local! {
    static CACHED: RefCell<ShaderSlot<ClippedSurfaceShader>> = const { RefCell::new(ShaderSlot::Pending) };
}

pub fn shader(renderer: &mut GlesRenderer) -> Option<ClippedSurfaceShader> {
    CACHED.with(|slot| {
        slot.borrow_mut()
            .get_or_compile("clipped_surface", || {
                ClippedSurfaceShader::compile(renderer)
            })
            .cloned()
    })
}

#[derive(Debug)]
pub struct ClippedSurfaceRenderElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    id: Id,
    geometry: Rectangle<f64, Logical>,
    scale: f32,
    radius: f32,
    program: GlesTexProgram,
}

impl ClippedSurfaceRenderElement {
    pub fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        scale: Scale<f64>,
        geometry: Rectangle<f64, Logical>,
        radius: f32,
        program: GlesTexProgram,
    ) -> Self {
        let namespace = ((radius.max(0.0) * 100.0).round() as usize).wrapping_add(0xC11F);
        let id = inner.id().namespaced(namespace);
        Self {
            inner,
            id,
            geometry,
            scale: scale.x as f32,
            radius,
            program,
        }
    }

    fn compute_uniforms(&self) -> Vec<Uniform<'static>> {
        let scale = Scale::from(f64::from(self.scale));

        let elem_geo = self.inner.geometry(scale);
        let elem_loc = (elem_geo.loc.x as f32, elem_geo.loc.y as f32);
        let elem_size = (elem_geo.size.w as f32, elem_geo.size.h as f32);

        let clip_geo: Rectangle<i32, Physical> = self.geometry.to_physical_precise_round(scale);
        let clip_loc = (clip_geo.loc.x as f32, clip_geo.loc.y as f32);
        let clip_size = (clip_geo.size.w.max(1) as f32, clip_geo.size.h.max(1) as f32);

        let buffer_size = self.inner.buffer_size();
        let buffer_size = (buffer_size.w.max(1) as f32, buffer_size.h.max(1) as f32);

        let view = self.inner.view();
        let src_loc = (view.src.loc.x as f32, view.src.loc.y as f32);
        let src_size = (
            view.src.size.w.max(1.0) as f32,
            view.src.size.h.max(1.0) as f32,
        );

        let transform = match self.inner.transform() {
            // Matches niri's correction for Smithay texture coordinates.
            Transform::_90 => Transform::_270,
            Transform::_270 => Transform::_90,
            x => x,
        };
        let transform_matrix = mat_mul(
            mat_translate(0.5, 0.5),
            mat_mul(transform_matrix(transform), mat_translate(-0.5, -0.5)),
        );

        let y_invert = if buffer_y_inverted(self.inner.buffer()).unwrap_or(false) {
            mat_scale(1.0, -1.0)
        } else {
            MAT_IDENTITY
        };

        let input_to_geo = mat_mul(
            transform_matrix,
            mat_mul(
                mat_scale(elem_size.0 / clip_size.0, elem_size.1 / clip_size.1),
                mat_mul(
                    mat_translate(
                        (elem_loc.0 - clip_loc.0) / elem_size.0.max(1.0),
                        (elem_loc.1 - clip_loc.1) / elem_size.1.max(1.0),
                    ),
                    mat_mul(
                        mat_scale(buffer_size.0 / src_size.0, buffer_size.1 / src_size.1),
                        mat_mul(
                            mat_translate(-src_loc.0 / buffer_size.0, -src_loc.1 / buffer_size.1),
                            y_invert,
                        ),
                    ),
                ),
            ),
        );

        vec![
            Uniform::new("geo_size", clip_size),
            Uniform::new("corner_radius", self.radius * self.scale),
            Uniform {
                name: "input_to_geo".into(),
                value: UniformValue::Matrix3x3 {
                    matrices: vec![input_to_geo],
                    transpose: false,
                },
            },
        ]
    }
}

impl Element for ClippedSurfaceRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let damage = self.inner.damage_since(scale, commit);
        let elem_loc = self.geometry(scale).loc;
        let mut clip = self.geometry.to_physical_precise_round(scale);
        clip.loc -= elem_loc;
        damage
            .into_iter()
            .filter_map(|rect| rect.intersection(clip))
            .collect()
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // Keep the surface's own opaque region inside the clip, minus the
        // anti-aliased outline and rounded corners. Reporting nothing made
        // the damage tracker redraw the clear colour and the wallpaper under
        // every damaged window pixel.
        let inner = self.inner.opaque_regions(scale);
        if inner.is_empty() {
            return inner;
        }
        let elem_loc = self.geometry(scale).loc;
        let mut clip = self.geometry.to_physical_precise_round(scale);
        clip.loc -= elem_loc;
        clipped_opaque_regions(&inner, clip, f64::from(self.radius) * scale.x)
            .into_iter()
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for ClippedSurfaceRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(self.program.clone(), self.compute_uniforms());
        let result = RenderElement::<GlesRenderer>::draw(
            &self.inner,
            frame,
            src,
            dst,
            damage,
            opaque_regions,
            cache,
        );
        frame.clear_tex_program_override();
        result
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// Opaque part of a rounded-clipped surface, element-local physical.
///
/// `inner` is what the wrapped surface reports as opaque, `clip` the rounded
/// clip rect and `radius` its corner radius, all physical and relative to the
/// element. The clip's 1 px anti-aliased outline and the four `radius`
/// corner squares are translucent; everything else inside the clip keeps the
/// surface's own opacity.
fn clipped_opaque_regions(
    inner: &[Rectangle<i32, Physical>],
    clip: Rectangle<i32, Physical>,
    radius: f64,
) -> Vec<Rectangle<i32, Physical>> {
    let visible: Vec<_> = inner.iter().filter_map(|r| r.intersection(clip)).collect();
    if radius <= 0.0 {
        // The shader's mask is a hard 0/1 clip without a radius.
        return visible;
    }
    // The 1 px anti-aliased outline runs along every edge of the clip.
    let mut cut = vec![
        Rectangle::new(clip.loc, (clip.size.w, 1).into()),
        Rectangle::new(
            (clip.loc.x, clip.loc.y + clip.size.h - 1).into(),
            (clip.size.w, 1).into(),
        ),
        Rectangle::new(clip.loc, (1, clip.size.h).into()),
        Rectangle::new(
            (clip.loc.x + clip.size.w - 1, clip.loc.y).into(),
            (1, clip.size.h).into(),
        ),
    ];
    // The curve lives in the `radius` square at each corner (the shader
    // clamps the radius to half the smaller side; a bigger square just
    // covers everything).
    let r = radius.ceil() as i32;
    for (x, y) in [
        (clip.loc.x, clip.loc.y),
        (clip.loc.x + clip.size.w - r, clip.loc.y),
        (clip.loc.x, clip.loc.y + clip.size.h - r),
        (clip.loc.x + clip.size.w - r, clip.loc.y + clip.size.h - r),
    ] {
        cut.push(Rectangle::new((x, y).into(), (r, r).into()));
    }
    Rectangle::subtract_rects_many(visible, cut)
}

const MAT_IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

fn mat_translate(tx: f32, ty: f32) -> [f32; 9] {
    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, ty, 1.0]
}

fn mat_scale(sx: f32, sy: f32) -> [f32; 9] {
    [sx, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 1.0]
}

fn mat_mul(a: [f32; 9], b: [f32; 9]) -> [f32; 9] {
    let mut out = [0.0; 9];
    for col in 0..3 {
        for row in 0..3 {
            out[col * 3 + row] =
                a[row] * b[col * 3] + a[3 + row] * b[col * 3 + 1] + a[6 + row] * b[col * 3 + 2];
        }
    }
    out
}

fn transform_matrix(transform: Transform) -> [f32; 9] {
    match transform {
        Transform::Normal => MAT_IDENTITY,
        Transform::_90 => [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        Transform::_180 => [-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0],
        Transform::_270 => [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        Transform::Flipped => [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        Transform::Flipped90 => [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        Transform::Flipped180 => [1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0],
        Transform::Flipped270 => [0.0, -1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    }
}

// NOTE: smithay's `compile_custom_pixel_shader` prepends its own
// `#version` + GLES header. Adding our own here puts a second
// `#version` on line 2 and the GLSL preprocessor rejects the whole
// shader. See `shadow.rs` for the same lesson.
const FRAG_SRC: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform vec2 geo_size;
uniform float corner_radius;
uniform mat3 input_to_geo;

float rounded_rect_alpha(vec2 p, vec2 size, float radius) {
    float r = min(radius, min(size.x, size.y) * 0.5);
    if (r <= 0.0) {
        return 1.0;
    }

    vec2 half_size = size * 0.5;
    vec2 q = abs(p - half_size) - (half_size - vec2(r));
    float dist = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    return 1.0 - smoothstep(-1.0, 1.0, dist);
}

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    vec3 geo = input_to_geo * vec3(v_coords, 1.0);
    if (geo.x < 0.0 || geo.x > 1.0 || geo.y < 0.0 || geo.y > 1.0) {
        color = vec4(0.0);
    } else {
        color *= rounded_rect_alpha(geo.xy * geo_size, geo_size, corner_radius);
    }

    color *= alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0) {
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
    }
#endif

    gl_FragColor = color;
}
"#;

#[cfg(test)]
mod tests {
    use super::clipped_opaque_regions;
    use smithay::utils::{Physical, Rectangle};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn covers(rects: &[Rectangle<i32, Physical>], x: i32, y: i32) -> bool {
        rects
            .iter()
            .any(|r| r.contains::<smithay::utils::Point<i32, Physical>>((x, y).into()))
    }

    fn area(rects: &[Rectangle<i32, Physical>]) -> i32 {
        // The result is built by subtraction, so the pieces are disjoint.
        rects.iter().map(|r| r.size.w * r.size.h).sum()
    }

    #[test]
    fn an_opaque_surface_stays_opaque_except_the_outline_and_corners() {
        let out = clipped_opaque_regions(&[rect(0, 0, 100, 100)], rect(0, 0, 100, 100), 10.0);
        assert!(covers(&out, 50, 50));
        assert!(covers(&out, 1, 50), "inside the straight edge");
        assert!(covers(&out, 10, 1), "just past the corner square");
        assert!(!covers(&out, 0, 50), "the 1 px anti-aliased outline");
        assert!(!covers(&out, 99, 50));
        assert!(!covers(&out, 50, 99));
        for (x, y) in [(5, 5), (95, 5), (5, 95), (95, 95), (9, 9)] {
            assert!(!covers(&out, x, y), "corner pixel {x},{y}");
        }
        // 98x98 inset minus four 9x9 corner pieces.
        assert_eq!(area(&out), 98 * 98 - 4 * 81);
    }

    #[test]
    fn only_the_surface_opaque_part_inside_the_clip_is_kept() {
        // Opaque bottom half; the clip is offset inside the element.
        let out = clipped_opaque_regions(&[rect(0, 50, 200, 150)], rect(20, 30, 100, 100), 4.0);
        assert!(covers(&out, 60, 60));
        assert!(!covers(&out, 60, 40), "not opaque in the surface");
        assert!(!covers(&out, 150, 60), "outside the clip");
        assert!(!covers(&out, 21, 128), "bottom-left corner of the clip");
        assert!(covers(&out, 21, 120));
    }

    #[test]
    fn a_fractional_radius_rounds_the_corner_up() {
        let out = clipped_opaque_regions(&[rect(0, 0, 50, 50)], rect(0, 0, 50, 50), 7.5);
        assert!(!covers(&out, 7, 7));
        assert!(covers(&out, 8, 7));
        assert!(covers(&out, 7, 8));
    }

    #[test]
    fn a_radius_past_half_the_size_leaves_nothing_opaque() {
        let out = clipped_opaque_regions(&[rect(0, 0, 40, 40)], rect(0, 0, 40, 40), 30.0);
        assert_eq!(area(&out), 0);
    }

    #[test]
    fn no_radius_means_a_plain_hard_clip() {
        let out = clipped_opaque_regions(&[rect(0, 0, 100, 100)], rect(10, 10, 50, 50), 0.0);
        assert_eq!(out, vec![rect(10, 10, 50, 50)]);
    }

    #[test]
    fn a_translucent_surface_stays_translucent() {
        assert!(clipped_opaque_regions(&[], rect(0, 0, 100, 100), 10.0).is_empty());
    }
}
