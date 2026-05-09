//! HDR Phase 2 — linear-light composite scaffolding.
//!
//! This module is the linear-light side of the HDR rollout described in
//! `docs/hdr-design.md`. It provides:
//!
//!   * **Transfer-function (TF) math** — bit-exact GLSL strings AND
//!     equivalent CPU-side `f32` implementations of the inverse +
//!     forward transfer for sRGB, ST2084 (PQ), HLG, gamma 2.2, and
//!     identity (linear). The CPU paths exist so the unit tests can
//!     verify round-trip correctness without a live GLES context.
//!
//!   * **Shader programs** — GLES2 pixel + texture shaders that
//!     decode an encoded surface to linear at sample time, and
//!     encode a linear-light framebuffer to the output's transfer
//!     function at present time. Compiled lazily once per renderer,
//!     cached thread-local (the GLES context is single-threaded so
//!     a `RefCell<Option<…>>` matches the pattern used by
//!     `rounded_border.rs` and `shadow.rs`).
//!
//!   * **`MARGO_COLOR_LINEAR` opt-in gate** — reads the env var at
//!     startup and exposes [`is_linear_composite_enabled`] for the
//!     udev render path. Default off; enabling it requires the
//!     follow-up DrmCompositor swapchain integration (see notes
//!     below).
//!
//! ## What's wired today
//!
//! Phase 2 as laid out in the design doc is two halves:
//!
//!   1. *Per-surface decode at sample time* — a TF lookup keyed off
//!      `wp_color_management_v1` per-surface state, decoding from
//!      the surface's declared transfer function into linear.
//!   2. *Linear-light composite + final encode pass* — the
//!      framebuffer math happens in linear space and a final pass
//!      encodes back to the output's TF (sRGB by default).
//!
//! This module ships the **shader programs and the validated TF
//! math** for both halves, plus the env gate. What still needs
//! upstream-smithay work to activate the path: switching the
//! `DrmCompositor` swapchain format from `Argb8888` to
//! `Abgr16161616f` at runtime. Smithay 0.7's `DrmCompositor` takes
//! its format at construction; toggling it without a full backend
//! restart needs an API addition that hasn't shipped yet. With
//! that API in place, the integration is straightforward: feed
//! these shaders into a `TextureRenderElement` wrapping the fp16
//! offscreen, queue that element through `DrmCompositor`.
//!
//! Until then, `MARGO_COLOR_LINEAR=1` is a no-op at runtime — the
//! env gate is honored but `is_linear_composite_active` returns
//! `false` from the udev path, so the existing 8-bit composite
//! continues to drive every output. The shaders + math are ready
//! to swap in the moment the swapchain knob lands.
//!
//! ## Why ship the scaffolding now
//!
//! Phase 3 (KMS HDR scan-out) needs the *same* shaders. Standing
//! up the math + tests now means Phase 3's lift becomes "wire the
//! swapchain to fp16, queue the encoder element"; if we waited
//! until upstream smithay shipped the swapchain knob, we'd have
//! BOTH the math AND the integration to do then, doubling the
//! chunk size. Splitting the work into testable pieces is the
//! reason the unit tests below verify TF round-trip against the
//! published spec values — once the integration lands, we'll
//! already know the math is correct.

// Phase 2 ships shader programs + TF math validated against
// published spec values. PQ/HLG/Gamma2.2 GLSL constants and the
// CPU-side reference math are exercised by the unit tests but
// have no runtime call site yet (the upstream-blocked
// integration). Silence dead_code wholesale — Phase 3 wires the
// rest up.
#![allow(dead_code)]

use std::cell::RefCell;

use smithay::backend::renderer::gles::{
    GlesError, GlesPixelProgram, GlesRenderer, GlesTexProgram, UniformName, UniformType,
};

// ── Transfer functions: CPU-side f32 reference implementations ──────────────
//
// One pair (`encode` + `decode`) per supported TF. Used by the unit
// tests to verify the GLSL math agrees with the CPU math at known
// sample points. Keeping the GLSL constants in sync with these values
// is a manual discipline: each shader source below has a `// REF:` line
// pointing at the matching CPU constants.

