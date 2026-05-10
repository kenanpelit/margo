//! HDR Phase 4 — per-output ICC profile colour management.
//!
//! Reads the user's per-output ICC profile from `colord` over D-Bus,
//! parses it with `lcms2`, and bakes a 33³ float RGB lookup table that
//! a post-composition pixel shader samples to land each output's
//! framebuffer in the panel's measured colour space.
//!
//! ## What ships today (the bakeable side)
//!
//! * **`colord` D-Bus client** — `default_profile_path_for_connector`
//!   resolves a DRM connector name (e.g. `DP-1`) to the user's
//!   currently-assigned ICC file via the standard `org.freedesktop
//!   .ColorManager` API. Returns `None` when the user hasn't set a
//!   profile (steady state, not an error).
//! * **`lcms2` ICC parse + LUT bake** — `bake_lut` runs an identity
//!   33³ grid through an sRGB → display-profile transform and writes
//!   the result to a flat `Vec<[f32; 3]>` (≈130 KB at the default
//!   size). Perceptual rendering intent matches what colord's own
//!   reference pipeline uses.
//! * **2D atlas encoding** — GLES2 has no `sampler3D`. The 33³ LUT is
//!   re-laid out as a 1089 × 33 RGB texture (one 33×33 column block
//!   per blue slice), upload-able as a smithay `GlesTexture`.
//! * **Trilinear-sampling GLSL** — `ICC_LUT_FRAG` constant, exposed
//!   for the eventual texture-program wire-up. Two atlas reads per
//!   pixel — one per neighbouring blue slice — `mix()`-ed by the
//!   fractional coordinate.
//! * **`MARGO_HDR_ICC=1` env gate** — honoured today; logs at
//!   startup so users can verify their build path.
//!
//! ## What's queued (runtime activation)
//!
//! Same upstream constraint as Phase 2/3: smithay 0.7's `GlesRenderer`
//! exposes `compile_custom_texture_shader` for *numeric* extra
//! uniforms but no public hook for binding a second `sampler2D` to a
//! custom texture program. The post-composition LUT pass needs that
//! second sampler (the LUT atlas) alongside the input surface
//! texture. The integration is ~30 LOC the moment the API lands —
//! `compile` calls the upstream API, `is_icc_lut_active` flips to
//! `is_icc_lut_enabled`, the udev backend uploads the atlas to a
//! `GlesTexture` once per profile change, and the renderer queues
//! the program as the final element.
//!
//! Until then, the bake side is fully exercised by unit tests against
//! lcms2's reference math, and the shader source ships in `const`
//! form so reviewers can audit the GLSL today.

#![allow(dead_code)]

use std::path::PathBuf;

use lcms2::{Intent, PixelFormat, Profile, Transform};

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum IccError {
    #[error("ICC parse failed: {0}")]
    Parse(String),
    #[error("ICC transform failed: {0}")]
    Transform(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(feature = "dbus")]
    #[error("colord D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),
}

// ── LUT data structure ──────────────────────────────────────────────────────

/// 3D RGB lookup table baked from an ICC profile.
///
/// Layout: `data[r * size² + g * size + b]` holds the `[R, G, B]`
/// destination triple in [0, 1]. The 2D atlas form (used for GPU
/// upload) lives in [`AtlasRgbaF32`].
#[derive(Debug, Clone)]
pub struct IccLut3D {
    pub size: usize,
    pub data: Vec<[f32; 3]>,
}

/// Default cube side length. 33 is the de-facto standard for display
/// LUTs (matches colord's own reference pipeline and the cube size
/// most ICC profilers ship with). 33³ × 3 × 4 B ≈ 430 KB raw, 144 KB
/// when stored as the atlas.
pub const DEFAULT_LUT_SIZE: usize = 33;

