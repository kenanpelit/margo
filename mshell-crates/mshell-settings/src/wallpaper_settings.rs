use mshell_cache::wallpaper::{fetch_daily_wallpaper, set_wallpaper};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, WallpaperRotationMode, WallpaperStoreFields,
};
use mshell_config::schema::content_fit::ContentFit;
use mshell_config::schema::wallpaper::ThemeFilterStrength;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, EntryExt, FileExt, OrientableExt, WidgetExt,
};
use relm4::gtk::{gio, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct WallpaperSettingsModel {
    wallpaper_directory: String,
    content_fit: ContentFit,
    apply_theme_filter: bool,
    filter_strength: f64,
    rotation_enabled: bool,
    rotation_interval_minutes: u32,
    rotation_mode: WallpaperRotationMode,
    daily_enabled: bool,
    /// "bing" or "nasa".
    daily_source: String,
    daily_locale: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WallpaperSettingsInput {
    ChangeWallpaperDirectoryClicked,
    ContentFitChanged(ContentFit),
    ThemeFilterChanged(bool),
    FilterStrengthChanged(f64),
    RotationEnabledChanged(bool),
    RotationIntervalChanged(u32),
    RotationModeChanged(WallpaperRotationMode),
    DailyEnabledChanged(bool),
    DailySourceChanged(u32),
    DailyLocaleChanged(String),
    FetchNowClicked,

    WallpaperDirectoryEffect(String),
    ContentFitEffect(ContentFit),
    ThemeFilterEffect(bool),
    FilterStrengthEffect(f64),
    RotationEnabledEffect(bool),
    RotationIntervalEffect(u32),
    RotationModeEffect(WallpaperRotationMode),
    DailyEnabledEffect(bool),
    DailySourceEffect(String),
    DailyLocaleEffect(String),
}

#[derive(Debug)]
pub(crate) enum WallpaperSettingsOutput {}

pub(crate) struct WallpaperSettingsInit {}

#[derive(Debug)]
pub(crate) enum WallpaperSettingsCommandOutput {
    /// A daily-wallpaper fetch finished (off-thread); apply / report on main.
    DailyFetched(Result<PathBuf, String>),
}

#[relm4::component(pub)]
impl Component for WallpaperSettingsModel {
    type CommandOutput = WallpaperSettingsCommandOutput;
    type Input = WallpaperSettingsInput;
    type Output = WallpaperSettingsOutput;
    type Init = WallpaperSettingsInit;

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
                        set_icon_name: Some("preferences-desktop-wallpaper-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Wallpaper",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-tag wallpaper assignment with optional rotation and source directory.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Wallpaper Directory",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        #[watch]
                        set_label: model.wallpaper_directory.as_str(),
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },

                    gtk::Button {
                        set_css_classes: &["ok-button-primary"],
                        set_label: "Change Directory",
                        set_halign: gtk::Align::Start,
                        set_hexpand: false,
                        connect_clicked[sender] => move |_| {
                            sender.input(WallpaperSettingsInput::ChangeWallpaperDirectoryClicked);
                        },
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
                            set_label: "Content fit",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How the wallpaper should fit into the space.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 150,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&ContentFit::display_names())),
                        #[watch]
                        #[block_signal(handler)]
                        set_selected: model.content_fit.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(WallpaperSettingsInput::ContentFitChanged(
                                ContentFit::from_index(dd.selected())
                            ));
                        } @handler,
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
                            set_label: "Theme filter",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Apply a filter to the wallpaper when a static theme is selected. Wallpaper transitions may take longer with this enabled.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(apply_theme_filter_handler)]
                        set_active: model.apply_theme_filter,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(WallpaperSettingsInput::ThemeFilterChanged(enabled));
                            glib::Propagation::Proceed
                        } @apply_theme_filter_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Theme filter strength",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "A higher value will more aggressively apply theme colors.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 1.0),
                        set_increments: (0.1, 0.1),
                        set_digits: 2,
                        #[watch]
                        #[block_signal(filter_strength_handler)]
                        set_value: model.filter_strength,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WallpaperSettingsInput::FilterStrengthChanged(s.value()));
                        } @filter_strength_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Wallpaper Rotation",
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
                            set_label: "Auto-rotate",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Automatically change the wallpaper on a timer.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(rotation_enabled_handler)]
                        set_active: model.rotation_enabled,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(WallpaperSettingsInput::RotationEnabledChanged(enabled));
                            glib::Propagation::Proceed
                        } @rotation_enabled_handler,
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
                            set_label: "Interval (minutes)",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How long to wait between automatic wallpaper changes.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 1440.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(rotation_interval_handler)]
                        set_value: model.rotation_interval_minutes as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WallpaperSettingsInput::RotationIntervalChanged(
                                s.value() as u32,
                            ));
                        } @rotation_interval_handler,
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
                            set_label: "Order",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Walk the directory in order, or pick a random wallpaper each time.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 150,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&WallpaperRotationMode::display_names())),
                        #[watch]
                        #[block_signal(rotation_mode_handler)]
                        set_selected: model.rotation_mode.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(WallpaperSettingsInput::RotationModeChanged(
                                WallpaperRotationMode::from_index(dd.selected())
                            ));
                        } @rotation_mode_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Daily Wallpaper",
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
                            set_label: "Fetch daily",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Download a fresh Bing or NASA image-of-the-day on login and periodically. Saved under ~/Pictures/Wallpapers; downloads older than 5 days are pruned.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(daily_enabled_handler)]
                        set_active: model.daily_enabled,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(WallpaperSettingsInput::DailyEnabledChanged(enabled));
                            glib::Propagation::Proceed
                        } @daily_enabled_handler,
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
                            set_label: "Source",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Bing's daily photo or NASA's Astronomy Picture of the Day.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 150,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&["Bing", "NASA"])),
                        #[watch]
                        #[block_signal(daily_source_handler)]
                        set_selected: if model.daily_source.eq_ignore_ascii_case("nasa") { 1 } else { 0 },
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(WallpaperSettingsInput::DailySourceChanged(dd.selected()));
                        } @daily_source_handler,
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
                            set_label: "Bing region",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Market locale for Bing (e.g. en-US, de-DE). Empty = en-US. Unused for NASA.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 150,
                        set_placeholder_text: Some("en-US"),
                        #[watch]
                        #[block_signal(daily_locale_handler)]
                        set_text: &model.daily_locale,
                        connect_changed[sender] => move |e| {
                            sender.input(WallpaperSettingsInput::DailyLocaleChanged(e.text().to_string()));
                        } @daily_locale_handler,
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_halign: gtk::Align::Start,
                    set_label: "Fetch today's wallpaper now",
                    connect_clicked[sender] => move |_| {
                        sender.input(WallpaperSettingsInput::FetchNowClicked);
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let wallpaper_dir = config_manager().config().wallpaper().wallpaper_dir().get();
            sender_clone.input(WallpaperSettingsInput::WallpaperDirectoryEffect(
                wallpaper_dir,
            ));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager().config().wallpaper().content_fit().get();
            sender_clone.input(WallpaperSettingsInput::ContentFitEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .wallpaper()
                .apply_theme_filter()
                .get();
            sender_clone.input(WallpaperSettingsInput::ThemeFilterEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .wallpaper()
                .theme_filter_strength()
                .get();
            sender_clone.input(WallpaperSettingsInput::FilterStrengthEffect(value.get()));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .wallpaper()
                .rotation_enabled()
                .get();
            sender_clone.input(WallpaperSettingsInput::RotationEnabledEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .wallpaper()
                .rotation_interval_minutes()
                .get();
            sender_clone.input(WallpaperSettingsInput::RotationIntervalEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager().config().wallpaper().rotation_mode().get();
            sender_clone.input(WallpaperSettingsInput::RotationModeEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .wallpaper()
                .daily_wallpaper_enabled()
                .get();
            sender_clone.input(WallpaperSettingsInput::DailyEnabledEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .wallpaper()
                .daily_wallpaper_source()
                .get();
            sender_clone.input(WallpaperSettingsInput::DailySourceEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .wallpaper()
                .daily_wallpaper_locale()
                .get();
            sender_clone.input(WallpaperSettingsInput::DailyLocaleEffect(value));
        });

        let model = WallpaperSettingsModel {
            wallpaper_directory: "".to_string(),
            content_fit: config_manager()
                .config()
                .wallpaper()
                .content_fit()
                .get_untracked(),
            apply_theme_filter: config_manager()
                .config()
                .wallpaper()
                .apply_theme_filter()
                .get_untracked(),
            filter_strength: config_manager()
                .config()
                .wallpaper()
                .theme_filter_strength()
                .get_untracked()
                .get(),
            rotation_enabled: config_manager()
                .config()
                .wallpaper()
                .rotation_enabled()
                .get_untracked(),
            rotation_interval_minutes: config_manager()
                .config()
                .wallpaper()
                .rotation_interval_minutes()
                .get_untracked(),
            rotation_mode: config_manager()
                .config()
                .wallpaper()
                .rotation_mode()
                .get_untracked(),
            daily_enabled: config_manager()
                .config()
                .wallpaper()
                .daily_wallpaper_enabled()
                .get_untracked(),
            daily_source: config_manager()
                .config()
                .wallpaper()
                .daily_wallpaper_source()
                .get_untracked(),
            daily_locale: config_manager()
                .config()
                .wallpaper()
                .daily_wallpaper_locale()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();

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
            WallpaperSettingsInput::ChangeWallpaperDirectoryClicked => {
                let dialog = gtk::FileDialog::builder()
                    .title("Choose Wallpaper Directory")
                    .modal(true)
                    .build();

                dialog.select_folder(gtk::Window::NONE, gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        config_manager().update_config(|config| {
                            config.wallpaper.wallpaper_dir = path.to_string_lossy().to_string();
                        });
                    }
                });
            }
            WallpaperSettingsInput::ContentFitChanged(content_fit) => {
                config_manager().update_config(|config| {
                    config.wallpaper.content_fit = content_fit;
                });
            }
            WallpaperSettingsInput::ThemeFilterChanged(apply) => {
                config_manager().update_config(|config| {
                    config.wallpaper.apply_theme_filter = apply;
                })
            }
            WallpaperSettingsInput::FilterStrengthChanged(strength) => config_manager()
                .update_config(|config| {
                    config.wallpaper.theme_filter_strength = ThemeFilterStrength::new(strength)
                }),
            WallpaperSettingsInput::RotationEnabledChanged(enabled) => {
                config_manager().update_config(|config| {
                    config.wallpaper.rotation_enabled = enabled;
                });
            }
            WallpaperSettingsInput::RotationIntervalChanged(minutes) => {
                config_manager().update_config(|config| {
                    config.wallpaper.rotation_interval_minutes = minutes;
                });
            }
            WallpaperSettingsInput::RotationModeChanged(mode) => {
                config_manager().update_config(|config| {
                    config.wallpaper.rotation_mode = mode;
                });
            }

            WallpaperSettingsInput::WallpaperDirectoryEffect(path) => {
                self.wallpaper_directory = path;
            }
            WallpaperSettingsInput::ContentFitEffect(content_fit) => {
                self.content_fit = content_fit;
            }
            WallpaperSettingsInput::ThemeFilterEffect(filter) => {
                self.apply_theme_filter = filter;
            }
            WallpaperSettingsInput::FilterStrengthEffect(filter) => {
                self.filter_strength = filter;
            }
            WallpaperSettingsInput::RotationEnabledEffect(enabled) => {
                self.rotation_enabled = enabled;
            }
            WallpaperSettingsInput::RotationIntervalEffect(minutes) => {
                self.rotation_interval_minutes = minutes;
            }
            WallpaperSettingsInput::RotationModeEffect(mode) => {
                self.rotation_mode = mode;
            }
            WallpaperSettingsInput::DailyEnabledChanged(enabled) => {
                config_manager().update_config(|c| c.wallpaper.daily_wallpaper_enabled = enabled);
            }
            WallpaperSettingsInput::DailySourceChanged(idx) => {
                let src = if idx == 1 { "nasa" } else { "bing" }.to_string();
                config_manager()
                    .update_config(|c| c.wallpaper.daily_wallpaper_source = src.clone());
            }
            WallpaperSettingsInput::DailyLocaleChanged(locale) => {
                config_manager()
                    .update_config(|c| c.wallpaper.daily_wallpaper_locale = locale.clone());
            }
            WallpaperSettingsInput::FetchNowClicked => {
                let source = self.daily_source.clone();
                let locale = self.daily_locale.clone();
                // Download off the UI thread; apply on the main thread in update_cmd
                // (set_wallpaper touches the reactive store).
                sender.oneshot_command(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        fetch_daily_wallpaper(&source, &locale)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("join error: {e}")));
                    WallpaperSettingsCommandOutput::DailyFetched(result)
                });
            }
            WallpaperSettingsInput::DailyEnabledEffect(v) => self.daily_enabled = v,
            WallpaperSettingsInput::DailySourceEffect(s) => self.daily_source = s,
            WallpaperSettingsInput::DailyLocaleEffect(l) => self.daily_locale = l,
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let WallpaperSettingsCommandOutput::DailyFetched(result) = message;
        match result {
            Ok(path) => {
                // On the main thread → safe to touch the reactive config store.
                set_wallpaper(&path);
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("wallpaper")
                    .to_string();
                notify("Daily wallpaper", &format!("Applied {name}"));
            }
            Err(e) => notify("Daily wallpaper failed", &e),
        }
    }
}

/// Fire-and-forget desktop toast (reaped via the async runtime).
fn notify(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    relm4::spawn(async move {
        let _ = tokio::process::Command::new("notify-send")
            .args([
                "-a",
                "mshell",
                "-i",
                "preferences-desktop-wallpaper-symbolic",
                &summary,
                &body,
            ])
            .status()
            .await;
    });
}
