//! Per-menu configuration panel — the rendered card for one
//! menu inside the cross-cutting `Menus` settings page.
//!
//! Each panel embeds two sub-components:
//!
//! 1. `WidgetMenuSettingsModel` — Position / Min Width / Max
//!    Height (already used standalone by `Widgets → <menu name>`
//!    sub-sidebar entries; reused here so the two pages stay in
//!    sync).
//! 2. `MenuWidgetListModel` — the drag-reorder widget-list
//!    editor (which sub-widgets live inside this menu).
//!
//! The panel is parameterised by `MenuKind`, so adding a new
//! menu to the aggregate Menus page is now: one entry in
//! `MenuKind::all()` + the existing `MenuKind` schema arms (read
//! / tracked / write for position / min_width / max_height /
//! widgets). The 250-line copy-paste-per-menu block that used to
//! live in `menu_settings.rs` is gone.

use crate::menu_settings::menu_widget_list::{
    MenuWidgetListInit, MenuWidgetListInput, MenuWidgetListModel, MenuWidgetListOutput,
};
use crate::widget_menu_settings::{MenuKind, WidgetMenuSettingsInit, WidgetMenuSettingsModel};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::schema::menu_widgets::MenuWidget;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

#[derive(Debug)]
pub(crate) struct MenuConfigPanelModel {
    kind: MenuKind,
    settings: Controller<WidgetMenuSettingsModel>,
    widget_list: Controller<MenuWidgetListModel>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MenuConfigPanelInput {
    /// MenuWidgetListModel emitted a Changed — persist to config.
    WidgetListChanged(Vec<MenuWidget>),
    /// Reactive effect heard a widget-list change from disk —
    /// push it back into the FactoryVecDeque so the UI repaints
    /// without an mshell restart.
    WidgetListEffect(Vec<MenuWidget>),
}

#[derive(Debug)]
pub(crate) enum MenuConfigPanelOutput {}

pub(crate) struct MenuConfigPanelInit {
    pub kind: MenuKind,
}

#[relm4::component(pub(crate))]
impl Component for MenuConfigPanelModel {
    type CommandOutput = ();
    type Input = MenuConfigPanelInput;
    type Output = MenuConfigPanelOutput;
    type Init = MenuConfigPanelInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "menu-config-panel",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            gtk::Label {
                add_css_class: "label-large-bold",
                set_label: model.kind.display_name(),
                set_halign: gtk::Align::Start,
            },

            // Position / Min Width / Max Height — delegated to
            // the same component that drives the per-menu page
            // under Widgets sub-sidebar.
            model.settings.widget().clone() {},

            // Drag-reorder widget list editor.
            model.widget_list.widget().clone() {},

            gtk::Separator {},
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let kind = params.kind;

        // Reuse the standalone Position/Min/Max page as a
        // sub-component. Its own reactive effects keep it in
        // sync with config-reload events.
        let settings = WidgetMenuSettingsModel::builder()
            .launch(WidgetMenuSettingsInit { kind })
            .detach();

        // Seed the widget-list factory with the current value;
        // forward Changed → persist via MenuKind::write_widgets.
        let widget_list = MenuWidgetListModel::builder()
            .launch(MenuWidgetListInit {
                widgets: kind.read_widgets(),
                draw_border: true,
            })
            .forward(sender.input_sender(), |msg| match msg {
                MenuWidgetListOutput::Changed(widgets) => {
                    MenuConfigPanelInput::WidgetListChanged(widgets)
                }
            });

        // Reactive effect: a config reload (mshellctl, hand-edit
        // YAML, …) fires `tracked_widgets()` → we forward to the
        // factory via SetWidgetsEffect so the UI repaints in
        // place instead of waiting for an mshell restart.
        let mut effects = EffectScope::new();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let widgets = kind.tracked_widgets();
            sender_clone.input(MenuConfigPanelInput::WidgetListEffect(widgets));
        });

        let model = MenuConfigPanelModel {
            kind,
            settings,
            widget_list,
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MenuConfigPanelInput::WidgetListChanged(widgets) => {
                self.kind.write_widgets(widgets);
            }
            MenuConfigPanelInput::WidgetListEffect(widgets) => {
                self.widget_list
                    .emit(MenuWidgetListInput::SetWidgetsEffect(widgets));
            }
        }
    }
}
