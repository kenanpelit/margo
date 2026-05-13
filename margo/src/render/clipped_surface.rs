//! Rounded-corner clipping for Wayland client surface content.
//!
//! The border shader rounds only the frame. This element wraps a normal
//! `WaylandSurfaceRenderElement` and temporarily replaces Smithay's default
//! texture shader so the client pixels are masked to the same rounded rect.

use std::cell::RefCell;

use smithay::{
    backend::renderer::{
        buffer_y_inverted,
        element::{
            surface::WaylandSurfaceRenderElement, Element, Id, Kind, RenderElement,
            UnderlyingStorage,
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
    static CACHED: RefCell<Option<ClippedSurfaceShader>> = const { RefCell::new(None) };
}

pub fn shader(renderer: &mut GlesRenderer) -> Option<ClippedSurfaceShader> {
    CACHED.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            return Some(s.clone());
        }

        match ClippedSurfaceShader::compile(renderer) {
            Ok(s) => {
                *slot.borrow_mut() = Some(s.clone());
                Some(s)
            }
            Err(e) => {
                tracing::error!("clipped_surface shader compile failed: {e:?}");
                None
            }
        }
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

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // Rounded masks have anti-aliased corners and the compositor must not
        // assume the clipped surface is fully opaque.
        OpaqueRegions::default()
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
