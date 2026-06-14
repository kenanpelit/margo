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

/// Time-aware greeting headline, e.g. "Good evening, Kenan". The name is
/// the capitalised `$USER` (best-effort; dropped when unavailable so the
/// greeting still reads cleanly). Recomputed on the minute tick so it
/// flips as the day rolls from morning → afternoon → evening.
fn greeting_text() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let part = match now.hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Good night",
    };
    match std::env::var("USER").ok().filter(|u| !u.is_empty()) {
        Some(user) => {
            let mut chars = user.chars();
            let name = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => user,
            };
            format!("{part}, {name}")
        }
        None => part.to_string(),
    }
}

pub(crate) struct PanelHeaderModel {
    /// When true the title carries a live greeting instead of a static
    /// label; the minute tick refreshes it alongside the date.
    greeting: bool,
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
    pub greeting: bool,
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
            // `#[watch]` so the greeting variant refreshes on the tick.
            gtk::Label {
                add_css_class: "panel-title",
                #[watch]
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

        let title = if params.greeting {
            greeting_text()
        } else {
            params.title
        };
        let model = PanelHeaderModel {
            greeting: params.greeting,
            title,
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
                if self.greeting {
                    self.title = greeting_text();
                }
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
