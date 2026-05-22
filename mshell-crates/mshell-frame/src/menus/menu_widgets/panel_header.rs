//! Reusable §12 panel header (DESIGN.md §12 "Header region").
//!
//! Layout: a leading symbolic glyph, a SemiBold display title that
//! takes the slack, a live date as dim metadata, and a circular
//! settings gear pinned right:
//!
//! ```text
//! [▤]  Dashboard            Fri · May 22   (⚙)
//! ```
//!
//! The dashboard uses this in place of the old `Clock` hero — the big
//! time was redundant with the bar clock, so the header carries the
//! title + a quiet date instead. The date ticks once a minute (it only
//! flips at midnight; no need for the clock's 1 Hz).

use mshell_settings::open_settings;
use relm4::gtk::prelude::OrientableExt;
use relm4::{
    ComponentParts, ComponentSender, SimpleComponent,
    gtk::{
        self, Orientation,
        glib::{self, SourceId},
        prelude::*,
    },
    once_cell,
};
use time::OffsetDateTime;
use time::format_description::parse;

static DATE_FORMAT: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        // Short weekday + short month + day → "Fri · May 22". Quiet
        // date stamp, not a headline (the title is the headline).
        parse("[weekday repr:short] · [month repr:short] [day padding:none]").unwrap()
    });

fn current_date() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(&DATE_FORMAT).unwrap_or_default()
}

pub(crate) struct PanelHeaderModel {
    title: String,
    date_label: String,
    timer_id: Option<SourceId>,
}

#[derive(Debug)]
pub(crate) enum PanelHeaderInput {
    UpdateDate,
}

#[derive(Debug)]
pub(crate) enum PanelHeaderOutput {}

pub(crate) struct PanelHeaderInit {
    pub title: String,
}

#[relm4::component(pub)]
impl SimpleComponent for PanelHeaderModel {
    type Input = PanelHeaderInput;
    type Output = PanelHeaderOutput;
    type Init = PanelHeaderInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "panel-header",
            set_orientation: Orientation::Horizontal,
            set_hexpand: true,
            set_spacing: 12,
            set_valign: gtk::Align::Center,

            gtk::Image {
                add_css_class: "panel-header-icon",
                set_valign: gtk::Align::Center,
                set_icon_name: Some("view-grid-symbolic"),
            },

            // Title takes the slack, pushing the date + gear right.
            gtk::Label {
                add_css_class: "panel-title",
                set_label: model.title.as_str(),
                set_halign: gtk::Align::Start,
                set_valign: gtk::Align::Center,
                set_hexpand: true,
            },

            // Live date as dim metadata (DESIGN.md §12 — recedes
            // behind the title, sits at the --outline tier via SCSS).
            gtk::Label {
                add_css_class: "panel-header-meta",
                #[watch]
                set_label: model.date_label.as_str(),
                set_halign: gtk::Align::End,
                set_valign: gtk::Align::Center,
            },

            // Circular settings gear. Opening Settings runs the frame's
            // toggle_menu, which already hides this panel — so no
            // CloseMenu emit is needed (same trap the clipboard gear
            // hit: a CloseMenu after would slam Settings shut).
            gtk::Button {
                add_css_class: "panel-action-btn",
                set_valign: gtk::Align::Center,
                set_icon_name: "settings-symbolic",
                set_tooltip_text: Some("Settings"),
                connect_clicked => move |_| {
                    open_settings();
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let sender_clone = sender.clone();
        let id = glib::timeout_add_local(std::time::Duration::from_secs(60), move || {
            sender_clone.input(PanelHeaderInput::UpdateDate);
            glib::ControlFlow::Continue
        });

        let model = PanelHeaderModel {
            title: params.title,
            date_label: current_date(),
            timer_id: Some(id),
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            PanelHeaderInput::UpdateDate => {
                self.date_label = current_date();
            }
        }
    }
}

impl Drop for PanelHeaderModel {
    fn drop(&mut self) {
        if let Some(id) = self.timer_id.take() {
            id.remove();
        }
    }
}
