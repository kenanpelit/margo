//! GLSL sources of the blur programs: the dual-Kawase down/up passes and
//! the rounded composite.

// ── Shaders ─────────────────────────────────────────────────────────
//
// Compiled via the raw `link_program` helper, which does NOT inject a
// `#version` header — so unlike the `compile_custom_pixel_shader`
// shaders in `shadow.rs` / `rounded_border.rs`, these MUST declare
// `#version 100` themselves (matching smithay's own `texture.vert`).

pub(super) const VERT_SRC: &str = r#"#version 100
precision highp float;
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

// Composite vertex shader: the quad's `a_uv` (0..1) is mapped onto the
// destination rect, given in GL window coordinates of the framebuffer
// (`framebuffer_rect`, which already accounts for the output transform and
// smithay's Y flip), then into NDC against the viewport. `a_uv` is carried
// through unchanged for texture sampling.
pub(super) const COMPOSITE_VERT_SRC: &str = r#"#version 100
precision highp float;
attribute vec2 a_pos;
attribute vec2 a_uv;
uniform vec2 dst_origin;
uniform vec2 dst_size;
uniform vec4 viewport;   // x, y, w, h in framebuffer pixels
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    vec2 px = dst_origin + a_uv * dst_size;
    vec2 ndc = 2.0 * (px - viewport.xy) / viewport.zw - 1.0;
    gl_Position = vec4(ndc, 0.0, 1.0);
}
"#;

// Dual-Kawase down-filter. `half_texel` is half a texel of the SOURCE
// texture; `radius` widens the sample footprint. `uv_scale` maps our
// [0,1] quad UVs onto the valid sub-rect of a (possibly larger) pooled
// source texture.
pub(super) const DOWN_FRAG_SRC: &str = r#"#version 100
precision highp float;
uniform sampler2D tex;
uniform vec2 half_texel;
uniform float radius;
uniform vec2 uv_scale;
varying vec2 v_uv;
void main() {
    vec2 uv = v_uv * uv_scale;
    vec2 ht = half_texel * radius;
    vec4 sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - ht);
    sum += texture2D(tex, uv + ht);
    sum += texture2D(tex, uv + vec2(ht.x, -ht.y));
    sum += texture2D(tex, uv - vec2(ht.x, -ht.y));
    gl_FragColor = sum / 8.0;
}
"#;

// Dual-Kawase up-filter (tent / 8-tap).
pub(super) const UP_FRAG_SRC: &str = r#"#version 100
precision highp float;
uniform sampler2D tex;
uniform vec2 half_texel;
uniform float radius;
uniform vec2 uv_scale;
varying vec2 v_uv;
void main() {
    vec2 uv = v_uv * uv_scale;
    vec2 ht = half_texel * radius;
    vec4 sum = texture2D(tex, uv + vec2(-ht.x * 2.0, 0.0));
    sum += texture2D(tex, uv + vec2(-ht.x, ht.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, ht.y * 2.0));
    sum += texture2D(tex, uv + vec2(ht.x, ht.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(ht.x * 2.0, 0.0));
    sum += texture2D(tex, uv + vec2(ht.x, -ht.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, -ht.y * 2.0));
    sum += texture2D(tex, uv + vec2(-ht.x, -ht.y)) * 2.0;
    gl_FragColor = sum / 12.0;
}
"#;

// Composite: sample the blurred texture over the quad placed by
// COMPOSITE_VERT_SRC, apply
// brightness / contrast / saturation / noise, and mask to the rounded
// rect. Output is premultiplied alpha (smithay's blend expects it).
pub(super) const COMPOSITE_FRAG_SRC: &str = r#"#version 100
precision highp float;
uniform sampler2D tex;
uniform vec2 uv_scale;
uniform vec2 region_size;
uniform float corner_radius;
uniform float brightness;
uniform float contrast;
uniform float saturation;
uniform float noise;
varying vec2 v_uv;

float rounded_box_sdf(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

void main() {
    // Texel row 0 of the blurred texture is window row `dst_origin.y`
    // (the capture copies window rows in order), and `v_uv.y = 0` sits
    // on that same row, so no flip.
    vec2 sample_uv = v_uv * uv_scale;
    vec3 c = texture2D(tex, sample_uv).rgb;

    // brightness
    c *= brightness;
    // contrast around 0.5
    c = (c - 0.5) * contrast + 0.5;
    // saturation
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(luma), c, saturation);
    // noise (dither to break up banding on flat blurs)
    if (noise > 0.0) {
        c += (hash(gl_FragCoord.xy) - 0.5) * noise;
    }
    c = clamp(c, 0.0, 1.0);

    // rounded-rect mask in region pixel space
    vec2 p = v_uv * region_size - region_size * 0.5;
    vec2 half_size = region_size * 0.5;
    float dist = rounded_box_sdf(p, half_size, corner_radius);
    float mask = 1.0 - smoothstep(-1.0, 1.0, dist);

    gl_FragColor = vec4(c * mask, mask);
}
"#;
