use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use relm4::factory::{DynamicIndex, FactoryComponent};
use relm4::gtk::prelude::{BoxExt, ButtonExt, ListBoxRowExt, OrientableExt, WidgetExt};
use relm4::{FactorySender, gtk};

/// Which config list this factory child writes back into when its
/// reorder / remove buttons fire. The parent section knows where it
/// lives (bar + bar-section); we pipe that down so each card can
/// rewrite the right `Vec<BarWidget>` directly instead of routing
/// through the FactoryVecDeque output channel, which relm4's
/// single-worker private runtime never drains in this codebase (the
/// click reaches `sender.output()` and returns `Ok` but the parent's
/// forward task never wakes up to process it — same failure mode as
/// `theme_card`, fixed there by the same direct-config bypass).
#[derive(Debug, Clone, Copy)]
pub enum BarListLocation {
    TopStart,
    TopCenter,
    TopEnd,
    BottomStart,
    BottomCenter,
    BottomEnd,
}

#[derive(Debug)]
pub struct ActiveWidgetInit {
    pub widget: BarWidget,
    pub location: BarListLocation,
}

#[derive(Debug)]
pub struct ActiveWidgetModel {
    pub widget: BarWidget,
    pub location: BarListLocation,
}

#[derive(Debug)]
pub enum ActiveWidgetInput {}

// Kept for compatibility with the section; outputs are no longer the
// authoritative path (config mutations happen in-place above) but
// emitting still lets the parent log / react if it wants to.
#[derive(Debug)]
pub enum ActiveWidgetOutput {
    MoveUp(DynamicIndex),
    MoveDown(DynamicIndex),
    Remove(DynamicIndex),
}

#[relm4::factory(pub)]
impl FactoryComponent for ActiveWidgetModel {
    type Init = ActiveWidgetInit;
    type Input = ActiveWidgetInput;
    type Output = ActiveWidgetOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 8,

            gtk::Label {
                add_css_class: "label-small",
                set_hexpand: true,
                set_halign: gtk::Align::Start,
                #[watch]
                set_label: self.widget.display_name().to_string().as_str(),
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "menu-up-symbolic",
                connect_clicked[sender, index, location = self.location] => move |_| {
                    let idx = index.current_index();
                    tracing::debug!(idx, ?location, "bar_widget_factory: MoveUp");
                    reorder(location, idx, -1);
                    let _ = sender.output(ActiveWidgetOutput::MoveUp(index.clone()));
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "menu-down-symbolic",
                connect_clicked[sender, index, location = self.location] => move |_| {
                    let idx = index.current_index();
                    tracing::debug!(idx, ?location, "bar_widget_factory: MoveDown");
                    reorder(location, idx, 1);
                    let _ = sender.output(ActiveWidgetOutput::MoveDown(index.clone()));
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "close-symbolic",
                connect_clicked[sender, index, location = self.location] => move |_| {
                    let idx = index.current_index();
                    tracing::debug!(idx, ?location, "bar_widget_factory: Remove");
                    remove_at(location, idx);
                    let _ = sender.output(ActiveWidgetOutput::Remove(index.clone()));
                },
            },
        }
    }

    fn init_model(
        init: Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        Self {
            widget: init.widget,
            location: init.location,
        }
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
        widgets
    }
}

fn list_mut(
    config: &mut mshell_config::schema::config::Config,
    location: BarListLocation,
) -> &mut Vec<BarWidget> {
    match location {
        BarListLocation::TopStart => &mut config.bars.top_bar.left_widgets,
        BarListLocation::TopCenter => &mut config.bars.top_bar.center_widgets,
        BarListLocation::TopEnd => &mut config.bars.top_bar.right_widgets,
        BarListLocation::BottomStart => &mut config.bars.bottom_bar.left_widgets,
        BarListLocation::BottomCenter => &mut config.bars.bottom_bar.center_widgets,
        BarListLocation::BottomEnd => &mut config.bars.bottom_bar.right_widgets,
    }
}

fn reorder(location: BarListLocation, idx: usize, delta: i32) {
    config_manager().update_config(move |config| {
        let list = list_mut(config, location);
        if idx >= list.len() {
            return;
        }
        let target = idx as i32 + delta;
        if target < 0 || target >= list.len() as i32 {
            return;
        }
        let item = list.remove(idx);
        list.insert(target as usize, item);
    });
}

fn remove_at(location: BarListLocation, idx: usize) {
    config_manager().update_config(move |config| {
        let list = list_mut(config, location);
        if idx < list.len() {
            list.remove(idx);
        }
    });
}
