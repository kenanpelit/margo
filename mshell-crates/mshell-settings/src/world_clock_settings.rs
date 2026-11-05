//! Settings → Widgets → Clock → World clocks: whether the Clock menu
//! shows the World Clocks section, how many cities ride in the bar clock,
//! and the working day used for the "working hours" hints
//! (`config.tempo.*`, `menus.clock_menu.widgets`). The city list itself is
//! edited in the menu section. Shell-owned, through the reactive store.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_config::schema::menu_widgets::MenuWidget;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct WorldClockSettingsModel {
    show_in_menu: bool,
    in_bar: u32,
    work_start: u32,
    work_end: u32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WorldClockSettingsInput {
    ShowInMenuChanged(bool),
    InBarChanged(u32),
    WorkStartChanged(u32),
    WorkEndChanged(u32),
    /// Store → page.
    Sync,
}

#[derive(Debug)]
pub(crate) enum WorldClockSettingsOutput {}

pub(crate) struct WorldClockSettingsInit {}

/// Add (after the calendar, else at the end) or remove the World Clocks
/// section in a menu's widget list. Idempotent.
pub(crate) fn set_world_clocks(widgets: &mut Vec<MenuWidget>, on: bool) {
    let present = widgets.contains(&MenuWidget::WorldClocks);
    if on && !present {
        let at = widgets
            .iter()
            .position(|w| matches!(w, MenuWidget::Calendar | MenuWidget::CalendarGrid))
            .map_or(widgets.len(), |i| i + 1);
        widgets.insert(at, MenuWidget::WorldClocks);
    } else if !on && present {
        widgets.retain(|w| *w != MenuWidget::WorldClocks);
    }
}

fn read() -> (bool, u32, u32, u32) {
    let c = config_manager().config();
    let tempo = c.clone().tempo().get_untracked();
    let shown = c
        .menus()
        .clock_menu()
        .widgets()
        .get_untracked()
        .contains(&MenuWidget::WorldClocks);
    (
        shown,
        tempo.world_clocks_in_bar,
        tempo.work_day_start,
        tempo.work_day_end,
    )
}

#[relm4::component(pub)]
impl Component for WorldClockSettingsModel {
    type CommandOutput = ();
    type Input = WorldClockSettingsInput;
    type Output = WorldClockSettingsOutput;
    type Init = WorldClockSettingsInit;

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
                        set_icon_name: Some("globe-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "World clocks",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Compare cities with the device time: offset, the other side's day, working hours and an hour strip. Add, reorder, remove or pin cities (make one the device timezone) in the Clock menu's World Clocks section.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Clock menu",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Show world clocks in the Clock menu" },
                        #[template_child] desc { set_label: "Adds the World Clocks section below the calendar (Settings → Menus can move it)." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(show_in_menu_handler)]
                            set_active: model.show_in_menu,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(WorldClockSettingsInput::ShowInMenuChanged(v));
                                glib::Propagation::Proceed
                            } @show_in_menu_handler,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Bar clock",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/tempo/world_clocks_in_bar",
                        #[template_child] title { set_label: "Cities in the bar clock" },
                        #[template_child] desc { set_label: "How many of the first cities ride next to the bar clock (0 = none). All of them are listed in its tooltip." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 3.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(in_bar_handler)]
                            set_value: model.in_bar as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(WorldClockSettingsInput::InBarChanged(sb.value() as u32));
                            } @in_bar_handler,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Working hours",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/tempo/work_day_start",
                        #[template_child] title { set_label: "Working day starts" },
                        #[template_child] desc { set_label: "Hour (0–23) the working day starts in every city." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 23.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(work_start_handler)]
                            set_value: model.work_start as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(WorldClockSettingsInput::WorkStartChanged(sb.value() as u32));
                            } @work_start_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/tempo/work_day_end",
                        #[template_child] title { set_label: "Working day ends" },
                        #[template_child] desc { set_label: "Hour (1–24) it ends. An end before the start wraps past midnight." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(1.0, 1.0, 24.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(work_end_handler)]
                            set_value: model.work_end as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(WorldClockSettingsInput::WorkEndChanged(sb.value() as u32));
                            } @work_end_handler,
                        },
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
        let s = sender.clone();
        effects.push(move |_| {
            let _ = config_manager().config().tempo().get();
            let _ = config_manager()
                .config()
                .menus()
                .clock_menu()
                .widgets()
                .get();
            s.input(WorldClockSettingsInput::Sync);
        });
        let (show_in_menu, in_bar, work_start, work_end) = read();
        let model = WorldClockSettingsModel {
            show_in_menu,
            in_bar,
            work_start,
            work_end,
            _effects: effects,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            WorldClockSettingsInput::ShowInMenuChanged(on) => {
                config_manager().update_config(move |c| {
                    set_world_clocks(&mut c.menus.clock_menu.widgets, on);
                });
            }
            WorldClockSettingsInput::InBarChanged(v) => {
                config_manager().update_config(move |c| c.tempo.world_clocks_in_bar = v.min(3));
            }
            WorldClockSettingsInput::WorkStartChanged(v) => {
                config_manager().update_config(move |c| c.tempo.work_day_start = v.min(23));
            }
            WorldClockSettingsInput::WorkEndChanged(v) => {
                config_manager().update_config(move |c| c.tempo.work_day_end = v.clamp(1, 24));
            }
            WorldClockSettingsInput::Sync => {
                (
                    self.show_in_menu,
                    self.in_bar,
                    self.work_start,
                    self.work_end,
                ) = read();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_goes_after_the_calendar_once_and_comes_out_cleanly() {
        let mut w = vec![MenuWidget::Calendar, MenuWidget::Weather];
        set_world_clocks(&mut w, true);
        assert_eq!(
            w,
            [
                MenuWidget::Calendar,
                MenuWidget::WorldClocks,
                MenuWidget::Weather
            ]
        );
        set_world_clocks(&mut w, true);
        assert_eq!(w.len(), 3, "idempotent");
        set_world_clocks(&mut w, false);
        assert_eq!(w, [MenuWidget::Calendar, MenuWidget::Weather]);
        let mut none = vec![MenuWidget::Weather];
        set_world_clocks(&mut none, true);
        assert_eq!(none, [MenuWidget::Weather, MenuWidget::WorldClocks]);
    }
}