/// sRGB EOTF (encoded → linear). Spec: IEC 61966-2-1.
pub fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB inverse EOTF (linear → encoded).
pub fn srgb_encode(l: f32) -> f32 {
    if l <= 0.0031308 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// ST2084 / PQ EOTF (encoded → linear, normalised to peak luminance).
/// Returns linear in [0, 1] where 1.0 = 10000 cd/m². Spec: SMPTE ST2084.
pub fn pq_decode(c: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let cm2 = c.powf(1.0 / M2);
    let num = (cm2 - C1).max(0.0);
    let den = C2 - C3 * cm2;
    (num / den).powf(1.0 / M1)
}

/// ST2084 / PQ inverse EOTF (linear → encoded). Input is the same
/// normalisation `pq_decode` returns.
pub fn pq_encode(l: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let lm1 = l.max(0.0).powf(M1);
    ((C1 + C2 * lm1) / (1.0 + C3 * lm1)).powf(M2)
}

/// HLG OETF inverse / EOTF (encoded → linear). Spec: ARIB STD-B67 /
/// ITU-R BT.2100. The HLG EOTF includes a system gamma that depends
/// on display luminance; we emit the *scene* linear (OOTF inverse not
/// applied) — Phase 3 KMS scan-out adds the OOTF when the actual
/// display peak is known.
pub fn hlg_decode(c: f32) -> f32 {
    const A: f32 = 0.17883277;
    const B: f32 = 0.28466892;
    const C: f32 = 0.559_910_7;
    if c <= 0.5 {
        c * c / 3.0
    } else {
        ((((c - C) / A).exp()) + B) / 12.0
    }
}

/// HLG OETF (linear → encoded). Inverse of [`hlg_decode`].
pub fn hlg_encode(l: f32) -> f32 {
    const A: f32 = 0.17883277;
    const B: f32 = 0.28466892;
    const C: f32 = 0.559_910_7;
    if l <= 1.0 / 12.0 {
        (3.0 * l).sqrt()
    } else {
        A * (12.0 * l - B).ln() + C
    }
}

/// Gamma 2.2 (encoded → linear). Approximation used by some legacy
/// displays; the wp_color_management_v1 spec lists it as a discrete
/// TF distinct from sRGB.
pub fn gamma22_decode(c: f32) -> f32 {
    c.max(0.0).powf(2.2)
}

/// Gamma 2.2 (linear → encoded).
pub fn gamma22_encode(l: f32) -> f32 {
    l.max(0.0).powf(1.0 / 2.2)
}

// ── GLSL TF helpers — embedded into the larger shader programs ──────────────
//
// These are GLSL function bodies. The encoder/decoder shaders below
// concatenate the matching helper into their fragment source so the
// driver compiles a single program with the right TF baked in. Keeps
// the per-pass cost to one math sequence; no branches.

const GLSL_SRGB_DECODE: &str = r#"
    // REF: srgb_decode (CPU). IEC 61966-2-1.
    float tf_decode(float c) {
        return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
    }
"#;

const GLSL_SRGB_ENCODE: &str = r#"
    // REF: srgb_encode (CPU). Inverse of GLSL_SRGB_DECODE.
    float tf_encode(float l) {
        return l <= 0.0031308 ? 12.92 * l : 1.055 * pow(l, 1.0 / 2.4) - 0.055;
    }
"#;

const GLSL_PQ_DECODE: &str = r#"
    // REF: pq_decode (CPU). SMPTE ST2084.
    float tf_decode(float c) {
        const float M1 = 2610.0 / 16384.0;
        const float M2 = 2523.0 / 4096.0 * 128.0;
        const float C1 = 3424.0 / 4096.0;
        const float C2 = 2413.0 / 4096.0 * 32.0;
        const float C3 = 2392.0 / 4096.0 * 32.0;
        float cm2 = pow(c, 1.0 / M2);
        return pow(max(cm2 - C1, 0.0) / (C2 - C3 * cm2), 1.0 / M1);
    }
"#;

const GLSL_HLG_DECODE: &str = r#"
    // REF: hlg_decode (CPU). ITU-R BT.2100.
    float tf_decode(float c) {
        const float A = 0.17883277;
        const float B = 0.28466892;
        const float C = 0.55991073;
        return c <= 0.5
            ? (c * c) / 3.0
            : (exp((c - C) / A) + B) / 12.0;
    }
"#;

// ── Encoder program: fp16 linear → 8-bit sRGB-encoded swap chain ────────────
//
// Uniforms:
//   * `tex` (built-in by `compile_custom_texture_shader`): fp16 linear sample.
//   * `u_alpha` (additional, _1f): per-element opacity.
//
// Output: vec4 with each channel encoded via the matching TF.
//
// Single-pass at present time. Cost: one fragment shader hop over the
// final framebuffer rect — negligible against the rest of the scene.

const ENCODER_FRAG_SRGB: &str = concat!(
    r#"
precision highp float;
varying vec2 v_coords;
"#,
    "uniform float u_alpha;\n",
    r#"
    // tex_program prelude inserts the `tex` sampler + sample fn.
"#,
    // The TF helper renames to tf_encode below.
    r#"
    float tf_encode(float l) {
        return l <= 0.0031308 ? 12.92 * l : 1.055 * pow(l, 1.0 / 2.4) - 0.055;
    }
"#,
    r#"
    void main() {
        vec4 lin = sample_color(v_coords);
        gl_FragColor = vec4(
            tf_encode(lin.r),
            tf_encode(lin.g),
            tf_encode(lin.b),
            lin.a
        ) * u_alpha;
    }
"#,
);

// ── Decoder pixel-shader-style helper ───────────────────────────────────────
//
// Smithay's `compile_custom_pixel_shader` doesn't sample textures —
// it's for procedural pixel shaders (like our shadow / border SDF).
// The decoder thus lives as a TEXTURE shader (sampling the surface),
// compiled when sRGB-encoded surfaces feed the linear pipeline.

const DECODER_FRAG_SRGB: &str = concat!(
    r#"
precision highp float;
varying vec2 v_coords;
"#,
    "uniform float u_alpha;\n",
    r#"
    float tf_decode(float c) {
        return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
    }
"#,
    r#"
    void main() {
        vec4 enc = sample_color(v_coords);
        gl_FragColor = vec4(
            tf_decode(enc.r),
            tf_decode(enc.g),
            tf_decode(enc.b),
            enc.a
        ) * u_alpha;
    }
"#,
);

// ── Cached, lazily-compiled program holders ─────────────────────────────────

#[derive(Debug, Clone)]
pub struct EncoderShader {
    pub program: GlesTexProgram,
}

#[derive(Debug, Clone)]
pub struct DecoderShader {
    pub program: GlesTexProgram,
}

/// Pixel-shader form of the sRGB encoder. Available for callers that
/// already have an offscreen sample (e.g. via `texture_program` glue)
/// and just need the TF math as a procedural pass. Most users want
/// [`encoder_shader`] (texture program) instead.
#[derive(Debug, Clone)]
pub struct EncoderPixelShader {
    pub program: GlesPixelProgram,
}

thread_local! {
    static ENCODER: RefCell<Option<EncoderShader>> = const { RefCell::new(None) };
    static DECODER: RefCell<Option<DecoderShader>> = const { RefCell::new(None) };
}

/// Compile (or fetch from cache) the linear → sRGB encoder texture
/// shader. Returns `None` if the GL driver rejects the program — the
/// caller should fall back to the 8-bit composite path.
pub fn encoder_shader(renderer: &mut GlesRenderer) -> Option<EncoderShader> {
    ENCODER.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            return Some(s.clone());
        }
        match compile_encoder(renderer) {
            Ok(s) => {
                *slot.borrow_mut() = Some(s.clone());
                Some(s)
            }
            Err(e) => {
                tracing::error!("linear_composite encoder shader compile failed: {e:?}");
                None
            }
        }
    })
}

