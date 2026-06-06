//! Weather settings page — location query (coordinates or city /
//! district name) + temperature unit. Lives in its own sidebar entry
//! now that Weather is a standalone bar pill + menu; the config keys are
//! still `general.weather_location_query` and `general.temperature_unit`
//! (no schema change — this page just reads/writes them in one focused
//! place instead of being buried in the General page).

use mshell_common::scoped_effects::EffectScope;
use mshell_common::text_entry_dialog::{
    TextEntryDialogInit, TextEntryDialogModel, TextEntryDialogOutput,
};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields, SavedLocation};
use mshell_config::schema::location_query::{LocationQueryConfig, LocationQueryType, OrdF64};
use mshell_config::schema::temperature::TemperatureUnitConfig;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, Controller, gtk};

pub(crate) struct WeatherSettingsModel {
    location_query_types: gtk::StringList,
    active_location_query_type: LocationQueryType,
    location_lat_lon: String,
    location_city: String,
    lat_lon_dialog: Option<Controller<TextEntryDialogModel>>,
    city_dialog: Option<Controller<TextEntryDialogModel>>,
    weather_unit_types: gtk::StringList,
    active_weather_unit_type: TemperatureUnitConfig,
    poll_minutes: u32,
    retry_minutes: u32,
    /// Bookmarked locations the weather-menu switcher flips between.
    saved_locations: Vec<SavedLocation>,
    active_query: LocationQueryConfig,
    save_dialog: Option<Controller<TextEntryDialogModel>>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WeatherSettingsInput {
    LocationQueryTypeSelected(LocationQueryType),
    LocationQueryEffect(LocationQueryConfig),
    ChangeCoordinatesClicked,
    ChangeCityClicked,
    LatLonChosen(String, String),
    CityChosen(String, String),
    WeatherUnitTypeSelected(TemperatureUnitConfig),
    WeatherUnitTypeEffect(TemperatureUnitConfig),
    SetPollMinutes(u32),
    SetRetryMinutes(u32),
    DialogCanceled,
    /// Saved-location bookmarks changed in config — refresh the list.
    SavedLocationsEffect(Vec<SavedLocation>),
    /// "Save current location as…" pressed — prompt for a name.
    SaveCurrentClicked,
    /// Name entered: bookmark the current active query under it.
    SaveNameChosen(String),
    /// Remove the bookmark at this list index.
    RemoveLocation(usize),
}

#[derive(Debug)]
pub(crate) enum WeatherSettingsOutput {}

pub(crate) struct WeatherSettingsInit {}

#[derive(Debug)]
pub(crate) enum WeatherSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for WeatherSettingsModel {
    type CommandOutput = WeatherSettingsCommandOutput;
    type Input = WeatherSettingsInput;
    type Output = WeatherSettingsOutput;
    type Init = WeatherSettingsInit;

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
                        set_icon_name: Some("weather-few-clouds-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_halign: gtk::Align::Start,
                            set_label: "Weather",
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_halign: gtk::Align::Start,
                            set_label: "Location and units for the weather pill, menu and dashboard.",
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
                            set_label: "Location Query Type",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How to determine the location.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "location_query_type_dropdown"]
                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.location_query_types),
                        #[watch]
                        #[block_signal(lqt_handler)]
                        set_selected: LocationQueryType::all()
                            .iter()
                            .position(|k| k == &model.active_location_query_type)
                            .unwrap_or(0) as u32,
                        connect_selected_notify[sender] => move |dd| {
                            let idx = dd.selected() as usize;
                            if let Some(kind) = LocationQueryType::all().get(idx) {
                                sender.input(WeatherSettingsInput::LocationQueryTypeSelected(*kind));
                            }
                        } @lqt_handler,
                    },
                },

                gtk::Box {
                    #[watch]
                    set_visible: model.active_location_query_type == LocationQueryType::Coordinates,
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Lat Lon",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            #[watch]
                            set_label: model.location_lat_lon.as_str(),
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Button {
                        set_css_classes: &["ok-button-primary", "ok-button-cell"],
                        set_label: "Change Coordinates",
                        set_halign: gtk::Align::Start,
                        set_hexpand: false,
                        connect_clicked[sender] => move |_| {
                            sender.input(WeatherSettingsInput::ChangeCoordinatesClicked);
                        },
                    },
                },

                gtk::Box {
                    #[watch]
                    set_visible: model.active_location_query_type == LocationQueryType::City,
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "City / district, Country",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            #[watch]
                            set_label: model.location_city.as_str(),
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Button {
                        set_css_classes: &["ok-button-primary", "ok-button-cell"],
                        set_label: "Change Location",
                        set_halign: gtk::Align::Start,
                        set_hexpand: false,
                        connect_clicked[sender] => move |_| {
                            sender.input(WeatherSettingsInput::ChangeCityClicked);
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
                            set_label: "Weather units",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Units in which weather information should be displayed.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "weather_unit_type_dropdown"]
                    gtk::DropDown {
                        set_width_request: 200,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.weather_unit_types),
                        #[watch]
                        #[block_signal(unit_handler)]
                        set_selected: TemperatureUnitConfig::all()
                            .iter()
                            .position(|k| k == &model.active_weather_unit_type)
                            .unwrap_or(0) as u32,
                        connect_selected_notify[sender] => move |dd| {
                            let idx = dd.selected() as usize;
                            if let Some(kind) = TemperatureUnitConfig::all().get(idx) {
                                sender.input(WeatherSettingsInput::WeatherUnitTypeSelected(*kind));
                            }
                        } @unit_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Update interval",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Minutes between weather refreshes. On a failed fetch the shell retries faster until it recovers.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "poll_minutes_spin"]
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 180.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        set_value: model.poll_minutes as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WeatherSettingsInput::SetPollMinutes(s.value() as u32));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Retry interval on failure",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Minutes between retries while a fetch keeps failing (faster fallback). Returns to the normal interval once it recovers.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "retry_minutes_spin"]
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 60.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        set_value: model.retry_minutes as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WeatherSettingsInput::SetRetryMinutes(s.value() as u32));
                        },
                    },
                },

                // ── Saved locations: bookmark the current location under a
                //    name; the weather menu's switcher flips between them. ──
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Saved Locations",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Bookmark the current location, then switch between bookmarks from the weather menu.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Button {
                        set_css_classes: &["ok-button-primary", "ok-button-cell"],
                        set_label: "Save current as…",
                        set_halign: gtk::Align::Start,
                        set_valign: gtk::Align::Center,
                        set_hexpand: false,
                        connect_clicked[sender] => move |_| {
                            sender.input(WeatherSettingsInput::SaveCurrentClicked);
                        },
                    },
                },

                #[name = "saved_locations_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
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

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let location_query = config.general().weather_location_query().get();
            sender_clone.input(WeatherSettingsInput::LocationQueryEffect(location_query));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.general().temperature_unit().get();
            sender_clone.input(WeatherSettingsInput::WeatherUnitTypeEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let saved = config_manager()
                .config()
                .general()
                .weather_saved_locations()
                .get();
            sender_clone.input(WeatherSettingsInput::SavedLocationsEffect(saved));
        });

        let location_query_types = gtk::StringList::new(
            &LocationQueryType::all()
                .iter()
                .map(|p| p.label())
                .collect::<Vec<_>>(),
        );

        let weather_unit_types = gtk::StringList::new(
            &TemperatureUnitConfig::all()
                .iter()
                .map(|p| p.label())
                .collect::<Vec<_>>(),
        );

        let model = WeatherSettingsModel {
            location_query_types,
            active_location_query_type: config_manager()
                .config()
                .general()
                .weather_location_query()
                .get_untracked()
                .kind(),
            location_lat_lon: "0.0, 0.0".to_string(),
            location_city: "Nowhere".to_string(),
            lat_lon_dialog: None,
            city_dialog: None,
            weather_unit_types,
            active_weather_unit_type: config_manager()
                .config()
                .general()
                .temperature_unit()
                .get_untracked(),
            poll_minutes: config_manager()
                .config()
                .general()
                .weather_poll_minutes()
                .get_untracked(),
            retry_minutes: config_manager()
                .config()
                .general()
                .weather_retry_minutes()
                .get_untracked(),
            saved_locations: config_manager()
                .config()
                .general()
                .weather_saved_locations()
                .get_untracked(),
            active_query: config_manager()
                .config()
                .general()
                .weather_location_query()
                .get_untracked(),
            save_dialog: None,
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
            WeatherSettingsInput::LocationQueryTypeSelected(query_type) => {
                config_manager().update_config(move |config| {
                    config.general.weather_location_query =
                        if query_type == LocationQueryType::Coordinates {
                            LocationQueryConfig::Coordinates {
                                lat: OrdF64(0.0),
                                lon: OrdF64(0.0),
                            }
                        } else {
                            LocationQueryConfig::City {
                                name: String::new(),
                                country: String::new(),
                            }
                        };
                });
            }
            WeatherSettingsInput::LocationQueryEffect(query) => {
                self.active_query = query.clone();
                match query {
                    LocationQueryConfig::Coordinates { lat, lon } => {
                        self.location_lat_lon = format!("{}, {}", lat.0, lon.0);
                        self.active_location_query_type = LocationQueryType::Coordinates;
                    }
                    LocationQueryConfig::City { name, country } => {
                        self.location_city = format!("{}, {}", name, country);
                        self.active_location_query_type = LocationQueryType::City;
                    }
                }
            }
            WeatherSettingsInput::ChangeCoordinatesClicked => {
                let dialog = TextEntryDialogModel::builder()
                    .launch(TextEntryDialogInit {
                        message: "Enter location coordinates".to_string(),
                        negative_label: "Cancel".to_string(),
                        positive_label: "Done".to_string(),
                        entry_placeholder: "lat".to_string(),
                        entry2_placeholder: "lon".to_string(),
                        show_second_entry: true,
                    })
                    .forward(sender.input_sender(), |msg| match msg {
                        TextEntryDialogOutput::PositiveSelected(lat, lon) => {
                            WeatherSettingsInput::LatLonChosen(lat, lon)
                        }
                        TextEntryDialogOutput::NegativeSelected => {
                            WeatherSettingsInput::DialogCanceled
                        }
                    });
                self.lat_lon_dialog = Some(dialog);
            }
            WeatherSettingsInput::ChangeCityClicked => {
                let dialog = TextEntryDialogModel::builder()
                    .launch(TextEntryDialogInit {
                        message: "Enter a city or district name".to_string(),
                        negative_label: "Cancel".to_string(),
                        positive_label: "Done".to_string(),
                        entry_placeholder: "City / district (e.g. Kadıköy)".to_string(),
                        entry2_placeholder: "Country code (e.g. TR)".to_string(),
                        show_second_entry: true,
                    })
                    .forward(sender.input_sender(), |msg| match msg {
                        TextEntryDialogOutput::PositiveSelected(city, country) => {
                            WeatherSettingsInput::CityChosen(city, country)
                        }
                        TextEntryDialogOutput::NegativeSelected => {
                            WeatherSettingsInput::DialogCanceled
                        }
                    });
                self.city_dialog = Some(dialog);
            }
            WeatherSettingsInput::LatLonChosen(lat, lon) => {
                if let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) {
                    config_manager().update_config(|config| {
                        config.general.weather_location_query = LocationQueryConfig::Coordinates {
                            lat: OrdF64(lat),
                            lon: OrdF64(lon),
                        };
                    });
                }
            }
            WeatherSettingsInput::CityChosen(city, country) => {
                config_manager().update_config(|config| {
                    config.general.weather_location_query = LocationQueryConfig::City {
                        name: city,
                        country,
                    }
                });
            }
            WeatherSettingsInput::WeatherUnitTypeSelected(unit) => {
                config_manager().update_config(|config| {
                    config.general.temperature_unit = unit;
                });
            }
            WeatherSettingsInput::WeatherUnitTypeEffect(unit) => {
                self.active_weather_unit_type = unit;
            }
            WeatherSettingsInput::SetPollMinutes(mins) => {
                let mins = mins.clamp(1, 180);
                self.poll_minutes = mins;
                config_manager().update_config(move |config| {
                    config.general.weather_poll_minutes = mins;
                });
            }
            WeatherSettingsInput::SetRetryMinutes(mins) => {
                let mins = mins.clamp(1, 60);
                self.retry_minutes = mins;
                config_manager().update_config(move |config| {
                    config.general.weather_retry_minutes = mins;
                });
            }
            WeatherSettingsInput::DialogCanceled => {}
            WeatherSettingsInput::SavedLocationsEffect(saved) => {
                self.saved_locations = saved;
                self.rebuild_saved_list(&widgets.saved_locations_box, &sender);
            }
            WeatherSettingsInput::SaveCurrentClicked => {
                let dialog = TextEntryDialogModel::builder()
                    .launch(TextEntryDialogInit {
                        message: "Name this location".to_string(),
                        negative_label: "Cancel".to_string(),
                        positive_label: "Save".to_string(),
                        entry_placeholder: "e.g. Home, Work".to_string(),
                        entry2_placeholder: String::new(),
                        show_second_entry: false,
                    })
                    .forward(sender.input_sender(), |msg| match msg {
                        TextEntryDialogOutput::PositiveSelected(name, _) => {
                            WeatherSettingsInput::SaveNameChosen(name)
                        }
                        TextEntryDialogOutput::NegativeSelected => {
                            WeatherSettingsInput::DialogCanceled
                        }
                    });
                self.save_dialog = Some(dialog);
            }
            WeatherSettingsInput::SaveNameChosen(name) => {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    let query = self.active_query.clone();
                    config_manager().update_config(move |config| {
                        // Replace an existing bookmark with the same name
                        // rather than stacking duplicates.
                        config
                            .general
                            .weather_saved_locations
                            .retain(|l| l.name != name);
                        config
                            .general
                            .weather_saved_locations
                            .push(SavedLocation { name, query });
                    });
                }
            }
            WeatherSettingsInput::RemoveLocation(idx) => {
                config_manager().update_config(move |config| {
                    if idx < config.general.weather_saved_locations.len() {
                        config.general.weather_saved_locations.remove(idx);
                    }
                });
            }
        }

        self.update_view(widgets, sender);
    }
}

