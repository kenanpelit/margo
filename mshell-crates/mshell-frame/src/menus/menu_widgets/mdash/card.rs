//! The one dashboard card: title INSIDE the card (icon + SemiBold caption
//! + dim trailing meta), the same `--card-*` tokens everywhere, and — for
//! cards that lead somewhere — a chevron button (keyboard / a11y) plus a
//! click anywhere on the card that isn't a control of its own.

use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, WidgetExt};

/// The one gap of the dashboard (`--space-2`): between cards, between
/// card rows and between tiles — every tab uses this and nothing else.
pub(crate) const GAP: i32 = 8;

pub(crate) struct Card {
    pub(crate) root: gtk::Box,
    pub(crate) meta: gtk::Label,
    pub(crate) body: gtk::Box,
    header: gtk::Box,
    title: gtk::Label,
}

impl Card {
    /// A card titled `title` with a leading symbolic `icon`.
    pub(crate) fn new(icon: &str, title: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("mdash-card");
        root.set_hexpand(true);
        // Natural height only: a card never grows to fill a column (set
        // explicitly so a child's expand flag can't propagate up). The
        // one flexible card (the agenda) opts in.
        root.set_vexpand(false);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("mdash-card-header");
        let image = gtk::Image::from_icon_name(icon);
        image.add_css_class("mdash-card-icon");
        header.append(&image);
        let label = gtk::Label::new(Some(title));
        label.add_css_class("mdash-card-title");
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.append(&label);
        let meta = gtk::Label::new(None);
        meta.add_css_class("mdash-card-meta");
        meta.set_xalign(1.0);
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        meta.set_visible(false);
        header.append(&meta);
        root.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.add_css_class("mdash-card-body");
        root.append(&body);

        Self {
            root,
            meta,
            body,
            header,
            title: label,
        }
    }

    pub(crate) fn set_title(&self, text: &str) {
        if self.title.label() != text {
            self.title.set_label(text);
        }
    }

    /// A control at the end of the header row (after the meta).
    pub(crate) fn append_to_header(&self, w: &impl gtk::prelude::IsA<gtk::Widget>) {
        self.header.append(w);
    }

    /// Trailing metadata in the header ("Istanbul", "3 running").
    pub(crate) fn set_meta(&self, text: &str) {
        self.meta.set_label(text);
        self.meta.set_visible(!text.is_empty());
    }

    /// The card opens `name` (its detailed menu): a chevron button in the
    /// header (focusable, named) and a click on any free spot of the card.
    pub(crate) fn opens(self, name: &str, on_open: impl Fn() + Clone + 'static) -> Self {
        self.root.add_css_class("activatable");
        let more = gtk::Button::from_icon_name("go-next-symbolic");
        more.add_css_class("mdash-card-open");
        mshell_common::a11y::name(&more, &format!("Open {name}"));
        let f = on_open.clone();
        more.connect_clicked(move |_| f());
        self.header.append(&more);

        // Bubble phase: a button inside the card claims its own click
        // first, so only clicks on the card's free area open the menu.
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_PRIMARY);
        click.connect_released(move |_, _, _, _| on_open());
        self.root.add_controller(click);
        self
    }
}

/// A dim, centred one-line state ("No events", "Nothing playing").
pub(crate) fn empty_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("mdash-empty");
    l.set_wrap(true);
    l.set_justify(gtk::Justification::Center);
    l.set_valign(gtk::Align::Center);
    l
}

/// A small caption + value pair, the metric idiom of the System tab.
pub(crate) fn value_label(css: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.add_css_class(css);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l
}