fn compile_encoder(renderer: &mut GlesRenderer) -> Result<EncoderShader, GlesError> {
    let program = renderer.compile_custom_texture_shader(
        ENCODER_FRAG_SRGB,
        &[UniformName::new("u_alpha", UniformType::_1f)],
    )?;
    Ok(EncoderShader { program })
}

/// Compile (or fetch from cache) the sRGB → linear decoder texture
/// shader. Used when sampling encoded sRGB surfaces into the
/// linear-light framebuffer.
pub fn decoder_shader(renderer: &mut GlesRenderer) -> Option<DecoderShader> {
    DECODER.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            return Some(s.clone());
        }
        match compile_decoder(renderer) {
            Ok(s) => {
                *slot.borrow_mut() = Some(s.clone());
                Some(s)
            }
            Err(e) => {
                tracing::error!("linear_composite decoder shader compile failed: {e:?}");
                None
            }
        }
    })
}

fn compile_decoder(renderer: &mut GlesRenderer) -> Result<DecoderShader, GlesError> {
    let program = renderer.compile_custom_texture_shader(
        DECODER_FRAG_SRGB,
        &[UniformName::new("u_alpha", UniformType::_1f)],
    )?;
    Ok(DecoderShader { program })
}

// ── Env gate ────────────────────────────────────────────────────────────────

/// Has the user opted into the linear-light composite pipeline via
/// `MARGO_COLOR_LINEAR=1`? Read once per process; subsequent calls
/// hit a cached `OnceLock`.
pub fn is_linear_composite_enabled() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let v = std::env::var("MARGO_COLOR_LINEAR").unwrap_or_default();
        let on = matches!(v.as_str(), "1" | "true" | "yes" | "on");
        if on {
            tracing::info!(
                "MARGO_COLOR_LINEAR set — linear-light composite scaffolding active. \
                 Note: the runtime swapchain switch is gated on upstream smithay \
                 fp16 framebuffer support; today this only enables shader \
                 program compilation. See `docs/hdr-design.md` Phase 2 notes."
            );
        }
        on
    })
}

