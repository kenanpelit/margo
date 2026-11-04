//! `mscreenshot palette` — dominant colours of a region.
//!
//! Median cut over the region's opaque pixels, done natively (no
//! ImageMagick): repeatedly split the box with the widest channel range
//! at its median until `n` boxes exist, then report each box's mean
//! colour, most-populous first. Deterministic for a given image, so it is
//! tested on synthetic pixel buffers.

/// Default and ceiling for `--count`.
pub const DEFAULT_COLOURS: usize = 8;
pub const MAX_COLOURS: usize = 16;

/// Pixels sampled at most; larger regions are strided evenly. 64k keeps a
/// full 4K output under a few ms while still seeing every large area.
const MAX_SAMPLES: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Swatch {
    pub rgb: [u8; 3],
    /// Sampled pixels this colour stands for.
    pub count: usize,
}

impl Swatch {
    /// `#RRGGBB`, upper-case.
    pub fn hex(&self) -> String {
        let [r, g, b] = self.rgb;
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// `--count` value clamped to `1..=MAX_COLOURS`; junk → default.
pub fn resolve_count(value: Option<&str>) -> usize {
    value
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, MAX_COLOURS))
        .unwrap_or(DEFAULT_COLOURS)
}

/// Opaque RGB samples from an RGBA buffer (alpha < 128 skipped), strided
/// down to at most [`MAX_SAMPLES`].
pub fn sample(rgba: &[u8]) -> Vec<[u8; 3]> {
    let pixels = rgba.len() / 4;
    let stride = pixels.div_ceil(MAX_SAMPLES).max(1);
    rgba.chunks_exact(4)
        .step_by(stride)
        .filter(|p| p[3] >= 128)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn widest_channel(px: &[[u8; 3]]) -> (usize, u8) {
    let mut best = (0, 0u8);
    for c in 0..3 {
        let (lo, hi) = px.iter().fold((u8::MAX, u8::MIN), |(lo, hi), p| {
            (lo.min(p[c]), hi.max(p[c]))
        });
        let range = hi.saturating_sub(lo);
        if range > best.1 {
            best = (c, range);
        }
    }
    best
}

fn mean(px: &[[u8; 3]]) -> [u8; 3] {
    let mut sum = [0u64; 3];
    for p in px {
        for c in 0..3 {
            sum[c] += u64::from(p[c]);
        }
    }
    let n = px.len().max(1) as u64;
    // Rounded mean; each channel sum / n is ≤ 255 by construction.
    sum.map(|s| ((s + n / 2) / n).min(255) as u8)
}

/// Up to `n` dominant colours of `samples`, most-populous first. Fewer
/// come back when the image has fewer distinct colours; identical means
/// (two boxes averaging to the same hex) are merged.
pub fn extract(mut samples: Vec<[u8; 3]>, n: usize) -> Vec<Swatch> {
    if samples.is_empty() || n == 0 {
        return Vec::new();
    }
    // Boxes are index ranges into `samples`, which is re-sorted in place
    // per split so every box stays contiguous.
    let mut boxes: Vec<(usize, usize)> = vec![(0, samples.len())];
    while boxes.len() < n {
        // Split the box whose widest channel range is largest; ties go to
        // the more populous box.
        let pick = boxes
            .iter()
            .enumerate()
            .map(|(i, &(s, e))| (i, widest_channel(&samples[s..e]), e - s))
            .filter(|(_, (_, range), len)| *range > 0 && *len > 1)
            .max_by_key(|(_, (_, range), len)| (*range, *len));
        let Some((i, (channel, _), _)) = pick else {
            break; // every box is a single colour
        };
        let (s, e) = boxes[i];
        let slice = &mut samples[s..e];
        slice.sort_unstable_by_key(|p| p[channel]);
        // Split at the median, nudged off a run of equal values so both
        // halves are non-empty and actually differ.
        let mid = slice.len() / 2;
        let v = slice[mid][channel];
        let mut cut = slice.partition_point(|p| p[channel] < v);
        if cut == 0 {
            cut = slice.partition_point(|p| p[channel] <= v);
        }
        boxes[i] = (s, s + cut);
        boxes.push((s + cut, e));
    }
    let mut swatches: Vec<Swatch> = Vec::new();
    for (s, e) in boxes {
        let rgb = mean(&samples[s..e]);
        match swatches.iter_mut().find(|w| w.rgb == rgb) {
            Some(w) => w.count += e - s,
            None => swatches.push(Swatch { rgb, count: e - s }),
        }
    }
    swatches.sort_by(|a, b| b.count.cmp(&a.count).then(a.rgb.cmp(&b.rgb)));
    swatches
}

/// Clipboard text: one hex per line.
pub fn clipboard_text(swatches: &[Swatch]) -> String {
    swatches
        .iter()
        .map(Swatch::hex)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Notification body: hex codes with their share of the region.
pub fn summary(swatches: &[Swatch]) -> String {
    let total: usize = swatches.iter().map(|s| s.count).sum::<usize>().max(1);
    swatches
        .iter()
        .map(|s| format!("{}  {}%", s.hex(), (s.count * 100 + total / 2) / total))
        .collect::<Vec<_>>()
        .join("\n")
}

/// RGBA buffer of a horizontal swatch strip (`cell`×`cell` per colour),
/// shown as the notification image.
pub fn strip_rgba(swatches: &[Swatch], cell: u32) -> (u32, u32, Vec<u8>) {
    let w = cell * swatches.len().max(1) as u32;
    let h = cell;
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    for _y in 0..h {
        for x in 0..w {
            let i = (x / cell) as usize;
            let [r, g, b] = swatches.get(i).map(|s| s.rgb).unwrap_or([0, 0, 0]);
            buf.extend_from_slice(&[r, g, b, 255]);
        }
    }
    (w, h, buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(px: &[([u8; 3], usize)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (rgb, n) in px {
            for _ in 0..*n {
                out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        out
    }

    #[test]
    fn two_flat_colours_come_back_exactly_most_common_first() {
        let buf = rgba(&[([255, 0, 0], 30), ([0, 0, 255], 70)]);
        let p = extract(sample(&buf), 8);
        assert_eq!(
            p,
            [
                Swatch {
                    rgb: [0, 0, 255],
                    count: 70
                },
                Swatch {
                    rgb: [255, 0, 0],
                    count: 30
                }
            ]
        );
        assert_eq!(clipboard_text(&p), "#0000FF\n#FF0000");
        assert_eq!(summary(&p), "#0000FF  70%\n#FF0000  30%");
    }

    #[test]
    fn single_colour_and_empty_input() {
        let buf = rgba(&[([18, 52, 86], 10)]);
        let p = extract(sample(&buf), 8);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].hex(), "#123456");
        assert!(extract(Vec::new(), 8).is_empty());
        assert!(extract(sample(&buf), 0).is_empty());
    }

    #[test]
    fn transparent_pixels_are_ignored() {
        let mut buf = rgba(&[([10, 20, 30], 5)]);
        buf.extend_from_slice(&[255, 255, 255, 0]);
        buf.extend_from_slice(&[255, 255, 255, 127]);
        let p = extract(sample(&buf), 8);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].rgb, [10, 20, 30]);
    }

    #[test]
    fn asks_for_n_returns_at_most_n_clusters_of_a_gradient() {
        // A 256-step grey ramp: 4 boxes → 4 distinct greys, each a quarter.
        let px: Vec<([u8; 3], usize)> = (0..=255u8).map(|v| ([v, v, v], 1)).collect();
        let p = extract(sample(&rgba(&px)), 4);
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|s| s.count == 64), "{p:?}");
        let mut greys: Vec<u8> = p.iter().map(|s| s.rgb[0]).collect();
        greys.sort_unstable();
        assert_eq!(greys, [32, 96, 160, 224]);
    }

    #[test]
    fn three_well_separated_clusters_are_found() {
        let buf = rgba(&[
            ([250, 10, 10], 40),
            ([246, 14, 12], 40),
            ([10, 200, 10], 50),
            ([12, 204, 14], 50),
            ([20, 20, 240], 20),
        ]);
        let p = extract(sample(&buf), 3);
        let hex: Vec<String> = p.iter().map(Swatch::hex).collect();
        assert_eq!(hex, ["#0BCA0C", "#F80C0B", "#1414F0"], "{p:?}");
    }

    #[test]
    fn large_inputs_are_strided() {
        let buf = rgba(&[([1, 2, 3], 200_000)]);
        let s = sample(&buf);
        assert!(s.len() <= MAX_SAMPLES, "{}", s.len());
        assert!(s.len() > MAX_SAMPLES / 2);
    }

    #[test]
    fn count_flag_is_clamped() {
        assert_eq!(resolve_count(None), 8);
        assert_eq!(resolve_count(Some("4")), 4);
        assert_eq!(resolve_count(Some("0")), 1);
        assert_eq!(resolve_count(Some("99")), 16);
        assert_eq!(resolve_count(Some("lots")), 8);
    }

    #[test]
    fn strip_has_one_cell_per_colour() {
        let p = [
            Swatch {
                rgb: [1, 2, 3],
                count: 1,
            },
            Swatch {
                rgb: [4, 5, 6],
                count: 1,
            },
        ];
        let (w, h, buf) = strip_rgba(&p, 2);
        assert_eq!((w, h, buf.len()), (4, 2, 32));
        assert_eq!(&buf[0..4], [1, 2, 3, 255]);
        assert_eq!(&buf[8..12], [4, 5, 6, 255]);
    }
}
