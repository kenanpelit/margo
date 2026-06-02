use crate::menu_settings::menu_widget_row::{MenuWidgetRowModel, MenuWidgetRowOutput};
use mshell_config::schema::menu_widgets::MenuWidget;
use relm4::factory::{DynamicIndex, FactoryVecDeque};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, PopoverExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub struct MenuWidgetListInit {
    pub widgets: Vec<MenuWidget>,
    pub draw_border: bool,
}

#[derive(Debug)]
pub struct MenuWidgetListModel {
    widgets: FactoryVecDeque<MenuWidgetRowModel>,
}

#[derive(Debug)]
pub enum MenuWidgetListInput {
    AddWidget(MenuWidget),
    RemoveWidget(DynamicIndex),
    MoveUp(DynamicIndex),
    MoveDown(DynamicIndex),
    Reorder(usize, usize),
    WidgetChanged(DynamicIndex, MenuWidget),
    SetWidgetsEffect(Vec<MenuWidget>),
}

#[derive(Debug)]
pub enum MenuWidgetListOutput {
    Changed(Vec<MenuWidget>),
}

#[relm4::component(pub)]
impl Component for MenuWidgetListModel {
    type CommandOutput = ();
    type Input = MenuWidgetListInput;
    type Output = MenuWidgetListOutput;
    type Init = MenuWidgetListInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 4,
            // Use `set_css_classes` (not `add_css_class`) so the
            // no-border case applies an empty slice rather than an empty
            // string — `add_css_class("")` trips
            // `gtk_widget_add_css_class: css_class[0] != '\0'`.
            set_css_classes: if init.draw_border {
                &["settings-menu-widget-section"] as &[&str]
            } else {
                &[]
            },

            #[local_ref]
            widget_list -> gtk::ListBox {
                add_css_class: "settings-menu-widget-section-list",
                set_selection_mode: gtk::SelectionMode::None,
            },

            #[name = "add_button"]
            gtk::MenuButton {
                set_label: "Add widget",
                set_halign: gtk::Align::Start,
                set_always_show_arrow: false,
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut widgets = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .forward(sender.input_sender(), |output| match output {
                MenuWidgetRowOutput::MoveUp(idx) => MenuWidgetListInput::MoveUp(idx),
                MenuWidgetRowOutput::MoveDown(idx) => MenuWidgetListInput::MoveDown(idx),
                MenuWidgetRowOutput::Reorder(from, to) => {
                    MenuWidgetListInput::Reorder(from, to)
                }
                MenuWidgetRowOutput::Remove(idx) => MenuWidgetListInput::RemoveWidget(idx),
                MenuWidgetRowOutput::WidgetChanged(idx, w) => {
                    MenuWidgetListInput::WidgetChanged(idx, w)
                }
            });

        {
            let mut guard = widgets.guard();
            for w in init.widgets {
                guard.push_back(w);
            }
        }

        let model = MenuWidgetListModel { widgets };

        let widget_list = model.widgets.widget();
        let view_widgets = view_output!();

        Self::build_add_menu(&view_widgets.add_button, &sender);

        ComponentParts {
            model,
            widgets: view_widgets,
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
            MenuWidgetListInput::AddWidget(widget) => {
                self.widgets.guard().push_back(widget);
                self.emit_changed(&sender);
            }
            MenuWidgetListInput::RemoveWidget(index) => {
                self.widgets.guard().remove(index.current_index());
                self.emit_changed(&sender);
            }
            MenuWidgetListInput::MoveUp(index) => {
                let idx = index.current_index();
                if idx > 0 {
                    self.widgets.guard().move_to(idx, idx - 1);
                    self.emit_changed(&sender);
                }
            }
            MenuWidgetListInput::MoveDown(index) => {
                let idx = index.current_index();
                if idx + 1 < self.widgets.len() {
                    self.widgets.guard().move_to(idx, idx + 1);
                    self.emit_changed(&sender);
                }
            }
            MenuWidgetListInput::Reorder(from, to) => {
                let len = self.widgets.len();
                if from < len && to < len && from != to {
                    self.widgets.guard().move_to(from, to);
                    self.emit_changed(&sender);
                }
            }
            MenuWidgetListInput::WidgetChanged(index, widget) => {
                let idx = index.current_index();
                if let Some(row) = self.widgets.guard().get_mut(idx) {
                    row.widget = widget;
                }
                self.emit_changed(&sender);
            }
            MenuWidgetListInput::SetWidgetsEffect(new_widgets) => {
                let mut guard = self.widgets.guard();
                guard.clear();
                for widget in new_widgets {
                    guard.push_back(widget);
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

impl MenuWidgetListModel {
    fn emit_changed(&self, sender: &ComponentSender<Self>) {
        let widgets: Vec<MenuWidget> = self.widgets.iter().map(|row| row.widget.clone()).collect();
        let _ = sender.output(MenuWidgetListOutput::Changed(widgets));
    }

    /// Build the "Add widget" popover — a height-capped, scrollable list of
    /// the available menu widgets. Matches the Bar page's add-widget popover
    /// (reusing its CSS) instead of a `gio::Menu`, which GTK renders as a
    /// native menu that doesn't scroll — long catalogues ran off the panel.
    fn build_add_menu(button: &gtk::MenuButton, sender: &ComponentSender<Self>) {
        let popover = gtk::Popover::new();
        popover.add_css_class("settings-bar-widget-add-popover");

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vscrollbar_policy(gtk::PolicyType::Automatic);
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled.set_max_content_height(360);
        scrolled.set_propagate_natural_height(true);
        scrolled.set_propagate_natural_width(true);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("settings-bar-widget-add-list");

        for widget in MenuWidget::all_defaults() {
            let btn = gtk::Button::with_label(widget.display_name());
            btn.set_css_classes(&["settings-bar-widget-add-item"]);
            btn.set_halign(gtk::Align::Fill);
            btn.set_has_frame(false);

            let widget_clone = widget.clone();
            let popover_clone = popover.clone();
            let input = sender.input_sender().clone();
            btn.connect_clicked(move |_| {
                input.emit(MenuWidgetListInput::AddWidget(widget_clone.clone()));
                popover_clone.popdown();
            });

            list.append(&btn);
        }

        scrolled.set_child(Some(&list));
        popover.set_child(Some(&scrolled));
        button.set_popover(Some(&popover));
    }
}
