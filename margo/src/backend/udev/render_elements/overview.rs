//! Overlay scenes: the MRU (alt-tab) switcher and the scroller overview,
//! with its off-screen cell culling.

use super::*;

/// MRU window-switcher overlay: a centred row of scaled-down window
/// thumbnails drawn on top of the normal desktop while the switcher is open,
/// the selected one ringed with the accent colour. Thumbnails are off-screen
/// captures cached per window (`take_mru_thumbnails`, re-captured only when
/// the window commits); every element keeps a stable id with a commit counter
/// that changes only with its content, so an idle switcher costs no damage.
/// Returned elements are appended above windows but below the cursor.
pub(super) fn build_mru_switcher_elements(
    renderer: &mut GlesRenderer,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
) -> Vec<MargoRenderElement> {
    use crate::render::open_close::{OpenCloseKind, OpenCloseRenderElement};
    use smithay::backend::renderer::utils::CommitCounter;

    let Some(sw) = state.mru_switcher.as_ref() else {
        return Vec::new();
    };
    let scale = Scale::from(output_scale);
    let mut out: Vec<MargoRenderElement> = Vec::new();

    // Layout (logical px, output-local).
    let th = (state.config.mru_thumb_height as i32).clamp(60, 600);
    let show_labels = state.config.mru_show_labels;
    let gap = (state.config.mru_thumb_gap as i32).clamp(0, 64);
    let pad = (state.config.mru_panel_padding as i32).clamp(0, 80);
    let max = (state.config.mru_max as usize).clamp(2, 20);
    const TITLE_H: i32 = 22;
    let label_h: i32 = if show_labels { 20 } else { 0 };

    // Accent theming: tie the selection ring, scope title and selected label to
    // the matugen focus colour when enabled, else keep the neutral greys.
    let accent = state.config.mru_accent_selection;
    let fc = state.config.focuscolor.0;
    let accent_rgb: [u8; 3] = [
        (fc[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (fc[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (fc[2] * 255.0).round().clamp(0.0, 255.0) as u8,
    ];
    let ring_color = if accent {
        fc
    } else {
        state.config.group_active_color.0
    };
    let title_rgb = if accent { accent_rgb } else { [200, 200, 210] };

    // Sliding window of at most `max` candidates, centred on the
    // selection. `sw.selected` indexes the FULL candidate list (it wraps
    // modulo `sw.candidates.len()` in `mru_advance`) — a flat
    // `.take(max)` from the front left the strip frozen to the first
    // `max` entries while cycling kept advancing `selected` (and focus)
    // past what was ever rendered, so the ring landed on the wrong
    // thumbnail (or none) for anyone with more open windows than `max`.
    let window_start = mru_window_start(sw.selected, sw.candidates.len(), max);

    // (window, thumb_width, app_id). Thumb width keeps the window's aspect.
    let mut cells: Vec<(smithay::desktop::Window, i32, String)> = Vec::new();
    for win in sw.candidates.iter().skip(window_start).take(max) {
        let g = win.geometry().size;
        let (gw, gh) = (g.w.max(1), g.h.max(1));
        let tw = ((f64::from(gw) * f64::from(th) / f64::from(gh)).round() as i32).clamp(60, th * 2);
        let app_id = state
            .clients
            .iter()
            .find(|c| c.window == *win)
            .map(|c| c.app_id.clone())
            .unwrap_or_default();
        cells.push((win.clone(), tw, app_id));
    }
    if cells.is_empty() {
        return out;
    }

    // Row-relative left edge of each thumbnail (cumulative).
    let mut left_edges: Vec<i32> = Vec::with_capacity(cells.len());
    let mut acc = 0;
    for (_, tw, _) in &cells {
        left_edges.push(acc);
        acc += tw + gap;
    }
    let inner_w = acc - gap; // total row width (drop the trailing gap)
    let panel_h = TITLE_H + th + label_h + 2 * pad;
    let oy = (output_geo.size.h - panel_h) / 2;
    // Carousel: scroll the row so the SELECTED thumbnail is centred on the
    // output. Thumbnails left/right of it slide off the edges.
    let sel = sw.selected.min(cells.len() - 1);
    let shift = output_geo.size.w / 2 - (left_edges[sel] + cells[sel].1 / 2);

    let prog = crate::render::rounded_solid::shader(renderer).map(|p| p.0);
    let radius = (14.0 * output_scale) as f32;

    // Scope title, centred at the band top (niri shows the active scope).
    let scope_txt = match sw.scope {
        crate::state::mru_switcher::MruScope::All => "All windows",
        crate::state::mru_switcher::MruScope::Output => "This output",
        crate::state::mru_switcher::MruScope::Workspace => "This workspace",
    };
    let title_pos: Point<i32, Physical> =
        Point::<i32, Logical>::from((output_geo.size.w / 2 - 80, oy + 4))
            .to_physical_precise_round(scale);
    if let Some(el) = crate::render::text::label_element(
        renderer,
        scope_txt,
        (f64::from(TITLE_H) * output_scale * 0.78) as i32,
        (320.0 * output_scale) as i32,
        title_rgb,
        title_pos.to_f64(),
    ) {
        out.push(MargoRenderElement::Cursor(el));
    }

    // ── Thumbnails (topmost) + labels + per-thumb selection ring ─────
    let row_y = oy + pad + TITLE_H;
    for (i, (win, tw, app_id)) in cells.iter().enumerate() {
        let cell_x = shift + left_edges[i];
        let cell_y = row_y;

        // Draw the pre-captured snapshot scaled into the cell. Using a static
        // OpenCloseRenderElement (Fade, progress=1, alpha=1) just blits the
        // texture into `geometry` — works for windows on other tags because
        // the snapshot was taken off-screen before render.
        if let Some(thumb) = sw.thumb(win)
            && let (Some(tex), Some(key)) = (thumb.capture.current(), thumb.capture.key())
        {
            let geom = Rectangle::<i32, Logical>::new(
                Point::from((cell_x, cell_y)),
                smithay::utils::Size::from((*tw, th)),
            );
            out.push(MargoRenderElement::OpenClose(OpenCloseRenderElement::new(
                thumb.id.clone(),
                tex.clone(),
                geom,
                scale,
                1.0,
                1.0,
                OpenCloseKind::Fade,
                false,
                1.0,
                CommitCounter::from(key as usize),
                0.0,
                None,
            )));
        }

        // app-id label under the thumbnail.
        if show_labels && !app_id.is_empty() {
            let lpos: Point<i32, Physical> = Point::<i32, Logical>::from((cell_x, cell_y + th + 2))
                .to_physical_precise_round(scale);
            let rgb = if i == sw.selected {
                if accent { accent_rgb } else { [240, 240, 255] }
            } else {
                [165, 165, 175]
            };
            if let Some(el) = crate::render::text::label_element(
                renderer,
                app_id,
                (f64::from(label_h) * output_scale * 0.85) as i32,
                (f64::from(*tw) * output_scale) as i32,
                rgb,
                lpos.to_f64(),
            ) {
                out.push(MargoRenderElement::Cursor(el));
            }
        }

        // Selection ring behind the selected thumbnail (a slightly larger
        // rounded fill; the thumb covers the centre, leaving a border).
        if i == sw.selected
            && let Some(p) = prog.clone()
        {
            let ring = Rectangle::<i32, Logical>::new(
                Point::from((cell_x - 5, cell_y - 5)),
                smithay::utils::Size::from((tw + 10, th + 10)),
            )
            .to_physical_precise_round(scale);
            out.push(MargoRenderElement::RoundedSolid(
                crate::render::rounded_solid::RoundedSolidElement::new(
                    sw.chrome_ids.ring.clone(),
                    ring,
                    radius,
                    ring_color,
                    p,
                )
                .with_commit(color_commit(ring_color)),
            ));
        }
    }

    // ── Title bar under the panel (mango 7a1f90ed): the selected
    //    window's title, then its app_id, centred on the output. Shown
    //    with the labels (`mru_show_labels`).
    if show_labels
        && let Some(client) = sw
            .candidates
            .get(sw.selected)
            .and_then(|w| state.clients.iter().find(|c| c.window == *w))
    {
        let text = match (client.title.trim(), client.app_id.trim()) {
            ("", app) => app.to_string(),
            (title, "") => title.to_string(),
            (title, app) => format!("{title}  —  {app}"),
        };
        const BAR_H: i32 = 34;
        let text_h = (f64::from(BAR_H) * output_scale * 0.5) as i32;
        let max_w = ((output_geo.size.w - 120).max(100) as f64 * output_scale) as i32;
        if let Some(text_w) = crate::render::text::label_width(&text, text_h, max_w) {
            let text_w_logical = (f64::from(text_w) / output_scale).ceil() as i32;
            let bar_w = text_w_logical + 2 * 18;
            let bar_x = (output_geo.size.w - bar_w) / 2;
            let bar_y = oy + panel_h + 10;
            let text_pos: Point<i32, Physical> = Point::<i32, Logical>::from((
                bar_x + 18,
                bar_y + (BAR_H - (f64::from(text_h) / output_scale).round() as i32) / 2 - 2,
            ))
            .to_physical_precise_round(scale);
            if let Some(el) = crate::render::text::label_element(
                renderer,
                &text,
                text_h,
                max_w,
                if accent { accent_rgb } else { [235, 235, 245] },
                text_pos.to_f64(),
            ) {
                out.push(MargoRenderElement::Cursor(el));
            }
            if let Some(p) = prog.clone() {
                let bar = Rectangle::<i32, Logical>::new(
                    Point::from((bar_x, bar_y)),
                    smithay::utils::Size::from((bar_w, BAR_H)),
                )
                .to_physical_precise_round(scale);
                out.push(MargoRenderElement::RoundedSolid(
                    crate::render::rounded_solid::RoundedSolidElement::new(
                        sw.chrome_ids.title_bar.clone(),
                        bar,
                        radius,
                        [0.0, 0.0, 0.0, 0.62],
                        p,
                    ),
                ));
            }
        }
    }

    // ── Backing panel: hugs the thumbnail row (preview-sized), scrolls
    //    with it via the same `shift`. Not a full-width band.
    if let Some(p) = prog {
        let panel = Rectangle::<i32, Logical>::new(
            Point::from((shift - pad, oy)),
            smithay::utils::Size::from((inner_w + 2 * pad, panel_h)),
        )
        .to_physical_precise_round(scale);
        out.push(MargoRenderElement::RoundedSolid(
            crate::render::rounded_solid::RoundedSolidElement::new(
                sw.chrome_ids.panel.clone(),
                panel,
                radius,
                [0.0, 0.0, 0.0, 0.62],
                p,
            ),
        ));
    }

    // Backdrop dim over the whole output (still in front of the windows) —
    // pushed last so it sits behind the panel + thumbnails, darkening only the
    // desktop beneath the switcher.
    let dim = state.config.mru_dim_alpha.clamp(0.0, 0.9);
    if dim > 0.0 {
        let full = Rectangle::<i32, Physical>::new(
            Point::from((0, 0)),
            smithay::utils::Size::from((output_geo.size.w, output_geo.size.h))
                .to_physical_precise_round(scale),
        );
        out.push(MargoRenderElement::Solid(
            smithay::backend::renderer::element::solid::SolidColorRenderElement::new(
                sw.chrome_ids.dim.clone(),
                full,
                color_commit([0.0, 0.0, 0.0, dim]),
                [0.0, 0.0, 0.0, dim],
                smithay::backend::renderer::element::Kind::Unspecified,
            ),
        ));
    }

    out
}

pub(super) fn build_scroller_overview_elements(
    renderer: &mut GlesRenderer,
    od: &OutputDevice,
    state: &MargoState,
    output_geo: Rectangle<i32, Logical>,
    output_scale: f64,
    include_cursor: bool,
) -> Vec<MargoRenderElement> {
    use smithay::backend::renderer::element::Kind;
    use smithay::backend::renderer::element::solid::SolidColorRenderElement;
    use smithay::backend::renderer::element::utils::{
        Relocate, RelocateRenderElement, RescaleRenderElement,
    };
    use smithay::utils::{Physical, Size};

    let scale = Scale::from(output_scale);
    let mut elements: Vec<MargoRenderElement> = Vec::new();

    // Cursor on top (mirrors the normal path).
    let ptr_global = Point::<f64, _>::from((state.input_pointer.x, state.input_pointer.y));
    if include_cursor && output_geo.to_f64().contains(ptr_global) {
        let ptr_pos = ptr_global - output_geo.loc.to_f64();
        match &state.cursor_status {
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .and_then(|attrs| attrs.lock().ok().map(|attrs| attrs.hotspot))
                        .unwrap_or_default()
                });
                let ptr_i = (ptr_pos - hotspot.to_f64())
                    .to_physical_precise_round::<f64, i32>(output_scale);
                for e in render_elements_from_surface_tree(
                    renderer,
                    surface,
                    ptr_i,
                    output_scale,
                    1.0f32,
                    Kind::Cursor,
                ) {
                    elements.push(MargoRenderElement::WaylandSurface(e));
                }
            }
            CursorImageStatus::Hidden => {}
            _ => {
                if let Some(cursor_elem) =
                    state
                        .cursor_manager
                        .render_element(renderer, ptr_pos, output_scale)
                {
                    elements.push(MargoRenderElement::Cursor(cursor_elem));
                }
            }
        }
    }

    // Stable across frames (see `ScrollerOverview::backdrop_id`).
    let backdrop_id = state
        .scroller_overview
        .as_ref()
        .map(|overview| overview.backdrop_id.clone())
        .unwrap_or_else(smithay::backend::renderer::element::Id::new);
    let Some(mon_idx) = state.monitors.iter().position(|m| m.output == od.output) else {
        // No monitor for this output — just dim it.
        let dst = Rectangle::<i32, Physical>::new(
            Point::from((0, 0)),
            Size::from((output_geo.size.w, output_geo.size.h)).to_physical_precise_round(scale),
        );
        elements.push(MargoRenderElement::Solid(SolidColorRenderElement::new(
            backdrop_id,
            dst,
            color_commit(state.config.overview_backdrop_color.0),
            state.config.overview_backdrop_color.0,
            Kind::Unspecified,
        )));
        return elements;
    };

    let tags = state.scroller_overview_tags(mon_idx);
    // This monitor's continuous scroll position (centred cell).
    let pos = state
        .scroller_overview
        .as_ref()
        .and_then(|ov| ov.mon.get(mon_idx))
        .map(|m| m.pos)
        .unwrap_or(0.0);
    // Interpolate the effective zoom from the open/close animation
    // progress (niri's formula): progress 0 → zoom 1.0 (centred tag
    // full-screen), progress 1 → the configured zoom (full strip). The
    // gap grows with progress too, so cells start flush and fan out.
    let config_zoom = f64::from(state.config.scroller_overview_zoom.clamp(0.1, 1.0));
    let progress = state
        .scroller_overview
        .as_ref()
        .map(|o| o.progress)
        .unwrap_or(1.0);
    let zoom = 1.0 - progress * (1.0 - config_zoom);
    let gap = (f64::from(state.config.scroller_overview_gap.max(0)) * progress) as i32;
    let output_rect = crate::layout::Rect::new(
        output_geo.loc.x,
        output_geo.loc.y,
        output_geo.size.w,
        output_geo.size.h,
    );
    let loop_strip = state.config.scroller_overview_loop;
    let mut cells = crate::state::overview_cells(output_rect, &tags, zoom, gap, pos, loop_strip);
    // Keys stay as they are, so a cell keeps its namespace while it
    // scrolls in and out of view.
    cells.retain(|cell| overview_cell_visible(cell.rect, output_rect, gap));

    // The output's wallpaper (background + bottom layer-shell surfaces),
    // drawn into every cell behind the windows — niri zooms these with
    // each workspace. Collected once; each cell gets a namespaced copy.
    let layer_map = layer_map_for_output(&od.output);
    let bg_layers: Vec<&smithay::desktop::LayerSurface> = layer_map
        .layers()
        .filter(|s| matches!(s.layer(), WlrLayer::Background | WlrLayer::Bottom))
        .collect();

    for cell in &cells {
        let cell_w = output_geo.size.w.max(1);
        let cell_scale = f64::from(cell.rect.width) / f64::from(cell_w);
        let cell_origin_phys: Point<i32, Physical> = Point::from((
            cell.rect.x - output_geo.loc.x,
            cell.rect.y - output_geo.loc.y,
        ))
        .to_physical_precise_round(scale);

        // Each window of this tag, scaled into the cell.
        for client in state.clients.iter().filter(|c| {
            c.monitor == mon_idx
                && (c.tags & (1 << (cell.tag - 1))) != 0
                && !c.is_initial_map_pending
                && !c.is_minimized
                && !c.is_killing
                && !c.is_in_scratchpad
        }) {
            let Some(surface) = client.window.wl_surface() else {
                continue;
            };
            let geo_loc = client.window.geometry().loc;
            let render_location = Point::<i32, smithay::utils::Logical>::from((
                client.geom.x - geo_loc.x,
                client.geom.y - geo_loc.y,
            ));
            let physical_location =
                (render_location - output_geo.loc).to_physical_precise_round(scale);
            let surf_elems = render_elements_from_surface_tree::<
                GlesRenderer,
                WaylandSurfaceRenderElement<GlesRenderer>,
            >(
                renderer,
                &surface,
                physical_location,
                output_scale,
                1.0,
                Kind::Unspecified,
            );
            for e in surf_elems {
                // Namespace by cell key so a tag repeated by the wrap-around
                // loop doesn't collide with its other copy in the tracker.
                let ns = crate::render::namespaced::NamespacedElement::new(e, cell.key);
                let scaled =
                    RescaleRenderElement::from_element(ns, Point::from((0, 0)), cell_scale);
                let placed = RelocateRenderElement::from_element(
                    scaled,
                    cell_origin_phys,
                    Relocate::Relative,
                );
                elements.push(MargoRenderElement::NamespacedSurface(placed));
            }
        }

        // Wallpaper behind the windows, namespaced per cell.
        for layer in &bg_layers {
            let Some(lgeo) = layer_map.layer_geometry(layer) else {
                continue;
            };
            let loc_phys = lgeo.loc.to_physical_precise_round(scale);
            let surf_elems = render_elements_from_surface_tree::<
                GlesRenderer,
                WaylandSurfaceRenderElement<GlesRenderer>,
            >(
                renderer,
                layer.wl_surface(),
                loc_phys,
                output_scale,
                1.0,
                Kind::Unspecified,
            );
            for e in surf_elems {
                let ns = crate::render::namespaced::NamespacedElement::new(e, cell.key);
                let scaled =
                    RescaleRenderElement::from_element(ns, Point::from((0, 0)), cell_scale);
                let placed = RelocateRenderElement::from_element(
                    scaled,
                    cell_origin_phys,
                    Relocate::Relative,
                );
                elements.push(MargoRenderElement::NamespacedSurface(placed));
            }
        }
        // The runtime wallpaper, standing in for a shell's background layer.
        let wallpaper = crate::wallpaper::runtime::cell_elements(
            renderer,
            state,
            &od.output,
            cell_origin_phys,
            cell_scale,
            cell.key,
        );
        elements.extend(wallpaper.into_iter().map(Into::into));
    }

    // Backdrop behind every cell. An optional `overview_backdrop_image`
    // (cover-fit) sits at the bottom of the visible stack; the solid
    // `overview_backdrop_color` goes below it as the base clear (shows
    // when no image is set, and as a fallback if the image element fails
    // to build). Elements[0] is topmost, so trailing pushes draw first.
    if let Some(bg) = state.overview_backdrop.as_ref()
        && let Some(elem) = bg.render_element(
            renderer,
            output_geo.size,
            margo_config::WallpaperFit::Cover,
            scale.x,
        )
    {
        elements.push(MargoRenderElement::Wallpaper(elem));
    }

    let backdrop = Rectangle::<i32, Physical>::new(
        Point::from((0, 0)),
        Size::from((output_geo.size.w, output_geo.size.h)).to_physical_precise_round(scale),
    );
    elements.push(MargoRenderElement::Solid(SolidColorRenderElement::new(
        backdrop_id,
        backdrop,
        color_commit(state.config.overview_backdrop_color.0),
        state.config.overview_backdrop_color.0,
        Kind::Unspecified,
    )));

    elements
}

/// Whether an overview cell at `cell` can show anything on `output`: its
/// rect, grown by `margin` on every side, meets the output. Off-screen
/// cells would still cost a full element build (every window of the tag
/// plus the background layers) only for the damage tracker to cull them.
fn overview_cell_visible(
    cell: crate::layout::Rect,
    output: crate::layout::Rect,
    margin: i32,
) -> bool {
    let m = margin.max(0);
    cell.x - m < output.x + output.width
        && cell.x + cell.width + m > output.x
        && cell.y - m < output.y + output.height
        && cell.y + cell.height + m > output.y
}

fn mru_window_start(selected: usize, total: usize, max: usize) -> usize {
    if total <= max {
        0
    } else {
        selected.saturating_sub(max / 2).min(total - max)
    }
}

#[cfg(test)]
mod mru_window_tests {
    use super::mru_window_start;

    #[test]
    fn fits_entirely_when_under_the_cap() {
        assert_eq!(mru_window_start(0, 5, 10), 0);
        assert_eq!(mru_window_start(4, 5, 10), 0);
    }

    #[test]
    fn centres_on_selection_away_from_either_edge() {
        // 50 candidates, cap 10: selecting #25 should show roughly #20..#30.
        let start = mru_window_start(25, 50, 10);
        assert_eq!(start, 20);
        assert!((start..start + 10).contains(&25));
    }

    #[test]
    fn clamps_at_the_start_without_underflow() {
        assert_eq!(mru_window_start(0, 50, 10), 0);
        assert_eq!(mru_window_start(2, 50, 10), 0);
    }

    #[test]
    fn clamps_at_the_end_without_running_past_total() {
        // Selecting the very last candidate must still show it — this is
        // exactly the case the old flat `.take(max)` got wrong: `selected`
        // (49) landed far past the rendered window (0..10).
        let start = mru_window_start(49, 50, 10);
        assert_eq!(start, 40);
        assert!((start..start + 10).contains(&49));
    }

    #[test]
    fn selection_always_lands_inside_the_window_across_a_full_cycle() {
        let (total, max) = (37usize, 6usize);
        for selected in 0..total {
            let start = mru_window_start(selected, total, max);
            assert!(
                (start..start + max).contains(&selected),
                "selected={selected} start={start} max={max}"
            );
            assert!(start + max <= total, "window must not run past `total`");
        }
    }
}

#[cfg(test)]
mod overview_cull_tests {
    use super::overview_cell_visible;
    use crate::layout::Rect;
    use crate::state::overview_cells;

    const OUT: Rect = Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1440,
    };

    #[test]
    fn only_the_cells_on_screen_are_built() {
        // 9 occupied tags at zoom 0.3 (432 px cells, 40 px gap), the 5th
        // centred: about three cells are on screen.
        let tags: Vec<usize> = (1..=9).collect();
        let cells = overview_cells(OUT, &tags, 0.3, 40, 4.0, false);
        assert_eq!(cells.len(), 9);
        let mut shown = cells.clone();
        shown.retain(|c| overview_cell_visible(c.rect, OUT, 40));
        assert!(shown.len() < cells.len());
        for c in &cells {
            // On screen, or within one gap of it.
            let on_screen =
                c.rect.y + c.rect.height + 40 > OUT.y && c.rect.y - 40 < OUT.y + OUT.height;
            assert_eq!(
                shown.iter().any(|s| s == c),
                on_screen,
                "cell for tag {} at y={}",
                c.tag,
                c.rect.y
            );
        }
        // The centred tag and its neighbours, keys untouched.
        assert!(shown.iter().any(|c| c.tag == 5 && c.key == 4));
    }

    #[test]
    fn a_cell_peeking_in_by_less_than_the_margin_is_kept() {
        let tags = [1, 2, 3];
        // 720 px cells, 760 px stride: cell 1's top sits 20 px below the
        // screen edge (inside one 40 px gap), cell 2 is far off.
        let cells = overview_cells(OUT, &tags, 0.5, 40, 1.0 - 1100.0 / 760.0, false);
        assert_eq!(cells[1].rect.y, 1460);
        let mut shown = cells.clone();
        shown.retain(|c| overview_cell_visible(c.rect, OUT, 40));
        assert_eq!(shown, cells[..2]);
    }

    #[test]
    fn the_wrap_around_window_is_cut_to_the_screen_too() {
        let tags: Vec<usize> = (1..=9).collect();
        let cells = overview_cells(OUT, &tags, 0.3, 40, 0.0, true);
        let mut shown = cells.clone();
        shown.retain(|c| overview_cell_visible(c.rect, OUT, 40));
        assert!(
            shown.len() < cells.len(),
            "{} of {}",
            shown.len(),
            cells.len()
        );
        assert!(
            shown
                .iter()
                .all(|c| c.rect.y + c.rect.height + 40 > 0 && c.rect.y - 40 < 1440)
        );
    }
}
