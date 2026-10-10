//! Settings → Translate.
//!
//! Configures the native translate feature (`mshell-translate`): provider
//! (key-free Google vs DeepL), target/secondary/source languages, and the
//! DeepL API key (stored in the keyring, only shown once DeepL is picked).
//! Saves through `mshell_translate::config` — own JSON file + keyring, the
//! same shape as `mshell-ai`'s settings page, so this follows that one's
//! hand-written pattern rather than the `reactive_settings!` macro (which is
//! tied to `mshell-config`'s reactive YAML store).

use crate::row::Row;
use mshell_translate::Provider;
use mshell_translate::config::{self, TranslateSettings};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub struct TranslateSettingsInit {}

pub struct TranslateSettingsModel {
    settings: TranslateSettings,
    provider_list: gtk::StringList,
}

#[derive(Debug)]
pub enum TranslateSettingsInput {
    ProviderPicked(u32),
    TargetLangChanged(String),
    SecondaryLangChanged(String),
    SourceLangChanged(String),
    DeepLKeyChanged(String),
}

/// Providers in dropdown order.
fn providers() -> [Provider; 2] {
    [Provider::Google, Provider::DeepL]
}

#[relm4::component(pub)]
impl Component for TranslateSettingsModel {
    type CommandOutput = ();
    type Input = TranslateSettingsInput;
    type Output = ();
    type Init = TranslateSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
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
                            set_label: "Translate",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Select or copy text, then press the translate keybind — or use the Translate menu panel directly.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Provider" },
                        #[template_child] desc { set_label: "Google works with zero setup; DeepL needs your own key." },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.provider_list),
                            #[watch]
                            set_selected: providers().iter().position(|p| p.id() == model.settings.provider).unwrap_or(0) as u32,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(TranslateSettingsInput::ProviderPicked(d.selected()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Target language" },
                        #[template_child] desc { set_label: "Language code to translate into (e.g. tr, en, es, de)." },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 100,
                            set_placeholder_text: Some("tr"),
                            #[watch]
                            set_text: &model.settings.target_lang,
                            connect_changed[sender] => move |e| {
                                sender.input(TranslateSettingsInput::TargetLangChanged(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Auto-flip to" },
                        #[template_child] desc { set_label: "If the selected text is already in the target language, translate to this instead. Blank disables the flip." },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 100,
                            set_placeholder_text: Some("en"),
                            #[watch]
                            set_text: &model.settings.secondary_lang,
                            connect_changed[sender] => move |e| {
                                sender.input(TranslateSettingsInput::SecondaryLangChanged(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Source language" },
                        #[template_child] desc { set_label: "Force a source language instead of auto-detecting. Leave blank for auto-detect." },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 100,
                            set_placeholder_text: Some("auto"),
                            #[watch]
                            set_text: &model.settings.source_lang,
                            connect_changed[sender] => move |e| {
                                sender.input(TranslateSettingsInput::SourceLangChanged(e.text().to_string()));
                            },
                        },
                    },
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[watch]
                    set_visible: Provider::parse(&model.settings.provider).needs_key(),

                    #[template]
                    Row {
                        #[template_child] title { set_label: "DeepL API key" },
                        #[template_child] desc { set_label: "Stored in the keyring. Free-tier keys end in “:fx”." },
                        #[name = "key_entry"]
                        gtk::PasswordEntry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 240,
                            set_show_peek_icon: true,
                            connect_changed[sender] => move |e| {
                                sender.input(TranslateSettingsInput::DeepLKeyChanged(e.text().to_string()));
                            },
                        },
                    },
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let settings = config::load();
        let provider_labels: Vec<&str> = providers().iter().map(|p| p.label()).collect();
        let model = TranslateSettingsModel {
            provider_list: gtk::StringList::new(&provider_labels),
            settings,
        };
        let widgets = view_output!();

        // Seed the DeepL key field from the keyring.
        widgets.key_entry.set_text(config::deepl_api_key().as_str());

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TranslateSettingsInput::ProviderPicked(idx) => {
                let p = providers()[idx as usize];
                if p.id() != self.settings.provider {
                    self.settings.provider = p.id().to_string();
                    self.save();
                }
            }
            TranslateSettingsInput::TargetLangChanged(lang) => {
                if lang != self.settings.target_lang {
                    self.settings.target_lang = lang;
                    self.save();
                }
            }
            TranslateSettingsInput::SecondaryLangChanged(lang) => {
                if lang != self.settings.secondary_lang {
                    self.settings.secondary_lang = lang;
                    self.save();
                }
            }
            TranslateSettingsInput::SourceLangChanged(lang) => {
                if lang != self.settings.source_lang {
                    self.settings.source_lang = lang;
                    self.save();
                }
            }
            TranslateSettingsInput::DeepLKeyChanged(key) => config::set_deepl_api_key(&key),
        }
    }
}

impl TranslateSettingsModel {
    fn save(&self) {
        config::save(&self.settings);
    }
}
