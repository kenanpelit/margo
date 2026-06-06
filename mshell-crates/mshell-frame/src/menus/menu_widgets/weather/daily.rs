use crate::menus::menu_widgets::weather::daily_item::build_daily_item;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_utils::scroll_extensions::wire_vertical_to_horizontal;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmRemoveAllExt, RelmWidgetExt, gtk};
use wayle_weather::{DailyForecast, TemperatureUnit};

pub(crate) struct DailyModel {
    /// Cached forecast so a unit change can rebuild without a refetch.
    daily: Vec<DailyForecast>,
    temperature_unit: TemperatureUnit,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum DailyInput {
    Update(Vec<DailyForecast>),
    UpdateTemperatureUnit(TemperatureUnit),
}

#[derive(Debug)]
pub(crate) enum DailyOutput {}

pub(crate) struct DailyInit {
    pub daily: Vec<DailyForecast>,
}

#[derive(Debug)]
pub(crate) enum DailyCommandOutput {}

/// Repaint the cells as plain widgets (cheap — no per-cell component/effects).
fn rebuild_items(container: &gtk::Box, daily: &[DailyForecast], unit: &TemperatureUnit) {
    container.remove_all();
    for forecast in daily {
        container.append(&build_daily_item(forecast, unit));
    }
}

#[relm4::component(pub)]
impl Component for DailyModel {
    type CommandOutput = DailyCommandOutput;
    type Input = DailyInput;
    type Output = DailyOutput;
    type Init = DailyInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "weather-container",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            gtk::Label {
                add_css_class: "label-large-bold",
                set_label: "Daily Conditions",
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

        // One effect TOTAL (not per cell): temperature unit.
        let mut effects = EffectScope::new();
        {
            let config = base_config.clone();
            let s = sender.clone();
            effects.push(move |_| {
                let config = config.clone();
                let unit = config.general().temperature_unit().get();
                s.input(DailyInput::UpdateTemperatureUnit(TemperatureUnit::from(
                    unit,
                )));
            });
        }

        let temperature_unit =
            TemperatureUnit::from(base_config.general().temperature_unit().get_untracked());

        let model = DailyModel {
            daily: params.daily,
            temperature_unit,
            _effects: effects,
        };

        let widgets = view_output!();

        rebuild_items(
            &widgets.widget_container,
            &model.daily,
            &model.temperature_unit,
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
            DailyInput::Update(daily) => {
                // The weather component re-emits Update on every store tick;
                // skip the rebuild when the forecast is unchanged.
                if self.daily == daily {
                    return;
                }
                self.daily = daily;
            }
            DailyInput::UpdateTemperatureUnit(unit) => {
                if self.temperature_unit == unit {
                    return;
                }
                self.temperature_unit = unit;
            }
        }
        rebuild_items(
            &widgets.widget_container,
            &self.daily,
            &self.temperature_unit,
        );
        self.update_view(widgets, sender);
    }
}
