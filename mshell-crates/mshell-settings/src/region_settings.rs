//! Settings → Region & Language.
//!
//! Wraps `localectl` for the **system locale** (`LANG`). Writes go through
//! `localectl set-locale`, which authenticates via polkit
//! (org.freedesktop.locale1) — margo's integrated agent prompts for it. The
//! keyboard layout lives on the Input page (it's a compositor / xkb setting),
//! so this page links there rather than duplicating it.

use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct RegionSettingsModel {
    locale: String,
    locales: gtk::StringList,
    locale_index: u32,
}

#[derive(Debug)]
pub(crate) enum RegionSettingsInput {
    SetLocale(u32),
}

#[derive(Debug)]
pub(crate) enum RegionSettingsOutput {}

pub(crate) struct RegionSettingsInit {}

#[derive(Debug)]
pub(crate) enum RegionSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for RegionSettingsModel {
    type CommandOutput = RegionSettingsCommandOutput;
    type Input = RegionSettingsInput;
    type Output = RegionSettingsOutput;
    type Init = RegionSettingsInit;

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

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("preferences-desktop-locale-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Region & Language",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "System language + formats. Applied via localectl (prompts for authentication).",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Language",
                    set_halign: gtk::Align::Start,
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "System language (LANG)" },
                    #[template_child] desc { set_label: "Type to search. Takes effect on the next login." },
                    #[name = "locale_dd"]
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_width_request: 260,
                        set_enable_search: true,
                        set_model: Some(&model.locales),
                        #[block_signal(locale_handler)]
                        set_selected: model.locale_index,
                        connect_selected_notify[sender] => move |d| {
                            sender.input(RegionSettingsInput::SetLocale(d.selected()));
                        } @locale_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_top: 8,
                    set_label: "Keyboard layout (xkb) is set on the Input page. Missing a language? Generate it first (e.g. uncomment it in /etc/locale.gen and run locale-gen).",
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let _ = &sender;
        let locale = read_locale();
        let locales = list_locales();
        let locale_index = locales.iter().position(|l| *l == locale).unwrap_or(0) as u32;
        let refs: Vec<&str> = locales.iter().map(|s| s.as_str()).collect();
        let model = RegionSettingsModel {
            locale,
            locales: gtk::StringList::new(&refs),
            locale_index,
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            RegionSettingsInput::SetLocale(idx) => {
                if let Some(locale) = self.locales.string(idx) {
                    let locale = locale.to_string();
                    self.locale = locale.clone();
                    self.locale_index = idx;
                    run_localectl(&["set-locale", &format!("LANG={locale}")]);
                }
            }
        }
    }
}

/// The system `LANG` from `localectl status`, falling back to $LANG then a
/// neutral default.
fn read_locale() -> String {
    if let Ok(out) = std::process::Command::new("localectl").arg("status").output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if line.contains("System Locale:")
                && let Some(lang) = line
                    .split_whitespace()
                    .find_map(|t| t.strip_prefix("LANG="))
                && !lang.is_empty()
            {
                return lang.to_string();
            }
        }
    }
    std::env::var("LANG")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "C.UTF-8".to_string())
}

/// Available locales from `localectl list-locales`; falls back to a small
/// set so the dropdown is never empty.
fn list_locales() -> Vec<String> {
    if let Ok(out) = std::process::Command::new("localectl").arg("list-locales").output()
        && out.status.success()
    {
        let locales: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !locales.is_empty() {
            return locales;
        }
    }
    ["C.UTF-8", "en_US.UTF-8", "tr_TR.UTF-8"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Run `localectl <args>` (polkit-authenticated), reaping asynchronously.
fn run_localectl(args: &[&str]) {
    match std::process::Command::new("localectl").args(args).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, ?args, "region: localectl failed to spawn"),
    }
}
