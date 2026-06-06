use crate::menus::menu_widgets::weather::hourly_item::build_hourly_item;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_utils::scroll_extensions::wire_vertical_to_horizontal;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmRemoveAllExt, RelmWidgetExt, gtk};
use wayle_weather::{HourlyForecast, TemperatureUnit};

pub(crate) struct HourlyModel {
    /// Cached forecast so a unit/format change can rebuild without a refetch.
    hourly: Vec<HourlyForecast>,
    temperature_unit: TemperatureUnit,
    format_24_h: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum HourlyInput {
    Update(Vec<HourlyForecast>),
    UpdateTemperatureUnit(TemperatureUnit),
    ChangeFormat(bool),
}

#[derive(Debug)]
pub(crate) enum HourlyOutput {}

pub(crate) struct HourlyInit {
    pub hourly: Vec<HourlyForecast>,
}

#[derive(Debug)]
pub(crate) enum HourlyCommandOutput {}

/// Repaint the cells as plain widgets (cheap — no per-cell component/effects).
fn rebuild_items(
    container: &gtk::Box,
    hourly: &[HourlyForecast],
    unit: &TemperatureUnit,
    format_24_h: bool,
) {
    container.remove_all();
    for forecast in hourly {
        container.append(&build_hourly_item(forecast, unit, format_24_h));
    }
}

#[relm4::component(pub)]
impl Component for HourlyModel {
    type CommandOutput = HourlyCommandOutput;
    type Input = HourlyInput;
    type Output = HourlyOutput;
    type Init = HourlyInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "weather-container",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            gtk::Label {
                add_css_class: "label-large-bold",
                set_label: "Hourly Conditions",
            },

            #[name = "scroll_window"]
            gtk::ScrolledWindow {
                set_vscrollbar_policy: gtk::PolicyType::Never,
                set_hscrollbar_policy: gtk::PolicyType::External,
                set_hexpand: true,
                set_vexpand: true,

                #[name = "widget_container"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 32,
                    set_align: gtk::Align::Center,
                }
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = config_manager().config();

        // Two effects TOTAL (not per cell): temperature unit + clock format.
        let mut effects = EffectScope::new();
        {
            let config = base_config.clone();
            let s = sender.clone();
            effects.push(move |_| {
                let config = config.clone();
                let unit = config.general().temperature_unit().get();
                s.input(HourlyInput::UpdateTemperatureUnit(TemperatureUnit::from(
                    unit,
                )));
            });
        }
        {
            let config = base_config.clone();
            let s = sender.clone();
            effects.push(move |_| {
                let config = config.clone();
                let fmt = config.general().clock_format_24_h().get();
                s.input(HourlyInput::ChangeFormat(fmt));
            });
        }

        let temperature_unit = TemperatureUnit::from(
            base_config
                .clone()
                .general()
                .temperature_unit()
                .get_untracked(),
        );
        let format_24_h = base_config.general().clock_format_24_h().get_untracked();

        let model = HourlyModel {
            hourly: params.hourly,
            temperature_unit,
            format_24_h,
            _effects: effects,
        };

        let widgets = view_output!();

        rebuild_items(
            &widgets.widget_container,
            &model.hourly,
            &model.temperature_unit,
            model.format_24_h,
        );
        wire_vertical_to_horizontal(&widgets.scroll_window, 32.0);

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
            HourlyInput::Update(hourly) => self.hourly = hourly,
            HourlyInput::UpdateTemperatureUnit(unit) => {
                if self.temperature_unit == unit {
                    return;
                }
                self.temperature_unit = unit;
            }
            HourlyInput::ChangeFormat(fmt) => {
                if self.format_24_h == fmt {
                    return;
                }
                self.format_24_h = fmt;
            }
        }
        rebuild_items(
            &widgets.widget_container,
            &self.hourly,
            &self.temperature_unit,
            self.format_24_h,
        );
        self.update_view(widgets, sender);
    }
}