impl WeatherSettingsModel {
    /// Imperatively rebuild the saved-location rows. This is a tiny,
    /// cold list (a handful of bookmarks, changed rarely), so a plain
    /// clear-and-append into a held Box is the right tool — no factory
    /// / virtualization needed.
    fn rebuild_saved_list(&self, container: &gtk::Box, sender: &ComponentSender<Self>) {
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }

        if self.saved_locations.is_empty() {
            let empty = gtk::Label::builder()
                .label("No saved locations yet.")
                .css_classes(["label-small"])
                .halign(gtk::Align::Start)
                .build();
            container.append(&empty);
            return;
        }

        for (idx, loc) in self.saved_locations.iter().enumerate() {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .build();

            let text = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .hexpand(true)
                .build();
            let name = gtk::Label::builder()
                .label(&loc.name)
                .css_classes(["label-medium-bold"])
                .halign(gtk::Align::Start)
                .build();
            let summary = gtk::Label::builder()
                .label(loc.query.summary())
                .css_classes(["label-small"])
                .halign(gtk::Align::Start)
                .build();
            text.append(&name);
            text.append(&summary);

            let remove = gtk::Button::builder()
                .label("Remove")
                .css_classes(["ok-button-surface", "ok-button-cell"])
                .valign(gtk::Align::Center)
                .build();
            let sender = sender.clone();
            remove.connect_clicked(move |_| {
                sender.input(WeatherSettingsInput::RemoveLocation(idx));
            });

            row.append(&text);
            row.append(&remove);
            container.append(&row);
        }
    }
}
