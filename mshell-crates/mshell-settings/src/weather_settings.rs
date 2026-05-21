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
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
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
    DialogCanceled,
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
                        set_css_classes: &["label-medium", "ok-button-primary"],
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
                        set_css_classes: &["label-medium", "ok-button-primary"],
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
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
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
            WeatherSettingsInput::LocationQueryEffect(query) => match query {
                LocationQueryConfig::Coordinates { lat, lon } => {
                    self.location_lat_lon = format!("{}, {}", lat.0, lon.0);
                    self.active_location_query_type = LocationQueryType::Coordinates;
                }
                LocationQueryConfig::City { name, country } => {
                    self.location_city = format!("{}, {}", name, country);
                    self.active_location_query_type = LocationQueryType::City;
                }
            },
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
            WeatherSettingsInput::DialogCanceled => {}
        }
    }
}
