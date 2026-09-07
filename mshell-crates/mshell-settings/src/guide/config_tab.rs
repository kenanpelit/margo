//! Settings -> Guide -> Settings: every `config.conf` key from
//! `margo/src/config.example.conf`, grouped by its own section dividers,
//! searchable, click-to-expand.

use super::config_parser::{self, ConfigKey};
use relm4::gtk::prelude::*;
use relm4::gtk::{self, gio, glib};

/// One row's data: the key itself plus which section it's under (shown as
/// a prefix so search results out of context are still legible).
struct Row {
    section: String,
    key: ConfigKey,
}

pub fn build() -> gtk::Widget {
    let source = include_str!("../../../../margo/src/config.example.conf");
    let sections = config_parser::parse(source);

    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for section in sections {
        for key in section.keys {
            store.append(&glib::BoxedAnyObject::new(Row {
                section: section.title.clone(),
                key,
            }));
        }
    }

    let query = std::rc::Rc::new(std::cell::RefCell::new(String::new()));

    let filter_query = query.clone();
    let filter = gtk::CustomFilter::new(move |obj| {
        let Some(bo) = obj.downcast_ref::<glib::BoxedAnyObject>() else {
            return false;
        };
        let row: std::cell::Ref<Row> = bo.borrow();
        let q = filter_query.borrow();
        if q.is_empty() {
            return true;
        }
        let q = q.as_str();
        row.key.key.to_lowercase().contains(q) || row.key.description.to_lowercase().contains(q)
    });
    let filter_model = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
    let selection = gtk::NoSelection::new(Some(filter_model));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        list_item.set_child(Some(&build_row()));
    });
    factory.connect_bind(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(item) = list_item.item() else { return };
        let bo = item.downcast_ref::<glib::BoxedAnyObject>().unwrap();
        let row: std::cell::Ref<Row> = bo.borrow();
        let Some(widget) = list_item.child() else {
            return;
        };
        bind_row(&widget, &row);
    });

    let list_view = gtk::ListView::new(Some(selection), Some(factory));
    list_view.add_css_class("guide-config-list");

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list_view)
        .build();

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search settings…")
        .build();
    let search_filter = filter.clone();
    let search_query = query;
    search.connect_search_changed(move |entry| {
        *search_query.borrow_mut() = entry.text().to_lowercase();
        search_filter.changed(gtk::FilterChange::Different);
    });

    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_top(8);
    page.append(&search);
    page.append(&scroller);
    page.upcast()
}

fn build_row() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.add_css_class("guide-row");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let key_value = gtk::Label::new(None);
    key_value.add_css_class("guide-row-name");
    key_value.set_halign(gtk::Align::Start);
    key_value.set_xalign(0.0);
    let section = gtk::Label::new(None);
    section.add_css_class("guide-row-summary");
    section.set_halign(gtk::Align::End);
    section.set_hexpand(true);
    header.append(&key_value);
    header.append(&section);

    let revealer = gtk::Revealer::new();
    let description = gtk::Label::new(None);
    description.add_css_class("guide-row-detail");
    description.set_halign(gtk::Align::Start);
    description.set_xalign(0.0);
    description.set_wrap(true);
    revealer.set_child(Some(&description));

    root.append(&header);
    root.append(&revealer);

    let click = gtk::GestureClick::new();
    let revealer_for_click = revealer.clone();
    click.connect_released(move |_, _, _, _| {
        revealer_for_click.set_reveal_child(!revealer_for_click.reveals_child());
    });
    root.add_controller(click);

    root
}

fn bind_row(widget: &gtk::Widget, row: &Row) {
    let root = widget.downcast_ref::<gtk::Box>().unwrap();
    let header = root.first_child().and_downcast::<gtk::Box>().unwrap();
    let key_value = header.first_child().and_downcast::<gtk::Label>().unwrap();
    let section = key_value
        .next_sibling()
        .and_downcast::<gtk::Label>()
        .unwrap();
    let revealer = header
        .next_sibling()
        .and_downcast::<gtk::Revealer>()
        .unwrap();
    let description = revealer.child().and_downcast::<gtk::Label>().unwrap();

    key_value.set_label(&format!("{} = {}", row.key.key, row.key.default));
    section.set_label(&row.section);
    description.set_label(&row.key.description);
    revealer.set_reveal_child(false);
}