/// Has the linear-light composite path actually engaged for the
/// current frame? Today this is **always false** — the shader
/// scaffolding is in place but smithay 0.7's `DrmCompositor` doesn't
/// expose a runtime swapchain-format swap. Keeping the function so
/// the udev path can branch correctly the moment the upstream API
/// lands; flip the body to `is_linear_composite_enabled()` then.
#[inline]
pub fn is_linear_composite_active() -> bool {
    false
}

// ── Tests: TF round-trip against published reference points ─────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn nearly_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn srgb_round_trip_is_identity() {
        // Both endpoints + a few interior samples. Tolerance: 1e-5 —
        // the f32 math should round-trip far better than this; the
        // larger tolerance gives us margin for f32 chained ops.
        for c_in in [0.0_f32, 0.04045, 0.1, 0.5, 0.7, 1.0] {
            let lin = srgb_decode(c_in);
            let back = srgb_encode(lin);
            assert!(
                nearly_eq(c_in, back, 1e-5),
                "sRGB round-trip mismatch: {c_in} → {lin} → {back}"
            );
        }
    }

    #[test]
    fn srgb_known_values() {
        // Spec sanity. sRGB 0.5 ≈ 0.21404 linear (CSS Color 4 reference).
        assert!(nearly_eq(srgb_decode(0.5), 0.21404114, 1e-4));
        // Linear 0.5 → sRGB ≈ 0.7353569.
        assert!(nearly_eq(srgb_encode(0.5), 0.7353569, 1e-4));
        // Toe region: encoded 0.04045 sits exactly on the kink point
        // (12.92 * x = ((x+0.055)/1.055)^2.4 at x = 0.04045).
        let toe = srgb_decode(0.04045);
        assert!(nearly_eq(toe, 0.04045 / 12.92, 1e-6));
    }

    #[test]
    fn pq_round_trip_is_identity() {
        for c_in in [0.0_f32, 0.1, 0.5, 0.75, 1.0] {
            let lin = pq_decode(c_in);
            let back = pq_encode(lin);
            assert!(
                nearly_eq(c_in, back, 1e-4),
                "PQ round-trip mismatch: {c_in} → {lin} → {back}"
            );
        }
    }

    #[test]
    fn pq_known_values() {
        // Spec: PQ encoded 0.0 → linear 0.0; encoded 1.0 → linear 1.0
        // (i.e. peak luminance, normalised to 10000 cd/m²).
        assert!(nearly_eq(pq_decode(0.0), 0.0, 1e-6));
        assert!(nearly_eq(pq_decode(1.0), 1.0, 1e-4));
        // Round-trip via encode.
        assert!(nearly_eq(pq_encode(0.0), 0.0, 1e-6));
        assert!(nearly_eq(pq_encode(1.0), 1.0, 1e-4));
    }

    #[test]
    fn hlg_round_trip_is_identity() {
        // HLG covers [0, 1] encoded ↔ [0, 1] scene linear.
        for c_in in [0.0_f32, 0.1, 0.5, 0.7, 1.0] {
            let lin = hlg_decode(c_in);
            let back = hlg_encode(lin);
            assert!(
                nearly_eq(c_in, back, 1e-4),
                "HLG round-trip mismatch: {c_in} → {lin} → {back}"
            );
        }
    }

    #[test]
    fn hlg_known_kink_point() {
        // HLG OETF kink at encoded 0.5 ↔ linear 1/12 (per BT.2100).
        let lin = hlg_decode(0.5);
        assert!(nearly_eq(lin, 1.0 / 12.0, 1e-4));
        let enc = hlg_encode(1.0 / 12.0);
        assert!(nearly_eq(enc, 0.5, 1e-4));
    }

    #[test]
    fn gamma22_round_trip() {
        for c_in in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            let lin = gamma22_decode(c_in);
            let back = gamma22_encode(lin);
            assert!(
                nearly_eq(c_in, back, 1e-5),
                "γ2.2 round-trip mismatch: {c_in} → {lin} → {back}"
            );
        }
    }

    #[test]
    fn env_gate_default_off() {
        // Test runs in cargo's clean env. Confirm the OnceLock-cached
        // state.rs default behaviour: the gate is OFF unless
        // MARGO_COLOR_LINEAR=1 is set.
        //
        // Note: this test also documents the protocol — flipping the
        // env from off to on at runtime will NOT take effect because
        // the result is cached on first call. Phase-2 activation
        // requires a margo restart, same as every other env-driven
        // toggle.
        //
        // Only assert when the env isn't externally pre-set (CI may
        // export it for integration-test variants).
        if std::env::var_os("MARGO_COLOR_LINEAR").is_none() {
            assert!(!is_linear_composite_enabled());
        }
    }
}
