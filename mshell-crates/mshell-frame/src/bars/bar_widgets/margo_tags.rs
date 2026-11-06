use crate::bars::bar_widgets::margo_tag::{MargoTagInput, MargoTagModel};
use crate::bars::bar_widgets::margo_tag_windows::{MargoTagWindowsInit, MargoTagWindowsModel};
use futures::StreamExt;
use mshell_common::dynamic_box::dynamic_box::{
    DynamicBoxFactory, DynamicBoxInit, DynamicBoxInput, DynamicBoxModel,
};
use mshell_common::dynamic_box::generic_widget_controller::{
    GenericWidgetController, GenericWidgetControllerExtSafe,
};
use mshell_config::schema::config::MargoTagsDisplay;
use mshell_margo_client::indicators::TagMasks;
use mshell_margo_client::{MargoEvent, Workspace, WorkspaceId, WorkspaceInfo};
use mshell_services::margo_service;
use mshell_utils::hover_scroll::attach_hover_scroll;
use mshell_utils::margo::{get_active_workspaces, go_down_workspace, go_up_workspace};
use relm4::gtk::{Orientation, RevealerTransitionType};
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk,
    gtk::prelude::*,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum WsRow {
    Workspace(Arc<Workspace>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WsRowKey {
    Workspace(WorkspaceId),
}

pub(crate) struct MargoTagsModel {
    dynamic_box: Controller<DynamicBoxModel<WsRow, WsRowKey>>,
    /// The active tag's window icons (`bars.widgets.margo_tags.window_icons`;
    /// hides itself while the option is off).
    tag_windows: Controller<MargoTagWindowsModel>,
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
        Self::spawn_workspace_list_watcher(&sender);

        let factory = DynamicBoxFactory::<WsRow, WsRowKey> {
            id: Box::new(|item| match item {
                WsRow::Workspace(workspace) => WsRowKey::Workspace(workspace.id.get()),
            }),
            create: Box::new(move |item| match item {
                WsRow::Workspace(workspace) => {
                    let controller: Controller<MargoTagModel> =
                        MargoTagModel::builder().launch(workspace.clone()).detach();
                    Box::new(controller) as Box<dyn GenericWidgetController>
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
                // Instant reveal (no enter animation). A non-zero
                // duration makes the DynamicBox reveal each pill via
                // a GtkRevealer transition, and a transition started
                // before its child has been styled + measured (which
                // is exactly the case on the bar's first paint)
                // animates open to a 0-size child — the tag row then
                // stays collapsed until some later re-render. margo's
                // tag set is fixed at 9, so there's no meaningful
                // enter/exit animation to lose here anyway.
                transition_duration_ms: 0,
                reverse: false,
                retain_entries: false,
                allow_drag_and_drop: false,
            })
            .detach();

        let tag_windows = MargoTagWindowsModel::builder()
            .launch(MargoTagWindowsInit {
                orientation: params.orientation,
            })
            .detach();

        let model = MargoTagsModel {
            dynamic_box: dynamic,
            tag_windows,
            orientation: params.orientation,
        };

        let widgets = view_output!();

        widgets.workspace_box.append(model.dynamic_box.widget());
        widgets.workspace_box.append(model.tag_windows.widget());

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

        // Population now flows entirely through
        // `spawn_workspace_list_watcher`'s snapshot-on-subscribe
        // stream — the watcher's first emission is the current
        // workspaces vec, so the row fills in on the first poll-task
        // wakeup with no race window. The legacy belt-and-suspender
        // stack (sync state.json read, 30s/250ms cold-start poll,
        // 30s/500ms brute-force timer) was deleted with the
        // snapshot-on-subscribe Reactive fix.

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
                        let _ = ctrl
                            .sender()
                            .send(MargoTagInput::ActiveUpdate(active_workspaces.clone()));
                    }
                })
            }
        }
    }
}

impl MargoTagsModel {
    fn spawn_main_watcher(sender: &ComponentSender<Self>) {
        sender.command(move |out, shutdown| async move {
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
        });
    }

