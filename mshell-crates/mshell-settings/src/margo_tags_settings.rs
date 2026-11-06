//! Settings → Bar pills → Margo Tags: the tag row's presentation options
//! (`bars.widgets.margo_tags`): which tags show, a glyph and accent per
//! state, and the per-tag minimap. Ports of the lunar-workspaces and
//! niri-workspace-preview plugins' options onto MargoTags.
//!
//! Entries are seeded once and own their text (a reactive `set_text`
//! round-trip would reset the cursor on every keystroke, as in
//! `session_settings.rs`); dropdowns and the switch write straight to the
//! config store.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, MargoTagsConfig, MargoTagsDisplay,
    TagAccent,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

/// The four tag states, in row order: (label, config-writer).
const GLYPH_STATES: [(&str, fn(&mut MargoTagsConfig) -> &mut String); 4] = [
    ("Focused", |c| &mut c.glyph_focused),
    ("Urgent", |c| &mut c.glyph_urgent),
    ("Occupied", |c| &mut c.glyph_occupied),
    ("Empty", |c| &mut c.glyph_empty),
];

/// The states with an accent choice (an empty tag stays dim).
const ACCENT_STATES: [(&str, fn(&mut MargoTagsConfig) -> &mut TagAccent); 3] = [
    ("Focused", |c| &mut c.accent_focused),
    ("Urgent", |c| &mut c.accent_urgent),
    ("Occupied", |c| &mut c.accent_occupied),
];

pub(crate) struct MargoTagsSettingsModel;

#[derive(Debug)]
pub(crate) enum MargoTagsSettingsInput {
    Display(u32),
    Glyph(usize, String),
    Accent(usize, u32),
    Minimap(bool),
}

/// One boxed-list row: title, description and a trailing control.
fn row(name: &str, title: &str, desc: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("action-row");
    row.set_widget_name(name);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_halign(gtk::Align::Start);
    let d = gtk::Label::new(Some(desc));
    d.add_css_class("label-small");
    d.set_halign(gtk::Align::Start);
    d.set_xalign(0.0);
    d.set_wrap(true);
    d.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    text.append(&t);
    text.append(&d);
    row.append(&text);
    control.set_valign(gtk::Align::Center);
    row.append(control);
    row
}

fn dropdown(labels: &[&str], selected: u32) -> gtk::DropDown {
    let dd = gtk::DropDown::from_strings(labels);
    dd.set_selected(selected);
    dd
}

impl SimpleComponent for MargoTagsSettingsModel {
    type Input = MargoTagsSettingsInput;
    type Output = ();
    type Init = ();
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("boxed-list");
        root
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = config_manager()
            .config()
            .bars()
            .widgets()
            .margo_tags()
            .get_untracked();

        let display = dropdown(
            &MargoTagsDisplay::ALL.map(MargoTagsDisplay::label),
            cfg.display.index(),
        );
        let s = sender.clone();
        display.connect_selected_notify(move |d| {
            s.input(MargoTagsSettingsInput::Display(d.selected()))
        });
        root.append(&row(
            "setting:/bars/widgets/margo_tags/display",
            "Show",
            "All tags, or only the focused and occupied ones (optionally with the next empty tag to open a new one).",
            &display,
        ));

        let minimap = gtk::Switch::new();
        minimap.set_active(cfg.minimap);
        let s = sender.clone();
        minimap.connect_active_notify(move |w| {
            s.input(MargoTagsSettingsInput::Minimap(w.is_active()))
        });
        root.append(&row(
            "setting:/bars/widgets/margo_tags/minimap",
            "Minimap",
            "Draw each tag as a tiny map of its windows' layout (focused, urgent and floating windows tinted; the visible area outlined when the scroller strip runs wider than the screen) instead of its number.",
            &minimap,
        ));

        let mut current = cfg.clone();
        for (i, (state, field)) in GLYPH_STATES.iter().enumerate() {
            let entry = gtk::Entry::new();
            entry.set_text(field(&mut current));
            entry.set_placeholder_text(Some("number"));
            entry.set_max_width_chars(8);
            let s = sender.clone();
            entry.connect_changed(move |e| {
                s.input(MargoTagsSettingsInput::Glyph(i, e.text().to_string()));
            });
            root.append(&row(
                &format!("setting:/bars/widgets/margo_tags/glyph_{}", state.to_lowercase()),
                &format!("{state} glyph"),
                "Text or emoji shown for a tag in this state (e.g. 🌕). Empty shows the tag number.",
                &entry,
            ));
        }

        let accent_labels = TagAccent::ALL.map(TagAccent::label);
        for (i, (state, field)) in ACCENT_STATES.iter().enumerate() {
            let dd = dropdown(&accent_labels, field(&mut current).index());
            let s = sender.clone();
            dd.connect_selected_notify(move |d| {
                s.input(MargoTagsSettingsInput::Accent(i, d.selected()))
            });
            root.append(&row(
                &format!("setting:/bars/widgets/margo_tags/accent_{}", state.to_lowercase()),
                &format!("{state} colour"),
                "Theme colour for this state: the capsule fill on a focused / urgent tag, the glyph tint on an occupied one. Default keeps the standard look.",
                &dd,
            ));
        }

        ComponentParts {
            model: MargoTagsSettingsModel,
            widgets: (),
        }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            MargoTagsSettingsInput::Display(i) => {
                let v = MargoTagsDisplay::from_index(i);
                config_manager().update_config(|c| c.bars.widgets.margo_tags.display = v);
            }
            MargoTagsSettingsInput::Minimap(v) => {
                config_manager().update_config(|c| c.bars.widgets.margo_tags.minimap = v);
            }
            MargoTagsSettingsInput::Glyph(i, text) => {
                let field = GLYPH_STATES[i].1;
                let text = text.trim().to_string();
                config_manager().update_config(|c| *field(&mut c.bars.widgets.margo_tags) = text);
            }
            MargoTagsSettingsInput::Accent(i, idx) => {
                let field = ACCENT_STATES[i].1;
                let v = TagAccent::from_index(idx);
                config_manager().update_config(|c| *field(&mut c.bars.widgets.margo_tags) = v);
            }
        }
    }
}
