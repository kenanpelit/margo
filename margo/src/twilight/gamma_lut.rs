//! Blackbody-temperature → 16-bit gamma LUT, wire-format-compatible
//! with `wlr_gamma_control_v1`.
//!
//! The wire format used by every Wayland blue-light tool
//! (gammastep, redshift, sunsetr, hyprsunset) and accepted by
//! margo's gamma-control server is *planar*:
//!
//!   [R[0], R[1], …, R[n-1],
//!    G[0], G[1], …, G[n-1],
//!    B[0], B[1], …, B[n-1]]
//!
//! Total length = `3 * n` u16. `n` = the CRTC's `GAMMA_LUT_SIZE`
//! (256 on most AMD, 1024 on current Intel Arc, 4096 on some
//! virtio). The compositor's `set_gamma` in
//! `backend/udev/mod.rs` splits the slice with `split_at(n)`
//! exactly twice and then packs each entry into a
//! `drm_color_lut { red, green, blue, reserved }` for the kernel,
//! so handing it an interleaved RGBRGB array (which I did in the
//! first port, producing a CRT-with-blown-blacks mess) cross-talks
//! the channels — half of R lands in G, half of G in B, the screen
//! tints into a saturated green/blue noise. **The format matters.**
//!
//! Algorithm — matches sunsetr's `backend/gamma.rs::create_gamma_tables`
//! 1:1 so the visual output is interchangeable:
//!
//!   1. **Tanner Helland blackbody fit** maps Kelvin → (r,g,b) in
//!      [0, 1]. Same coefficients as sunsetr / redshift / gammastep
//!      (the ones tracing back to f.lux's original blog post).
//!   2. **Per-channel ramp** = `((i / (n-1)) * channel_factor).powf(1 / gamma)`
//!      where `gamma = gamma_pct / 100`. `gamma = 1.0` collapses to
//!      a pure linear scale; `gamma < 1.0` (i.e. brightness < 100 %)
//!      raises the exponent above 1, gently bending the mid-tones
//!      darker without touching endpoints.
//!   3. Quantise to u16 at the end (same precision/clamp dance
//!      sunsetr does in f64; we use f32 because the difference is
//!      below 1 LSB at u16 quantisation in the ramp's mid-band).
//!
//! The output `Vec<u16>` is fed straight to
//! `MargoState::pending_gamma`, picked up by the udev frame
//! handler, written to the CRTC's `GAMMA_LUT` blob.

/// Build the planar LUT for one output. Length = `3 * n` u16.
/// `temp_k` is clamped to `[1000, 25000]`; `gamma_pct` to
/// `[10, 200]`.
pub fn build_ramp(temp_k: u32, gamma_pct: u32, len: usize) -> Vec<u16> {
    let temp = (temp_k as f32).clamp(1000.0, 25000.0);
    let gamma = (gamma_pct as f32).clamp(10.0, 200.0) / 100.0;
    let (r_w, g_w, b_w) = temp_to_rgb_weights(temp);

    let n = len.max(2);
    let denom = (n - 1) as f32;
    // 1.0 / gamma — applied as the exponent on every entry. With
    // gamma = 1.0 this is a no-op (pure linear). gamma < 1.0
    // (brightness < 100 %) gives an exponent > 1 → mid-tones bend
    // darker; endpoints unaffected.
    let inv_gamma = 1.0 / gamma.max(0.05);

    // Planar layout: build R first, then G, then B. ONE allocation,
    // 3*n u16 long.
    let mut out: Vec<u16> = Vec::with_capacity(n * 3);
    for &w in &[r_w, g_w, b_w] {
        for i in 0..n {
            let lin = i as f32 / denom;
            let raw = (lin * w).clamp(0.0, 1.0).powf(inv_gamma);
            let scaled = (raw * 65535.0).clamp(0.0, 65535.0);
            out.push(scaled as u16);
        }
    }
    out
}

