use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use relm4::factory::{DynamicIndex, FactoryComponent};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EventControllerExt, ListBoxRowExt, OrientableExt, ToValue, WidgetExt,
};
use relm4::gtk::{gdk, glib};
use relm4::{FactorySender, gtk};
use std::cell::RefCell;

thread_local! {
    /// The row currently being dragged: `(source list, source index)`.
    /// GTK content-providers are awkward to round-trip a typed payload
    /// through, so the identity travels via this main-thread-only side
    /// channel (same trick as `dynamic_box`'s dock reorder). Captured on
    /// drag prepare, consumed on drop, cleared on drag end.
    static DRAG_SRC: RefCell<Option<(BarListLocation, usize)>> = const { RefCell::new(None) };
}

/// Which config list this factory child writes back into when its
/// reorder / remove buttons fire. The parent section knows where it
/// lives (bar + bar-section); we pipe that down so each card can
/// rewrite the right `Vec<BarWidget>` directly instead of routing
/// through the FactoryVecDeque output channel, which relm4's
/// single-worker private runtime never drains in this codebase (the
/// click reaches `sender.output()` and returns `Ok` but the parent's
/// forward task never wakes up to process it — same failure mode as
/// `theme_card`, fixed there by the same direct-config bypass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarListLocation {
    TopStart,
    TopCenter,
    TopEnd,
    TopHidden,
    BottomStart,
    BottomCenter,
    BottomEnd,
    BottomHidden,
}

#[derive(Debug)]
pub struct ActiveWidgetInit {
    pub widget: BarWidget,
    pub location: BarListLocation,
}

#[derive(Debug)]
pub struct ActiveWidgetModel {
    pub location: BarListLocation,
    /// Display label — friendly plugin/custom name, not "Custom Widget".
    label: String,
}

#[derive(Debug)]
pub enum ActiveWidgetInput {}

// The reorder / remove buttons mutate the config directly (see the
// note on `BarListLocation`), so this component has no output —
// the parent re-syncs through the reactive `SetWidgetsEffect` path.
#[derive(Debug)]
pub enum ActiveWidgetOutput {}

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
                set_label: self.label.as_str(),
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "menu-up-symbolic",
                connect_clicked[index, location = self.location] => move |_| {
                    let idx = index.current_index();
                    tracing::debug!(idx, ?location, "bar_widget_factory: MoveUp");
                    reorder(location, idx, -1);
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "menu-down-symbolic",
                connect_clicked[index, location = self.location] => move |_| {
                    let idx = index.current_index();
                    tracing::debug!(idx, ?location, "bar_widget_factory: MoveDown");
                    reorder(location, idx, 1);
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_icon_name: "close-symbolic",
                connect_clicked[index, location = self.location] => move |_| {
                    let idx = index.current_index();
                    tracing::debug!(idx, ?location, "bar_widget_factory: Remove");
                    remove_at(location, idx);
                },
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        let label = match &init.widget {
            BarWidget::Custom(name) => super::bar_widget_section::custom_widget_label(name),
            other => other.display_name().to_string(),
        };
        Self {
            location: init.location,
            label,
        }
    }

    fn init_widgets(
        &mut self,
        index: &DynamicIndex,
        root: Self::Root,
        returned_widget: &gtk::ListBoxRow,
        _sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();
        returned_widget.set_activatable(false);
        returned_widget.set_selectable(false);
        returned_widget.set_focusable(false);
        returned_widget.set_can_focus(false);
        // Drag-to-reorder, in addition to the up/down buttons: grab a row
        // and drop it onto another within the same list.
        attach_reorder_dnd(returned_widget, self.location, index);
        widgets
    }
}

/// Wire GTK4 drag-and-drop reordering onto a factory row. The dragged
/// row's `(location, index)` is stashed in [`DRAG_SRC`] on prepare; on
/// drop onto another row in the *same* list we move the item to the drop
/// row's position. Cross-list drops are ignored (the up/down buttons
/// also only move within a list). Pairs with `.dragging` styling.
fn attach_reorder_dnd(row: &gtk::ListBoxRow, location: BarListLocation, index: &DynamicIndex) {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    let src_index = index.clone();
    source.connect_prepare(move |_, _, _| {
        DRAG_SRC.with(|c| *c.borrow_mut() = Some((location, src_index.current_index())));
        // The real payload travels via DRAG_SRC; the content provider just
        // needs to offer the STRING type the DropTarget below accepts.
        Some(gdk::ContentProvider::for_value(&"".to_value()))
    });
    source.connect_drag_begin(|src, _| {
        if let Some(w) = src.widget() {
            w.add_css_class("dragging");
        }
    });
    source.connect_drag_end(|src, _, _| {
        if let Some(w) = src.widget() {
            w.remove_css_class("dragging");
        }
        DRAG_SRC.with(|c| *c.borrow_mut() = None);
    });
    row.add_controller(source);

    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let dst_index = index.clone();
    target.connect_drop(move |_, _, _, _| {
        let to = dst_index.current_index();
        let from = DRAG_SRC.with(|c| c.borrow_mut().take());
        if let Some((src_loc, from)) = from
            && src_loc == location
            && from != to
        {
            move_item(location, from, to);
            return true;
        }
        false
    });
    row.add_controller(target);
}

fn list_mut(
    config: &mut mshell_config::schema::config::Config,
    location: BarListLocation,
) -> &mut Vec<BarWidget> {
    match location {
        BarListLocation::TopStart => &mut config.bars.top_bar.left_widgets,
        BarListLocation::TopCenter => &mut config.bars.top_bar.center_widgets,
        BarListLocation::TopEnd => &mut config.bars.top_bar.right_widgets,
        BarListLocation::TopHidden => &mut config.bars.top_bar.hidden_widgets,
        BarListLocation::BottomStart => &mut config.bars.bottom_bar.left_widgets,
        BarListLocation::BottomCenter => &mut config.bars.bottom_bar.center_widgets,
        BarListLocation::BottomEnd => &mut config.bars.bottom_bar.right_widgets,
        BarListLocation::BottomHidden => &mut config.bars.bottom_bar.hidden_widgets,
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

/// Move the item at `from` to the drop row's index `to` within one list.
/// `to` is the drop target's index *before* the move; after removing
/// `from` we insert at `to.min(len)`, which gives the natural drag
/// behaviour (dragging down lands after the target, dragging up lands
/// before it).
fn move_item(location: BarListLocation, from: usize, to: usize) {
    config_manager().update_config(move |config| {
        let list = list_mut(config, location);
        if from >= list.len() || from == to {
            return;
        }
        let item = list.remove(from);
        let dest = to.min(list.len());
        list.insert(dest, item);
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
