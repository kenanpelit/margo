//! Settings -> Guide -> Actions: every dispatch action `mctl::actions`
//! knows about, grouped, searchable, click-to-expand.

use mctl::actions::{ACTIONS, Action, Group};
use relm4::gtk::prelude::*;
use relm4::gtk::{self, gio, glib};

const GROUP_ORDER: &[Group] = &[
    Group::Tag,
    Group::Focus,
    Group::Layout,
    Group::Scroller,
    Group::Window,
    Group::Scratchpad,
    Group::Overview,
    Group::System,
];

pub fn build() -> gtk::Widget {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    // One row per action, ordered by GROUP_ORDER then by name within a
    // group -- matches `mctl actions --verbose`'s own section order, so
    // this list reads the same way the CLI output does.
    let mut sorted: Vec<&'static Action> = ACTIONS.iter().collect();
    sorted.sort_by_key(|a| {
        let group_rank = GROUP_ORDER
            .iter()
            .position(|g| *g == a.group)
            .unwrap_or(usize::MAX);
        (group_rank, a.name)
    });
    for action in sorted {
        store.append(&glib::BoxedAnyObject::new(action));
    }

    let query = std::rc::Rc::new(std::cell::RefCell::new(String::new()));

    let filter_query = query.clone();
    let filter = gtk::CustomFilter::new(move |obj| {
        let Some(bo) = obj.downcast_ref::<glib::BoxedAnyObject>() else {
            return false;
        };
        let action = bo.borrow::<&'static Action>();
        let q = filter_query.borrow();
        if q.is_empty() {
            return true;
        }
        let q = q.as_str();
        action.name.contains(q)
            || action.summary.to_lowercase().contains(q)
            || action.aliases.iter().any(|a| a.contains(q))
    });
    let filter_model = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
    let selection = gtk::NoSelection::new(Some(filter_model));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let row = build_row();
        list_item.set_child(Some(&row));
    });
    factory.connect_bind(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(item) = list_item.item() else { return };
        let bo = item.downcast_ref::<glib::BoxedAnyObject>().unwrap();
        let action: std::cell::Ref<&'static Action> = bo.borrow();
        let action = *action;
        let Some(row) = list_item.child() else { return };
        bind_row(&row, action);
    });

    let list_view = gtk::ListView::new(Some(selection), Some(factory));
    list_view.add_css_class("guide-actions-list");

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list_view)
        .build();

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search actions…")
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

/// One list row's static widget structure: name + summary on top, an
/// expandable revealer underneath carrying `args` / `detail`. The
/// revealer only ever shows/hides -- its content is set once per bind in
/// `bind_row`, matching the factory's setup/bind split (structure built
/// once per pooled row in `connect_setup`, content refreshed per item in
/// `connect_bind`).
fn build_row() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.add_css_class("guide-row");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::new(None);
    name.add_css_class("guide-row-name");
    name.set_halign(gtk::Align::Start);
    name.set_xalign(0.0);
    let summary = gtk::Label::new(None);
    summary.add_css_class("guide-row-summary");
    summary.set_halign(gtk::Align::Start);
    summary.set_xalign(0.0);
    summary.set_hexpand(true);
    summary.set_wrap(true);
    header.append(&name);
    header.append(&summary);

    let revealer = gtk::Revealer::new();
    let detail = gtk::Label::new(None);
    detail.add_css_class("guide-row-detail");
    detail.set_halign(gtk::Align::Start);
    detail.set_xalign(0.0);
    detail.set_wrap(true);
    revealer.set_child(Some(&detail));

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

fn bind_row(row: &gtk::Widget, action: &'static Action) {
    let root = row.downcast_ref::<gtk::Box>().unwrap();
    let header = root.first_child().and_downcast::<gtk::Box>().unwrap();
    let name = header.first_child().and_downcast::<gtk::Label>().unwrap();
    let summary = name.next_sibling().and_downcast::<gtk::Label>().unwrap();
    let revealer = header
        .next_sibling()
        .and_downcast::<gtk::Revealer>()
        .unwrap();
    let detail = revealer.child().and_downcast::<gtk::Label>().unwrap();

    name.set_label(action.name);
    summary.set_label(action.summary);

    let has_extra = !action.args.is_empty() || !action.detail.is_empty();
    if has_extra {
        let mut text = String::new();
        if !action.args.is_empty() {
            text.push_str("Args: ");
            text.push_str(action.args);
        }
        if !action.detail.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(action.detail);
        }
        detail.set_label(&text);
    }
    revealer.set_reveal_child(false);
    root.set_can_target(true);
    root.set_sensitive(true);
    let _ = has_extra; // row stays clickable either way; an empty revealer just toggles open on nothing
}