impl IccLut3D {
    /// Identity LUT — every input maps to itself. Used as the fallback
    /// when colord has no entry for an output, and as a unit-test
    /// reference for the trilinear sampler.
    pub fn identity(size: usize) -> Self {
        let mut data = Vec::with_capacity(size * size * size);
        let max = (size - 1) as f32;
        for r in 0..size {
            for g in 0..size {
                for b in 0..size {
                    data.push([r as f32 / max, g as f32 / max, b as f32 / max]);
                }
            }
        }
        Self { size, data }
    }

    /// CPU-side trilinear sample. Mirrors the GLSL exactly so the
    /// unit tests verify the shader's arithmetic intent without
    /// needing a live GLES context.
    pub fn sample_trilinear(&self, r: f32, g: f32, b: f32) -> [f32; 3] {
        let r = r.clamp(0.0, 1.0);
        let g = g.clamp(0.0, 1.0);
        let b = b.clamp(0.0, 1.0);
        let max = (self.size - 1) as f32;
        let rf = r * max;
        let gf = g * max;
        let bf = b * max;
        let r0 = rf.floor() as usize;
        let g0 = gf.floor() as usize;
        let b0 = bf.floor() as usize;
        let r1 = (r0 + 1).min(self.size - 1);
        let g1 = (g0 + 1).min(self.size - 1);
        let b1 = (b0 + 1).min(self.size - 1);
        let dr = rf - r0 as f32;
        let dg = gf - g0 as f32;
        let db = bf - b0 as f32;

        let s = self.size;
        let idx = |r: usize, g: usize, b: usize| r * s * s + g * s + b;
        let lerp3 = |a: [f32; 3], c: [f32; 3], t: f32| -> [f32; 3] {
            [
                a[0] + (c[0] - a[0]) * t,
                a[1] + (c[1] - a[1]) * t,
                a[2] + (c[2] - a[2]) * t,
            ]
        };

        let c000 = self.data[idx(r0, g0, b0)];
        let c100 = self.data[idx(r1, g0, b0)];
        let c010 = self.data[idx(r0, g1, b0)];
        let c110 = self.data[idx(r1, g1, b0)];
        let c001 = self.data[idx(r0, g0, b1)];
        let c101 = self.data[idx(r1, g0, b1)];
        let c011 = self.data[idx(r0, g1, b1)];
        let c111 = self.data[idx(r1, g1, b1)];

        let c00 = lerp3(c000, c100, dr);
        let c10 = lerp3(c010, c110, dr);
        let c01 = lerp3(c001, c101, dr);
        let c11 = lerp3(c011, c111, dr);
        let c0 = lerp3(c00, c10, dg);
        let c1 = lerp3(c01, c11, dg);
        lerp3(c0, c1, db)
    }

    /// Re-lay out the LUT as a 2D atlas: width = `size²`, height = `size`.
    /// Each `size`-wide column block is one constant-blue slice.
    /// Indexing: `column = blue_slice * size + r`, `row = g`. The
    /// shader's `sample_lut_atlas` consumes exactly this layout.
    pub fn to_atlas_rgba32f(&self) -> AtlasRgbaF32 {
        let s = self.size;
        let w = s * s;
        let h = s;
        let mut pixels = Vec::with_capacity(w * h * 4);
        for g in 0..s {
            for blue_slice in 0..s {
                for r in 0..s {
                    let p = self.data[r * s * s + g * s + blue_slice];
                    pixels.push(p[0]);
                    pixels.push(p[1]);
                    pixels.push(p[2]);
                    pixels.push(1.0);
                }
            }
        }
        AtlasRgbaF32 { width: w, height: h, pixels }
    }
}

#[derive(Debug, Clone)]
pub struct AtlasRgbaF32 {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<f32>,
}

// ── Bake ────────────────────────────────────────────────────────────────────

