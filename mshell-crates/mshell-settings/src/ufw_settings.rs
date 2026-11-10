//! Settings → Widgets → UFW Firewall — the bar pill's options
//! (`bars.widgets.ufw`). Composed with the generic per-menu geometry
//! page.

use crate::github_settings::row_text;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, UfwBarWidgetStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct UfwSettingsModel {
    show_count: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum UfwSettingsInput {
    ShowCountChanged(bool),
    Effect(bool),
}

#[derive(Debug)]
pub(crate) enum UfwSettingsOutput {}

pub(crate) struct UfwSettingsInit {}

#[relm4::component(pub)]
impl Component for UfwSettingsModel {
    type CommandOutput = ();
    type Input = UfwSettingsInput;
    type Output = UfwSettingsOutput;
    type Init = UfwSettingsInit;

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

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Bar pill",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        set_widget_name: "setting:/bars/widgets/ufw/show_count",
                        append: &row_text(
                            "Show the rule count",
                            "While the firewall is on, show how many rules allow incoming traffic (allow and limit) next to the shield.",
                        ),
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(count_handler)]
                            set_active: model.show_count,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(UfwSettingsInput::ShowCountChanged(v));
                                glib::Propagation::Proceed
                            } @count_handler,
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
            s.input(UfwSettingsInput::Effect(
                config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .ufw()
                    .show_count()
                    .get(),
            ));
        });
        let model = UfwSettingsModel {
            show_count: config_manager()
                .config()
                .bars()
                .widgets()
                .ufw()
                .show_count()
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
        root: &Self::Root,
    ) {
        match message {
            UfwSettingsInput::ShowCountChanged(v) => {
                if v != self.show_count {
                    config_manager().update_config(|c| c.bars.widgets.ufw.show_count = v);
                }
            }
            UfwSettingsInput::Effect(v) => self.show_count = v,
        }
        self.update_view(widgets, sender);
        let _ = root;
    }
}
