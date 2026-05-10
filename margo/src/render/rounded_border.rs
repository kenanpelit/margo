//! Anti-aliased rounded-rectangle border, drawn as a single ring via a
//! custom GLES pixel shader.
//!
//! One [`RoundedBorderShader`] is compiled per `GlesRenderer` (lazy, on
//! first use). For each visible client we emit a single
//! [`RoundedBorderElement`] sized to the *outer* bounding box of the
//! border frame. The fragment shader does an SDF rounded-rect minus an
//! inner SDF rounded-rect to produce the ring shape with smooth edges.
//!
//! Visual model (matches CSS `border-radius`):
//! ```text
//!   outer corner radius  = `radius`            (configurable)
//!   inner corner radius  = max(radius - bw, 0) (derived)
//!   border thickness     = `bw`
//! ```

use std::cell::RefCell;

use smithay::backend::renderer::{
    element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
    gles::{GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType},
    utils::{CommitCounter, DamageSet, OpaqueRegions},
};
use smithay::utils::{
    Buffer as BufferCoord, Logical, Physical, Rectangle, Scale, Size, Transform,
};

/// Compiled shader program — shared across all `RoundedBorderElement`
/// instances drawn on the same `GlesRenderer`.
#[derive(Debug, Clone)]
pub struct RoundedBorderShader(pub GlesPixelProgram);

impl RoundedBorderShader {
    /// Compile the program once. Call again is fine — the GLES driver
    /// will return the same handle after the first link.
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let program = renderer.compile_custom_pixel_shader(
            FRAG_SRC,
            &[
                UniformName::new("u_color", UniformType::_4f),
                UniformName::new("u_radius", UniformType::_1f),
                UniformName::new("u_border_width", UniformType::_1f),
            ],
        )?;
        Ok(Self(program))
    }
}

thread_local! {
    /// Renderer-scoped (GLES context lives on a single thread) compile cache.
    /// Smithay's `GlesPixelProgram` is `Arc`-backed so cloning is cheap.
    static CACHED: RefCell<Option<RoundedBorderShader>> = const { RefCell::new(None) };
}

/// Get the program, compiling it if not yet built. Must be called from
/// the render thread (where the GLES context is current).
pub fn shader(renderer: &mut GlesRenderer) -> Option<RoundedBorderShader> {
    CACHED.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            return Some(s.clone());
        }
        match RoundedBorderShader::compile(renderer) {
            Ok(s) => {
                *slot.borrow_mut() = Some(s.clone());
                Some(s)
            }
            Err(e) => {
                tracing::error!("rounded_border shader compile failed: {e:?}");
                None
            }
        }
    })
}

/// One per visible window. `geometry` is the *outer* logical rect of the
/// border frame (i.e. window-geom expanded by `border_width` on each side).
#[derive(Debug, Clone)]
pub struct RoundedBorderElement {
    id: Id,
    /// Outer logical rect of the border frame.
    geometry: Rectangle<i32, Logical>,
    color: [f32; 4],
    /// Outer corner radius in logical pixels.
    radius: f32,
    /// Border thickness in logical pixels.
    border_width: f32,
    alpha: f32,
    commit: CommitCounter,
    program: GlesPixelProgram,
}

impl RoundedBorderElement {
    pub fn new(
        id: Id,
        geometry: Rectangle<i32, Logical>,
        color: [f32; 4],
        radius: f32,
        border_width: f32,
        alpha: f32,
        commit: CommitCounter,
        program: GlesPixelProgram,
    ) -> Self {
        Self {
            id,
            geometry,
            color,
            radius,
            border_width,
            alpha,
            commit,
            program,
        }
    }
}

impl Element for RoundedBorderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoord> {
        Rectangle::from_size(self.geometry.size.to_f64().to_buffer(
            1.0,
            Transform::Normal,
        ))
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // Conservative: a rounded ring has anti-aliased corners → not opaque.
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        // If the caller knows about the current commit, nothing changed since
        // last frame — emit no damage so the renderer can skip this element
        // entirely. We bump `commit` whenever geometry / color / radius
        // actually change (see `border.rs::ClientBorder::update`).
        if commit == Some(self.commit) {
            return DamageSet::default();
        }
        DamageSet::from_slice(&[Rectangle::from_size(
            self.geometry.size.to_physical_precise_round(scale),
        )])
    }
}

impl RenderElement<GlesRenderer> for RoundedBorderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        // We override `src`/`size` so that smithay's `build_texture_mat`
        // produces a tex_matrix that maps `dst` pixels to `v_coords ∈ [0, 1]²`
        // — required for our SDF shader to read position correctly. The
        // `_src` parameter passed in (from `Element::src()`) is ignored.
        let buf_size: Size<i32, BufferCoord> = Size::from((dst.size.w, dst.size.h));
        let src = Rectangle::from_size(buf_size.to_f64());

        // Logical → physical scale factor for converting border thickness
        // and corner radius (stored in logical pixels) to the pixel-space
        // SDF math the shader runs in.
        let scale = if self.geometry.size.w > 0 {
            dst.size.w as f32 / self.geometry.size.w as f32
        } else {
            1.0
        };
        let physical_radius = self.radius * scale;
        let physical_border = self.border_width * scale;

        frame.render_pixel_shader_to(
            &self.program,
            src,
            dst,
            buf_size,
            Some(damage),
            self.alpha,
            &[
                Uniform::new("u_color", self.color),
                Uniform::new("u_radius", physical_radius),
                Uniform::new("u_border_width", physical_border),
            ],
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// SDF-based rounded-rectangle ring fragment shader.
///
/// `size` and `vert_position` are provided automatically by smithay's
/// pixel-shader scaffold. We compute the signed-distance to the outer
/// rounded rect minus the signed-distance to the inner rounded rect to
/// get a ring with smooth (1-pixel) anti-aliased edges.
const FRAG_SRC: &str = r#"
precision mediump float;

uniform float alpha;
uniform vec2 size;
uniform vec4 u_color;
uniform float u_radius;
uniform float u_border_width;

varying vec2 v_coords;

float sd_rounded_rect(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    // Fragment position in pixels, centered.
    vec2 p = v_coords * size - size * 0.5;
    vec2 outer_b = size * 0.5;

    float outer_d = sd_rounded_rect(p, outer_b, u_radius);

    // Inner geometry (the hole).
    vec2 inner_b = outer_b - vec2(u_border_width);
    float inner_r = max(u_radius - u_border_width, 0.0);
    float inner_d = sd_rounded_rect(p, inner_b, inner_r);

    // Anti-aliased ring mask: inside outer (negative outer_d) AND outside
    // inner (positive inner_d). 1px transition each.
    float aa = 1.0;
    float outer_a = 1.0 - smoothstep(-aa, aa, outer_d);
    float inner_a = 1.0 - smoothstep(-aa, aa, inner_d);
    float ring = outer_a * (1.0 - inner_a);

    // Pre-multiplied alpha output.
    float a = u_color.a * alpha * ring;
    gl_FragColor = vec4(u_color.rgb * a, a);
}
"#;
