use mshell_common::scoped_effects::EffectScope;
use mshell_common::text_entry_dialog::{
    TextEntryDialogInit, TextEntryDialogModel, TextEntryDialogOutput,
};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, GeneralStoreFields, SizingStoreFields, ThemeAttributesStoreFields,
    ThemeStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, CastNone, ListModelExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, Controller, gtk};

pub(crate) struct GeneralSettingsModel {
    active_profile: Option<String>,
    available_profiles: gtk::StringList,
    new_profile_dialog: Option<Controller<TextEntryDialogModel>>,
    time_format_24_h: bool,
    show_screen_corners: bool,
    screen_corner_radius: i32,
    network_osd_enabled: bool,
    /// Settings-panel font-size multiplier. Persisted to
    /// `theme.attributes.sizing.settings_font_scale`. Drives the
    /// `--font-scale-settings` CSS variable that every `.settings-*`
    /// `font-size` declaration in `_settings.scss` multiplies
    /// against.
    settings_font_scale: f64,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum GeneralSettingsInput {
    TimeFormat24HToggled(bool),
    TimeFormat24HEffect(bool),
    ActiveProfileEffect(Option<String>),
    AvailableProfilesEffect(Vec<String>),
    NewProfileClicked,
    ActiveProfileSelected(Option<String>),
    NewProfileNameChosen(String),
    DialogCanceled,
    DeleteProfileClicked,
    ShowScreenCornersToggled(bool),
    ShowScreenCornersEffect(bool),
    ScreenCornerRadiusChanged(i32),
    ScreenCornerRadiusEffect(i32),
    NetworkOsdEnabledToggled(bool),
    NetworkOsdEnabledEffect(bool),
    SettingsFontScaleChanged(f64),
    SettingsFontScaleEffect(f64),
}

#[derive(Debug)]
pub(crate) enum GeneralSettingsOutput {}

pub(crate) struct GeneralSettingsInit {}

#[derive(Debug)]
pub(crate) enum GeneralSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for GeneralSettingsModel {
    type CommandOutput = GeneralSettingsCommandOutput;
    type Input = GeneralSettingsInput;
    type Output = GeneralSettingsOutput;
    type Init = GeneralSettingsInit;

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
                        set_icon_name: Some("preferences-system-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "General",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "App-wide preferences — profile, scaling, accent, behaviour.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Profile",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    #[name = "profile_dropdown"]
                    gtk::DropDown {
                        set_hexpand: true,
                        set_model: Some(&model.available_profiles),
                        set_selected: (0..model.available_profiles.n_items())
                            .find(|&i| model.available_profiles.string(i).as_deref() == model.active_profile.as_deref())
                            .unwrap_or(0),
                        connect_selected_notify[sender] => move |dd| {
                            let selected = dd.selected_item()
                                .and_downcast::<gtk::StringObject>()
                                .map(|s| s.string().to_string());
                            sender.input(GeneralSettingsInput::ActiveProfileSelected(selected));
                        },
                    },

                    gtk::Button {
                        set_css_classes: &["label-medium", "ok-button-primary"],
                        set_label: "New Profile",
                        set_halign: gtk::Align::Start,
                        set_hexpand: false,
                        connect_clicked[sender] => move |_| {
                            sender.input(GeneralSettingsInput::NewProfileClicked);
                        },
                    },

                    gtk::Button {
                        set_css_classes: &["label-medium", "ok-button-primary"],
                        #[watch]
                        set_sensitive: model.available_profiles.n_items() > 1,
                        set_label: "Delete Profile",
                        set_halign: gtk::Align::Start,
                        set_hexpand: false,
                        connect_clicked[sender] => move |_| {
                            sender.input(GeneralSettingsInput::DeleteProfileClicked);
                        },
                    },
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Clock",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "24 hour time format",
                        set_hexpand: true,
                    },

