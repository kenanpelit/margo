use crate::bar_settings::bar_widget_factory::{
    ActiveWidgetInit, ActiveWidgetModel, BarListLocation,
};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use relm4::factory::FactoryVecDeque;
use relm4::gtk::gio;
use relm4::gtk::prelude::{ActionMapExt, BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarSection {
    Start,
    Center,
    End,
}

impl BarSection {
    pub fn display_name(&self) -> &'static str {
        match self {
            BarSection::Start => "Start",
            BarSection::Center => "Center",
            BarSection::End => "End",
        }
    }
}

#[derive(Debug)]
pub struct WidgetSectionModel {
    section: BarSection,
    location: BarListLocation,
    widgets: FactoryVecDeque<ActiveWidgetModel>,
}

#[derive(Debug)]
pub enum WidgetSectionInput {
    /// Replay the section's widget list into the factory. Driven
    /// from `bar_settings.rs`'s reactive effects — the add /
    /// reorder / remove controls all write the config directly,
    /// so this is the only message the section needs.
    SetWidgetsEffect(Vec<BarWidget>),
}

#[derive(Debug)]
pub enum WidgetSectionOutput {}

pub struct WidgetSectionInit {
    pub bar_section: BarSection,
    pub location: BarListLocation,
    pub widgets: Vec<BarWidget>,
}

#[relm4::component(pub)]
impl Component for WidgetSectionModel {
    type CommandOutput = ();
    type Input = WidgetSectionInput;
    type Output = WidgetSectionOutput;
    type Init = WidgetSectionInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            add_css_class: "settings-bar-widget-section",

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_halign: gtk::Align::Start,
                #[watch]
                set_label: model.section.display_name(),
            },

            #[local_ref]
            widget_list -> gtk::ListBox {
                set_selection_mode: gtk::SelectionMode::None,
                add_css_class: "settings-bar-widget-section-list",
            },

            #[name = "add_widget_button"]
            gtk::MenuButton {
                set_label: "Add widget",
                set_halign: gtk::Align::Start,
                set_always_show_arrow: false,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let location = params.location;
        // The factory children mutate the config directly, so their
        // (empty) output is just detached.
        let mut widgets = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .detach();

        params.widgets.iter().for_each(|widget| {
            widgets.guard().push_back(ActiveWidgetInit {
                widget: widget.clone(),
                location,
            });
        });

        let model = WidgetSectionModel {
            section: params.bar_section,
            location,
            widgets,
        };

        let widget_list = model.widgets.widget();
        let widgets_view = view_output!();

        // Build the add-widget menu
        Self::build_add_menu(&widgets_view.add_widget_button, location);

        ComponentParts {
            model,
            widgets: widgets_view,
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            WidgetSectionInput::SetWidgetsEffect(new_widgets) => {
                let location = self.location;
                let mut guard = self.widgets.guard();
                guard.clear();
                for widget in new_widgets {
                    guard.push_back(ActiveWidgetInit { widget, location });
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

impl WidgetSectionModel {
    fn build_add_menu(button: &gtk::MenuButton, location: BarListLocation) {
        let menu = gio::Menu::new();
        let action_group = gio::SimpleActionGroup::new();

        for widget in BarWidget::all() {
            let action_name = widget.action_name();
            let action = gio::SimpleAction::new(&action_name, None);

            // Direct config write — bypass the parent message channel
            // for the same reason as the factory children.
            let widget_clone = widget.clone();
            action.connect_activate(move |_, _| {
                let widget_clone = widget_clone.clone();
                config_manager().update_config(move |config| {
                    let list = match location {
                        BarListLocation::TopStart => &mut config.bars.top_bar.left_widgets,
                        BarListLocation::TopCenter => &mut config.bars.top_bar.center_widgets,
                        BarListLocation::TopEnd => &mut config.bars.top_bar.right_widgets,
                        BarListLocation::BottomStart => &mut config.bars.bottom_bar.left_widgets,
                        BarListLocation::BottomCenter => {
                            &mut config.bars.bottom_bar.center_widgets
                        }
                        BarListLocation::BottomEnd => &mut config.bars.bottom_bar.right_widgets,
                    };
                    list.push(widget_clone);
                });
            });

            action_group.add_action(&action);
            menu.append(
                Some(widget.display_name()),
                Some(&format!("widget.{action_name}")),
            );
        }

        button.insert_action_group("widget", Some(&action_group));
        button.set_menu_model(Some(&menu));
    }
}