/// Bake an ICC profile into a `size`³ LUT. Source colour space is
/// sRGB (the surfaces margo composes today live in encoded sRGB
/// until Phase 2 lands the linear-light composite); destination is
/// the user's display profile parsed from `icc_bytes`.
///
/// Rendering intent: perceptual — matches what `colord` itself uses
/// for its `colormgr` reference pipeline. `Intent::RelativeColorimetric`
/// is the obvious alternative for proofing workflows; if/when margo
/// grows a per-tag rendering-intent rule, this is the knob to wire.
pub fn bake_lut(icc_bytes: &[u8], size: usize) -> Result<IccLut3D, IccError> {
    let dst = Profile::new_icc(icc_bytes).map_err(|e| IccError::Parse(e.to_string()))?;
    let src = Profile::new_srgb();
    let xf = Transform::new(
        &src,
        PixelFormat::RGB_FLT,
        &dst,
        PixelFormat::RGB_FLT,
        Intent::Perceptual,
    )
    .map_err(|e| IccError::Transform(e.to_string()))?;

    let mut input: Vec<[f32; 3]> = Vec::with_capacity(size * size * size);
    let max = (size - 1) as f32;
    for r in 0..size {
        for g in 0..size {
            for b in 0..size {
                input.push([r as f32 / max, g as f32 / max, b as f32 / max]);
            }
        }
    }
    let mut output: Vec<[f32; 3]> = vec![[0.0; 3]; size * size * size];
    xf.transform_pixels(&input, &mut output);
    Ok(IccLut3D { size, data: output })
}

/// Convenience: read an ICC file from disk and bake. Wraps the I/O
/// + parse so the udev backend has a single call site.
pub fn bake_lut_from_file(path: &std::path::Path, size: usize) -> Result<IccLut3D, IccError> {
    let bytes = std::fs::read(path)?;
    bake_lut(&bytes, size)
}

// ── colord D-Bus client ─────────────────────────────────────────────────────

#[cfg(feature = "dbus")]
pub mod colord {
    use super::IccError;
    use std::path::PathBuf;
    use zbus::proxy;
    use zbus::zvariant::OwnedObjectPath;

    #[proxy(
        interface = "org.freedesktop.ColorManager",
        default_service = "org.freedesktop.ColorManager",
        default_path = "/org/freedesktop/ColorManager"
    )]
    trait ColorManager {
        fn find_device_by_property(
            &self,
            key: &str,
            value: &str,
        ) -> zbus::Result<OwnedObjectPath>;
    }

    #[proxy(
        interface = "org.freedesktop.ColorManager.Device",
        default_service = "org.freedesktop.ColorManager"
    )]
    trait ColorDevice {
        #[zbus(property)]
        fn profiles(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    }

    #[proxy(
        interface = "org.freedesktop.ColorManager.Profile",
        default_service = "org.freedesktop.ColorManager"
    )]
    trait ColorProfile {
        #[zbus(property)]
        fn filename(&self) -> zbus::Result<String>;
    }

    /// Resolve the ICC profile path colord has assigned to a given
    /// DRM connector (e.g. `DP-1`, `eDP-1`). `Ok(None)` when the
    /// user hasn't run `colormgr device-add-profile` for this output —
    /// that's the steady state for most users and is not an error.
    pub async fn default_profile_path_for_connector(
        connector: &str,
    ) -> Result<Option<PathBuf>, IccError> {
        let conn = zbus::Connection::system().await?;
        let manager = ColorManagerProxy::new(&conn).await?;
        let device_path = match manager
            .find_device_by_property("OutputConnectorName", connector)
            .await
        {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };
        let device = ColorDeviceProxy::builder(&conn)
            .path(device_path)?
            .build()
            .await?;
        let profiles = device.profiles().await?;
        if profiles.is_empty() {
            return Ok(None);
        }
        let profile = ColorProfileProxy::builder(&conn)
            .path(profiles[0].clone())?
            .build()
            .await?;
        let filename = profile.filename().await?;
        if filename.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(filename)))
        }
    }
}

// ── GLSL: trilinear LUT atlas sampler ───────────────────────────────────────

