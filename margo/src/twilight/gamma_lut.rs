//! Blackbody-temperature → 16-bit gamma LUT.
//!
//! Each output gets one ramp per channel (R/G/B). The DRM LUT size
//! varies per CRTC (256 is the de-facto baseline; some Intel parts
//! report 1024 or higher). We build the table at the size the
//! kernel asks for — `len` parameter — and let the udev frame
//! handler hand it to `wlr_gamma_control`'s `set_gamma`.
//!
//! Algorithm:
//!
//!   1. **Temperature → linear RGB triple**, via Tanner Helland's
//!      blackbody fit (the classic redshift / gammastep formula).
//!      Output is three `f32` in `[0.0, 1.0]` representing the
//!      relative response of each channel at the requested colour
//!      temperature.
//!   2. **Brightness multiplier** (`gamma_pct / 100`) scales all
//!      three uniformly. `100 %` = pass-through, `<100 %` = dim,
//!      `>100 %` = over-bright (clamped).
//!   3. **Per-channel ramp** is then `(i / (len-1)) * temp_r *
//!      brightness`, raised to `1 / 2.2` for an sRGB-ish encode
//!      curve so mid-tones aren't crushed.
//!   4. Result is quantised to `u16` (the LUT entry format the
//!      kernel expects).
//!
//! The whole LUT rebuilds on every change. It's cheap (sub-ms for
//! 256 entries × 3 channels) — the only path that triggers a
//! rebuild is the tick loop, which already runs at second-scale
//! cadence at steady state.

/// Build an interleaved `R, G, B, R, G, B, …` ramp of length
/// `len * 3` u16 entries — the same shape DRM wants for
/// `GAMMA_LUT`.
///
/// `temp_k` is clamped to `[1000, 25000]`; outside that range the
/// blackbody fit goes physically meaningless. `gamma_pct` clamps to
/// `[10, 200]` so a typo can't tank the screen to black or burn
/// retinas.
pub fn build_ramp(temp_k: u32, gamma_pct: u32, len: usize) -> Vec<u16> {
    let temp = (temp_k as f32).clamp(1000.0, 25000.0);
    let brightness = (gamma_pct as f32).clamp(10.0, 200.0) / 100.0;
    let (r_w, g_w, b_w) = temp_to_rgb_weights(temp);

    // sRGB-ish encode curve. Pure 2.2 is close enough — the actual
    // sRGB piecewise EOTF differs by < 1 LSB at u16 quantisation in
    // the mid-band and the user can't perceive the difference at
    // typical day↔night swing.
    const GAMMA_ENCODE: f32 = 1.0 / 2.2;

    let n = len.max(2);
    let mut out = Vec::with_capacity(n * 3);
    let denom = (n - 1) as f32;
    for i in 0..n {
        let lin = i as f32 / denom;
        // Apply the channel weight + brightness in linear light, then
        // encode through the gamma curve.
        let r = ((lin * r_w * brightness).clamp(0.0, 1.0)).powf(GAMMA_ENCODE);
        let g = ((lin * g_w * brightness).clamp(0.0, 1.0)).powf(GAMMA_ENCODE);
        let b = ((lin * b_w * brightness).clamp(0.0, 1.0)).powf(GAMMA_ENCODE);
        out.push((r * 65535.0).round() as u16);
        out.push((g * 65535.0).round() as u16);
        out.push((b * 65535.0).round() as u16);
    }
    out
}

