//! A tag drawn as a tiny proportional map of its windows (the MargoTags
//! `minimap` option, port of the niri-workspace-preview plugin's idea onto
//! margo tags). Pure, so the geometry is unit-tested against a real state
//! frame; the bar pill only paints the result.

use crate::state_json::{RawOutput, StateJson};

/// One window in a tag minimap, in 0..1 units of the map's bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiniRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub focused: bool,
    pub urgent: bool,
    pub floating: bool,
}

/// A tag's minimap.
#[derive(Debug, Clone, PartialEq)]
pub struct TagMinimap {
    /// Tiled windows first, floating ones after (drawn on top).
    pub windows: Vec<MiniRect>,
    /// The output's area inside the bounds when windows run past it (an
    /// off-screen scroller column): the viewport outline. `None` when the
    /// bounds are exactly the output.
    pub viewport: Option<(f32, f32, f32, f32)>,
    /// Width / height of the bounds, so the pill keeps the proportions.
    pub aspect: f32,
}

impl Default for TagMinimap {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            viewport: None,
            aspect: 16.0 / 9.0,
        }
    }
}

/// Build the minimap of tag `bit` on `output`: every window of the tag on
/// that output that is on screen when the tag is shown (not minimized, not
/// a tucked-away scratchpad), normalised to the union of the output and
/// those windows. A hidden tag keeps its windows' last arranged geometry,
/// which is what the tag looks like when shown.
pub fn tag_minimap(state: &StateJson, output: &RawOutput, bit: u32) -> TagMinimap {
    let (out_w, out_h) = output.logical_size();
    let out = (output.x, output.y, out_w.max(1), out_h.max(1));
    let mut wins: Vec<_> = state
        .clients
        .iter()
        .filter(|c| c.monitor == output.name && c.tags & bit != 0)
        .filter(|c| !c.minimized && (!c.scratchpad || c.scratchpad_shown))
        .filter(|c| c.width > 0 && c.height > 0)
        .collect();
    // Floating windows sit above the tiles (stable: keeps stacking order).
    wins.sort_by_key(|c| c.floating);
    let (mut x0, mut y0) = (out.0, out.1);
    let (mut x1, mut y1) = (out.0 + out.2, out.1 + out.3);
    for c in &wins {
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x.saturating_add(c.width));
        y1 = y1.max(c.y.saturating_add(c.height));
    }
    let (bw, bh) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
    let norm = |x: i32, y: i32, w: i32, h: i32| {
        (
            (x - x0) as f32 / bw,
            (y - y0) as f32 / bh,
            w as f32 / bw,
            h as f32 / bh,
        )
    };
    let viewport = ((x0, y0, x1, y1) != (out.0, out.1, out.0 + out.2, out.1 + out.3))
        .then(|| norm(out.0, out.1, out.2, out.3));
    TagMinimap {
        windows: wins
            .into_iter()
            .map(|c| {
                let (x, y, w, h) = norm(c.x, c.y, c.width, c.height);
                MiniRect {
                    x,
                    y,
                    w,
                    h,
                    focused: c.focused,
                    urgent: c.urgent,
                    floating: c.floating,
                }
            })
            .collect(),
        viewport,
        aspect: bw / bh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `mctl get state` frame (titles / pids redacted): DP-3
    /// 2560x1440 at 0,0 in the scroller layout, eDP-1 1920x1200 at
    /// 320,1440. Tag 4 on DP-3 has a second column off screen to the right.
    fn fixture() -> StateJson {
        serde_json::from_str(include_str!("testdata/state-two-outputs.json"))
            .expect("fixture parses")
    }

    fn output<'a>(state: &'a StateJson, name: &str) -> &'a RawOutput {
        state.outputs.iter().find(|o| o.name == name).expect(name)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn a_single_window_maps_into_the_output() {
        let s = fixture();
        let m = tag_minimap(&s, output(&s, "DP-3"), 1 << 1);
        assert_eq!(m.viewport, None, "nothing runs past the screen");
        assert!(close(m.aspect, 2560.0 / 1440.0));
        let [w] = m.windows[..] else {
            panic!("one window: {m:?}")
        };
        assert!(w.focused && !w.floating && !w.urgent);
        assert!(close(w.x, 217.0 / 2560.0) && close(w.y, 40.0 / 1440.0));
        assert!(close(w.w, 2125.0 / 2560.0) && close(w.h, 1368.0 / 1440.0));
    }

    #[test]
    fn an_off_screen_scroller_column_widens_the_map_and_outlines_the_screen() {
        let s = fixture();
        let m = tag_minimap(&s, output(&s, "DP-3"), 1 << 3);
        assert_eq!(m.windows.len(), 2);
        // Bounds: x 0..(2354 + 2125), the output's full height.
        let bw = (2354 + 2125) as f32;
        assert!(close(m.aspect, bw / 1440.0));
        let (vx, vy, vw, vh) = m.viewport.expect("viewport outline");
        assert!(close(vx, 0.0) && close(vy, 0.0));
        assert!(close(vw, 2560.0 / bw) && close(vh, 1.0));
        let right = m.windows.iter().map(|w| w.x + w.w).fold(0.0, f32::max);
        assert!(close(right, 1.0), "the far column reaches the map's edge");
    }

    #[test]
    fn a_tag_is_mapped_on_its_own_output_only() {
        let s = fixture();
        // Tag 9 lives on eDP-1 (global origin 320,1440).
        let m = tag_minimap(&s, output(&s, "eDP-1"), 1 << 8);
        let [w] = m.windows[..] else { panic!("{m:?}") };
        assert!(close(w.x, 4.0 / 1920.0) && close(w.y, 40.0 / 1200.0));
        assert!(
            tag_minimap(&s, output(&s, "DP-3"), 1 << 8)
                .windows
                .is_empty()
        );
    }

    #[test]
    fn minimized_and_hidden_scratchpads_are_left_out_and_floats_go_on_top() {
        let mut s = fixture();
        let template = s.clients[0].clone();
        let mut float = template.clone();
        float.floating = true;
        float.focused = false;
        let mut minimized = template.clone();
        minimized.minimized = true;
        let mut tucked = template.clone();
        tucked.scratchpad = true;
        tucked.scratchpad_shown = false;
        s.clients = vec![float, minimized, tucked, template];
        let m = tag_minimap(&s, output(&s, "DP-3"), 1 << 1);
        let floating: Vec<bool> = m.windows.iter().map(|w| w.floating).collect();
        assert_eq!(floating, [false, true]);
    }

    #[test]
    fn an_empty_tag_is_an_empty_map_of_the_output() {
        let s = fixture();
        let m = tag_minimap(&s, output(&s, "DP-3"), 1 << 7);
        assert!(m.windows.is_empty() && m.viewport.is_none());
        assert!(close(m.aspect, 2560.0 / 1440.0));
    }
}
