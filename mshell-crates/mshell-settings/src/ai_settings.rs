//! Settings → AI.
//!
//! Configures the native AI assistant (the `mshell-ai` engine): provider →
//! **model cascade** (models are auto-offered, not hand-typed — fetched live
//! from the provider's list-models endpoint with a curated fallback), API key
//! (stored in the keyring), endpoint override, temperature, max tokens, system
//! prompt, and history persistence. Saves through `mshell_ai::config`.

use crate::row::Row;
use mshell_ai::Provider;
use mshell_ai::config::{self, AiSettings};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub struct AiSettingsInit {}

pub struct AiSettingsModel {
    settings: AiSettings,
    provider_list: gtk::StringList,
    model_list: gtk::StringList,
    /// Dropdown strings for the chat font (index 0 = inherit) + the matching
    /// family names (index 0 = "" / inherit) for save-back.
    font_list: gtk::StringList,
    font_families: Vec<String>,
    status: String,
}

#[derive(Debug)]
pub enum AiSettingsInput {
    ProviderPicked(u32),
    ModelPicked(u32),
    KeyChanged(String),
    EndpointChanged(String),
    TempChanged(f64),
    TokensChanged(u32),
    PromptChanged(String),
    PersistToggled(bool),
    FontSizeChanged(u32),
    /// Font family picked from the dropdown (index 0 = inherit).
    FontFamilyPicked(u32),
    RefreshModels,
}

#[derive(Debug)]
pub enum AiSettingsCmd {
    /// Live model fetch result (models, or an error message).
    Models(Result<Vec<String>, String>),
}

/// Providers in dropdown order.
fn providers() -> [Provider; 5] {
    Provider::all()
}

