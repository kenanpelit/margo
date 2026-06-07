//! Settings → Setup — an in-panel companion to the in-shell wizard menu.
//!
//! Consolidates the wizard's most-common knobs (profile, theme preset,
//! colour mode, font size, clock, wallpaper) into one page so the user
//! can do a quick setup without hunting across the Theme / Fonts /
//! Wallpaper / General pages. Everything here writes the LIVE config via
//! `config_manager` (the active profile), so it stays in sync with those
//! dedicated pages. Compositor-side bits the live config can't touch
//! (keyboard layout, monitor arrangement) are left to the full wizard,
//! reachable from the button at the top.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, GeneralStoreFields, MatugenStoreFields, SizingStoreFields,
    ThemeAttributesStoreFields, ThemeStoreFields, WallpaperStoreFields,
};
use mshell_config::schema::themes::{MatugenMode, Themes};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, CastNone, FileExt, ListModelExt, OrientableExt, WidgetExt,
};
use relm4::gtk::{gio, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Curated theme presets (the full 50+ catalogue lives on the Theme
/// page). `Wallpaper` is the matugen-from-wallpaper Material You mode.
const SETUP_THEMES: &[(Themes, &str)] = &[
    (Themes::Wallpaper, "Wallpaper (Material You)"),
    (Themes::Default, "Default"),
    (Themes::Margo, "Margo"),
    (Themes::Dracula, "Dracula"),
    (Themes::CatppuccinMocha, "Catppuccin Mocha"),
    (Themes::GruvboxDarkMedium, "Gruvbox Dark"),
    (Themes::KanagawaWave, "Kanagawa Wave"),
    (Themes::Cyberpunk, "Cyberpunk"),
];

/// Global UI font-scale presets (multiplies every `--font-*` token).
const SETUP_FONT_SCALES: &[(f64, &str)] = &[
    (0.9, "Compact (90%)"),
    (1.0, "Default (100%)"),
    (1.1, "Large (110%)"),
    (1.25, "Larger (125%)"),
];

pub(crate) struct SetupSettingsModel {
    available_profiles: gtk::StringList,
    active_profile: Option<String>,
    theme_scheme: Themes,
    matugen_mode: MatugenMode,
    font_scale: f64,
    clock_24h: bool,
    wallpaper_dir: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum SetupSettingsInput {
    ProfileSelected(Option<String>),
    /// Snapshot the live config into the `active` profile and select it.
    SaveAsActive,
    ThemeSelected(Themes),
    ModeSelected(MatugenMode),
    FontScaleSelected(f64),
    Clock24hToggled(bool),
    BrowseWallpaper,

    ActiveProfileEffect(Option<String>),
    AvailableProfilesEffect(Vec<String>),
    ThemeEffect(Themes),
    ModeEffect(MatugenMode),
    FontScaleEffect(f64),
    ClockEffect(bool),
    WallpaperDirEffect(String),
}

#[derive(Debug)]
pub(crate) enum SetupSettingsOutput {}

pub(crate) struct SetupSettingsInit {}

#[derive(Debug)]
pub(crate) enum SetupSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for SetupSettingsModel {
    type CommandOutput = SetupSettingsCommandOutput;
    type Input = SetupSettingsInput;
    type Output = SetupSettingsOutput;
    type Init = SetupSettingsInit;

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
                        set_icon_name: Some("emblem-system-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Setup",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Quick setup in one place — pick a profile, then dial in theme, size, clock and wallpaper. Changes apply live to the active profile.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // Guided 5-step setup — opens as an in-shell menu (a
                // layer-shell surface like every other menu), not a window.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Guided setup",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Walk the five steps — theme, fonts, keyboard, wallpaper — in a menu.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-primary"],
                        set_label: "Run setup wizard",
                        set_valign: gtk::Align::Center,
                        connect_clicked => move |_| {
                            crate::open_wizard();
                        },
                    },
                },

                gtk::Separator {},

                // ── Profile ─────────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Profile",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    #[name = "profile_dropdown"]
                    gtk::DropDown {
                        set_hexpand: true,
                        set_model: Some(&model.available_profiles),
                        #[watch]
                        set_selected: (0..model.available_profiles.n_items())
                            .find(|&i| model.available_profiles.string(i).as_deref() == model.active_profile.as_deref())
                            .unwrap_or(0),
                        connect_selected_notify[sender] => move |dd| {
                            let selected = dd.selected_item()
                                .and_downcast::<gtk::StringObject>()
                                .map(|s| s.string().to_string());
                            sender.input(SetupSettingsInput::ProfileSelected(selected));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium",
                            set_label: "Keep current setup",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Snapshot the live configuration into a named \"active\" profile and switch to it — carry your current setup forward as an editable base, no wizard needed.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Button {
                        set_css_classes: &["ok-button-primary"],
                        set_label: "Save as \"active\"",
                        set_valign: gtk::Align::Center,
                        connect_clicked[sender] => move |_| {
                            sender.input(SetupSettingsInput::SaveAsActive);
                        },
                    },
                },

                gtk::Separator {},

                // ── Appearance ──────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Appearance",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 16,
                    gtk::Label {
                        add_css_class: "label-medium",
                        set_label: "Theme",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                    },
                    #[name = "theme_dropdown"]
                    gtk::DropDown {
                        set_width_request: 220,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(
                            &SETUP_THEMES.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
                        )),
                        #[watch]
                        set_selected: SETUP_THEMES
                            .iter()
                            .position(|(t, _)| *t == model.theme_scheme)
                            .unwrap_or(0) as u32,
                        connect_selected_notify[sender] => move |dd| {
                            if let Some((t, _)) = SETUP_THEMES.get(dd.selected() as usize) {
                                sender.input(SetupSettingsInput::ThemeSelected(*t));
                            }
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 16,
                    gtk::Label {
                        add_css_class: "label-medium",
                        set_label: "Color mode",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                    },
                    #[name = "mode_dropdown"]
                    gtk::DropDown {
                        set_width_request: 220,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&["Dark", "Light"])),
                        #[watch]
                        set_selected: match model.matugen_mode {
                            MatugenMode::Dark => 0,
                            MatugenMode::Light => 1,
                        },
                        connect_selected_notify[sender] => move |dd| {
                            let mode = if dd.selected() == 0 {
                                MatugenMode::Dark
                            } else {
                                MatugenMode::Light
                            };
                            sender.input(SetupSettingsInput::ModeSelected(mode));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 16,
                    gtk::Label {
                        add_css_class: "label-medium",
                        set_label: "Font size",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                    },
                    #[name = "font_scale_dropdown"]
                    gtk::DropDown {
                        set_width_request: 220,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(
                            &SETUP_FONT_SCALES.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
                        )),
                        #[watch]
                        set_selected: SETUP_FONT_SCALES
                            .iter()
                            .position(|(s, _)| (*s - model.font_scale).abs() < 0.001)
                            .unwrap_or(1) as u32,
                        connect_selected_notify[sender] => move |dd| {
                            if let Some((s, _)) = SETUP_FONT_SCALES.get(dd.selected() as usize) {
                                sender.input(SetupSettingsInput::FontScaleSelected(*s));
                            }
                        },
                    },
                },

                gtk::Separator {},

                // ── Behaviour ───────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Behaviour",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 16,
                    gtk::Label {
                        add_css_class: "label-medium",
                        set_label: "24-hour clock",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(clock_handler)]
                        set_active: model.clock_24h,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(SetupSettingsInput::Clock24hToggled(v));
                            glib::Propagation::Proceed
                        } @clock_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium",
                            set_label: "Wallpaper directory",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            #[watch]
                            set_label: &model.wallpaper_dir,
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Browse…",
                        set_valign: gtk::Align::Center,
                        connect_clicked[sender] => move |_| {
                            sender.input(SetupSettingsInput::BrowseWallpaper);
                        },
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
        let mut effects = EffectScope::new();

        let s = sender.clone();
        effects.push(move |_| {
            s.input(SetupSettingsInput::ActiveProfileEffect(
                config_manager().active_profile().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(SetupSettingsInput::AvailableProfilesEffect(
                config_manager().available_profiles().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(SetupSettingsInput::ThemeEffect(
                config_manager().config().theme().theme().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(SetupSettingsInput::ModeEffect(
                config_manager().config().theme().matugen().mode().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(SetupSettingsInput::FontScaleEffect(
                config_manager()
                    .config()
                    .theme()
                    .attributes()
                    .sizing()
                    .font_scale()
                    .get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(SetupSettingsInput::ClockEffect(
                config_manager()
                    .config()
                    .general()
                    .clock_format_24_h()
                    .get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(SetupSettingsInput::WallpaperDirEffect(
                config_manager().config().wallpaper().wallpaper_dir().get(),
            ));
        });

        let model = SetupSettingsModel {
            available_profiles: gtk::StringList::new(&[]),
            active_profile: config_manager().active_profile().get_untracked(),
            theme_scheme: config_manager().config().theme().theme().get_untracked(),
            matugen_mode: config_manager()
                .config()
                .theme()
                .matugen()
                .mode()
                .get_untracked(),
            font_scale: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .font_scale()
                .get_untracked(),
            clock_24h: config_manager()
                .config()
                .general()
                .clock_format_24_h()
                .get_untracked(),
            wallpaper_dir: config_manager()
                .config()
                .wallpaper()
                .wallpaper_dir()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();
        let _ = root;
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
            SetupSettingsInput::ProfileSelected(name) => {
                config_manager().set_active_profile(name);
            }
            SetupSettingsInput::SaveAsActive => {
                // Snapshot the live config into the "active" profile +
                // switch to it. The reactive available_profiles / active
                // stores update, so the dropdown refreshes and selects it.
                if let Err(e) = config_manager().snapshot_active_as("active") {
                    tracing::warn!(error = ?e, "setup: failed to snapshot 'active' profile");
                }
            }
            SetupSettingsInput::ThemeSelected(theme) => {
                config_manager().update_config(|c| c.theme.theme = theme);
            }
            SetupSettingsInput::ModeSelected(mode) => {
                config_manager().update_config(|c| c.theme.matugen.mode = mode);
            }
            SetupSettingsInput::FontScaleSelected(scale) => {
                config_manager().update_config(|c| c.theme.attributes.sizing.font_scale = scale);
            }
            SetupSettingsInput::Clock24hToggled(v) => {
                config_manager().update_config(|c| c.general.clock_format_24_h = v);
            }
            SetupSettingsInput::BrowseWallpaper => {
                let dialog = gtk::FileDialog::builder()
                    .title("Choose Wallpaper Directory")
                    .modal(true)
                    .build();
                dialog.select_folder(gtk::Window::NONE, gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        config_manager().update_config(|c| {
                            c.wallpaper.wallpaper_dir = path.to_string_lossy().to_string();
                        });
                    }
                });
            }

            SetupSettingsInput::ActiveProfileEffect(p) => self.active_profile = p,
            SetupSettingsInput::AvailableProfilesEffect(list) => {
                // Mutate the existing StringList in place — the DropDown
                // holds a reference to THIS object, so replacing it (as we
                // did before) left the dropdown bound to the original empty
                // list and nothing ever showed.
                while self.available_profiles.n_items() > 0 {
                    self.available_profiles.remove(0);
                }
                for p in &list {
                    self.available_profiles.append(p);
                }
            }
            SetupSettingsInput::ThemeEffect(t) => self.theme_scheme = t,
            SetupSettingsInput::ModeEffect(m) => self.matugen_mode = m,
            SetupSettingsInput::FontScaleEffect(s) => self.font_scale = s,
            SetupSettingsInput::ClockEffect(v) => self.clock_24h = v,
            SetupSettingsInput::WallpaperDirEffect(d) => self.wallpaper_dir = d,
        }

        self.update_view(widgets, sender);
    }
}