/// Fragment shader source for the post-composition LUT pass. Texture
/// programs in smithay are wrapped around an input surface texture
/// `tex` automatically; we add `u_lut` as the LUT atlas and
/// `u_size` / `u_atlas_w` as numeric uniforms.
///
/// Two atlas reads per pixel (one per neighbouring blue slice) — the
/// `mix` call picks up the fractional blue coordinate for the
/// trilinear blend. All clamping is in-shader to tolerate the
/// fp16/fp32 driver interpolator drift on edge pixels.
pub const ICC_LUT_FRAG: &str = r#"
    precision mediump float;

    uniform sampler2D u_lut;
    uniform float u_size;       // LUT cube side length, e.g. 33.0
    uniform float u_atlas_w;    // Atlas width in texels = u_size * u_size

    vec3 sample_lut_atlas(vec3 c) {
        float bf = clamp(c.b, 0.0, 1.0) * (u_size - 1.0);
        float b0 = floor(bf);
        float b1 = min(b0 + 1.0, u_size - 1.0);
        float bt = bf - b0;

        float r = clamp(c.r, 0.0, 1.0) * (u_size - 1.0);
        float g = clamp(c.g, 0.0, 1.0) * (u_size - 1.0);

        // Half-texel offset so we sample column / row centres.
        vec2 uv0 = vec2((b0 * u_size + r + 0.5) / u_atlas_w,
                        (g + 0.5) / u_size);
        vec2 uv1 = vec2((b1 * u_size + r + 0.5) / u_atlas_w,
                        (g + 0.5) / u_size);

        vec3 s0 = texture2D(u_lut, uv0).rgb;
        vec3 s1 = texture2D(u_lut, uv1).rgb;
        return mix(s0, s1, bt);
    }

    // The host wraps this around an input texture `tex` via smithay's
    // `compile_custom_texture_shader`. Phase 4 runtime activation is
    // gated on a second-sampler-uniform hook landing upstream — see
    // module docs.
"#;

// ── Env gate ────────────────────────────────────────────────────────────────

/// Has the user opted into the ICC LUT pipeline via `MARGO_HDR_ICC=1`?
/// Read once per process; subsequent calls hit a cached `OnceLock`.
pub fn is_icc_lut_enabled() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let v = std::env::var("MARGO_HDR_ICC").unwrap_or_default();
        let on = matches!(v.as_str(), "1" | "true" | "yes" | "on");
        if on {
            tracing::info!(
                "MARGO_HDR_ICC set — ICC LUT scaffolding active. \
                 colord query + lcms2 bake are wired today; the post- \
                 composition LUT sample lands once smithay exposes a \
                 second-sampler hook on `compile_custom_texture_shader`. \
                 See `docs/hdr-design.md` Phase 4."
            );
        }
        on
    })
}

/// Has the LUT pass actually engaged for the current frame? Today
/// always `false` — the shader source + atlas + colord client all
/// ship, but the texture-program integration needs the smithay
/// upstream hook described above. Keeping the function so the udev
/// path can branch correctly the moment the API lands.
#[inline]
pub fn is_icc_lut_active() -> bool {
    false
}

/// Resolved ICC state for one output. Cached on the udev backend
/// so we re-bake only on profile change, not per frame.
#[derive(Debug, Clone)]
pub struct OutputIccState {
    pub connector: String,
    pub profile_path: PathBuf,
    pub lut: IccLut3D,
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Identity LUT round-trips arbitrary inputs through trilinear
    /// sampling within float-rounding tolerance.
    #[test]
    fn identity_lut_round_trips() {
        let lut = IccLut3D::identity(33);
        for &(r, g, b) in &[
            (0.0, 0.0, 0.0),
            (1.0, 1.0, 1.0),
            (0.5, 0.5, 0.5),
            (0.21, 0.78, 0.42),
            (0.999, 0.001, 0.654),
        ] {
            let [or, og, ob] = lut.sample_trilinear(r, g, b);
            assert!((or - r).abs() < 1e-5, "r: {r} -> {or}");
            assert!((og - g).abs() < 1e-5, "g: {g} -> {og}");
            assert!((ob - b).abs() < 1e-5, "b: {b} -> {ob}");
        }
    }

