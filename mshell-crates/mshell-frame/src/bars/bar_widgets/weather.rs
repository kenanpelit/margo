//! Weather — bar pill showing the current condition icon + temperature
//! for the configured location. Left click opens the standalone weather
//! menu (the same rich Current / Hourly / Daily surface the dashboard
//! embeds). Reads `weather_service()`, so it shares the poll + cache with
//! the menu; the location is set in Settings → General (coordinates or
//! city / district name).

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_services::weather_service;
use mshell_utils::weather::{get_temperature_string, get_weather_icon_name, spawn_weather_watcher};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_weather::{TemperatureUnit, WeatherStatus};

/// Fallback glyph while weather is still loading or errored — a real,
/// theme-present icon so the pill never renders blank.
const FALLBACK_ICON: &str = "weather-few-clouds-symbolic";

pub(crate) struct WeatherModel {
    icon: String,
    temp: String,
    /// Today's forecast high / low, pre-formatted as `↑24°` / `↓15°`
    /// (compact — the unit letter is already implied by `temp`). Empty
    /// while loading or if the daily forecast is unavailable.
    high: String,
    low: String,
    /// Whether the bar pill also surfaces today's high / low. Off by
    /// default — the pill reads icon + temp only (the classic compact
    /// form); right-click flips this on. Ephemeral (in-memory only),
    /// matching the CPU pill's RAM% toggle (DESIGN.md §4.3).
    show_hilo: bool,
    tooltip: String,
    _orientation: Orientation,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WeatherInput {
    /// Left click → frame opens the weather menu.
    Clicked,
    /// Right click → toggle today's high / low in the bar cluster.
    ToggleHilo,
    /// Weather data or the temperature unit changed → re-render.
    Refresh,
}

#[derive(Debug)]
pub(crate) enum WeatherOutput {
    Clicked,
}

pub(crate) struct WeatherInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum WeatherCommandOutput {
    WeatherChanged,
}

#[relm4::component(pub)]
impl Component for WeatherModel {
    type CommandOutput = WeatherCommandOutput;
    type Input = WeatherInput;
    type Output = WeatherOutput;
    type Init = WeatherInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["weather-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(model.tooltip.as_str()),

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat", "ok-bar-widget"],
                connect_clicked[sender] => move |_| {
                    sender.input(WeatherInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(model.icon.as_str()),
                    },
                    gtk::Label {
                        add_css_class: "weather-bar-label",
                        #[watch]
                        set_label: model.temp.as_str(),
                        #[watch]
                        set_visible: !model.temp.is_empty(),
                    },

                    // Today's high / low — a quieter secondary tier next
                    // to the current temp. Off by default (the pill stays
                    // the compact icon + temp); right-click opts in. Shown
                    // / hidden as a unit so a missing forecast never
                    // leaves a dangling arrow.
                    gtk::Box {
                        set_orientation: Orientation::Horizontal,
                        set_spacing: 4,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: model.show_hilo && !model.high.is_empty(),

                        gtk::Label {
                            add_css_class: "weather-bar-hilo",
                            #[watch]
                            set_label: model.high.as_str(),
                        },
                        gtk::Label {
                            add_css_class: "weather-bar-hilo",
                            #[watch]
                            set_label: model.low.as_str(),
                        },
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_weather_watcher(&sender, || WeatherCommandOutput::WeatherChanged);

        // Re-render when the temperature unit flips in Settings.
        let mut effects = EffectScope::new();
        let eff_sender = sender.clone();
        effects.push(move |_| {
            let _ = config_manager().config().general().temperature_unit().get();
            eff_sender.input(WeatherInput::Refresh);
        });

        let mut model = WeatherModel {
            icon: FALLBACK_ICON.to_string(),
            temp: String::new(),
            high: String::new(),
            low: String::new(),
            show_hilo: false,
            tooltip: "Weather".to_string(),
            _orientation: params.orientation,
            _effects: effects,
        };
        model.refresh();

        let widgets = view_output!();

        // Right-click toggles today's high / low in the bar cluster
        // (DESIGN.md §4.3 ephemeral right-click detail, like the CPU
        // pill's RAM%). Left-click still opens the menu.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_clone = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            sender_clone.input(WeatherInput::ToggleHilo);
        });
        widgets.button.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            WeatherInput::Clicked => {
                let _ = sender.output(WeatherOutput::Clicked);
            }
            WeatherInput::ToggleHilo => self.show_hilo = !self.show_hilo,
            WeatherInput::Refresh => self.refresh(),
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            WeatherCommandOutput::WeatherChanged => self.refresh(),
        }
    }
}

impl WeatherModel {
    /// Pull the latest snapshot from `weather_service()` + the configured
    /// unit and recompute the icon / temperature / tooltip.
    fn refresh(&mut self) {
        let service = weather_service();
        let unit: TemperatureUnit = config_manager()
            .config()
            .general()
            .temperature_unit()
            .get_untracked()
            .into();

        match service.status.get() {
            WeatherStatus::Loaded => {
                if let Some(weather) = service.weather.get() {
                    self.icon =
                        get_weather_icon_name(&weather.current.condition, weather.current.is_day)
                            .to_string();
                    self.temp = get_temperature_string(&weather.current.temperature, &unit);

                    // Today's high / low from the first daily forecast,
                    // compact (rounded, no unit letter): `↑24°` / `↓15°`.
                    if let Some(today) = weather.daily.first() {
                        self.high = format!("↑{}", temp_compact(&today.temp_high, &unit));
                        self.low = format!("↓{}", temp_compact(&today.temp_low, &unit));
                    } else {
                        self.high.clear();
                        self.low.clear();
                    }

                    let place = if !weather.location.city.is_empty() {
                        match &weather.location.region {
                            Some(region) => format!("{}, {}", weather.location.city, region),
                            None => {
                                format!("{}, {}", weather.location.city, weather.location.country)
                            }
                        }
                    } else {
                        format!("{}, {}", weather.location.lat, weather.location.lon)
                    };
                    let summary = if self.high.is_empty() {
                        format!("{place} · {}", self.temp)
                    } else {
                        format!("{place} · {} · {} {}", self.temp, self.high, self.low)
                    };
                    self.tooltip = format!("{summary}\nClick: open  ·  Right-click: high / low");
                    return;
                }
                self.fallback("Weather");
            }
            WeatherStatus::Loading => self.fallback("Weather: loading…"),
            WeatherStatus::Error(_) => self.fallback("Weather: unavailable"),
        }
    }

    fn fallback(&mut self, tooltip: &str) {
        self.icon = FALLBACK_ICON.to_string();
        self.temp = String::new();
        self.high = String::new();
        self.low = String::new();
        self.tooltip = tooltip.to_string();
    }
}

/// Compact temperature for the bar's high / low chips: the rounded value
/// + degree glyph only (no unit letter — `temp` already carries it), e.g.
/// `24°`. Honours the configured metric / imperial unit.
fn temp_compact(t: &wayle_weather::Temperature, unit: &TemperatureUnit) -> String {
    let v = match unit {
        TemperatureUnit::Metric => t.celsius(),
        TemperatureUnit::Imperial => t.fahrenheit(),
    };
    format!("{}°", v.round() as i32)
}
