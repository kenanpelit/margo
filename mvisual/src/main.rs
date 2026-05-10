//! mvisual — interactive GTK4 design tool for margo's 14 tiling
//! layouts × per-tag layout pinning.
//!
//! niri ships `niri-visual-tests` to inspect a single layout under a
//! single configuration. mvisual goes wider:
//!
//! 1. **Catalogue thumbnails.** All 14 tile-able layouts render side
//!    by side at the current parameters — pick one and the big
//!    preview switches to it. Direct visual comparison, not click-
//!    cycle inspection.
//! 2. **Per-tag pinning preview.** A 1‒9 tag rail at the bottom acts
//!    like margo's `Pertag` — switch tags, the active layout +
//!    parameters snap to whatever you pinned on that tag. Mirrors the
//!    real compositor's per-tag layout state so you can rehearse a
//!    workflow before committing to a config.
//! 3. **Live geometry.** Every tweak (window count / mfact / nmaster
//!    / gaps / focused index / scroller proportion) re-runs the same
//!    `arrange()` the compositor calls. The arithmetic is the
//!    standalone `margo-layouts` crate — what you see is what
//!    margo will tile.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::cairo;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Adjustment, Application, ApplicationWindow, Box as GtkBox, DrawingArea, Frame, Grid,
    HeaderBar, Label, Orientation, Scale, SpinButton, ToggleButton,
};

use margo_layouts::{arrange, ArrangeCtx, GapConfig, LayoutId, Rect, MAX_TAGS};

const APP_ID: &str = "dev.margo.visual";

// ── State ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct LayoutParams {
    layout: LayoutId,
    n_windows: u32,
    mfact: f32,
    nmaster: u32,
    inner_gap: i32,
    outer_gap: i32,
    focus: u32, // 1-based, 0 = no focus
    scroller_prop: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            layout: LayoutId::Tile,
            n_windows: 3,
            mfact: 0.55,
            nmaster: 1,
            inner_gap: 8,
            outer_gap: 12,
            focus: 1,
            scroller_prop: 0.8,
        }
    }
}

struct AppState {
    pertag: Vec<LayoutParams>, // index 0 unused; 1..=MAX_TAGS
    current_tag: usize,
}

impl AppState {
    fn new() -> Self {
        let mut pertag = vec![LayoutParams::default(); MAX_TAGS + 1];
        // Seed each tag with a different layout so the rail is
        // immediately illustrative on first launch.
        let seeds = LayoutId::all_tileable();
        for i in 1..=MAX_TAGS {
            pertag[i].layout = seeds[(i - 1) % seeds.len()];
        }
        Self { pertag, current_tag: 1 }
    }

    fn current(&self) -> &LayoutParams {
        &self.pertag[self.current_tag]
    }

