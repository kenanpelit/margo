//! ASCII preview + auto-placement of layout outputs.
//!
//! The preview draws each output as a coloured rectangle in a
//! fixed-width terminal grid, with the output's display name
//! centred inside. Used by `margo-layout list --preview` and the
//! `pick` TUI to give the user a visual sanity check that the
//! layout file matches the physical setup they think it does.
//!
//! ## Auto-placement
//!
//! Margo places outputs left-to-right by `monitorrule` order with
//! gaps where the user explicitly set `x:`/`y:`. When a layout
//! file omits position fields, the runtime walks the rule list and
//! puts each unplaced output flush-right of everything placed so
//! far. We mirror that behaviour here so the preview shape matches
//! what margo will end up producing on next reload — there's no
//! point teasing the user with a preview that diverges from the
//! actual outcome.
//!
//! Outputs *with* explicit position are placed first, in declared
//! order. If two collide, the second is bumped to the right edge
//! (margo prints a warning in that case; we silently adjust). Then
//! unpositioned outputs fill in left-to-right. The same algorithm
//! niri uses, ported one-to-one to margo's monitorrule semantics.

use crate::parser::{Layout, LayoutOutput};

/// Output placed onto the preview grid.
#[derive(Debug, Clone)]
pub struct PlacedOutput {
    pub label: String,
    pub color: u8,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Run margo's auto-placement algorithm over the layout's outputs
/// and return the resolved rectangles. Sorted by connector name
/// so the placement is deterministic across runs (margo also
/// sorts internally, since hot-plug order is non-deterministic).
pub fn place_outputs(layout: &Layout) -> Vec<PlacedOutput> {
    let mut outputs: Vec<&LayoutOutput> = layout.outputs.iter().collect();
    outputs.sort_by(|a, b| a.connector.cmp(&b.connector));

    let mut placed: Vec<PlacedOutput> = Vec::new();
    let mut auto_x = 0i32;

    // Pass 1: outputs with explicit (non-zero) position.
    for o in &outputs {
        if !has_explicit_position(o) {
            continue;
        }
        let (mut x, mut y) = (o.x, o.y);
        let (w, h) = (o.width, o.height);

        let overlap = placed.iter().any(|p| {
            x + w > p.x && x < p.x + p.width && y + h > p.y && y < p.y + p.height
        });
        if overlap {
            x = auto_x;
            y = 0;
        }
        placed.push(make(o, x, y));
        auto_x = auto_x.max(x + w);
    }

    // Pass 2: outputs without explicit position — flush-right.
    for o in &outputs {
        if has_explicit_position(o) {
            continue;
        }
        let (w, h) = (o.width, o.height);
        let (x, y) = (auto_x, 0);
        placed.push(make(o, x, y));
        let _ = h; // height doesn't shift the cursor; layout is row-only
        auto_x = x + w;
    }

    placed
}

fn has_explicit_position(o: &LayoutOutput) -> bool {
    o.has_position
}

fn make(o: &LayoutOutput, x: i32, y: i32) -> PlacedOutput {
    let label = o
        .label
        .clone()
        .unwrap_or_else(|| {
            if o.connector.is_empty() {
                "?".to_string()
            } else {
                o.connector.clone()
            }
        });
    let color = o.color.unwrap_or_else(|| auto_colour(&label));
    PlacedOutput {
        label,
        color,
        x,
        y,
        width: o.width,
        height: o.height,
    }
}

/// Hash the label to pick a stable palette index 1..=17. Index 0
/// (gray) is reserved for the auto-fallback when the connector
/// name is empty.
fn auto_colour(name: &str) -> u8 {
    let mut hash: u32 = 2_166_136_261;
    for byte in name.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    1 + (hash % 17) as u8
}

/// 17-colour palette → ANSI 256-colour codes, picked to roughly
/// match Tailwind's 500/700 family (saturated mid-tones that read
/// well on both light and dark terminals).
const PALETTE_BG: [u8; 18] = [
    240, // 0  gray
    124, // 1  red
    166, // 2  orange
    178, // 3  amber
    100, // 4  yellow
    70,  // 5  lime
    34,  // 6  green
    36,  // 7  emerald
    37,  // 8  teal
    38,  // 9  cyan
    39,  // 10 sky
    33,  // 11 blue
    62,  // 12 indigo
    98,  // 13 violet
    91,  // 14 purple
    164, // 15 fuchsia
    169, // 16 pink
    160, // 17 rose
];

const PALETTE_FG: [u8; 18] = [
    255, 255, 255, 232, 232, 232, 255, 255, 255, 255, 255, 255, 255, 255, 255,
    255, 255, 255,
];

/// Render the layout as a fixed-height ASCII grid, returning the
/// rendered string. The grid is laid out so the longest dimension
/// fits in `cols` columns; rectangles are aspect-correct (column
/// is ~2x the cell-height ratio).
pub fn render_ascii(layout: &Layout, cols: usize) -> String {
    let placed = place_outputs(layout);

    if placed.is_empty() {
        return "  (no outputs in this layout — add a `monitorrule` line)\n".to_string();
    }

    // Compute the bounding box.
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for p in &placed {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x + p.width);
        max_y = max_y.max(p.y + p.height);
    }
    let bbox_w = (max_x - min_x).max(1) as f64;
    let bbox_h = (max_y - min_y).max(1) as f64;

