use crate::bars::bar_widgets::margo_tag::{
    MargoTagInput, MargoTagModel,
};
use futures::StreamExt;
use mshell_common::dynamic_box::dynamic_box::{
    DynamicBoxFactory, DynamicBoxInit, DynamicBoxInput, DynamicBoxModel,
};
use mshell_common::dynamic_box::generic_widget_controller::{
    GenericWidgetController, GenericWidgetControllerExtSafe,
};
use mshell_common::dynamic_box::simple_widget_controller::SimpleWidgetController;
use mshell_services::margo_service;
use mshell_utils::hover_scroll::attach_hover_scroll;
use mshell_utils::margo::{get_active_workspaces, go_down_workspace, go_up_workspace};
use relm4::gtk::{Orientation, RevealerTransitionType};
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk,
    gtk::prelude::*,
};
use std::sync::Arc;
use mshell_margo_client::{MargoEvent, MonitorId, Workspace, WorkspaceId};

#[derive(Clone, Debug)]
pub enum WsRow {
    Divider(MonitorId),
    Workspace(Arc<Workspace>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WsRowKey {
    Divider(MonitorId),
    Workspace(WorkspaceId),
}

pub(crate) struct MargoTagsModel {
    dynamic_box: Controller<DynamicBoxModel<WsRow, WsRowKey>>,
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum MargoTagsInput {}

#[derive(Debug)]
pub(crate) enum MargoTagsOutput {}

pub(crate) struct MargoTagsInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum MargoTagsCommandOutput {
    WorkspacesChanged,
    ActiveWorkspaceChanged,
}

#[relm4::component(pub)]
impl Component for MargoTagsModel {
    type CommandOutput = MargoTagsCommandOutput;
    type Input = MargoTagsInput;
    type Output = MargoTagsOutput;
    type Init = MargoTagsInit;

    view! {
        #[root]
        #[name = "workspace_box"]
        gtk::Box {
            add_css_class: "margo-tags-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        Self::spawn_main_watcher(&sender);

        let divider_orientation = if params.orientation == Orientation::Horizontal {
            Orientation::Vertical
        } else {
            Orientation::Horizontal
        };

        let factory = DynamicBoxFactory::<WsRow, WsRowKey> {
            id: Box::new(|item| match item {
                WsRow::Divider(monitor_id) => WsRowKey::Divider(*monitor_id),
                WsRow::Workspace(workspace) => WsRowKey::Workspace(workspace.id.get()),
            }),
            create: Box::new(move |item| match item {
                WsRow::Workspace(workspace) => {
                    let controller: Controller<MargoTagModel> =
                        MargoTagModel::builder()
                            .launch(workspace.clone())
                            .detach();
                    Box::new(controller) as Box<dyn GenericWidgetController>
                }
                WsRow::Divider(_) => {
                    let sep = gtk::Separator::new(divider_orientation);
                    sep.add_css_class("workspace-divider");
                    Box::new(SimpleWidgetController::new(sep.upcast()))
                }
            }),
            update: None,
        };

        let transition_type = if params.orientation == Orientation::Horizontal {
            RevealerTransitionType::SwingLeft
        } else {
            RevealerTransitionType::SwingUp
        };

        let dynamic: Controller<DynamicBoxModel<WsRow, WsRowKey>> = DynamicBoxModel::builder()
            .launch(DynamicBoxInit {
                factory,
                orientation: params.orientation,
                spacing: 0,
                transition_type,
                transition_duration_ms: 200,
                reverse: false,
                retain_entries: false,
                allow_drag_and_drop: false,
            })
            .detach();

        let model = MargoTagsModel {
            dynamic_box: dynamic,
            orientation: params.orientation,
        };

        let widgets = view_output!();

        widgets.workspace_box.append(model.dynamic_box.widget());

        let hyprland = margo_service();
        let workspaces = hyprland.workspaces.get();

        let workspaces = Self::workspaces_with_dividers(workspaces);

        model
            .dynamic_box
            .sender()
            .send(DynamicBoxInput::SetItems(workspaces))
            .unwrap();

        let _handles = attach_hover_scroll(&widgets.workspace_box, move |_dx, dy, _hovered, _| {
            if dy < 0.0 {
                go_up_workspace()
            } else if dy > 0.0 {
                go_down_workspace()
            }
        });

        ComponentParts { model, widgets }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoTagsCommandOutput::WorkspacesChanged => {
                let hyprland = margo_service();
                let workspaces = hyprland.workspaces.get();

                let workspaces = Self::workspaces_with_dividers(workspaces);

                self.dynamic_box
                    .sender()
                    .send(DynamicBoxInput::SetItems(workspaces))
                    .unwrap();
            }
            MargoTagsCommandOutput::ActiveWorkspaceChanged => {
                let active_workspaces = get_active_workspaces();

                self.dynamic_box.model().for_each_entry(|_, entry| {
                    if let Some(ctrl) = entry
                        .controller
                        .as_ref()
                        .downcast_ref::<Controller<MargoTagModel>>()
                    {
                        let _ = ctrl.sender().send(MargoTagInput::ActiveUpdate(
                            active_workspaces.clone(),
                        ));
                    }
                })
            }
        }
    }
}

impl MargoTagsModel {
    fn spawn_main_watcher(sender: &ComponentSender<Self>) {
        sender.command(move |out, shutdown| {
            async move {
                let hyprland = margo_service();
                let mut events = hyprland.events();
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);

                loop {
                    tokio::select! {
                        () = &mut shutdown_fut => return,
                        event = events.next() => {
                            let Some(event) = event else { continue; };
                            match event {
                                MargoEvent::WorkspaceV2 { .. } => {
                                    let _ = out.send(MargoTagsCommandOutput::ActiveWorkspaceChanged);
                                }
                                MargoEvent::CreateWorkspaceV2 { .. }
                                | MargoEvent::DestroyWorkspaceV2 { .. }
                                | MargoEvent::MoveWorkspaceV2 { .. }
                                | MargoEvent::RenameWorkspace { .. }
                                | MargoEvent::ActiveSpecialV2 { .. }
                                | MargoEvent::MonitorAddedV2 { .. }
                                | MargoEvent::MonitorRemovedV2 { .. } => {
                                    let _ = out.send(MargoTagsCommandOutput::WorkspacesChanged);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        });
    }

    /// Linear 1..N pill row, no per-monitor dividers.
    ///
    /// margo tags are GLOBAL bit positions (1..=9), not per-monitor
    /// like Hyprland workspaces. Same tag id can be active on
    /// monitor A and occupied on monitor B at the same time — they
    /// refer to the same logical bit. Splitting the row by "owner
    /// monitor" the way `wayle-hyprland` does ends up putting tags
    /// on either side of a divider purely based on which output
    /// happens to hold them right now (e.g. `2 3 4 5 6 | 1 7 8 9`
    /// when DP-3 was on tag 2 and eDP-1 was on tag 1, even though
    /// the user thinks of them as a single linear row). The bar
    /// should mirror the user's mental model: nine slots, fixed
    /// position, state determined by the focused monitor.
    ///
    /// `MonitorId` import + the `WsRow::Divider` variant stay for
    /// upstream-compat (the dynamic-box factory still understands
    /// them); we just don't emit any divider rows here.
    fn workspaces_with_dividers(mut workspaces: Vec<Arc<Workspace>>) -> Vec<WsRow> {
        workspaces.sort_by_key(|w| w.id.get());
        workspaces
            .into_iter()
            .filter(|w| !w.name.get().starts_with("special:"))
            .map(WsRow::Workspace)
            .collect()
    }
}
