//! Reusable §12 panel header (DESIGN.md §12 "Header region").
//!
//! [`PanelHeader`] builds the one header row every panel uses: a leading
//! symbolic glyph, a SemiBold title that takes the slack (optionally with
//! a dim status line under it), optional dim trailing metadata, and
//! circular icon actions pinned right, each with an accessible name:
//!
//! ```text
//! [▤]  Dashboard            Fri · May 22   (⚙)
//! [⚠]  Crashes                             (⟳)
//! ```
//!
//! [`PanelHeaderModel`] is the `MenuWidget::PanelHeader` config widget
//! built on it: the dashboard's header (in place of the old `Clock`
//! hero) with a live date as metadata and the Settings gear. The date
//! ticks once a minute (it only flips at midnight; no need for 1 Hz).

use mshell_settings::open_settings;
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
pub(crate) fn greeting_text() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    greeting_for(now.hour(), std::env::var("USER").ok().as_deref())
}

/// The greeting for `hour` (0–23) and login `user`.
pub(crate) fn greeting_for(hour: u8, user: Option<&str>) -> String {
    let part = match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Good night",
    };
    match user.filter(|u| !u.is_empty()) {
        Some(user) => {
            let mut chars = user.chars();
            let name = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => user.to_string(),
            };
            format!("{part}, {name}")
        }
        None => part.to_string(),
    }
}

/// The §12 header row. Build it, keep the handles you update (title,
/// metadata, status line), and put [`Self::root`] at the
/// top of the panel (`#[local_ref] header -> gtk::Box {}` in a `view!`).
pub(crate) struct PanelHeader {
    root: gtk::Box,
    title: gtk::Label,
    meta: Option<gtk::Label>,
    subtitle: Option<gtk::Label>,
}

impl PanelHeader {
    /// Leading `icon` + `title`, nothing else yet. Pass the menu's
    /// `MenuWidget::display_name()` as the title so the header and
    /// Settings → Menus never name the panel differently.
    pub(crate) fn new(icon: &str, title: &str) -> Self {
        Self::fill(gtk::Box::new(Orientation::Horizontal, 0), icon, title)
    }

    /// Build the header into an existing (empty) box — a component whose
    /// root *is* the header.
    pub(crate) fn fill(root: gtk::Box, icon: &str, title: &str) -> Self {
        root.add_css_class("panel-header");
        root.set_orientation(Orientation::Horizontal);
        root.set_hexpand(true);
        root.set_spacing(12);
        root.set_valign(gtk::Align::Center);

        let image = gtk::Image::from_icon_name(icon);
        image.add_css_class("panel-header-icon");
        image.set_valign(gtk::Align::Center);
        root.append(&image);

        let title_label = gtk::Label::new(Some(title));
        title_label.add_css_class("panel-title");
        title_label.set_halign(gtk::Align::Start);
        title_label.set_valign(gtk::Align::Center);
        title_label.set_hexpand(true);
        title_label.set_xalign(0.0);
        root.append(&title_label);

        Self {
            root,
            title: title_label,
            meta: None,
            subtitle: None,
        }
    }

    /// A dim status line under the title (the title and it share the
    /// slack; e.g. USB's "2 drives · 1 mounted").
    pub(crate) fn with_subtitle(mut self) -> Self {
        let column = gtk::Box::new(Orientation::Vertical, 0);
        column.set_hexpand(true);
        column.set_valign(gtk::Align::Center);
        self.title.set_hexpand(false);
        self.root.remove(&self.title);
        column.append(&self.title);
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("label-small");
        subtitle.set_halign(gtk::Align::Start);
        subtitle.set_xalign(0.0);
        column.append(&subtitle);
        // Right after the leading icon.
        self.root
            .insert_child_after(&column, self.root.first_child().as_ref());
        self.subtitle = Some(subtitle);
        self
    }

    /// Dim trailing metadata (`--outline` tier), before the actions —
    /// a date, a plan name, "3 unread".
    pub(crate) fn with_meta(mut self) -> Self {
        let meta = gtk::Label::new(None);
        meta.add_css_class("panel-header-meta");
        meta.set_halign(gtk::Align::End);
        meta.set_valign(gtk::Align::Center);
        self.root.append(&meta);
        self.meta = Some(meta);
        self
    }

    /// Append a circular icon action. `name` is its accessible name and
    /// tooltip (§13.8 — an icon-only button needs one).
    pub(crate) fn action(self, icon: &str, name: &str, on_click: impl Fn() + 'static) -> Self {
        let button = gtk::Button::from_icon_name(icon);
        mshell_common::a11y::name(&button, name);
        button.add_css_class("panel-action-btn");
        button.set_valign(gtk::Align::Center);
        button.connect_clicked(move |_| on_click());
        self.root.append(&button);
        self
    }

    pub(crate) fn root(&self) -> &gtk::Box {
        &self.root
    }

    pub(crate) fn title(&self) -> &gtk::Label {
        &self.title
    }

    /// The metadata label ([`Self::with_meta`]).
    pub(crate) fn meta(&self) -> Option<&gtk::Label> {
        self.meta.as_ref()
    }

    /// The status line ([`Self::with_subtitle`]).
    pub(crate) fn subtitle(&self) -> Option<&gtk::Label> {
        self.subtitle.as_ref()
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

impl SimpleComponent for PanelHeaderModel {
    type Input = PanelHeaderInput;
    type Output = PanelHeaderOutput;
    type Init = PanelHeaderInit;
    type Root = gtk::Box;
    type Widgets = PanelHeader;

    fn init_root() -> Self::Root {
        gtk::Box::default()
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

        // Opening Settings runs the frame's toggle_menu, which already
        // hides this panel — so no CloseMenu emit is needed (same trap the
        // clipboard gear hit: a CloseMenu after would slam Settings shut).
        let widgets = PanelHeader::fill(root, "view-grid-symbolic", &model.title)
            .with_meta()
            .action("settings-symbolic", "Settings", open_settings);
        if let Some(meta) = widgets.meta() {
            meta.set_label(&model.date_label);
        }

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

    fn update_view(&self, widgets: &mut Self::Widgets, _sender: ComponentSender<Self>) {
        widgets.title().set_label(&self.title);
        if let Some(meta) = widgets.meta() {
            meta.set_label(&self.date_label);
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

#[cfg(test)]
mod tests {
    use super::greeting_for;

    #[test]
    fn greeting_follows_the_hour_and_capitalises_the_user() {
        assert_eq!(greeting_for(8, Some("kenan")), "Good morning, Kenan");
        assert_eq!(greeting_for(13, Some("kenan")), "Good afternoon, Kenan");
        assert_eq!(greeting_for(20, Some("kenan")), "Good evening, Kenan");
        assert_eq!(greeting_for(2, None), "Good night");
        assert_eq!(greeting_for(20, Some("")), "Good evening");
    }
}
