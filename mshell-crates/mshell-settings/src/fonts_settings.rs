//! Fonts settings page — primary / secondary / tertiary font
//! pickers. Lifted out of [`crate::theme_settings`] so the user
//! can find typography in one obvious place instead of buried
//! under "Theme".
//!
//! Each pick is written back to `theme.attributes.font.*` on the
//! shared `config_manager` store; matugen + the style manager
//! pick the change up via their effect subscriptions and rewrite
//! the CSS variables that drive every label across the shell.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, FontStoreFields, SizingStoreFields, ThemeAttributesStoreFields,
    ThemeStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, CastNone, ListModelExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct FontsSettingsModel {
    available_fonts: gtk::StringList,
    active_primary_font: String,
    active_secondary_font: String,
    active_tertiary_font: String,
    active_monospace_font: String,
    font_scale: f64,
    bar_font_scale: f64,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum FontsSettingsInput {
    PrimaryFontSelected(Option<String>),
    SecondaryFontSelected(Option<String>),
    TertiaryFontSelected(Option<String>),
    MonospaceFontSelected(Option<String>),

    PrimaryFontEffect(String),
    SecondaryFontEffect(String),
    TertiaryFontEffect(String),
    MonospaceFontEffect(String),

    FontScaleChanged(f64),
    FontScaleEffect(f64),
    BarFontScaleChanged(f64),
    BarFontScaleEffect(f64),
}

#[derive(Debug)]
pub(crate) enum FontsSettingsOutput {}

pub(crate) struct FontsSettingsInit {}

#[derive(Debug)]
pub(crate) enum FontsSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for FontsSettingsModel {
    type CommandOutput = FontsSettingsCommandOutput;
    type Input = FontsSettingsInput;
    type Output = FontsSettingsOutput;
    type Init = FontsSettingsInit;

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
                        set_icon_name: Some("font-x-generic-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Fonts",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Default fonts matugen feeds to every label across the shell. Primary covers body text; secondary / tertiary surface in opt-in components.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Choose the typefaces matugen feeds to every label across the shell. The three slots are independent — primary covers body text, secondary / tertiary are advisory and surface in components that opt in.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                // ── Primary ─────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Primary",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Primary font",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "The primary font in mshell. Sent to matugen as mshell.font.primary.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.available_fonts),
                        #[watch]
                        #[block_signal(primary_font_handler)]
                        set_selected: (0..model.available_fonts.n_items())
                            .find(|&i| model.available_fonts.string(i).as_deref() == Some(model.active_primary_font.as_str()))
                            .unwrap_or(0),
                        connect_selected_notify[sender] => move |dd| {
                            let selected = dd.selected_item()
                                .and_downcast::<gtk::StringObject>()
                                .map(|s| s.string().to_string());
                            sender.input(FontsSettingsInput::PrimaryFontSelected(selected));
                        } @primary_font_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "font-preview",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_top: 2,
                    #[watch]
                    set_markup: &font_preview_markup(&model.active_primary_font),
                },

                gtk::Separator {},

                // ── Secondary ───────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Secondary",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Secondary font",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Sent to matugen as mshell.font.secondary.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.available_fonts),
                        #[watch]
                        #[block_signal(secondary_font_handler)]
                        set_selected: (0..model.available_fonts.n_items())
                            .find(|&i| model.available_fonts.string(i).as_deref() == Some(model.active_secondary_font.as_str()))
                            .unwrap_or(0),
                        connect_selected_notify[sender] => move |dd| {
                            let selected = dd.selected_item()
                                .and_downcast::<gtk::StringObject>()
                                .map(|s| s.string().to_string());
                            sender.input(FontsSettingsInput::SecondaryFontSelected(selected));
                        } @secondary_font_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "font-preview",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_top: 2,
                    #[watch]
                    set_markup: &font_preview_markup(&model.active_secondary_font),
                },

                gtk::Separator {},

                // ── Tertiary ────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Tertiary",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Tertiary font",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Sent to matugen as mshell.font.tertiary.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.available_fonts),
                        #[watch]
                        #[block_signal(tertiary_font_handler)]
                        set_selected: (0..model.available_fonts.n_items())
                            .find(|&i| model.available_fonts.string(i).as_deref() == Some(model.active_tertiary_font.as_str()))
                            .unwrap_or(0),
                        connect_selected_notify[sender] => move |dd| {
                            let selected = dd.selected_item()
                                .and_downcast::<gtk::StringObject>()
                                .map(|s| s.string().to_string());
                            sender.input(FontsSettingsInput::TertiaryFontSelected(selected));
                        } @tertiary_font_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "font-preview",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_top: 2,
                    #[watch]
                    set_markup: &font_preview_markup(&model.active_tertiary_font),
                },

                gtk::Separator {},

                // ── Monospace ───────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Monospace",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Monospace font",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Fixed-width family for clipboard entries, detected codes and other tabular text. Empty = the system monospace.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.available_fonts),
                        #[watch]
                        #[block_signal(monospace_font_handler)]
                        set_selected: (0..model.available_fonts.n_items())
                            .find(|&i| model.available_fonts.string(i).as_deref() == Some(model.active_monospace_font.as_str()))
                            .unwrap_or(0),
                        connect_selected_notify[sender] => move |dd| {
                            let selected = dd.selected_item()
                                .and_downcast::<gtk::StringObject>()
                                .map(|s| s.string().to_string());
                            sender.input(FontsSettingsInput::MonospaceFontSelected(selected));
                        } @monospace_font_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "font-preview",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_top: 2,
                    #[watch]
                    set_markup: &font_preview_markup(&model.active_monospace_font),
                },

                gtk::Separator {},

                // ── Size ────────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Size",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Global UI font size",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Multiplier on every font across the shell — bar, menus, dashboard. 1.0 = the designed sizes; raise it on hi-DPI displays. (The Settings panel keeps its own scale in General.)",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.5, 2.0),
                        set_increments: (0.05, 0.1),
                        set_digits: 2,
                        #[watch]
                        #[block_signal(font_scale_handler)]
                        set_value: model.font_scale,
                        connect_value_changed[sender] => move |s| {
                            sender.input(FontsSettingsInput::FontScaleChanged(s.value()));
                        } @font_scale_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Bar pill font size",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Multiplier on the bar pill labels (clock, battery, media, network) on top of the global scale. Useful to nudge just the bar without resizing menus.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.5, 2.0),
                        set_increments: (0.05, 0.1),
                        set_digits: 2,
                        #[watch]
                        #[block_signal(bar_font_scale_handler)]
                        set_value: model.bar_font_scale,
                        connect_value_changed[sender] => move |s| {
                            sender.input(FontsSettingsInput::BarFontScaleChanged(s.value()));
                        } @bar_font_scale_handler,
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut fonts = available_fonts();
        fonts.insert(0, "(none)".to_string());
        let font_refs: Vec<&str> = fonts.iter().map(|s| s.as_str()).collect();
        let available_fonts = gtk::StringList::new(&font_refs);

        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.theme().attributes().font().primary().get();
            sender_clone.input(FontsSettingsInput::PrimaryFontEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.theme().attributes().font().secondary().get();
            sender_clone.input(FontsSettingsInput::SecondaryFontEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.theme().attributes().font().tertiary().get();
            sender_clone.input(FontsSettingsInput::TertiaryFontEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.theme().attributes().font().monospace().get();
            sender_clone.input(FontsSettingsInput::MonospaceFontEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .font_scale()
                .get();
            sender_clone.input(FontsSettingsInput::FontScaleEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .bar_font_scale()
                .get();
            sender_clone.input(FontsSettingsInput::BarFontScaleEffect(v));
        });

        let model = FontsSettingsModel {
            available_fonts,
            active_primary_font: config_manager()
                .config()
                .theme()
                .attributes()
                .font()
                .primary()
                .get_untracked(),
            active_secondary_font: config_manager()
                .config()
                .theme()
                .attributes()
                .font()
                .secondary()
                .get_untracked(),
            active_tertiary_font: config_manager()
                .config()
                .theme()
                .attributes()
                .font()
                .tertiary()
                .get_untracked(),
            active_monospace_font: config_manager()
                .config()
                .theme()
                .attributes()
                .font()
                .monospace()
                .get_untracked(),
            font_scale: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .font_scale()
                .get_untracked(),
            bar_font_scale: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .bar_font_scale()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            FontsSettingsInput::PrimaryFontSelected(font) => {
                config_manager().update_config(|config| match font.as_deref() {
                    Some("(none)") | None => config.theme.attributes.font.primary = String::new(),
                    Some(font) => config.theme.attributes.font.primary = font.to_string(),
                });
            }
            FontsSettingsInput::SecondaryFontSelected(font) => {
                config_manager().update_config(|config| match font.as_deref() {
                    Some("(none)") | None => config.theme.attributes.font.secondary = String::new(),
                    Some(font) => config.theme.attributes.font.secondary = font.to_string(),
                });
            }
            FontsSettingsInput::TertiaryFontSelected(font) => {
                config_manager().update_config(|config| match font.as_deref() {
                    Some("(none)") | None => config.theme.attributes.font.tertiary = String::new(),
                    Some(font) => config.theme.attributes.font.tertiary = font.to_string(),
                });
            }
            FontsSettingsInput::MonospaceFontSelected(font) => {
                config_manager().update_config(|config| match font.as_deref() {
                    Some("(none)") | None => {
                        config.theme.attributes.font.monospace = String::new()
                    }
                    Some(font) => config.theme.attributes.font.monospace = font.to_string(),
                });
            }
            FontsSettingsInput::PrimaryFontEffect(font) => self.active_primary_font = font,
            FontsSettingsInput::SecondaryFontEffect(font) => self.active_secondary_font = font,
            FontsSettingsInput::TertiaryFontEffect(font) => self.active_tertiary_font = font,
            FontsSettingsInput::MonospaceFontEffect(font) => self.active_monospace_font = font,
            FontsSettingsInput::FontScaleChanged(v) => {
                // Snap to the 2-digit display so the reactive effect
                // doesn't fire a fresh write on every fractional tick.
                let snapped = ((v * 100.0).round() / 100.0).clamp(0.5, 2.0);
                config_manager().update_config(|c| {
                    c.theme.attributes.sizing.font_scale = snapped;
                });
            }
            FontsSettingsInput::FontScaleEffect(v) => self.font_scale = v,
            FontsSettingsInput::BarFontScaleChanged(v) => {
                let snapped = ((v * 100.0).round() / 100.0).clamp(0.5, 2.0);
                config_manager().update_config(|c| {
                    c.theme.attributes.sizing.bar_font_scale = snapped;
                });
            }
            FontsSettingsInput::BarFontScaleEffect(v) => self.bar_font_scale = v,
        }
    }
}

/// Pango markup that renders a sample line in `family` so the user sees
/// the typeface, not just its name. Empty family → the inherited default.
fn font_preview_markup(family: &str) -> String {
    let sample = "The quick brown fox jumps over the lazy dog · 0123456789";
    if family.is_empty() {
        format!("<span size=\"large\">{sample}</span>")
    } else {
        format!(
            "<span face=\"{}\" size=\"large\">{sample}</span>",
            gtk::glib::markup_escape_text(family)
        )
    }
}

fn available_fonts() -> Vec<String> {
    let Some(fc) = fontconfig::Fontconfig::new() else {
        return vec![];
    };

    let pattern = fontconfig::Pattern::new(&fc);
    let font_set = fontconfig::list_fonts(&pattern, None);

    let mut families = std::collections::HashSet::new();
    for pattern in font_set.iter() {
        if let Some(family) = pattern.get_string(fontconfig::FC_FAMILY) {
            families.insert(family.to_string());
        }
    }

    let mut families: Vec<_> = families.into_iter().collect();
    families.sort();
    families
}