    fn current_mut(&mut self) -> &mut LayoutParams {
        &mut self.pertag[self.current_tag]
    }
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn render_layout(cr: &cairo::Context, w: i32, h: i32, p: &LayoutParams, big: bool) {
    // Background.
    let bg = if big { (0.08, 0.09, 0.12) } else { (0.06, 0.07, 0.09) };
    cr.set_source_rgb(bg.0, bg.1, bg.2);
    let _ = cr.paint();

    if p.n_windows == 0 || w < 4 || h < 4 {
        return;
    }

    let tiled: Vec<usize> = (0..p.n_windows as usize).collect();
    let proportions = vec![p.scroller_prop; p.n_windows as usize];
    let gaps = GapConfig {
        gappih: p.inner_gap,
        gappiv: p.inner_gap,
        gappoh: p.outer_gap,
        gappov: p.outer_gap,
    };
    let focus_pos = if p.focus >= 1 && (p.focus as usize) <= tiled.len() {
        Some(p.focus as usize - 1)
    } else {
        None
    };
    let ctx = ArrangeCtx {
        work_area: Rect::new(0, 0, w, h),
        tiled: &tiled,
        nmaster: p.nmaster,
        mfact: p.mfact,
        gaps: &gaps,
        scroller_proportions: &proportions,
        default_scroller_proportion: p.scroller_prop,
        focused_tiled_pos: focus_pos,
        scroller_structs: 24,
        scroller_focus_center: true,
        scroller_prefer_center: true,
        scroller_prefer_overspread: false,
        canvas_pan: (0.0, 0.0),
    };
    let result = arrange(p.layout, &ctx);

    let n = result.len().max(1) as f64;
    for (i, rect) in result {
        if rect.width <= 0 || rect.height <= 0 {
            continue;
        }
        let is_focus = focus_pos == Some(i);
        let (r, g, b) = if is_focus {
            (0.95, 0.50, 0.30)
        } else {
            let hue = (i as f64) / n;
            hsv_to_rgb(hue, 0.42, 0.78)
        };
        let alpha = if big { 0.92 } else { 0.85 };
        cr.set_source_rgba(r, g, b, alpha);
        cr.rectangle(
            rect.x as f64,
            rect.y as f64,
            rect.width as f64,
            rect.height as f64,
        );
        let _ = cr.fill_preserve();
        cr.set_source_rgba(r * 0.5, g * 0.5, b * 0.5, 1.0);
        cr.set_line_width(if big { 1.5 } else { 1.0 });
        let _ = cr.stroke();

        // Index label inside each rect (only when reasonably big).
        if big && rect.width > 28 && rect.height > 22 {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
            cr.set_font_size(14.0);
            let label = format!("{}", i + 1);
            cr.move_to(rect.x as f64 + 8.0, rect.y as f64 + 18.0);
            let _ = cr.show_text(&label);
        }
    }
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let h = h.fract().abs();
    let i = (h * 6.0).floor() as i32 % 6;
    let f = h * 6.0 - h * 6.0_f64.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

// ── UI assembly ──────────────────────────────────────────────────────────────

struct Ui {
    thumbs: Vec<DrawingArea>,
    big: DrawingArea,
    selected_label: Label,
    pertag_label: Label,
    n_windows_spin: SpinButton,
    mfact_scale: Scale,
    nmaster_spin: SpinButton,
    inner_gap_spin: SpinButton,
    outer_gap_spin: SpinButton,
    focus_spin: SpinButton,
    scroller_scale: Scale,
    tag_toggles: Vec<ToggleButton>,
}

fn redraw_all(ui: &Ui) {
    for t in &ui.thumbs {
        t.queue_draw();
    }
    ui.big.queue_draw();
}

fn refresh_pertag_label(ui: &Ui, state: &AppState) {
    let mut parts = Vec::with_capacity(MAX_TAGS);
    for t in 1..=MAX_TAGS {
        let l = state.pertag[t].layout;
        let mark = if t == state.current_tag { "•" } else { " " };
        parts.push(format!("{mark}{t}={}", l.symbol()));
    }
    ui.pertag_label.set_text(&format!("pinned: {}", parts.join("  ")));
}

fn refresh_controls_from_state(ui: &Ui, state: &AppState, suppress: &Cell<bool>) {
    suppress.set(true);
    let p = state.current().clone();

    ui.selected_label
        .set_markup(&format!("<b>{}</b>  ({})", p.layout.name(), p.layout.symbol()));

    ui.n_windows_spin.set_value(p.n_windows as f64);
    ui.mfact_scale.set_value(p.mfact as f64);
    ui.nmaster_spin.set_value(p.nmaster as f64);
    ui.inner_gap_spin.set_value(p.inner_gap as f64);
    ui.outer_gap_spin.set_value(p.outer_gap as f64);
    ui.focus_spin
        .set_range(0.0, p.n_windows.max(1) as f64);
    ui.focus_spin.set_value(p.focus as f64);
    ui.scroller_scale.set_value(p.scroller_prop as f64);

    for (i, b) in ui.tag_toggles.iter().enumerate() {
        b.set_active(i + 1 == state.current_tag);
    }

    refresh_pertag_label(ui, state);
    suppress.set(false);
}

fn build_thumbnail(
    state: Rc<RefCell<AppState>>,
    ui_cell: Rc<RefCell<Option<Rc<Ui>>>>,
    layout: LayoutId,
) -> (Frame, DrawingArea) {
    let frame = Frame::new(Some(layout.name()));
    frame.set_label_align(0.5);
    frame.add_css_class("mvisual-thumb");

    let area = DrawingArea::builder()
        .content_width(180)
        .content_height(110)
        .build();
    let frame_state = state.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let st = frame_state.borrow();
        let mut p = st.current().clone();
        p.layout = layout;
        render_layout(cr, w, h, &p, false);
    });

