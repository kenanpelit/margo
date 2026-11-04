use crate::relm_app::{Shell, ShellInput, WindowGroup};
use mshell_utils::gtk as utils;
use relm4::gtk::glib::SignalHandlerId;
use relm4::{gtk::gdk, gtk::prelude::DisplayExt, gtk::prelude::*, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use tracing::info;

pub(crate) fn setup_monitor_watcher(sender: &ComponentSender<Shell>) {
    let display = gdk::Display::default().expect("No display");
    let monitors = display.monitors();
    let sender_clone = sender.clone();

    monitors.connect_items_changed(move |model, position, _removed, added| {
        for i in position..position + added {
            if let Some(monitor) = utils::monitor_at_position(model, i) {
                let sender_inner = sender_clone.clone();
                let handler_id: Rc<RefCell<Option<SignalHandlerId>>> = Rc::new(RefCell::new(None));
                let handler_id_clone = handler_id.clone();

                let id = monitor.connect_notify_local(Some("connector"), move |m, _| {
                    if m.connector().is_some() {
                        sender_inner.input(ShellInput::SyncMonitors);
                        if let Some(id) = handler_id_clone.borrow_mut().take() {
                            m.disconnect(id);
                        }
                    }
                });
                *handler_id.borrow_mut() = Some(id);
            }
        }
        sender_clone.input(ShellInput::SyncMonitors);
    });
}

pub(crate) fn sync_monitors(
    window_groups: &HashMap<String, WindowGroup>,
    sender: &ComponentSender<Shell>,
) {
    let display = gdk::Display::default().expect("No display");
    let monitors = utils::list_model_to_monitors(&display.monitors());

    let groups: Vec<GroupView<'_, gdk::Monitor>> = window_groups
        .iter()
        .map(|(connector, g)| GroupView {
            connector: connector.as_str(),
            monitor: &g.monitor,
            valid: g.monitor.is_valid(),
        })
        .collect();
    let current: Vec<(String, gdk::Monitor)> = monitors
        .iter()
        .filter_map(|m| m.connector().map(|c| (c.to_string(), m.clone())))
        .collect();
    let plan = plan_sync(&groups, &current);

    info!(remove = ?plan.remove, add = ?plan.add.iter().map(|(c, _)| c).collect::<Vec<_>>(), "Syncing window groups to monitors");
    for connector in plan.remove {
        sender.input(ShellInput::RemoveWindowGroup(connector));
    }
    for (connector, monitor) in plan.add {
        sender.input(ShellInput::AddWindowGroup(connector, monitor.clone()));
    }
}

/// What the reconcile sees of one window group.
pub(crate) struct GroupView<'a, M> {
    pub connector: &'a str,
    pub monitor: &'a M,
    /// `GdkMonitor::is_valid()`: false once GDK dropped the output.
    pub valid: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) struct SyncPlan<'a, M> {
    pub remove: Vec<String>,
    pub add: Vec<(String, &'a M)>,
}

/// Decide which window groups to drop and which monitors need one.
///
/// A group is stale when its connector is gone, its monitor object is no
/// longer valid, or the connector now belongs to a *different* monitor
/// object (`M`'s `==` is object identity for GObjects). The last case is a
/// fast unplug/replug (power cycle, KVM switch, DP link retrain): GDK
/// replaces the `GdkMonitor` under the same connector before our queued sync
/// runs, and matching by name alone left every surface bound to the dead
/// one — no bar or wallpaper on that output until a restart. Such a
/// connector is removed and re-added.
pub(crate) fn plan_sync<'a, M: PartialEq>(
    groups: &[GroupView<'_, M>],
    monitors: &'a [(String, M)],
) -> SyncPlan<'a, M> {
    let mut remove = Vec::new();
    for g in groups {
        let live = monitors
            .iter()
            .any(|(c, m)| c == g.connector && m == g.monitor);
        if !live || !g.valid {
            remove.push(g.connector.to_string());
        }
    }
    let add = monitors
        .iter()
        .filter(|(c, _)| remove.contains(c) || !groups.iter().any(|g| g.connector == c))
        .map(|(c, m)| (c.clone(), m))
        .collect();
    SyncPlan { remove, add }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(connector: &'a str, monitor: &'a u32, valid: bool) -> GroupView<'a, u32> {
        GroupView {
            connector,
            monitor,
            valid,
        }
    }

    #[test]
    fn unchanged_outputs_are_left_alone() {
        let groups = [view("eDP-1", &1, true), view("DP-3", &2, true)];
        let monitors = [("eDP-1".to_string(), 1), ("DP-3".to_string(), 2)];
        let plan = plan_sync(&groups, &monitors);
        assert!(plan.remove.is_empty() && plan.add.is_empty());
    }

    #[test]
    fn unplug_and_plug_by_connector() {
        let groups = [view("eDP-1", &1, true), view("DP-3", &2, true)];
        let monitors = [("eDP-1".to_string(), 1), ("HDMI-A-1".to_string(), 7)];
        let plan = plan_sync(&groups, &monitors);
        assert_eq!(plan.remove, vec!["DP-3".to_string()]);
        assert_eq!(plan.add, vec![("HDMI-A-1".to_string(), &7)]);
    }

    /// Same connector, new monitor object (fast replug): rebuild the group.
    #[test]
    fn replugged_connector_with_a_new_monitor_is_rebuilt() {
        let groups = [view("DP-3", &2, true)];
        let monitors = [("DP-3".to_string(), 9)];
        let plan = plan_sync(&groups, &monitors);
        assert_eq!(plan.remove, vec!["DP-3".to_string()]);
        assert_eq!(plan.add, vec![("DP-3".to_string(), &9)]);
    }

    #[test]
    fn an_invalidated_monitor_is_rebuilt() {
        let groups = [view("DP-3", &2, false)];
        let monitors = [("DP-3".to_string(), 2)];
        let plan = plan_sync(&groups, &monitors);
        assert_eq!(plan.remove, vec!["DP-3".to_string()]);
        assert_eq!(plan.add, vec![("DP-3".to_string(), &2)]);
    }
}
