//! Settings → Default Applications page.
//!
//! Presents one row per content-type category (Web Browser, Email,
//! Calendar, Files, Archive Manager, Music, Video, Photos, Text Editor,
//! Code Editor, PDF Viewer, Documents, Spreadsheet, Presentation,
//! BitTorrent). Each row shows the category name on the left and a
//! `gtk::DropDown` on the right listing the installed applications that
//! declare one of the category's MIME types; the category table and the
//! choice rules live in [`choices`].
//!
//! Selecting a different application calls
//! `AppInfoExt::set_as_default_for_type` for every MIME in the category
//! the app can open — this writes `~/.config/mimeapps.list` via GIO
//! without any `pkexec` or system-level privileges.

mod choices;

use choices::{App, CATEGORIES, Choice};
use mshell_launcher::notify;
use relm4::gtk::gio::{self, prelude::AppInfoExt};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

// ── Model ─────────────────────────────────────────────────────────────────────

pub(crate) struct DefaultAppsSettingsModel {}

#[derive(Debug)]
pub(crate) enum DefaultAppsSettingsInput {}

#[derive(Debug)]
pub(crate) enum DefaultAppsSettingsOutput {}

pub(crate) struct DefaultAppsSettingsInit {}

#[derive(Debug)]
pub(crate) enum DefaultAppsSettingsCommandOutput {}

// ── Component ─────────────────────────────────────────────────────────────────

#[relm4::component(pub)]
impl Component for DefaultAppsSettingsModel {
    type CommandOutput = DefaultAppsSettingsCommandOutput;
    type Input = DefaultAppsSettingsInput;
    type Output = DefaultAppsSettingsOutput;
    type Init = DefaultAppsSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                // ── Hero header ──────────────────────────────────
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("application-x-executable-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Default Apps",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Choose which application opens each kind of file or link.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Category rows (populated imperatively in init) ──
                #[name = "rows_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        _root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = DefaultAppsSettingsModel {};
        let widgets = view_output!();

        // Build one row per category. The DropDown items are derived from
        // the installed-apps list at init time (apps don't change live).
        //
        // Anything that handles http links is a browser: browsers only
        // belong in the Web Browser row (see `choices`).
        let browser_ids: HashSet<String> = gio::AppInfo::all_for_type("x-scheme-handler/http")
            .iter()
            .filter_map(|a| a.id().map(String::from))
            .collect();
        let to_app = |info: &gio::AppInfo| -> Option<App> {
            let id = info.id()?.to_string();
            Some(App {
                browser: browser_ids.contains(&id),
                name: info.display_name().to_string(),
                id,
            })
        };

        for category in CATEGORIES {
            let mut infos: HashMap<String, gio::AppInfo> = HashMap::new();
            let mut remember = |info: gio::AppInfo| -> Option<App> {
                let app = to_app(&info)?;
                infos.entry(app.id.clone()).or_insert(info);
                Some(app)
            };
            // Apps that declare one of the row's types (exact MimeType=
            // entries plus the user's own associations — no parent-type
            // handlers, so file managers stay out of the office rows).
            let lists: Vec<Vec<App>> = category
                .mimes
                .iter()
                .chain(category.also_list)
                .map(|mime| {
                    gio::AppInfo::recommended_for_type(mime)
                        .into_iter()
                        .filter(|a| a.should_show())
                        .filter_map(&mut remember)
                        .collect()
                })
                .collect();
            let default =
                gio::AppInfo::default_for_type(category.primary, false).and_then(&mut remember);
            let model = choices::choices(category, lists, default);

            let labels: Vec<String> = model
                .items
                .iter()
                .map(|c| match c {
                    Choice::NotSet if model.items.len() == 1 => {
                        "No application installed".to_string()
                    }
                    Choice::NotSet => "Not set".to_string(),
                    Choice::App(app) => app.name.clone(),
                })
                .collect();
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let string_list = gtk::StringList::new(&label_refs);

            let dd = gtk::DropDown::new(Some(string_list), gtk::Expression::NONE);
            dd.set_selected(model.selected);
            dd.set_valign(gtk::Align::Center);
            dd.set_sensitive(model.items.iter().any(|c| matches!(c, Choice::App(_))));

            // Wire the selection change to set_as_default_for_type.
            let items = Rc::new(model.items);
            let infos = Rc::new(infos);
            dd.connect_selected_notify(move |dropdown| {
                let Some(Choice::App(app)) = items.get(dropdown.selected() as usize) else {
                    // "Not set" — GIO has no "unset", leave mimeapps.list alone.
                    return;
                };
                let Some(info) = infos.get(&app.id) else {
                    return;
                };
                // Including MIME subclasses: a text/plain editor opens
                // text/x-python, so it can be the code editor.
                let handles = |mime: &str| {
                    gio::AppInfo::all_for_type(mime)
                        .iter()
                        .any(|a| a.id().as_deref() == Some(app.id.as_str()))
                };
                for mime in choices::mimes_to_set(category, handles) {
                    if let Err(e) = info.set_as_default_for_type(mime) {
                        notify::toast("Default Apps", e.to_string());
                    }
                }
            });

            // Build the row: label on the left, dropdown on the right.
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            row.add_css_class("default-app-row");
            row.set_hexpand(true);

            let label = gtk::Label::new(Some(category.label));
            label.add_css_class("label-medium-bold");
            label.set_halign(gtk::Align::Start);
            label.set_hexpand(true);
            label.set_valign(gtk::Align::Center);

            row.append(&label);
            row.append(&dd);

            widgets.rows_box.append(&row);
        }

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        _message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        // No inputs to handle.
    }
}
