//! Drop-shadow render element.
//!
//! Adds a soft-edged shadow underneath floating / decorated client
//! windows when `Config::shadows = true`. Like `RoundedBorderElement`
//! it's a pure-GLSL pass — no wl_buffer involved, the shader
//! computes the alpha mask analytically per fragment so there's no
//! framebuffer ping-pong like a Kawase blur would need. That keeps
//! the cost roughly free per shadowed window: one extra fragment pass
//! over the (window + shadow_padding) rect.
//!
//! Cost trade-off:
//!
//!   * Real Gaussian / Kawase blur looks better on big shadows but
//!     needs offscreen buffers and 4–8 sample passes per draw —
//!     real-time only at moderate radius and adds GPU memory
//!     pressure.
//!   * SDF-driven analytic shadow (this) is one pass, exact, and
//!     handles arbitrary corner radius for free; the trade-off is
//!     shadows look "perfectly sharp" rather than naturalistic at
//!     huge sizes. For the 10–25 px shadows the user's config asks
//!     for, this is indistinguishable from a real blur.
//!
//! The shader is the same SDF approach as `rounded_border.rs`'s
//! frame, with two differences:
//!
//!   * The rect is shrunk by `shadows_size` and the alpha is
//!     spread over `shadows_blur` pixels via `smoothstep` —
//!     producing the soft glow.
//!   * Final colour is `shadow_color` (with the alpha curve)
//!     instead of border colour.
//!
//! Wiring lives in `udev.rs::push_client_elements`: when
//! `Config::shadows` is true and the client is a) floating (or
//! `shadow_only_floating = false`), b) not fullscreen, c) doesn't
//! have `no_shadow:1` from a windowrule, the renderer pushes a
//! `ShadowRenderElement` directly underneath the window's
//! `WaylandSurface`.

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType,
    UniformValue,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Transform};

#[derive(Debug, Clone)]
pub struct ShadowProgram(pub GlesPixelProgram);

impl ShadowProgram {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let program = renderer.compile_custom_pixel_shader(
            FRAG_SRC,
            &[
                UniformName::new("rect_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_1f),
                UniformName::new("blur_radius", UniformType::_1f),
                UniformName::new("inset", UniformType::_1f),
                UniformName::new("shadow_color", UniformType::_4f),
            ],
        )?;
        Ok(ShadowProgram(program))
    }
}

thread_local! {
    static CACHED: std::cell::RefCell<Option<ShadowProgram>> = const { std::cell::RefCell::new(None) };
}

/// Compile-once-per-renderer cache. Mirrors the `clipped_surface`
/// pattern: failures are non-fatal — `push_client_elements` skips
/// the shadow if the program isn't available.
pub fn shader(renderer: &mut GlesRenderer) -> Option<ShadowProgram> {
    CACHED.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            return Some(s.clone());
        }
        match ShadowProgram::compile(renderer) {
            Ok(p) => {
                *slot.borrow_mut() = Some(p.clone());
                Some(p)
            }
            Err(e) => {
                tracing::error!("shadow shader compile failed: {e:?}");
                None
            }
        }
    })
}

#[derive(Debug)]
pub struct ShadowRenderElement {
    id: Id,
    /// Outer rect of the shadow draw — window rect inflated by
    /// `(blur_radius + inset)` on every side and shifted by the
    /// configured X / Y offset.
    geometry: Rectangle<i32, Logical>,
    /// The window rect (logical), used to compute the SDF inset
    /// inside the fragment shader. Currently the shader derives
    /// inner-rect bounds from `rect_size - 2*(inset+blur_radius)`
    /// directly, so the field is informational; kept around for the
    /// upcoming Kawase-blur path that needs the original window
    /// rect to alpha-mask the area underneath the window itself.
    #[allow(dead_code)]
    window_size: (i32, i32),
    /// Window's corner radius (logical px).
    corner_radius: f32,
    /// Soft-edge falloff in px. Bigger = blurrier.
    blur_radius: f32,
    /// How far the shadow extends past the window edge before the
    /// soft falloff starts. Combined with blur_radius it sets the
    /// total shadow extent.
    inset: f32,
    /// Premultiplied RGBA colour.
    color: [f32; 4],
    scale: Scale<f64>,
    commit: CommitCounter,
    program: GlesPixelProgram,
}

