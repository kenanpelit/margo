//! Settings → Default Applications page.
//!
//! Presents one row per content-type category (Web Browser, Email,
//! Calendar, Music, Video, Photos, Files). Each row shows the
//! category name on the left and a `gtk::DropDown` on the right whose
//! items are the installed applications that handle that MIME type,
//! as reported by `gio::AppInfo::all_for_type`.
//!
//! Selecting a different application calls
//! `AppInfoExt::set_as_default_for_type` for every MIME in the
//! category — this writes `~/.config/mimeapps.list` via GIO without
//! any `pkexec` or system-level privileges.

use mshell_launcher::notify;
use relm4::gtk::gio::{self, prelude::AppInfoExt};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::rc::Rc;

// ── Category table ────────────────────────────────────────────────────────────
//
// (label, primary mime used to look up the current default,
//  all mimes to set when the user picks a new default)

const CATEGORIES: &[(&str, &str, &[&str])] = &[
    (
        "Web Browser",
        "x-scheme-handler/http",
        &[
            "x-scheme-handler/http",
            "x-scheme-handler/https",
            "text/html",
        ],
    ),
    (
        "Email",
        "x-scheme-handler/mailto",
        &["x-scheme-handler/mailto"],
    ),
    ("Calendar", "text/calendar", &["text/calendar"]),
    (
        "Music",
        "audio/mpeg",
        &["audio/mpeg", "audio/flac", "audio/x-vorbis+ogg"],
    ),
    (
        "Video",
        "video/mp4",
        &["video/mp4", "video/x-matroska"],
    ),
    ("Photos", "image/jpeg", &["image/jpeg", "image/png"]),
    ("Files", "inode/directory", &["inode/directory"]),
];

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
                            set_label: "Default Applications",
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

        // Build one row per category.  The DropDown items are derived from
        // the installed-apps list at init time (apps don't change live).
        for &(category_label, primary_mime, all_mimes) in CATEGORIES {
            // Gather candidates — apps that handle the primary MIME and
            // should be shown in the application chooser.
            let candidates: Vec<gio::AppInfo> = gio::AppInfo::all_for_type(primary_mime)
                .into_iter()
                .filter(|a| a.should_show())
                .collect();

            // Build a parallel StringList for the DropDown model.
            let names: Vec<glib::GString> = candidates.iter().map(|a| a.display_name()).collect();
            let string_items: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            let string_list = gtk::StringList::new(&string_items);

            // Determine which entry is currently the default.
            let current_idx: u32 = {
                let default_app = gio::AppInfo::default_for_type(primary_mime, false);
                default_app
                    .and_then(|def| {
                        let def_id = def.id();
                        candidates.iter().position(|c| c.id() == def_id)
                    })
                    .unwrap_or(0) as u32
            };

            // Build the DropDown.
            let dd = gtk::DropDown::new(
                Some(string_list),
                gtk::Expression::NONE,
            );
            dd.set_selected(current_idx);
            dd.set_valign(gtk::Align::Center);

            // Wire the selection change to set_as_default_for_type.
            // Wrap the candidates Vec in Rc so it can be shared into
            // the closure without requiring Clone on AppInfo.
            let candidates_rc = Rc::new(candidates);
            let mimes: Vec<&'static str> = all_mimes.to_vec();
            dd.connect_selected_notify(move |dropdown| {
                let idx = dropdown.selected() as usize;
                let Some(app) = candidates_rc.get(idx) else {
                    return;
                };
                for &mime in &mimes {
                    if let Err(e) = app.set_as_default_for_type(mime) {
                        notify::toast("Default Apps", e.to_string());
                    }
                }
            });

            // Build the row: label on the left, dropdown on the right.
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            row.add_css_class("default-app-row");
            row.set_hexpand(true);

            let label = gtk::Label::new(Some(category_label));
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

// Needed by relm4 — gio::AppInfo is not glib-friendly for use_glib_ffi,
// but since DefaultAppsSettingsModel is empty, we just need the import.
use relm4::gtk::glib;
