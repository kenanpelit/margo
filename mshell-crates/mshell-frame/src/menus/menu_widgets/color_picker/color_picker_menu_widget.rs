//! Color Picker menu — the panel for `MenuType::ColorPicker`, opened by a
//! right click on the ColorPicker pill (`mshellctl menu color-picker`).
//! Port of the noctalia `color_picker` panel on top of margo's own
//! `mpicker`:
//!
//! * the selected colour as a swatch (a checkerboard shows through when it
//!   is not opaque), with editable **HEX**, **RGB** and **HSL** fields and
//!   an **Alpha** slider that update each other; Enter applies a field, the
//!   copy button next to it copies that form (and records the colour);
//! * **Recent** — the last [`mshell_cache::color_history::HISTORY_LIMIT`] colours picked anywhere in the
//!   shell (`mshell_cache::color_history`), alpha included; clicking one
//!   selects it and copies it again;
//! * the header's eyedropper closes the menu and starts `mpicker`; the
//!   picked colour lands at the front of Recent and becomes the selection.

use super::color::{ColorForm, Rgba};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_cache::color_history::{
    ColorHistoryStateStoreFields, clear as clear_history, color_history_store,
    record as record_color,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::picker::spawn_color_picker;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;

/// The forms the field rows edit, top to bottom.
const FORMS: [ColorForm; 3] = [ColorForm::Hex, ColorForm::Rgb, ColorForm::Hsl];

pub(crate) struct ColorPickerMenuWidgetModel {
    /// The selected colour.
    color: Rgba,
    /// Shared with the swatch's draw function.
    swatch_color: Rc<Cell<Rgba>>,
    swatch: gtk::DrawingArea,
    /// One entry per [`FORMS`] row.
    entries: Vec<gtk::Entry>,
    alpha: gtk::Scale,
    alpha_label: gtk::Label,
    /// Set while the alpha slider is moved from code, so its
    /// `value-changed` doesn't feed back.
    alpha_syncing: Rc<Cell<bool>>,
    history: Vec<Rgba>,
    history_box: gtk::FlowBox,
    history_empty: gtk::Label,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ColorPickerMenuWidgetInput {
    /// Close the menu and start mpicker.
    Pick,
    /// Enter in a field.
    FieldActivated(usize),
    /// Copy one form of the selected colour.
    Copy(usize),
    AlphaChanged(f64),
    /// A Recent swatch was clicked.
    HistoryClicked(usize),
    ClearHistory,
    HistoryEffect(Vec<Rgba>),
}

#[derive(Debug)]
pub(crate) enum ColorPickerMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct ColorPickerMenuWidgetInit {}

/// What a history update means for the selection: a colour that arrived at
/// the front (a new pick) becomes the selection; anything else (a re-copy
/// moving an entry, a clear) leaves it alone.
fn new_front(old: &[Rgba], new: &[Rgba]) -> Option<Rgba> {
    let front = *new.first()?;
    (old.first() != Some(&front)).then_some(front)
}

/// Slider position (0–100 %) for an alpha byte, and back.
fn alpha_percent(a: u8) -> f64 {
    (f64::from(a) / 255.0 * 100.0).round()
}

fn alpha_byte(percent: f64) -> u8 {
    (percent.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8
}

/// Paint `color` over a checkerboard made from the widget's own foreground
/// colour, so transparency reads on any palette without a fixed colour.
fn paint_swatch(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, color: Rgba) {
    let fg = area.color();
    let (w, h) = (f64::from(w), f64::from(h));
    if color.a < 255 {
        let cell = 6.0;
        let (light, dark) = (0.08_f32, 0.24_f32);
        let mut y = 0.0;
        let mut row = 0;
        while y < h {
            let mut x = 0.0;
            let mut col = row;
            while x < w {
                let a = if col % 2 == 0 { light } else { dark };
                cr.set_source_rgba(
                    f64::from(fg.red()),
                    f64::from(fg.green()),
                    f64::from(fg.blue()),
                    f64::from(a),
                );
                cr.rectangle(x, y, cell, cell);
                let _ = cr.fill();
                x += cell;
                col += 1;
            }
            y += cell;
            row += 1;
        }
    }
    let ch = |v: u8| f64::from(v) / 255.0;
    cr.set_source_rgba(ch(color.r), ch(color.g), ch(color.b), ch(color.a));
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
}

/// A swatch: a drawing area clipped to the `.color-swatch` corner.
fn swatch_widget(size: i32, color: Rc<Cell<Rgba>>) -> (gtk::Box, gtk::DrawingArea) {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("color-swatch");
    frame.set_overflow(gtk::Overflow::Hidden);
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_draw_func(move |area, cr, w, h| paint_swatch(area, cr, w, h, color.get()));
    frame.append(&area);
    (frame, area)
}

#[relm4::component(pub(crate))]
impl Component for ColorPickerMenuWidgetModel {
    type CommandOutput = ();
    type Input = ColorPickerMenuWidgetInput;
    type Output = ColorPickerMenuWidgetOutput;
    type Init = ColorPickerMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "color-picker-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header ──
            #[local_ref]
            header_root -> gtk::Box {},

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                #[local_ref]
                swatch_frame -> gtk::Box {
                    set_valign: gtk::Align::Start,
                },

                #[local_ref]
                fields -> gtk::Box {
                    add_css_class: "ok-boxed-list",
                    add_css_class: "compact",
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                },
            },

            gtk::Label {
                add_css_class: "color-picker-section-label",
                set_label: "Recent",
                set_halign: gtk::Align::Start,
            },

            #[local_ref]
            history_box -> gtk::FlowBox {
                add_css_class: "color-picker-history",
                set_selection_mode: gtk::SelectionMode::None,
                set_homogeneous: true,
                set_min_children_per_line: 6,
                set_max_children_per_line: 6,
                set_column_spacing: 8,
                set_row_spacing: 8,
            },

            #[local_ref]
            history_empty -> gtk::Label {
                add_css_class: "color-picker-empty",
                set_label: "No colours picked yet. Use the eyedropper to pick one from the screen.",
                set_wrap: true,
                set_xalign: 0.0,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let pick_sender = sender.clone();
        let clear_sender = sender.clone();
        let header = PanelHeader::new(
            "color-select-symbolic",
            MenuWidget::ColorPicker.display_name(),
        )
        .action(
            "color-select-symbolic",
            "Pick a colour from the screen",
            move || {
                pick_sender.input(ColorPickerMenuWidgetInput::Pick);
            },
        )
        .action(
            "edit-clear-history-symbolic",
            "Clear recent colours",
            move || {
                clear_sender.input(ColorPickerMenuWidgetInput::ClearHistory);
            },
        );
        let header_root = header.root().clone();

        let history = color_history_store().colors().get_untracked();
        let color = history.first().copied().unwrap_or(Rgba::rgb(255, 255, 255));
        let swatch_color = Rc::new(Cell::new(color));
        let (swatch_frame, swatch) = swatch_widget(96, swatch_color.clone());

        // Field rows: label · entry · copy, then the alpha slider row.
        let fields = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let mut entries = Vec::new();
        for (i, form) in FORMS.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("color-picker-field-row");
            let label = gtk::Label::new(Some(form.label()));
            label.add_css_class("color-picker-field-label");
            label.set_xalign(0.0);
            let entry = gtk::Entry::new();
            entry.add_css_class("ok-entry-with-border");
            entry.add_css_class("color-picker-field");
            entry.set_hexpand(true);
            mshell_common::a11y::name(&entry, &format!("{} value", form.label()));
            let s = sender.clone();
            entry.connect_activate(move |_| {
                s.input(ColorPickerMenuWidgetInput::FieldActivated(i));
            });
            // Typing clears the invalid marker until the next Enter.
            entry.connect_changed(|e| e.remove_css_class("error"));
            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.add_css_class("ok-button-flat");
            mshell_common::a11y::name(&copy, &format!("Copy {}", form.label()));
            let s = sender.clone();
            copy.connect_clicked(move |_| s.input(ColorPickerMenuWidgetInput::Copy(i)));
            row.append(&label);
            row.append(&entry);
            row.append(&copy);
            fields.append(&row);
            entries.push(entry);
        }
        let alpha_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        alpha_row.add_css_class("color-picker-field-row");
        let alpha_title = gtk::Label::new(Some("Alpha"));
        alpha_title.add_css_class("color-picker-field-label");
        alpha_title.set_xalign(0.0);
        let alpha = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        alpha.set_hexpand(true);
        alpha.set_draw_value(false);
        mshell_common::a11y::name(&alpha, "Opacity");
        let alpha_label = gtk::Label::new(None);
        alpha_label.add_css_class("color-picker-alpha-value");
        let alpha_syncing = Rc::new(Cell::new(false));
        let s = sender.clone();
        let syncing = alpha_syncing.clone();
        alpha.connect_value_changed(move |sc| {
            if !syncing.get() {
                s.input(ColorPickerMenuWidgetInput::AlphaChanged(sc.value()));
            }
        });
        alpha_row.append(&alpha_title);
        alpha_row.append(&alpha);
        alpha_row.append(&alpha_label);
        fields.append(&alpha_row);

        let history_box = gtk::FlowBox::new();
        let history_empty = gtk::Label::new(None);

        let mut effects = EffectScope::new();
        let effect_sender = sender.clone();
        effects.push(move |_| {
            let colors = color_history_store().colors().get();
            effect_sender.input(ColorPickerMenuWidgetInput::HistoryEffect(colors));
        });

        let model = ColorPickerMenuWidgetModel {
            color,
            swatch_color,
            swatch,
            entries,
            alpha,
            alpha_label,
            alpha_syncing,
            history: Vec::new(),
            history_box: history_box.clone(),
            history_empty: history_empty.clone(),
            _effects: effects,
        };
        let widgets = view_output!();
        model.show_color();
        model.rebuild_history(&sender);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ColorPickerMenuWidgetInput::Pick => {
                // Close first so the frozen screenshot mpicker takes has no
                // menu in it (the Control Center tile waits as long).
                let _ = sender.output(ColorPickerMenuWidgetOutput::CloseMenu);
                spawn_color_picker(300);
            }
            ColorPickerMenuWidgetInput::FieldActivated(i) => {
                let (Some(form), Some(entry)) = (FORMS.get(i), self.entries.get(i)) else {
                    return;
                };
                match form.parse(entry.text().as_str()) {
                    Some(color) => {
                        self.color = color;
                        self.show_color();
                    }
                    None => entry.add_css_class("error"),
                }
            }
            ColorPickerMenuWidgetInput::Copy(i) => {
                let Some(form) = FORMS.get(i) else { return };
                let text = form.format(self.color);
                self.swatch.clipboard().set_text(&text);
                record_color(self.color);
            }
            ColorPickerMenuWidgetInput::AlphaChanged(percent) => {
                self.color.a = alpha_byte(percent);
                self.show_color();
            }
            ColorPickerMenuWidgetInput::HistoryClicked(i) => {
                if let Some(color) = self.history.get(i).copied() {
                    self.color = color;
                    self.show_color();
                    self.swatch
                        .clipboard()
                        .set_text(&ColorForm::Hex.format(color));
                }
            }
            ColorPickerMenuWidgetInput::ClearHistory => clear_history(),
            ColorPickerMenuWidgetInput::HistoryEffect(colors) => {
                if let Some(front) = new_front(&self.history, &colors)
                    && !self.history.is_empty()
                {
                    self.color = front;
                    self.show_color();
                }
                self.history = colors;
                self.rebuild_history(&sender);
            }
        }
    }
}