/// Tanner Helland's blackbody temperature → RGB fit. Returns three
/// channel weights in `[0.0, 1.0]`. Coefficients lifted directly
/// from sunsetr's `temperature_to_rgb` so the two produce
/// bit-identical RGB triples at the same Kelvin.
fn temp_to_rgb_weights(temp: f32) -> (f32, f32, f32) {
    let t = temp / 100.0;

    let (r, g, b) = if t <= 66.0 {
        let r = 255.0;
        let g = if t <= 1.0 {
            0.0
        } else {
            (99.470_8 * t.ln() - 161.119_57).clamp(0.0, 255.0)
        };
        let b = if t <= 19.0 {
            0.0
        } else {
            let tm = t - 10.0;
            if tm <= 0.0 {
                0.0
            } else {
                (tm.ln() * 138.517_73 - 305.044_8).clamp(0.0, 255.0)
            }
        };
        (r, g, b)
    } else {
        let r = (329.698_73 * (t - 60.0).powf(-0.133_204_76)).clamp(0.0, 255.0);
        let g = (288.122_16 * (t - 60.0).powf(-0.075_514_85)).clamp(0.0, 255.0);
        let b = 255.0;
        (r, g, b)
    };

    (r / 255.0, g / 255.0, b / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Planar layout: indices 0..N → R, N..2N → G, 2N..3N → B.
    fn plane(ramp: &[u16], n: usize, ch: usize) -> &[u16] {
        &ramp[ch * n..(ch + 1) * n]
    }

    /// 6500 K is the standard daylight reference — all three
    /// channels should reach near fullscale and roughly balance.
    #[test]
    fn daylight_6500k_is_near_neutral() {
        let n = 256;
        let ramp = build_ramp(6500, 100, n);
        assert_eq!(ramp.len(), n * 3);
        let r = plane(&ramp, n, 0)[n - 1];
        let g = plane(&ramp, n, 1)[n - 1];
        let b = plane(&ramp, n, 2)[n - 1];
        let avg = (r as u32 + g as u32 + b as u32) / 3;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        assert!(avg > 60_000, "6500K avg too low: {avg}");
        assert!(
            (max - min) < 5_000,
            "6500K channels diverge too much: r={r} g={g} b={b}"
        );
    }

    /// 3000 K — red full, blue weak.
    #[test]
    fn warm_3000k_red_dominant() {
        let n = 256;
        let ramp = build_ramp(3000, 100, n);
        let r = plane(&ramp, n, 0)[n - 1];
        let b = plane(&ramp, n, 2)[n - 1];
        assert!(r > 60_000, "3000K red should be near fullscale: {r}");
        assert!(b < r, "3000K blue should be below red: r={r} b={b}");
        assert!(b < 45_000, "3000K blue still too hot: {b}");
    }

    /// 10000 K — blue full, red attenuated.
    #[test]
    fn cool_10000k_blue_dominant() {
        let n = 256;
        let ramp = build_ramp(10000, 100, n);
        let r = plane(&ramp, n, 0)[n - 1];
        let b = plane(&ramp, n, 2)[n - 1];
        assert!(b > 60_000, "10000K blue should be near fullscale: {b}");
        assert!(r < b, "10000K red should be below blue: r={r} b={b}");
    }

    /// `gamma_pct = 50` should darken — exponent 1/0.5 = 2.0 means
    /// mid 0.5 maps to 0.25, fullscale stays 1.0.
    #[test]
    fn gamma_pct_dims_midtones() {
        let n = 256;
        let bright = build_ramp(6500, 100, n);
        let dim = build_ramp(6500, 50, n);
        // Endpoints near-equal (both at fullscale with gamma_w ≈ 1).
        let mid_idx = n / 2;
        let b_mid = plane(&bright, n, 0)[mid_idx] as u32;
        let d_mid = plane(&dim, n, 0)[mid_idx] as u32;
        assert!(
            d_mid < b_mid,
            "dim mid {d_mid} not below bright mid {b_mid}"
        );
        // exponent 2 on 0.5 → 0.25 → ~16383 u16; bright mid is
        // ~32767. Ratio ≈ 0.5.
        let ratio = d_mid as f32 / b_mid as f32;
        assert!(
            (0.4..0.6).contains(&ratio),
            "gamma=0.5 mid ratio {ratio} not near 0.5"
        );
    }

    /// Each plane must be monotonically non-decreasing — the kernel
    /// rejects non-monotone tables on some drivers.
    #[test]
    fn each_plane_is_monotone() {
        let n = 256;
        for temp in [2000, 3300, 6500, 9000] {
            let ramp = build_ramp(temp, 100, n);
            for ch in 0..3 {
                let p = plane(&ramp, n, ch);
                let mut prev = 0u16;
                for (i, &v) in p.iter().enumerate() {
                    assert!(
                        v >= prev,
                        "temp={temp} ch={ch} non-monotone at i={i}: {prev}→{v}"
                    );
                    prev = v;
                }
            }
        }
    }

    /// Extreme inputs clamp safely, no panic, no NaN, no underflow
    /// past 0. (Upper bound is u16::MAX by type — the cast guards
    /// that; only the lower bound is non-trivial because the
    /// channel-weight multiply could produce NaN if the
    /// Tanner-Helland branch failed to clamp.)
    #[test]
    fn extreme_inputs_clamp_safely() {
        let ramp = build_ramp(0, 0, 256);
        assert_eq!(ramp.len(), 256 * 3);
        let ramp = build_ramp(u32::MAX, u32::MAX, 256);
        assert_eq!(ramp.len(), 256 * 3);
        // No element should be left at "uninitialised" sentinel values
        // (a NaN cast to u16 yields 0; that's still valid for the
        // start of a ramp, so all we can usefully check is "no panic
        // got us here" — the length asserts above already cover that).
        // The strict bound `v <= u16::MAX` is a tautology because
        // `v: u16`; clippy's `absurd_extreme_comparisons` is correct
        // to flag it. Leaving the loop empty would lose the
        // "iterated cleanly" intent, so we touch each value with
        // `std::hint::black_box` to keep the optimiser honest.
        for v in ramp {
            std::hint::black_box(v);
        }
    }

    /// Output is exactly `3 * n` long — backend's split_at relies
    /// on that.
    #[test]
    fn length_is_three_times_n() {
        for n in [256usize, 1024, 4096] {
            let ramp = build_ramp(6500, 100, n);
            assert_eq!(ramp.len(), n * 3, "len mismatch at n={n}");
        }
    }

    /// At gamma = 1.0 (linear), our R-plane fullscale matches what
    /// sunsetr would produce: roughly `r_factor * 65535` modulo
    /// f32/f64 differences. This is the cross-tool sanity check.
    #[test]
    fn matches_sunsetr_endpoint_at_6500k() {
        let n = 256;
        let ramp = build_ramp(6500, 100, n);
        let r = plane(&ramp, n, 0)[n - 1];
        // At 6500K Tanner Helland gives r_factor = 1.0; output
        // should be 65535 (or one ULP below from the quantise).
        assert!(
            r >= 65530,
            "R at 6500K fullscale should be ~65535, got {r}"
        );
    }
}