    // Click → set current tag's layout to this one.
    let gesture = gtk::GestureClick::new();
    let click_state = state.clone();
    let click_ui = ui_cell.clone();
    gesture.connect_pressed(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        click_state.borrow_mut().current_mut().layout = layout;
        if let Some(ui) = click_ui.borrow().as_ref() {
            ui.selected_label.set_markup(&format!(
                "<b>{}</b>  ({})",
                layout.name(),
                layout.symbol()
            ));
            refresh_pertag_label(ui, &click_state.borrow());
            redraw_all(ui);
        }
    });
    area.add_controller(gesture);

    frame.set_child(Some(&area));
    (frame, area)
}

fn build_catalogue(
    state: Rc<RefCell<AppState>>,
    ui_cell: Rc<RefCell<Option<Rc<Ui>>>>,
) -> (GtkBox, Vec<DrawingArea>) {
    let outer = GtkBox::new(Orientation::Vertical, 6);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_margin_top(12);
    outer.set_margin_bottom(12);

    let title = Label::new(Some("Layout catalogue"));
    title.add_css_class("title-4");
    title.set_xalign(0.0);
    outer.append(&title);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(8);
    grid.set_row_homogeneous(true);
    grid.set_column_homogeneous(true);

    let layouts = LayoutId::all_tileable();
    let cols: i32 = 4;
    let mut areas = Vec::with_capacity(layouts.len());

    for (i, layout) in layouts.iter().enumerate() {
        let (frame, area) = build_thumbnail(state.clone(), ui_cell.clone(), *layout);
        let row = (i as i32) / cols;
        let col = (i as i32) % cols;
        grid.attach(&frame, col, row, 1, 1);
        areas.push(area);
    }

    outer.append(&grid);
    (outer, areas)
}

struct ParamsPanel {
    root: GtkBox,
    big: DrawingArea,
    n_windows_spin: SpinButton,
    mfact_scale: Scale,
    nmaster_spin: SpinButton,
    inner_gap_spin: SpinButton,
    outer_gap_spin: SpinButton,
    focus_spin: SpinButton,
    scroller_scale: Scale,
    selected_label: Label,
    pertag_label: Label,
}