impl ColorPickerMenuWidgetModel {
    /// Push the selected colour into the swatch, every field and the slider.
    fn show_color(&self) {
        self.swatch_color.set(self.color);
        self.swatch.queue_draw();
        for (form, entry) in FORMS.iter().zip(&self.entries) {
            entry.set_text(&form.format(self.color));
            entry.remove_css_class("error");
        }
        let percent = alpha_percent(self.color.a);
        self.alpha_syncing.set(true);
        self.alpha.set_value(percent);
        self.alpha_syncing.set(false);
        self.alpha_label.set_label(&format!("{percent:.0}%"));
    }

    fn rebuild_history(&self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.history_box.first_child() {
            self.history_box.remove(&child);
        }
        for (i, color) in self.history.iter().enumerate() {
            let (frame, _) = swatch_widget(32, Rc::new(Cell::new(*color)));
            let button = gtk::Button::new();
            button.add_css_class("color-picker-history-swatch");
            button.set_child(Some(&frame));
            let name = ColorForm::Hex.format(*color);
            mshell_common::a11y::name(&button, &format!("Copy {name}"));
            let s = sender.clone();
            button.connect_clicked(move |_| s.input(ColorPickerMenuWidgetInput::HistoryClicked(i)));
            self.history_box.insert(&button, -1);
        }
        let empty = self.history.is_empty();
        self.history_box.set_visible(!empty);
        self.history_empty.set_visible(empty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_cache::color_history::HISTORY_LIMIT;

    #[test]
    fn a_new_pick_at_the_front_becomes_the_selection() {
        let red = Rgba::rgb(255, 0, 0);
        let green = Rgba::rgb(0, 255, 0);
        assert_eq!(new_front(&[red], &[green, red]), Some(green));
        assert_eq!(new_front(&[], &[red]), Some(red));
        // A re-copy of the front colour or a clear leaves the selection.
        assert_eq!(new_front(&[red, green], &[red, green]), None);
        assert_eq!(new_front(&[red], &[]), None);
    }

    #[test]
    fn alpha_slider_maps_to_bytes_and_back() {
        assert_eq!(alpha_byte(100.0), 255);
        assert_eq!(alpha_byte(0.0), 0);
        assert_eq!(alpha_byte(50.0), 128);
        assert_eq!(alpha_byte(150.0), 255, "clamped");
        for a in [0u8, 64, 128, 200, 255] {
            assert!(alpha_byte(alpha_percent(a)).abs_diff(a) <= 2, "{a}");
        }
        assert_eq!(alpha_percent(255), 100.0);
    }

    #[test]
    fn history_limit_matches_the_flowbox_rows() {
        // Two rows of six in the menu.
        assert_eq!(HISTORY_LIMIT % 6, 0);
    }
}