    // Terminal cells are roughly 2x taller than wide; multiply
    // the column-direction scale to keep aspect ratio readable.
    let cell_aspect = 2.0_f64;
    let cols = cols.max(20);
    let scale_x = (cols as f64) / bbox_w;
    let max_rows = 14_usize;
    let scale_y = (max_rows as f64 * cell_aspect) / bbox_h;
    let scale = scale_x.min(scale_y);

    let grid_w = ((bbox_w * scale).round() as usize).max(20);
    let grid_h = ((bbox_h * scale / cell_aspect).round() as usize).max(4);

    // Cell stores (bg_index, ch). We render every output as filled
    // rectangle, label centred. Later rectangles overwrite earlier
    // ones — same as the live render's painter's algorithm.
    let mut cells: Vec<(Option<u8>, char)> = vec![(None, ' '); grid_w * grid_h];

    for p in &placed {
        let rx = ((p.x - min_x) as f64 * scale).round() as i32;
        let ry = ((p.y - min_y) as f64 * scale / cell_aspect).round() as i32;
        let rw = ((p.width as f64 * scale).round() as i32).max(1);
        let rh = ((p.height as f64 * scale / cell_aspect).round() as i32).max(1);

        let x0 = rx.max(0) as usize;
        let y0 = ry.max(0) as usize;
        let x1 = ((rx + rw) as usize).min(grid_w);
        let y1 = ((ry + rh) as usize).min(grid_h);

        for yy in y0..y1 {
            for xx in x0..x1 {
                cells[yy * grid_w + xx] = (Some(p.color), ' ');
            }
        }

        // Centre the label.
        let label = if p.label.len() <= rw as usize {
            p.label.clone()
        } else {
            p.label.chars().take((rw as usize).max(1)).collect()
        };
        let lx = x0 + (((x1 - x0).saturating_sub(label.len())) / 2);
        let ly = y0 + (y1 - y0) / 2;
        if ly < grid_h {
            for (i, ch) in label.chars().enumerate() {
                let xx = lx + i;
                if xx < x1 {
                    cells[ly * grid_w + xx] = (Some(p.color), ch);
                }
            }
        }
    }

    let mut out = String::with_capacity(grid_w * grid_h * 4);
    for y in 0..grid_h {
        for x in 0..grid_w {
            let (bg, ch) = cells[y * grid_w + x];
            match bg {
                Some(idx) => {
                    let bg = PALETTE_BG[idx as usize];
                    let fg = PALETTE_FG[idx as usize];
                    out.push_str(&format!(
                        "\x1b[48;5;{}m\x1b[38;5;{}m{}\x1b[0m",
                        bg, fg, ch
                    ));
                }
                None => out.push(ch),
            }
        }
        out.push('\n');
    }
    out
}

/// Single-line summary string for `list` output: the layout's
/// outputs with their colours.
pub fn render_inline(layout: &Layout) -> String {
    let placed = place_outputs(layout);
    let mut buf = String::new();
    for (i, p) in placed.iter().enumerate() {
        if i > 0 {
            buf.push(' ');
        }
        let bg = PALETTE_BG[p.color as usize];
        let fg = PALETTE_FG[p.color as usize];
        buf.push_str(&format!(
            "\x1b[48;5;{}m\x1b[38;5;{}m {} \x1b[0m",
            bg, fg, p.label
        ));
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Layout, LayoutOutput};

    fn out(name: &str, x: i32, y: i32, w: i32, h: i32) -> LayoutOutput {
        LayoutOutput {
            connector: name.to_string(),
            label: None,
            color: None,
            x,
            y,
            has_position: x != 0 || y != 0,
            width: w,
            height: h,
            transform: 0,
        }
    }

    #[test]
    fn places_explicit_then_auto() {
        let layout = Layout {
            path: Default::default(),
            slug: "t".into(),
            name: "t".into(),
            shortcuts: vec![],
            outputs: vec![
                out("DP-1", 0, 0, 1920, 1080),    // no explicit → auto
                out("DP-2", 0, 0, 1280, 720),     // no explicit → auto
                out("DP-3", 1920, 0, 1920, 1080), // explicit
            ],
        };
        let placed = place_outputs(&layout);
        // Pass 1 places DP-3 first; Pass 2 fills DP-1, DP-2 flush-right.
        assert_eq!(placed[0].label, "DP-3");
        assert_eq!(placed[0].x, 1920);
        let dp1 = placed.iter().find(|p| p.label == "DP-1").unwrap();
        let dp2 = placed.iter().find(|p| p.label == "DP-2").unwrap();
        // Both auto-placed land >= the right edge of DP-3 (3840).
        assert!(dp1.x >= 3840);
        assert!(dp2.x >= 3840);
    }

    #[test]
    fn render_ascii_produces_output() {
        let layout = Layout {
            path: Default::default(),
            slug: "t".into(),
            name: "t".into(),
            shortcuts: vec![],
            outputs: vec![out("DP-1", 0, 0, 1920, 1080)],
        };
        let s = render_ascii(&layout, 40);
        // ANSI escapes interleave between chars, so we just check
        // each label glyph is present somewhere in the buffer.
        for ch in "DP-1".chars() {
            assert!(
                s.contains(ch),
                "expected `{}` in rendered preview:\n{}",
                ch,
                s
            );
        }
    }
}
