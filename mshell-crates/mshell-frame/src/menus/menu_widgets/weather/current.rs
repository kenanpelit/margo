use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_utils::weather::{
    condition_label, get_percent_string, get_precipitation_string, get_pressure_string,
    get_temperature_string, get_visibility_string, get_weather_icon_name, get_wind_speed,
    wind_direction_label, wind_speed_unit_short,
};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, GridExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_weather::{Astronomy, CurrentWeather, TemperatureUnit};

#[derive(Debug, Clone)]
pub(crate) struct CurrentModel {
    current_weather: CurrentWeather,
    astronomy: Astronomy,
    temperature_unit: TemperatureUnit,
    sunrise_time: String,
    sunset_time: String,
    format_24_h: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum CurrentInput {
    Update(CurrentWeather, Astronomy),
    UpdateTemperatureUnit(TemperatureUnit),
    ChangeFormat(bool),
}

#[derive(Debug)]
pub(crate) enum CurrentOutput {}

pub(crate) struct CurrentInit {
    pub current_weather: CurrentWeather,
    pub astronomy: Astronomy,
}

#[derive(Debug)]
pub(crate) enum CurrentCommandOutput {}

#[relm4::component(pub)]
impl Component for CurrentModel {
    type CommandOutput = CurrentCommandOutput;
    type Input = CurrentInput;
    type Output = CurrentOutput;
    type Init = CurrentInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "weather-container",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            gtk::Label {
                add_css_class: "label-large-bold",
                set_halign: gtk::Align::Start,
                set_label: "Current Conditions",
            },

