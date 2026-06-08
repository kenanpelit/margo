//! Tab-strip chrome for tabbed window groups (`togglegroup`).
//!
//! A grouped tile shows one member at a time (see `state::groups`). To
//! make the hidden members visible/pickable, we draw a thin strip of
//! solid-colour chips along the top edge of the active member's slot —
//! one chip per group member, the active member's chip filled with
//! `group_active_color`, the rest with `group_inactive_color` (both
//! matugen-overridable via `colors.conf`).
//!
//! This is deliberately a **minimal** chrome: flat solid quads, no text
//! / icons / rounded corners. It reuses smithay's `SolidColorRenderElement`
//! (the same primitive `config_error_overlay` and the screencast blackout
//! use) so it needs no new GLES program and slots straight into the
//! existing `MargoRenderElement::Solid` variant. Per-chip click/scroll
//! hit-testing is a follow-up (the `changegroupactive` keybind drives
//! cycling today); the geometry helper here is written so that input
//! code can re-derive the same chip rects when that lands.
//!
//! Drawn only when `config.group_bar_height > 0`. Default is `0`, so
//! the strip is invisible until the user opts in — groups still work
//! by keybind without it.

use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::GlesPixelProgram;
use smithay::utils::{Physical, Point, Rectangle, Size};

use crate::layout::Rect;
use crate::render::rounded_solid::RoundedSolidElement;

/// One tab chip in output-local **logical** coordinates, paired with the
/// index of the client it represents. Shared shape for both rendering
/// and (future) pointer hit-testing.
#[derive(Clone, Copy, Debug)]
pub struct TabChip {
    /// Index into `MargoState::clients` of the member this chip selects.
    /// Consumed by the (deferred) pointer hit-test that maps a click on
    /// a chip to `activate_group_member`; carried now so the geometry
    /// is computed in exactly one place.
    #[allow(dead_code)]
    pub client_idx: usize,
    /// Chip rect in logical, output-local coords.
    pub rect: Rect,
    /// Whether this chip's member is the active (displayed) one.
    pub active: bool,
}

/// Lay out the tab chips for a group occupying `slot` (the active
/// member's geometry), given the ordered `members` (client indices)
/// and which one is `active_idx`. Coordinates are global-logical
/// (same space as `client.geom`); callers translate to output-local /
/// physical as needed.
///
/// The strip sits directly above the slot's top edge so it never
/// overlaps window content. Chips divide the slot width evenly, minus
/// `gap` between them.
pub fn chip_rects(
    slot: Rect,
    members: &[usize],
    active_idx: usize,
    bar_height: i32,
    gap: i32,
) -> Vec<TabChip> {
    let n = members.len();
    if n == 0 || bar_height <= 0 || slot.width <= 0 {
        return Vec::new();
    }
    let gap = gap.max(0);
    let total_gap = gap * (n as i32 - 1).max(0);
    let avail = (slot.width - total_gap).max(n as i32); // ≥ 1px/chip
    let chip_w = avail / n as i32;
    let strip_y = slot.y - bar_height; // sit above the content

    let mut out = Vec::with_capacity(n);
    let mut x = slot.x;
    for (i, &client_idx) in members.iter().enumerate() {
        // Last chip absorbs integer-division remainder so the strip
        // spans the full slot width exactly.
        let w = if i + 1 == n {
            (slot.x + slot.width) - x
        } else {
            chip_w
        };
        out.push(TabChip {
            client_idx,
            rect: Rect::new(x, strip_y, w.max(1), bar_height),
            active: client_idx == active_idx,
        });
        x += w + gap;
    }
    out
}

/// Build the rounded-solid render elements for a group's tab strip.
///
/// `slot` is the active member's geometry (global-logical),
/// `output_origin` is the output's top-left in global-logical coords,
/// `scale` is the output scale, `radius` the corner radius (logical px),
/// and `program` the compiled rounded-solid shader. One element per member.
#[allow(clippy::too_many_arguments)]
pub fn render_elements(
    slot: Rect,
    members: &[usize],
    active_idx: usize,
    bar_height: i32,
    gap: i32,
    active_color: [f32; 4],
    inactive_color: [f32; 4],
    output_origin: Point<i32, smithay::utils::Logical>,
    output_scale: f64,
    radius: f32,
    program: GlesPixelProgram,
) -> Vec<RoundedSolidElement> {
    let radius_phys = radius * output_scale as f32;
    chip_rects(slot, members, active_idx, bar_height, gap)
        .into_iter()
        .map(|chip| {
            let loc = Point::<i32, Physical>::from((
                (((chip.rect.x - output_origin.x) as f64) * output_scale).round() as i32,
                (((chip.rect.y - output_origin.y) as f64) * output_scale).round() as i32,
            ));
            let size = Size::<i32, Physical>::from((
                ((chip.rect.width as f64) * output_scale).round() as i32,
                ((chip.rect.height as f64) * output_scale).round() as i32,
            ));
            let color = if chip.active {
                active_color
            } else {
                inactive_color
            };
            RoundedSolidElement::new(
                Id::new(),
                Rectangle::new(loc, size),
                radius_phys,
                color,
                program.clone(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_span_full_slot_width() {
        let slot = Rect::new(100, 200, 600, 400);
        let members = vec![3, 7, 9];
        let chips = chip_rects(slot, &members, 7, 24, 4);
        assert_eq!(chips.len(), 3);
        // Strip sits above the slot top.
        assert!(chips.iter().all(|c| c.rect.y == 200 - 24));
        assert!(chips.iter().all(|c| c.rect.height == 24));
        // First chip starts at slot.x, last ends at slot.x+slot.width.
        assert_eq!(chips[0].rect.x, 100);
        let last = chips.last().unwrap();
        assert_eq!(last.rect.x + last.rect.width, 700);
        // Exactly one active chip, matching active_idx.
        assert_eq!(chips.iter().filter(|c| c.active).count(), 1);
        assert!(chips[1].active);
    }

    #[test]
    fn disabled_when_bar_height_zero() {
        let slot = Rect::new(0, 0, 800, 600);
        assert!(chip_rects(slot, &[1, 2], 1, 0, 4).is_empty());
    }

    #[test]
    fn empty_for_no_members() {
        let slot = Rect::new(0, 0, 800, 600);
        assert!(chip_rects(slot, &[], 0, 24, 4).is_empty());
    }
}