fn build_params_panel(
    state: Rc<RefCell<AppState>>,
    suppress: Rc<Cell<bool>>,
    ui_cell: Rc<RefCell<Option<Rc<Ui>>>>,
) -> ParamsPanel {
    let outer = GtkBox::new(Orientation::Vertical, 8);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_margin_top(12);
    outer.set_margin_bottom(12);

    let selected_label = Label::new(None);
    selected_label.set_xalign(0.0);
    selected_label.add_css_class("title-3");
    outer.append(&selected_label);

    let pertag_label = Label::new(None);
    pertag_label.set_xalign(0.0);
    pertag_label.add_css_class("dim-label");
    pertag_label.set_use_markup(false);
    outer.append(&pertag_label);

    let big = DrawingArea::builder()
        .content_width(640)
        .content_height(380)
        .vexpand(true)
        .hexpand(true)
        .build();
    let big_state = state.clone();
    big.set_draw_func(move |_, cr, w, h| {
        let st = big_state.borrow();
        render_layout(cr, w, h, st.current(), true);
    });
    let big_frame = Frame::new(None);
    big_frame.set_child(Some(&big));
    big_frame.set_vexpand(true);
    big_frame.set_hexpand(true);
    outer.append(&big_frame);

    // Parameters grid.
    let params = Grid::new();
    params.set_row_spacing(6);
    params.set_column_spacing(10);
    params.set_margin_top(8);

    let mk_label = |s: &str| {
        let l = Label::new(Some(s));
        l.set_xalign(0.0);
        l.set_width_chars(14);
        l
    };

    // Window count.
    let n_windows_spin =
        SpinButton::with_range(0.0, 16.0, 1.0);
    n_windows_spin.set_value(3.0);
    params.attach(&mk_label("Windows"), 0, 0, 1, 1);
    params.attach(&n_windows_spin, 1, 0, 1, 1);

    // Master count.
    let nmaster_spin = SpinButton::with_range(1.0, 8.0, 1.0);
    nmaster_spin.set_value(1.0);
    params.attach(&mk_label("nmaster"), 0, 1, 1, 1);
    params.attach(&nmaster_spin, 1, 1, 1, 1);

    // mfact.
    let mfact_adj = Adjustment::new(0.55, 0.10, 0.90, 0.05, 0.10, 0.0);
    let mfact_scale = Scale::new(Orientation::Horizontal, Some(&mfact_adj));
    mfact_scale.set_digits(2);
    mfact_scale.set_value_pos(gtk::PositionType::Right);
    mfact_scale.set_hexpand(true);
    mfact_scale.set_width_request(220);
    params.attach(&mk_label("mfact"), 2, 0, 1, 1);
    params.attach(&mfact_scale, 3, 0, 1, 1);

    // Scroller proportion.
    let scroller_adj = Adjustment::new(0.80, 0.40, 1.0, 0.05, 0.10, 0.0);
    let scroller_scale = Scale::new(Orientation::Horizontal, Some(&scroller_adj));
    scroller_scale.set_digits(2);
    scroller_scale.set_value_pos(gtk::PositionType::Right);
    scroller_scale.set_hexpand(true);
    params.attach(&mk_label("scroller p"), 2, 1, 1, 1);
    params.attach(&scroller_scale, 3, 1, 1, 1);

    // Inner gap.
    let inner_gap_spin = SpinButton::with_range(0.0, 60.0, 1.0);
    inner_gap_spin.set_value(8.0);
    params.attach(&mk_label("Inner gap"), 0, 2, 1, 1);
    params.attach(&inner_gap_spin, 1, 2, 1, 1);

    // Outer gap.
    let outer_gap_spin = SpinButton::with_range(0.0, 80.0, 1.0);
    outer_gap_spin.set_value(12.0);
    params.attach(&mk_label("Outer gap"), 0, 3, 1, 1);
    params.attach(&outer_gap_spin, 1, 3, 1, 1);

    // Focused index (1-based; 0 = none).
    let focus_spin = SpinButton::with_range(0.0, 16.0, 1.0);
    focus_spin.set_value(1.0);
    params.attach(&mk_label("Focus"), 2, 2, 1, 1);
    params.attach(&focus_spin, 3, 2, 1, 1);

    outer.append(&params);

    // ── Wire signals ───────────────────────────────────────────────────────
    macro_rules! wire {
        ($widget:expr, $sig:ident, $apply:expr) => {{
            let s = state.clone();
            let sup = suppress.clone();
            let uic = ui_cell.clone();
            $widget.$sig(move |w| {
                if sup.get() {
                    return;
                }
                let mut st = s.borrow_mut();
                $apply(&mut *st.current_mut(), w);
                drop(st);
                if let Some(ui) = uic.borrow().as_ref() {
                    refresh_controls_from_state(ui, &s.borrow(), &sup);
                    redraw_all(ui);
                }
            });
        }};
    }

    wire!(n_windows_spin, connect_value_changed, |p: &mut LayoutParams, w: &SpinButton| {
        p.n_windows = w.value() as u32;
        if p.focus > p.n_windows { p.focus = p.n_windows; }
    });
    wire!(nmaster_spin, connect_value_changed, |p: &mut LayoutParams, w: &SpinButton| {
        p.nmaster = w.value().max(1.0) as u32;
    });
    wire!(mfact_scale, connect_value_changed, |p: &mut LayoutParams, w: &Scale| {
        p.mfact = w.value() as f32;
    });
    wire!(scroller_scale, connect_value_changed, |p: &mut LayoutParams, w: &Scale| {
        p.scroller_prop = w.value() as f32;
    });
    wire!(inner_gap_spin, connect_value_changed, |p: &mut LayoutParams, w: &SpinButton| {
        p.inner_gap = w.value() as i32;
    });
    wire!(outer_gap_spin, connect_value_changed, |p: &mut LayoutParams, w: &SpinButton| {
        p.outer_gap = w.value() as i32;
    });
    wire!(focus_spin, connect_value_changed, |p: &mut LayoutParams, w: &SpinButton| {
        p.focus = w.value() as u32;
    });

    ParamsPanel {
        root: outer,
        big,
        n_windows_spin,
        mfact_scale,
        nmaster_spin,
        inner_gap_spin,
        outer_gap_spin,
        focus_spin,
        scroller_scale,
        selected_label,
        pertag_label,
    }
}