            // ── Hero: big icon + temperature + worded condition + feels-like ──
            gtk::Box {
                add_css_class: "weather-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "current-weather-icon",
                    #[watch]
                    set_icon_name: Some(get_weather_icon_name(
                        &model.current_weather.condition,
                        model.current_weather.is_day,
                    )),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    set_hexpand: true,

                    gtk::Label {
                        add_css_class: "label-xl-bold",
                        set_halign: gtk::Align::Start,
                        #[watch]
                        set_label: get_temperature_string(
                            &model.current_weather.temperature,
                            &model.temperature_unit
                        ).as_str(),
                    },

                    gtk::Label {
                        add_css_class: "weather-condition-label",
                        set_halign: gtk::Align::Start,
                        #[watch]
                        set_label: condition_label(
                            &model.current_weather.condition,
                            model.current_weather.is_day,
                        ),
                    },

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        #[watch]
                        set_label: format!(
                            "Feels like {}",
                            get_temperature_string(
                                &model.current_weather.feels_like,
                                &model.temperature_unit
                            )
                        ).as_str(),
                    },
                },
            },

            // ── Two-column "label : value" detail grid (OpenWeather-style,
            //    rendered with margo's own label tokens). 4 cols × 4 rows. ──
            gtk::Grid {
                add_css_class: "weather-detail-grid",
                set_column_homogeneous: true,
                set_row_spacing: 6,
                set_column_spacing: 10,

                attach[0, 0, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "Humidity",
                },
                attach[1, 0, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: get_percent_string(&model.current_weather.humidity).as_str(),
                },
                attach[2, 0, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "Pressure",
                },
                attach[3, 0, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: get_pressure_string(
                        &model.current_weather.pressure,
                        &model.temperature_unit,
                    ).as_str(),
                },

                attach[0, 1, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "Wind",
                },
                attach[1, 1, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: format!(
                        "{} {} {}",
                        get_wind_speed(&model.current_weather.wind_speed, &model.temperature_unit),
                        wind_speed_unit_short(&model.temperature_unit),
                        wind_direction_label(&model.current_weather.wind_direction),
                    ).as_str(),
                },
                attach[2, 1, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "Gusts",
                },
                attach[3, 1, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: format!(
                        "{} {}",
                        get_wind_speed(&model.current_weather.wind_gust, &model.temperature_unit),
                        wind_speed_unit_short(&model.temperature_unit),
                    ).as_str(),
                },

                attach[0, 2, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "UV index",
                },
                attach[1, 2, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: model.current_weather.uv_index.get().to_string().as_str(),
                },
                attach[2, 2, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "Visibility",
                },
                attach[3, 2, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: get_visibility_string(
                        &model.current_weather.visibility,
                        &model.temperature_unit,
                    ).as_str(),
                },

                attach[0, 3, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "Cloud",
                },
                attach[1, 3, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: get_percent_string(&model.current_weather.cloud_cover).as_str(),
                },
                attach[2, 3, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-caption",
                    set_halign: gtk::Align::Start,
                    set_label: "Precip",
                },
                attach[3, 3, 1, 1] = &gtk::Label {
                    add_css_class: "weather-detail-value",
                    set_halign: gtk::Align::End,
                    #[watch]
                    set_label: get_precipitation_string(
                        &model.current_weather.precipitation,
                        &model.temperature_unit,
                    ).as_str(),
                },
            },

            // ── Sunrise / sunset row ──
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Sunrise",
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_halign: gtk::Align::Center,

                        gtk::Image {
                            add_css_class: "current-weather-detail-icon",
                            set_icon_name: Some("weather-sunrise-symbolic"),
                            set_margin_end: 8,
                        },

                        gtk::Label {
                            add_css_class: "label-small-bold",
                            #[watch]
                            set_label: model.sunrise_time.as_str(),
                        }
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Sunset",
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_halign: gtk::Align::Center,

                        gtk::Image {
                            add_css_class: "current-weather-detail-icon",
                            set_icon_name: Some("weather-sunset-symbolic"),
                            set_margin_end: 8,
                        },

                        gtk::Label {
                            add_css_class: "label-small-bold",
                            #[watch]
                            set_label: model.sunset_time.as_str(),
                        }
                    }
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = config_manager().config();

        let mut effects = EffectScope::new();

        let config = base_config.clone();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config.clone();
            let temperature_unit = config.general().temperature_unit().get();
            sender_clone.input(CurrentInput::UpdateTemperatureUnit(TemperatureUnit::from(
                temperature_unit,
            )));
        });

        let sender_clone = sender.clone();
        let config = base_config.clone();
        effects.push(move |_| {
            let config = config.clone();
            let format_24_h = config.general().clock_format_24_h().get();
            sender_clone.input(CurrentInput::ChangeFormat(format_24_h));
        });

        let config = base_config.clone();
        let format_24_h = config.general().clock_format_24_h().get_untracked();

        let sunrise_time: String;
        let sunset_time: String;

        if format_24_h {
            sunrise_time = params.astronomy.sunrise.format("%H:%M").to_string();
            sunset_time = params.astronomy.sunset.format("%H:%M").to_string();
        } else {
            sunrise_time = params.astronomy.sunrise.format("%I:%M %p").to_string();
            sunset_time = params.astronomy.sunset.format("%I:%M %p").to_string();
        }

        let model = CurrentModel {
            current_weather: params.current_weather,
            astronomy: params.astronomy,
            temperature_unit: TemperatureUnit::from(
                base_config.general().temperature_unit().get_untracked(),
            ),
            sunrise_time,
            sunset_time,
            format_24_h,
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
            CurrentInput::Update(current_weather, astronomy) => {
                self.current_weather = current_weather;
                self.astronomy = astronomy;

                let sunrise_time: String;
                let sunset_time: String;

                if self.format_24_h {
                    sunrise_time = self.astronomy.sunrise.format("%H:%M").to_string();
                    sunset_time = self.astronomy.sunset.format("%H:%M").to_string();
                } else {
                    sunrise_time = self.astronomy.sunrise.format("%I:%M %p").to_string();
                    sunset_time = self.astronomy.sunset.format("%I:%M %p").to_string();
                }

                self.sunrise_time = sunrise_time;
                self.sunset_time = sunset_time;
            }
            CurrentInput::UpdateTemperatureUnit(temperature_unit) => {
                self.temperature_unit = temperature_unit;
            }
            CurrentInput::ChangeFormat(format_24_h) => {
                let sunrise_time: String;
                let sunset_time: String;

                if format_24_h {
                    sunrise_time = self.astronomy.sunrise.format("%H:%M").to_string();
                    sunset_time = self.astronomy.sunset.format("%H:%M").to_string();
                } else {
                    sunrise_time = self.astronomy.sunrise.format("%I:%M %p").to_string();
                    sunset_time = self.astronomy.sunset.format("%I:%M %p").to_string();
                }

                self.sunrise_time = sunrise_time;
                self.sunset_time = sunset_time;
                self.format_24_h = format_24_h;
            }
        }

        self.update_view(widgets, sender);
    }
}