    /// Keep the pill row in sync with `margo_service().workspaces`.
    ///
    /// One stream subscription, that's it: `Reactive::watch()` now
    /// yields the current value immediately on subscribe
    /// (snapshot-on-subscribe semantics), so the first iteration of
    /// the loop fills the row. Subsequent `set()` calls on the
    /// workspaces vec — which on margo only happen on membership
    /// changes (monitor hotplug, never in steady state because
    /// `tag_count` is fixed at 9) — drive later updates. The
    /// per-tag inner state (windows count, occupied/active flags)
    /// flows through each `Workspace`'s own Reactive fields, watched
    /// by the child `MargoTagModel` widgets.
    fn spawn_workspace_list_watcher(sender: &ComponentSender<Self>) {
        sender.command(move |out, shutdown| async move {
            let mut stream = margo_service().workspaces.watch();
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => return,
                    next = stream.next() => match next {
                        Some(_) => {
                            let _ = out.send(MargoTagsCommandOutput::WorkspacesChanged);
                        }
                        None => return,
                    },
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
    fn workspaces_with_dividers(mut workspaces: Vec<Arc<Workspace>>) -> Vec<WsRow> {
        workspaces.sort_by_key(|w| w.id.get());
        workspaces
            .into_iter()
            .filter(|w| !w.name.get().starts_with("special:"))
            .map(WsRow::Workspace)
            .collect()
    }
}

/// Bit mask (bit `n - 1` = tag `n`) of the given tags; ids outside 1..=32
/// are ignored.
pub(crate) fn active_mask(infos: &[WorkspaceInfo]) -> u32 {
    infos
        .iter()
        .filter(|w| (1..=32).contains(&w.id))
        .fold(0, |m, w| m | 1u32 << (w.id - 1))
}

/// Which of `count` tags get a pill under `mode` (the lunar-workspaces
/// display modes), as a bit mask:
/// * `All` — every tag;
/// * `ActiveOccupied` — occupied, focused and urgent tags;
/// * `OccupiedPlusNext` — those, plus the first unoccupied tag after the
///   highest occupied one (there is always a pill to open a new tag on).
pub(crate) fn shown_tags(mode: MargoTagsDisplay, masks: TagMasks, active: u32, count: u32) -> u32 {
    let all = match count {
        0 => return 0,
        32.. => u32::MAX,
        n => (1u32 << n) - 1,
    };
    let base = (masks.occupied | masks.urgent | active) & all;
    match mode {
        MargoTagsDisplay::All => all,
        MargoTagsDisplay::ActiveOccupied => base,
        MargoTagsDisplay::OccupiedPlusNext => {
            let occupied = masks.occupied & all;
            let after_last = u32::BITS - occupied.leading_zeros();
            let next = (after_last < count.min(32)).then(|| 1u32 << after_last);
            base | next.unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masks(occupied: u32, urgent: u32) -> TagMasks {
        TagMasks { occupied, urgent }
    }

    #[test]
    fn all_shows_every_tag() {
        assert_eq!(
            shown_tags(MargoTagsDisplay::All, masks(0, 0), 0, 9),
            0b1_1111_1111
        );
        assert_eq!(
            shown_tags(MargoTagsDisplay::All, masks(0, 0), 0, 32),
            u32::MAX
        );
        assert_eq!(shown_tags(MargoTagsDisplay::All, masks(0, 0), 0, 0), 0);
    }

    #[test]
    fn active_occupied_is_occupied_focused_and_urgent() {
        // Tags 1, 3 occupied; 6 focused but empty; 8 urgent.
        let m = masks(0b101, 0b1000_0000);
        assert_eq!(
            shown_tags(MargoTagsDisplay::ActiveOccupied, m, 0b10_0000, 9),
            0b1010_0101
        );
    }

    #[test]
    fn occupied_plus_next_adds_the_slot_after_the_last_occupied_tag() {
        // Tags 1-6 occupied (the owner's DP-3), tag 2 focused: 1-7.
        let m = masks(0b11_1111, 0);
        assert_eq!(
            shown_tags(MargoTagsDisplay::OccupiedPlusNext, m, 0b10, 9),
            0b111_1111
        );
        // A gap is not the "next" slot: 1 and 4 occupied -> 1, 4, 5.
        let m = masks(0b1001, 0);
        assert_eq!(
            shown_tags(MargoTagsDisplay::OccupiedPlusNext, m, 0b1, 9),
            0b1_1001
        );
        // Nothing open: the first tag (and the focused one).
        assert_eq!(
            shown_tags(MargoTagsDisplay::OccupiedPlusNext, masks(0, 0), 0b100, 9),
            0b101
        );
        // The last tag occupied: no slot past the end.
        let m = masks(1 << 8, 0);
        assert_eq!(
            shown_tags(MargoTagsDisplay::OccupiedPlusNext, m, 1 << 8, 9),
            1 << 8
        );
    }

    #[test]
    fn active_mask_ignores_out_of_range_ids() {
        let info = |id| WorkspaceInfo {
            id,
            name: id.to_string(),
        };
        assert_eq!(active_mask(&[info(2), info(0), info(40)]), 0b10);
    }
}