fn build_tag_rail(
    state: Rc<RefCell<AppState>>,
    suppress: Rc<Cell<bool>>,
    ui_cell: Rc<RefCell<Option<Rc<Ui>>>>,
) -> (GtkBox, Vec<ToggleButton>) {
    let bar = GtkBox::new(Orientation::Horizontal, 4);
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    bar.set_margin_top(4);
    bar.set_margin_bottom(8);
    bar.set_halign(gtk::Align::Center);

    let label = Label::new(Some("Tag pinning  "));
    label.add_css_class("dim-label");
    bar.append(&label);

    let mut toggles = Vec::with_capacity(MAX_TAGS);
    for t in 1..=MAX_TAGS {
        let b = ToggleButton::with_label(&format!("{t}"));
        b.set_size_request(40, 32);
        if t == 1 {
            b.set_active(true);
        }
        let s = state.clone();
        let sup = suppress.clone();
        let uic = ui_cell.clone();
        b.connect_toggled(move |btn| {
            if sup.get() {
                return;
            }
            if !btn.is_active() {
                // Don't allow deselecting the active tag; force it back on.
                sup.set(true);
                btn.set_active(true);
                sup.set(false);
                return;
            }
            s.borrow_mut().current_tag = t;
            if let Some(ui) = uic.borrow().as_ref() {
                refresh_controls_from_state(ui, &s.borrow(), &sup);
                redraw_all(ui);
            }
        });
        bar.append(&b);
        toggles.push(b);
    }

    (bar, toggles)
}

fn build_ui(app: &Application) {
    let state = Rc::new(RefCell::new(AppState::new()));
    let suppress = Rc::new(Cell::new(false));
    let ui_cell: Rc<RefCell<Option<Rc<Ui>>>> = Rc::new(RefCell::new(None));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("margo • visual")
        .default_width(1280)
        .default_height(820)
        .build();

    let header = HeaderBar::builder().show_title_buttons(true).build();
    let title = Label::new(None);
    title.set_markup("<b>margo</b> • visual");
    header.set_title_widget(Some(&title));
    window.set_titlebar(Some(&header));

    let outer = GtkBox::new(Orientation::Vertical, 0);
    let body = GtkBox::new(Orientation::Horizontal, 0);
    body.set_vexpand(true);
    body.set_hexpand(true);

    let (catalogue, thumb_areas) = build_catalogue(state.clone(), ui_cell.clone());
    catalogue.set_width_request(820);

    let panel = build_params_panel(state.clone(), suppress.clone(), ui_cell.clone());
    panel.root.set_hexpand(true);

    body.append(&catalogue);
    body.append(&panel.root);

    let (tag_rail, tag_toggles) = build_tag_rail(state.clone(), suppress.clone(), ui_cell.clone());

    outer.append(&body);
    outer.append(&tag_rail);
    window.set_child(Some(&outer));

    let ui = Rc::new(Ui {
        thumbs: thumb_areas,
        big: panel.big,
        selected_label: panel.selected_label,
        pertag_label: panel.pertag_label,
        n_windows_spin: panel.n_windows_spin,
        mfact_scale: panel.mfact_scale,
        nmaster_spin: panel.nmaster_spin,
        inner_gap_spin: panel.inner_gap_spin,
        outer_gap_spin: panel.outer_gap_spin,
        focus_spin: panel.focus_spin,
        scroller_scale: panel.scroller_scale,
        tag_toggles,
    });
    *ui_cell.borrow_mut() = Some(ui.clone());

    refresh_controls_from_state(&ui, &state.borrow(), &suppress);
    redraw_all(&ui);

    window.present();
}

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pertag_seeds_walk_the_layout_catalogue() {
        // Each tag is seeded with a different default layout so the
        // tag rail is illustrative on first launch.
        let s = AppState::new();
        let seeds = LayoutId::all_tileable();
        for tag in 1..=MAX_TAGS {
            assert_eq!(s.pertag[tag].layout, seeds[(tag - 1) % seeds.len()]);
        }
    }

    #[test]
    fn current_tag_round_trips_layout_changes() {
        let mut s = AppState::new();
        s.current_tag = 4;
        s.current_mut().layout = LayoutId::Canvas;
        s.current_mut().mfact = 0.42;
        // Other tags untouched.
        assert_eq!(s.pertag[3].layout, LayoutId::all_tileable()[2]);
        // Switch away and back — pinned values survive.
        s.current_tag = 1;
        s.current_tag = 4;
        assert_eq!(s.current().layout, LayoutId::Canvas);
        assert_eq!(s.current().mfact, 0.42);
    }

    #[test]
    fn hsv_to_rgb_keeps_in_unit_cube() {
        for h in [0.0, 0.17, 0.33, 0.5, 0.67, 0.83, 0.99, 1.0] {
            let (r, g, b) = hsv_to_rgb(h, 0.5, 0.8);
            assert!((0.0..=1.0).contains(&r), "r={r} for h={h}");
            assert!((0.0..=1.0).contains(&g), "g={g} for h={h}");
            assert!((0.0..=1.0).contains(&b), "b={b} for h={h}");
        }
    }
}
