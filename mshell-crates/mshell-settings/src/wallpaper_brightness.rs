//! The wallpaper brightness slider (`wallpaper.brightness_percent`),
//! shared by Settings → Wallpaper and the Wallpaper menu. Moving it writes
//! the key after a short pause (each value re-decodes the wallpaper on
//! mshell-cache's worker, which skips straight to the newest request);
//! a change made elsewhere moves it back through a config effect.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, WallpaperStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, RangeExt, ScaleExt, WidgetExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub const MIN: f64 = 20.0;
pub const MAX: f64 = 150.0;

/// `Wallpaper brightness  [----o-----] 80 %  [reset]`. `compact` drops the
/// description (the menu).
pub fn brightness_row(compact: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("action-row");
    row.add_css_class("wallpaper-brightness-row");
    row.set_widget_name("setting:/wallpaper/brightness_percent");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Brightness"));
    title.add_css_class("label-medium-bold");
    title.set_halign(gtk::Align::Start);
    text.append(&title);
    if !compact {
        let desc = gtk::Label::new(Some(
            "Dim (or brighten) the wallpaper; kept across wallpaper changes. The image files are not changed.",
        ));
        desc.add_css_class("label-small");
        desc.set_halign(gtk::Align::Start);
        desc.set_xalign(0.0);
        desc.set_wrap(true);
        text.append(&desc);
        text.set_hexpand(true);
    }
    row.append(&text);

    let current = config_manager()
        .config()
        .wallpaper()
        .brightness_percent()
        .get_untracked();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, MIN, MAX, 5.0);
    scale.set_value(f64::from(current));
    scale.add_mark(100.0, gtk::PositionType::Bottom, None);
    scale.set_draw_value(false);
    scale.set_hexpand(compact);
    scale.set_width_request(180);
    scale.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&scale, "Wallpaper brightness");
    let value = gtk::Label::new(Some(&format!("{current} %")));
    value.add_css_class("label-small");
    value.set_width_chars(5);
    let reset = gtk::Button::from_icon_name("edit-undo-symbolic");
    mshell_common::a11y::name(&reset, "Reset the wallpaper brightness");
    reset.add_css_class("ok-button-flat");
    reset.set_valign(gtk::Align::Center);
    reset.set_tooltip_text(Some("Back to 100 %"));
    row.append(&scale);
    row.append(&value);
    row.append(&reset);

    // Programmatic moves (the effect) must not write back.
    let syncing = Rc::new(Cell::new(false));
    let generation = Rc::new(Cell::new(0u64));
    {
        let (syncing, generation, value) = (syncing.clone(), generation.clone(), value.clone());
        scale.connect_value_changed(move |s| {
            let v = s.value().round() as u32;
            value.set_label(&format!("{v} %"));
            if syncing.get() {
                return;
            }
            generation.set(generation.get().wrapping_add(1));
            let mine = generation.get();
            let generation = generation.clone();
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                if generation.get() == mine {
                    config_manager().update_config(move |c| c.wallpaper.brightness_percent = v);
                }
            });
        });
    }
    {
        let scale = scale.clone();
        reset.connect_clicked(move |_| scale.set_value(100.0));
    }
    let mut effects = EffectScope::new();
    {
        let (scale, syncing) = (scale.clone(), syncing.clone());
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .wallpaper()
                .brightness_percent()
                .get();
            if (scale.value().round() as u32) != v {
                syncing.set(true);
                scale.set_value(f64::from(v));
                syncing.set(false);
            }
        });
    }
    // The effects live as long as the row.
    let keep = RefCell::new(Some(effects));
    row.connect_destroy(move |_| {
        keep.borrow_mut().take();
    });
    row
}
