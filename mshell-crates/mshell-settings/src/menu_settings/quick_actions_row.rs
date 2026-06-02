use mshell_config::schema::menu_widgets::QuickActionWidget;
use relm4::factory::{DynamicIndex, FactoryComponent};
use relm4::gtk::prelude::{BoxExt, ButtonExt, ListBoxRowExt, OrientableExt, WidgetExt};
use relm4::{FactorySender, gtk};

#[derive(Debug)]
pub struct QuickActionRowModel {
    pub action: QuickActionWidget,
}

#[derive(Debug)]
pub enum QuickActionRowOutput {
    Remove(DynamicIndex),
    MoveUp(DynamicIndex),
    MoveDown(DynamicIndex),
    /// Grip drag-to-reorder: move from one index to another in this list.
    Reorder(usize, usize),
}

#[relm4::factory(pub)]
impl FactoryComponent for QuickActionRowModel {
    type Init = QuickActionWidget;
    type Input = ();
    type Output = QuickActionRowOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 8,
            add_css_class: "quick-action-row",

            #[name = "grip"]
            gtk::Image {
                set_icon_name: Some("list-drag-handle-symbolic"),
                add_css_class: "reorder-grip",
                set_tooltip_text: Some("Drag to reorder"),
            },

            gtk::Label {
                set_hexpand: true,
                set_halign: gtk::Align::Start,
                add_css_class: "label-small",
                set_label: self.action.display_name(),
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "menu-up-symbolic",
                connect_clicked[sender, index] => move |_| {
                    sender.output(QuickActionRowOutput::MoveUp(index.clone())).unwrap();
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "menu-down-symbolic",
                connect_clicked[sender, index] => move |_| {
                    sender.output(QuickActionRowOutput::MoveDown(index.clone())).unwrap();
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "close-symbolic",
                connect_clicked[sender, index] => move |_| {
                    sender.output(QuickActionRowOutput::Remove(index.clone())).unwrap();
                },
            },
        }
    }

    fn init_model(action: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { action }
    }

    fn init_widgets(
        &mut self,
        index: &DynamicIndex,
        root: Self::Root,
        returned_widget: &gtk::ListBoxRow,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();
        returned_widget.set_activatable(false);
        returned_widget.set_selectable(false);
        returned_widget.set_focusable(false);
        returned_widget.set_can_focus(false);
        let drag_index = index.clone();
        let drag_sender = sender.clone();
        crate::reorder_dnd::attach_grip_drag(&widgets.grip, returned_widget, move |delta| {
            let from = drag_index.current_index();
            let to = (from as i32 + delta).max(0) as usize;
            let _ = drag_sender.output(QuickActionRowOutput::Reorder(from, to));
        });
        widgets
    }
}