#[relm4::component(pub)]
impl Component for AiSettingsModel {
    type CommandOutput = AiSettingsCmd;
    type Input = AiSettingsInput;
    type Output = ();
    type Init = AiSettingsInit;

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
                        set_icon_name: Some("starred-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "AI",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Pick a provider and model for the in-shell assistant. Models are offered automatically; click Refresh to pull the live list once your key is set.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Provider" },
                    #[template_child] desc { set_label: "Gemini · OpenAI · Anthropic · Ollama · Custom (OpenAI-compatible)." },
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.provider_list),
                        #[watch]
                        set_selected: providers().iter().position(|p| p.id() == model.settings.provider).unwrap_or(0) as u32,
                        connect_selected_notify[sender] => move |d| {
                            sender.input(AiSettingsInput::ProviderPicked(d.selected()));
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Model" },
                    #[template_child] desc { set_label: "Auto-offered for the provider. Refresh pulls the live list." },
                    gtk::Box {
                        set_spacing: 6,
                        set_valign: gtk::Align::Center,
                        #[name="model_drop"]
                        gtk::DropDown {
                            set_width_request: 240,
                            set_model: Some(&model.model_list),
                            connect_selected_notify[sender] => move |d| {
                                sender.input(AiSettingsInput::ModelPicked(d.selected()));
                            },
                        },
                        gtk::Button {
                            set_icon_name: "view-refresh-symbolic",
                            set_tooltip_text: Some("Fetch the live model list from the provider"),
                            connect_clicked => AiSettingsInput::RefreshModels,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    add_css_class: "dim-label",
                    #[watch]
                    set_label: &model.status,
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_visible: !model.status.is_empty(),
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "API key" },
                    #[template_child] desc { set_label: "Stored in the keyring. Not needed for Ollama / local endpoints." },
                    #[name="key_entry"]
                    gtk::PasswordEntry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_show_peek_icon: true,
                        connect_changed[sender] => move |e| {
                            sender.input(AiSettingsInput::KeyChanged(e.text().to_string()));
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Endpoint override" },
                    #[template_child] desc { set_label: "Proxy / LocalAI / LM Studio / vLLM base URL. Blank = provider default." },
                    #[name="endpoint_entry"]
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        #[watch]
                        set_text: &model.settings.endpoint,
                        connect_changed[sender] => move |e| {
                            sender.input(AiSettingsInput::EndpointChanged(e.text().to_string()));
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Temperature" },
                    #[template_child] desc { set_label: "0 = focused / deterministic, 2 = creative." },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 2.0),
                        set_increments: (0.1, 0.5),
                        set_digits: 2,
                        #[watch]
                        set_value: model.settings.temperature,
                        connect_value_changed[sender] => move |s| {
                            sender.input(AiSettingsInput::TempChanged(s.value()));
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Max tokens" },
                    #[template_child] desc { set_label: "Upper bound on the reply length." },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (16.0, 32768.0),
                        set_increments: (64.0, 512.0),
                        set_digits: 0,
                        #[watch]
                        set_value: model.settings.max_tokens as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(AiSettingsInput::TokensChanged(s.value() as u32));
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "System prompt" },
                    #[template_child] desc { set_label: "Optional message prepended to every conversation." },
                    #[name="prompt_entry"]
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        #[watch]
                        set_text: &model.settings.system_prompt,
                        connect_changed[sender] => move |e| {
                            sender.input(AiSettingsInput::PromptChanged(e.text().to_string()));
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Persist history" },
                    #[template_child] desc { set_label: "Keep the conversation across restarts." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_active: model.settings.persist_history,
                        connect_state_set[sender] => move |_, on| {
                            sender.input(AiSettingsInput::PersistToggled(on));
                            gtk::glib::Propagation::Proceed
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Chat font size" },
                    #[template_child] desc { set_label: "Transcript text size in the AI menu (px)." },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (8.0, 32.0),
                        set_increments: (1.0, 2.0),
                        set_digits: 0,
                        #[watch]
                        set_value: model.settings.font_size as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(AiSettingsInput::FontSizeChanged(s.value() as u32));
                        },
                    },
                },

                #[template]
                Row {
                    #[template_child] title { set_label: "Chat font family" },
                    #[template_child] desc { set_label: "Font for the AI transcript; “Inherit” uses the shell font." },
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_width_request: 240,
                        set_enable_search: true,
                        set_model: Some(&model.font_list),
                        #[watch]
                        set_selected: font_index(&model.font_families, &model.settings.font_family),
                        connect_selected_notify[sender] => move |d| {
                            sender.input(AiSettingsInput::FontFamilyPicked(d.selected()));
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
        // Font dropdown: "Inherit" at index 0, then every installed family.
        // `font_families[0]` is the empty sentinel so indices line up.
        let mut font_families = vec![String::new()];
        font_families.extend(crate::fonts_settings::available_fonts());
        let font_labels: Vec<&str> = std::iter::once("Inherit (shell font)")
            .chain(font_families.iter().skip(1).map(String::as_str))
            .collect();
        let model = AiSettingsModel {
            provider_list: gtk::StringList::new(&provider_labels),
            model_list: gtk::StringList::new(&[]),
            font_list: gtk::StringList::new(&font_labels),
            font_families,
            settings,
            status: String::new(),
        };
        let widgets = view_output!();

        // Seed the API key field from the keyring + the model dropdown from the
        // curated fallback for the saved provider.
        widgets.key_entry.set_text(&config::api_key());
        populate_models(
            &widgets.model_drop,
            &model.model_list,
            Provider::parse(&model.settings.provider),
            &model.settings.model,
            None,
        );

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AiSettingsInput::ProviderPicked(idx) => {
                let p = providers()[idx as usize];
                if p.id() != self.settings.provider {
                    self.settings.provider = p.id().to_string();
                    self.settings.model = p.default_model().to_string();
                    self.status.clear();
                    self.save();
                    populate_models(
                        &widgets.model_drop,
                        &self.model_list,
                        p,
                        &self.settings.model,
                        None,
                    );
                }
            }
            AiSettingsInput::ModelPicked(idx) => {
                if let Some(m) = self.model_list.string(idx) {
                    let m = m.to_string();
                    if m != self.settings.model {
                        self.settings.model = m;
                        self.save();
                    }
                }
            }
            AiSettingsInput::KeyChanged(k) => config::set_api_key(&k),
            AiSettingsInput::EndpointChanged(e) => {
                if e != self.settings.endpoint {
                    self.settings.endpoint = e;
                    self.save();
                }
            }
            AiSettingsInput::TempChanged(t) => {
                if (t - self.settings.temperature).abs() > f64::EPSILON {
                    self.settings.temperature = t;
                    self.save();
                }
            }
            AiSettingsInput::TokensChanged(n) => {
                if n != self.settings.max_tokens {
                    self.settings.max_tokens = n;
                    self.save();
                }
            }
            AiSettingsInput::PromptChanged(p) => {
                if p != self.settings.system_prompt {
                    self.settings.system_prompt = p;
                    self.save();
                }
            }
            AiSettingsInput::PersistToggled(on) => {
                if on != self.settings.persist_history {
                    self.settings.persist_history = on;
                    self.save();
                }
            }
            AiSettingsInput::FontSizeChanged(n) => {
                if n != self.settings.font_size {
                    self.settings.font_size = n;
                    self.save();
                }
            }
            AiSettingsInput::FontFamilyPicked(idx) => {
                // Index 0 = "Inherit" → empty; otherwise the family name.
                let fam = if idx == 0 {
                    String::new()
                } else {
                    self.font_families
                        .get(idx as usize)
                        .cloned()
                        .unwrap_or_default()
                };
                if fam != self.settings.font_family {
                    self.settings.font_family = fam;
                    self.save();
                }
            }
            AiSettingsInput::RefreshModels => {
                self.status = "Fetching models…".to_string();
                let cfg = config::resolved();
                sender.command(|out, _shutdown| async move {
                    let res = tokio::task::spawn_blocking(move || mshell_ai::fetch_models(&cfg))
                        .await
                        .unwrap_or_else(|_| Err("worker panicked".into()));
                    let _ = out.send(AiSettingsCmd::Models(res));
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let AiSettingsCmd::Models(res) = message;
        match res {
            Ok(models) => {
                self.status = format!("{} models available.", models.len());
                let refs: Vec<&str> = models.iter().map(String::as_str).collect();
                populate_models(
                    &widgets.model_drop,
                    &self.model_list,
                    Provider::parse(&self.settings.provider),
                    &self.settings.model,
                    Some(&refs),
                );
            }
            Err(e) => self.status = format!("Couldn't fetch models: {e}"),
        }
    }
}

impl AiSettingsModel {
    fn save(&self) {
        config::save(&self.settings);
    }
}

/// Dropdown index for the saved font family (0 = inherit / not found).
fn font_index(families: &[String], current: &str) -> u32 {
    if current.is_empty() {
        return 0;
    }
    families.iter().position(|f| f == current).unwrap_or(0) as u32
}

/// Refill the model dropdown's `StringList` in place (splice, never
/// `set_model` — that would respin per the relm4 dropdown gotcha) with either
/// `models` or the provider's curated fallback, ensuring `current` is present
/// and selected.
fn populate_models(
    drop: &gtk::DropDown,
    list: &gtk::StringList,
    provider: Provider,
    current: &str,
    models: Option<&[&str]>,
) {
    let fallback: Vec<&str> = provider.fallback_models().to_vec();
    let mut items: Vec<String> = models
        .map(|m| m.iter().map(|s| s.to_string()).collect())
        .unwrap_or_else(|| fallback.iter().map(|s| s.to_string()).collect());
    let current = if current.is_empty() {
        provider.default_model().to_string()
    } else {
        current.to_string()
    };
    if !current.is_empty() && !items.iter().any(|m| m == &current) {
        items.insert(0, current.clone());
    }

    // Splice the whole list: remove all, then add the new set.
    list.splice(0, list.n_items(), &[]);
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    list.splice(0, 0, &refs);

    let sel = items.iter().position(|m| m == &current).unwrap_or(0) as u32;
    drop.set_selected(sel);
}