impl ShadowRenderElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Id,
        window_rect: Rectangle<i32, Logical>,
        corner_radius: f32,
        size: f32,
        blur: f32,
        offset: (i32, i32),
        color: [f32; 4],
        scale: Scale<f64>,
        program: GlesPixelProgram,
    ) -> Self {
        // Total padding needed around the window rect to fit the
        // soft falloff: the SDF starts at the window edge, the
        // shadow extends `size` pixels outside before the soft
        // edge, then `blur` more pixels for the falloff itself.
        let padding = (size + blur).ceil() as i32;
        let geometry = Rectangle::new(
            (
                window_rect.loc.x - padding + offset.0,
                window_rect.loc.y - padding + offset.1,
            )
                .into(),
            (window_rect.size.w + 2 * padding, window_rect.size.h + 2 * padding).into(),
        );
        Self {
            id,
            geometry,
            window_size: (window_rect.size.w, window_rect.size.h),
            corner_radius,
            blur_radius: blur,
            inset: size,
            color,
            scale,
            commit: CommitCounter::default(),
            program,
        }
    }
}

impl Element for ShadowRenderElement {
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
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.color[3]
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for ShadowRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // Pixel shader uniforms. Sizes pass through in physical units
        // so the SDF math matches the destination rect's pixel grid
        // exactly — no fractional-scale fudge factors at the shader.
        let phys_w = dst.size.w.max(1) as f32;
        let phys_h = dst.size.h.max(1) as f32;
        let scale_x = self.scale.x as f32;
        let uniforms = [
            Uniform::new("rect_size", (phys_w, phys_h)),
            Uniform::new("corner_radius", self.corner_radius * scale_x),
            Uniform::new("blur_radius", self.blur_radius * scale_x),
            Uniform::new("inset", self.inset * scale_x),
            Uniform {
                name: "shadow_color".into(),
                value: UniformValue::_4f(
                    self.color[0],
                    self.color[1],
                    self.color[2],
                    self.color[3],
                ),
            },
        ];
        let src: Rectangle<f64, Buffer> = Rectangle::new(
            (0.0, 0.0).into(),
            (phys_w as f64, phys_h as f64).into(),
        );
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
        // Pure GL output; no associated wl_buffer.
        None
    }
}

const FRAG_SRC: &str = r#"#version 100

//_DEFINES_

precision highp float;

uniform vec2 rect_size;
uniform float corner_radius;
uniform float blur_radius;
uniform float inset;
uniform vec4 shadow_color;
uniform float alpha;
varying vec2 v_coords;

// Signed distance to a rounded rectangle centred at the origin with
// half-extents `b` and corner radius `r`.
float rounded_box_sdf(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    // Pixel position in the output rect's coords (centred).
    vec2 p = v_coords * rect_size - rect_size * 0.5;
    // Inner rect: the actual window rect, smaller than the draw rect
    // by the shadow padding (`inset + blur_radius`) on each side.
    vec2 inner_half = rect_size * 0.5 - vec2(inset + blur_radius);
    inner_half = max(inner_half, vec2(0.0));
    float dist = rounded_box_sdf(p, inner_half, corner_radius);
    // Soft falloff: 1.0 at the window edge, 0 at `blur_radius`
    // away. The `dist - inset` shift offsets where the falloff
    // *starts* relative to the window edge — bigger inset means
    // the shadow doesn't kiss the window outline.
    float falloff = 1.0 - smoothstep(0.0, blur_radius, dist - inset);
    gl_FragColor = shadow_color * falloff * alpha;
}
"#;