                    gtk::Switch {
                        #[watch]
                        #[block_signal(time_format_handler)]
                        set_active: model.time_format_24_h,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(GeneralSettingsInput::TimeFormat24HToggled(enabled));
                            glib::Propagation::Proceed
                        } @time_format_handler,
                    }
                },

                // ── Screen corners ─────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Rounded screen corners",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Mask each monitor's four corners so the screen reads as having rounded edges. Click-through. Off by default — the bar already paints its own rounded corners at the CSS frame-border-radius (24 px). Enable only when you also want the area *outside* the bar curved (e.g. bezel-less monitor), and set the radius below to match the frame border-radius so the two arcs line up.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(screen_corners_handler)]
                        set_active: model.show_screen_corners,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(GeneralSettingsInput::ShowScreenCornersToggled(v));
                            glib::Propagation::Proceed
                        } @screen_corners_handler,
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
                            set_label: "Corner radius (px)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Radius (px) of the black overlay mask that rounds the physical SCREEN corners — only when 'Rounded screen corners' above is on. This does NOT change widget, button, card or menu corners (those follow the fixed design scale, not a setting). Applies after restarting mshell (systemctl --user restart mshell) or reconnecting the monitor.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 64.0),
                        set_increments: (1.0, 4.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(corner_radius_handler)]
                        set_value: model.screen_corner_radius as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(GeneralSettingsInput::ScreenCornerRadiusChanged(s.value() as i32));
                        } @corner_radius_handler,
                    },
                },

                // ── Settings font scale ────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Settings font scale",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Multiplier applied to every font-size inside the Settings panel. 1.0 keeps the +1pt-bumped defaults (~15.5 px); set 1.1 for ~17 px on hi-DPI displays, 0.9 to shrink for tight screens. Persists to `theme.attributes.sizing.settings_font_scale`.",
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
                        #[block_signal(settings_font_scale_handler)]
                        set_value: model.settings_font_scale,
                        connect_value_changed[sender] => move |s| {
                            sender.input(GeneralSettingsInput::SettingsFontScaleChanged(s.value()));
                        } @settings_font_scale_handler,
                    },
                },

                // ── Network OSD ────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Network change OSD",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Flash a 2-second popup at the bottom of the screen whenever the primary connection changes — \"Connected: <SSID>\", \"Ethernet connected\", \"Disconnected\". Fires only on transitions. Off by default because NetworkManager often shows the same information as a desktop notification — turn this on if you don't have NM notifications, or just prefer the in-shell OSD.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(network_osd_handler)]
                        set_active: model.network_osd_enabled,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(GeneralSettingsInput::NetworkOsdEnabledToggled(v));
                            glib::Propagation::Proceed
                        } @network_osd_handler,
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
            let active_profile = config_manager().active_profile().get();
            sender_clone.input(GeneralSettingsInput::ActiveProfileEffect(active_profile));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let available_profiles = config_manager().available_profiles().get();
            sender_clone.input(GeneralSettingsInput::AvailableProfilesEffect(
                available_profiles,
            ));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let format = config.general().clock_format_24_h().get();
            sender_clone.input(GeneralSettingsInput::TimeFormat24HEffect(format));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .general()
                .show_screen_corners()
                .get();
            sender_clone.input(GeneralSettingsInput::ShowScreenCornersEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .general()
                .screen_corner_radius()
                .get();
            sender_clone.input(GeneralSettingsInput::ScreenCornerRadiusEffect(v as i32));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .general()
                .network_osd_enabled()
                .get();
            sender_clone.input(GeneralSettingsInput::NetworkOsdEnabledEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .settings_font_scale()
                .get();
            sender_clone.input(GeneralSettingsInput::SettingsFontScaleEffect(v));
        });

        let model = GeneralSettingsModel {
            active_profile: None,
            available_profiles: gtk::StringList::new(&[]),
            new_profile_dialog: None,
            time_format_24_h: false,
            show_screen_corners: config_manager()
                .config()
                .general()
                .show_screen_corners()
                .get_untracked(),
            screen_corner_radius: config_manager()
                .config()
                .general()
                .screen_corner_radius()
                .get_untracked() as i32,
            network_osd_enabled: config_manager()
                .config()
                .general()
                .network_osd_enabled()
                .get_untracked(),
            settings_font_scale: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .settings_font_scale()
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
            GeneralSettingsInput::ActiveProfileSelected(selected_profile) => {
                config_manager().set_active_profile(selected_profile);
            }
            GeneralSettingsInput::ActiveProfileEffect(profile) => {
                self.active_profile = profile;
                let idx = (0..self.available_profiles.n_items())
                    .find(|&i| {
                        self.available_profiles.string(i).as_deref()
                            == self.active_profile.as_deref()
                    })
                    .unwrap_or(0);
                widgets.profile_dropdown.set_selected(idx);
            }
            GeneralSettingsInput::AvailableProfilesEffect(profiles) => {
                // Rebuild the list in-place
                while self.available_profiles.n_items() > 0 {
                    self.available_profiles.remove(0);
                }
                for p in &profiles {
                    self.available_profiles.append(p);
                }
                // Re-sync selected index
                let idx = (0..self.available_profiles.n_items())
                    .find(|&i| {
                        self.available_profiles.string(i).as_deref()
                            == self.active_profile.as_deref()
                    })
                    .unwrap_or(0);
                widgets.profile_dropdown.set_selected(idx);
            }
            GeneralSettingsInput::NewProfileClicked => {
                let dialog = TextEntryDialogModel::builder()
                    .launch(TextEntryDialogInit {
                        message: "Enter new profile name".to_string(),
                        negative_label: "Cancel".to_string(),
                        positive_label: "Create".to_string(),
                        entry_placeholder: "Profile name".to_string(),
                        entry2_placeholder: String::new(),
                        show_second_entry: false,
                    })
                    .forward(sender.input_sender(), |msg| match msg {
                        TextEntryDialogOutput::PositiveSelected(name, _) => {
                            GeneralSettingsInput::NewProfileNameChosen(name)
                        }
                        TextEntryDialogOutput::NegativeSelected => {
                            GeneralSettingsInput::DialogCanceled
                        }
                    });

                self.new_profile_dialog = Some(dialog);
            }
            GeneralSettingsInput::NewProfileNameChosen(name) => {
                let _ = config_manager().create_profile(name.as_str());
            }
            GeneralSettingsInput::DialogCanceled => {
                // do nothing
            }
            GeneralSettingsInput::DeleteProfileClicked => {
                if let Some(active) = &self.active_profile {
                    let _ = config_manager().delete_profile(active.as_str());
                }
            }
            GeneralSettingsInput::TimeFormat24HToggled(format) => {
                let config_manager = config_manager();
                config_manager.update_config(|config| {
                    config.general.clock_format_24_h = format;
                });
            }
            GeneralSettingsInput::TimeFormat24HEffect(format) => {
                self.time_format_24_h = format;
            }
            GeneralSettingsInput::ShowScreenCornersToggled(v) => {
                config_manager().update_config(|c| {
                    c.general.show_screen_corners = v;
                });
            }
            GeneralSettingsInput::ShowScreenCornersEffect(v) => {
                self.show_screen_corners = v;
            }
            GeneralSettingsInput::ScreenCornerRadiusChanged(r) => {
                let clamped = r.clamp(0, 64) as u32;
                config_manager().update_config(|c| {
                    c.general.screen_corner_radius = clamped;
                });
            }
            GeneralSettingsInput::ScreenCornerRadiusEffect(r) => {
                self.screen_corner_radius = r;
            }
            GeneralSettingsInput::NetworkOsdEnabledToggled(v) => {
                config_manager().update_config(|c| {
                    c.general.network_osd_enabled = v;
                });
            }
            GeneralSettingsInput::NetworkOsdEnabledEffect(v) => {
                self.network_osd_enabled = v;
            }
            GeneralSettingsInput::SettingsFontScaleChanged(v) => {
                // Snap to the SpinButton's 2-digit display so the
                // reactive effect doesn't fire a fresh write on
                // every fractional tick from the GTK control.
                let snapped = (v * 100.0).round() / 100.0;
                let clamped = snapped.clamp(0.5, 2.0);
                config_manager().update_config(|c| {
                    c.theme.attributes.sizing.settings_font_scale = clamped;
                });
            }
            GeneralSettingsInput::SettingsFontScaleEffect(v) => {
                self.settings_font_scale = v;
            }
        }

        self.update_view(widgets, sender);
    }
}