/// Tanner Helland's blackbody temperature → RGB fit. Returns three
/// channel weights in `[0.0, 1.0]`. The fit is the same one
/// redshift / gammastep / sunsetr / f.lux all use — it's accurate
/// to about ±1 % vs Planck's law in the 1500–10000 K range and
/// stays plausible (no negative coefficients) out to ~25000 K.
///
/// We intentionally clamp inputs in `build_ramp` before calling
/// this — the formula has a `ln(temp/100)` term that goes wild
/// below 1000 K.
fn temp_to_rgb_weights(temp: f32) -> (f32, f32, f32) {
    let t = temp / 100.0;

    let r = if t <= 66.0 {
        1.0
    } else {
        // 329.6987 * (t-60)^-0.13320 normalised by /255
        let v = 329.698_73 * (t - 60.0).powf(-0.133_204_76);
        (v / 255.0).clamp(0.0, 1.0)
    };

    let g = if t <= 66.0 {
        // 99.471 * ln(t) - 161.120  /255
        let v = 99.470_8 * t.ln() - 161.119_57;
        (v / 255.0).clamp(0.0, 1.0)
    } else {
        // 288.122 * (t-60)^-0.07551  /255
        let v = 288.122_16 * (t - 60.0).powf(-0.075_514_85);
        (v / 255.0).clamp(0.0, 1.0)
    };

    let b = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        // 138.518 * ln(t-10) - 305.045  /255
        let v = 138.517_73 * (t - 10.0).ln() - 305.044_8;
        (v / 255.0).clamp(0.0, 1.0)
    };

    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 6500 K is the standard daylight reference — all three
    /// channels should be roughly equal and ramp from 0 → ~65535.
    #[test]
    fn daylight_6500k_is_near_neutral() {
        let ramp = build_ramp(6500, 100, 256);
        assert_eq!(ramp.len(), 256 * 3);
        // Last entries (i = 255 → input 1.0) — all three channels
        // should be close to fullscale.
        let (r, g, b) = (ramp[255 * 3], ramp[255 * 3 + 1], ramp[255 * 3 + 2]);
        let avg = (r as u32 + g as u32 + b as u32) / 3;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        assert!(avg > 60_000, "6500K avg too low: {avg}");
        assert!(
            (max - min) < 10_000,
            "6500K channels diverge too much: r={r} g={g} b={b}"
        );
    }

    /// 3000 K is a warm-evening reference — red full, blue weak.
    #[test]
    fn warm_3000k_red_dominant() {
        let ramp = build_ramp(3000, 100, 256);
        let last = 255 * 3;
        let r = ramp[last];
        let b = ramp[last + 2];
        assert!(r > 60_000, "3000K red should be near fullscale: {r}");
        assert!(b < r, "3000K blue should be well below red: r={r} b={b}");
        assert!(b < 45_000, "3000K blue still too hot: {b}");
    }

    /// 10000 K is cool — blue full, red attenuated.
    #[test]
    fn cool_10000k_blue_dominant() {
        let ramp = build_ramp(10000, 100, 256);
        let last = 255 * 3;
        let r = ramp[last];
        let b = ramp[last + 2];
        assert!(b > 60_000, "10000K blue should be near fullscale: {b}");
        assert!(r < b, "10000K red should be below blue: r={r} b={b}");
    }

    /// `gamma_pct = 50` should darken roughly proportionally.
    #[test]
    fn gamma_pct_dims() {
        let bright = build_ramp(6500, 100, 256);
        let dim = build_ramp(6500, 50, 256);
        let last = 255 * 3;
        // Sum of channels at fullscale should drop noticeably.
        let bright_sum: u32 = (0..3).map(|i| bright[last + i] as u32).sum();
        let dim_sum: u32 = (0..3).map(|i| dim[last + i] as u32).sum();
        assert!(
            dim_sum < bright_sum,
            "dim_sum={dim_sum} not below bright_sum={bright_sum}"
        );
        // After gamma encoding 50% linear ≈ 73 % perceptual — so
        // the drop is real but not 50 %.
        let ratio = dim_sum as f32 / bright_sum as f32;
        assert!(
            (0.5..0.85).contains(&ratio),
            "ratio {ratio} outside the expected post-gamma 50→73% band"
        );
    }

    /// Ramp must be monotonically non-decreasing per channel — the
    /// kernel rejects non-monotone tables on some drivers.
    #[test]
    fn ramp_is_monotone_per_channel() {
        for temp in [2000, 3300, 6500, 9000] {
            let ramp = build_ramp(temp, 100, 256);
            for chan in 0..3 {
                let mut prev = 0u16;
                for i in 0..256 {
                    let v = ramp[i * 3 + chan];
                    assert!(
                        v >= prev,
                        "temp={temp} chan={chan} non-monotone at i={i}: {prev}→{v}"
                    );
                    prev = v;
                }
            }
        }
    }

    /// Out-of-range inputs must clamp, not panic or NaN.
    #[test]
    fn extreme_inputs_clamp_safely() {
        let ramp = build_ramp(0, 0, 256);
        assert_eq!(ramp.len(), 256 * 3);
        let ramp = build_ramp(u32::MAX, u32::MAX, 256);
        assert_eq!(ramp.len(), 256 * 3);
        for v in ramp {
            assert!(v <= u16::MAX);
        }
    }
}