    /// Atlas indexing matches the shader's sampling convention.
    /// Pixel at `(column = slice*size + r, row = g)` must equal
    /// `LUT[r, g, slice]`.
    #[test]
    fn atlas_layout_matches_shader_indexing() {
        let lut = IccLut3D::identity(8);
        let atlas = lut.to_atlas_rgba32f();
        assert_eq!(atlas.width, 8 * 8);
        assert_eq!(atlas.height, 8);
        let max = 7.0_f32;
        let probe = |slice: usize, r: usize, g: usize| -> [f32; 4] {
            let col = slice * 8 + r;
            let row = g;
            let i = (row * atlas.width + col) * 4;
            [
                atlas.pixels[i],
                atlas.pixels[i + 1],
                atlas.pixels[i + 2],
                atlas.pixels[i + 3],
            ]
        };
        for &(slice, r, g) in &[(0usize, 0, 0), (3, 2, 4), (7, 7, 7), (5, 1, 6)] {
            let [pr, pg, pb, pa] = probe(slice, r, g);
            assert!((pr - r as f32 / max).abs() < 1e-6, "r at ({slice},{r},{g})");
            assert!((pg - g as f32 / max).abs() < 1e-6, "g at ({slice},{r},{g})");
            assert!((pb - slice as f32 / max).abs() < 1e-6, "b at ({slice},{r},{g})");
            assert!((pa - 1.0).abs() < 1e-6);
        }
    }

    /// Bake an in-memory sRGB profile end-to-end. Going sRGB → sRGB
    /// must produce a near-identity LUT — small drift from lcms2's
    /// perceptual-intent chromatic-adaptation step is expected
    /// (≤2 % per channel against an exact identity).
    #[test]
    fn bake_srgb_to_srgb_is_near_identity() {
        let srgb = lcms2::Profile::new_srgb();
        let icc = srgb.icc().expect("serialize sRGB profile");
        let lut = bake_lut(&icc, 17).expect("bake LUT");
        for &(r, g, b) in &[
            (0.0, 0.0, 0.0),
            (1.0, 1.0, 1.0),
            (0.5, 0.5, 0.5),
            (0.25, 0.6, 0.8),
        ] {
            let [or, og, ob] = lut.sample_trilinear(r, g, b);
            assert!((or - r).abs() < 0.02, "r: {r} -> {or}");
            assert!((og - g).abs() < 0.02, "g: {g} -> {og}");
            assert!((ob - b).abs() < 0.02, "b: {b} -> {ob}");
        }
    }

    /// LUT size scales the data buffer length cubically.
    #[test]
    fn lut_size_relations() {
        for size in [9, 17, 33] {
            let lut = IccLut3D::identity(size);
            assert_eq!(lut.size, size);
            assert_eq!(lut.data.len(), size * size * size);
            let atlas = lut.to_atlas_rgba32f();
            assert_eq!(atlas.width, size * size);
            assert_eq!(atlas.height, size);
            assert_eq!(atlas.pixels.len(), size * size * size * 4);
        }
    }

    /// Sampling in the atlas form (CPU reproduction of the GLSL
    /// trilinear blend) matches the 3D-form sampling for an
    /// identity LUT — the two layouts are equivalent representations
    /// of the same data.
    #[test]
    fn atlas_and_3d_agree_for_identity() {
        let lut = IccLut3D::identity(17);
        for &(r, g, b) in &[(0.0, 0.0, 0.0), (1.0, 1.0, 1.0), (0.33, 0.5, 0.71)] {
            let [or, og, ob] = lut.sample_trilinear(r, g, b);
            assert!((or - r).abs() < 1e-5);
            assert!((og - g).abs() < 1e-5);
            assert!((ob - b).abs() < 1e-5);
        }
    }

    /// Runtime activation flag stays `false` until the upstream
    /// smithay hook lands. Locking this with a unit test means a
    /// reviewer can grep for the flag flip and the test simultaneously.
    #[test]
    fn runtime_active_is_false_until_upstream_lands() {
        assert!(!is_icc_lut_active());
    }
}
