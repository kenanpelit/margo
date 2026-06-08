//! Tabbed window groups (Hyprland `togglegroup` family) on `MargoState`.
//!
//! A *group* is a set of toplevels that share a single layout slot and
//! display one member at a time, like browser tabs. margo already has a
//! `Deck` layout (stacked, no tabs); groups reuse the same "collapse N
//! windows to one rect" idea but make it per-window and explicit, and
//! draw a tab strip (see `render::group_tabs`) so the user can see and
//! pick members.
//!
//! ## Model
//!
//! Group identity lives on the client: [`MargoClient::group_id`]
//! (`Option<u32>`) plus a [`MargoClient::group_active`] bool. The
//! invariants this module maintains:
//!
//!   * A group has **≥ 1** members. A 1-member group is degenerate —
//!     `togglegroup` and member-removal both dissolve it back to an
//!     ungrouped window so there's never a "group of one" tab strip.
//!   * Exactly **one** member has `group_active = true`.
//!   * Hidden (non-active) members return `false` from
//!     `is_visible_on`, so layout/focus/render all skip them and only
//!     the active member occupies the slot.
//!
//! Group ids come from [`MargoState::next_group_id`], a monotonic
//! counter — never reused, so stale `group_id`s can't alias a new
//! group. Because identity rides on the client (not an index-keyed
//! registry), the existing `clients.remove` + `shift_indices_after_remove`
//! churn needs no group-specific index fix-up; we only re-assert the
//! one-active-member invariant after a member leaves.
//!
//! ## Additivity
//!
//! Nothing here runs unless the user invokes a `*group*` dispatch verb
//! or a `group:1` windowrule fires. A fresh session has zero groups and
//! behaves exactly as before.

use super::{FocusTarget, MargoState};

impl MargoState {
    /// Visible tiled clients on a monitor's current tagset, in
    /// `clients` order. Used to find a window's "layout neighbour"
    /// for `togglegroup`. Note: hidden group members are already
    /// excluded by `is_visible_on`, so this returns one entry per
    /// group (the active member) — exactly the set the user sees.
    fn visible_tiled_on_focused_mon(&self, mon_idx: usize) -> Vec<usize> {
        let tagset = self.monitors[mon_idx].current_tagset();
        self.clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset) && c.is_tiled())
            .map(|(i, _)| i)
            .collect()
    }

    /// Ordered member indices of a group, in `clients` order (stable
    /// tab order). Empty when `gid` has no members.
    fn group_members(&self, gid: u32) -> Vec<usize> {
        self.clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.group_id == Some(gid))
            .map(|(i, _)| i)
            .collect()
    }

    /// Position of the active member within `group_members(gid)`, if any.
    fn group_active_pos(&self, gid: u32) -> Option<usize> {
        self.group_members(gid)
            .iter()
            .position(|&i| self.clients[i].group_active)
    }

    /// Make exactly `idx` the active member of its group, clearing the
    /// flag on every sibling. No-op if `idx` isn't grouped.
    fn set_group_active(&mut self, idx: usize) {
        let Some(gid) = self.clients[idx].group_id else {
            return;
        };
        for c in self.clients.iter_mut() {
            if c.group_id == Some(gid) {
                c.group_active = false;
            }
        }
        self.clients[idx].group_active = true;
    }

    /// Dissolve a group that has shrunk to a single member (or zero):
    /// the lone survivor becomes a normal ungrouped window. Keeps the
    /// "no group of one" invariant.
    fn dissolve_if_degenerate(&mut self, gid: u32) {
        let members = self.group_members(gid);
        if members.len() <= 1 {
            for &i in &members {
                self.clients[i].group_id = None;
                self.clients[i].group_active = false;
            }
        }
    }

    /// Re-assert the one-active-member invariant for `gid` (e.g. after
    /// the active member was removed / ungrouped). Promotes the first
    /// remaining member when none is active. Dissolves a group of one.
    pub(crate) fn repair_group(&mut self, gid: u32) {
        self.dissolve_if_degenerate(gid);
        let members = self.group_members(gid);
        if members.is_empty() {
            return;
        }
        if self.group_active_pos(gid).is_none() {
            self.set_group_active(members[0]);
        }
    }

    /// Called from the client-removal chokepoint *before* a client is
    /// dropped from `self.clients`, so we can repair its group once
    /// it's gone. Returns the group id (if any) for the caller to pass
    /// to [`Self::repair_group`] after `clients.remove`.
    pub(crate) fn group_of(&self, idx: usize) -> Option<u32> {
        self.clients.get(idx).and_then(|c| c.group_id)
    }

    // ── Dispatch verbs ──────────────────────────────────────────────

    /// `togglegroup` — group or ungroup the focused window.
    ///
    ///   * Focused window already grouped → pull it out of the group
    ///     (the group repairs / dissolves behind it).
    ///   * Focused window ungrouped → merge it with its layout
    ///     neighbour (the next visible tiled window on the same
    ///     monitor). If the neighbour is itself grouped, the focused
    ///     window joins that existing group; otherwise a fresh group
    ///     is created holding both.
    ///
    /// No-op when `groups_locked` (Hyprland `lockgroups 1`) or when an
    /// ungrouped focused window has no neighbour to merge with.
    pub fn toggle_group(&mut self) {
        if self.groups_locked {
            tracing::debug!("togglegroup: groups locked, ignoring");
            return;
        }
        let Some(focused) = self.focused_client_idx() else {
            return;
        };
        let mon_idx = self.clients[focused].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }

        if let Some(gid) = self.clients[focused].group_id {
            // Already grouped → ungroup this one window.
            self.clients[focused].group_id = None;
            self.clients[focused].group_active = false;
            self.repair_group(gid);
            self.arrange_monitor(mon_idx);
            self.mark_state_dirty();
            tracing::info!(
                client = focused,
                group = gid,
                "togglegroup: ungrouped window"
            );
            return;
        }

        // Ungrouped → merge with the layout neighbour.
        let visible = self.visible_tiled_on_focused_mon(mon_idx);
        let Some(pos) = visible.iter().position(|&i| i == focused) else {
            return;
        };
        if visible.len() < 2 {
            tracing::debug!("togglegroup: no neighbour to merge with");
            return;
        }
        let neighbour = visible[(pos + 1) % visible.len()];

        let gid = match self.clients[neighbour].group_id {
            // Neighbour is already a group — join it.
            Some(existing) => existing,
            // Neither grouped — mint a fresh group holding the neighbour.
            None => {
                let new_id = self.next_group_id;
                self.next_group_id = self.next_group_id.wrapping_add(1).max(1);
                self.clients[neighbour].group_id = Some(new_id);
                self.clients[neighbour].group_active = true;
                new_id
            }
        };
        self.clients[focused].group_id = Some(gid);
        // The window the user explicitly grouped becomes the visible tab.
        self.set_group_active(focused);
        self.arrange_monitor(mon_idx);
        self.mark_state_dirty();
        tracing::info!(
            client = focused,
            neighbour = neighbour,
            group = gid,
            "togglegroup: merged into group"
        );
    }

    /// `changegroupactive next|prev` — cycle which member of the
    /// focused window's group is displayed, wrapping at the ends.
    /// Moves keyboard focus to the newly-active member. No-op if the
    /// focused window isn't grouped.
    pub fn change_group_active(&mut self, direction: i32) {
        let Some(focused) = self.focused_client_idx() else {
            return;
        };
        let Some(gid) = self.clients[focused].group_id else {
            return;
        };
        let members = self.group_members(gid);
        let len = members.len();
        if len < 2 {
            return;
        }
        let cur = self.group_active_pos(gid).unwrap_or(0);
        let next = if direction >= 0 {
            (cur + 1) % len
        } else {
            (cur + len - 1) % len
        };
        let target = members[next];
        self.activate_group_member(target);
    }

    /// Make `target` the active (displayed) member of its group, take
    /// keyboard focus to it, and re-arrange. Shared by
    /// `changegroupactive` and the tab-strip click/scroll input.
    pub fn activate_group_member(&mut self, target: usize) {
        if target >= self.clients.len() || self.clients[target].group_id.is_none() {
            return;
        }
        let mon_idx = self.clients[target].monitor;
        self.set_group_active(target);
        let window = self.clients[target].window.clone();
        if mon_idx < self.monitors.len() {
            self.monitors[mon_idx].prev_selected = self.monitors[mon_idx].selected;
            self.monitors[mon_idx].selected = Some(target);
            self.arrange_monitor(mon_idx);
        }
        self.focus_surface(Some(FocusTarget::Window(window)));
        self.mark_state_dirty();
    }

    /// `movegroupwindow next|prev` — reorder the focused window within
    /// its group's tab strip by swapping it with the adjacent member in
    /// `clients` order (which is the tab order). No wrap. No-op if the
    /// focused window isn't grouped or is already at the end.
    pub fn move_group_window(&mut self, direction: i32) {
        let Some(focused) = self.focused_client_idx() else {
            return;
        };
        let Some(gid) = self.clients[focused].group_id else {
            return;
        };
        let members = self.group_members(gid);
        let Some(pos) = members.iter().position(|&i| i == focused) else {
            return;
        };
        let len = members.len();
        if len < 2 {
            return;
        }
        let other_pos = if direction >= 0 {
            if pos + 1 >= len {
                return;
            }
            pos + 1
        } else {
            if pos == 0 {
                return;
            }
            pos - 1
        };
        let a = members[pos];
        let b = members[other_pos];
        self.clients.swap(a, b);
        // `selected` slots may now point at the swapped indices; the
        // existing focus machinery re-resolves from the keyboard focus,
        // so a coarse fix-up is enough.
        let mon_idx = self.clients[a]
            .monitor
            .min(self.monitors.len().saturating_sub(1));
        if mon_idx < self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.mark_state_dirty();
    }

    /// `movewindowtogroup` — pull the focused window into the same group
    /// as its layout neighbour (or create one), without toggling it out
    /// if it's already grouped. Distinct from `togglegroup`: this only
    /// ever *adds* to a group, useful for binding "absorb the next
    /// window into my tab strip".
    pub fn move_window_to_group(&mut self) {
        if self.groups_locked {
            return;
        }
        let Some(focused) = self.focused_client_idx() else {
            return;
        };
        if self.clients[focused].group_id.is_some() {
            // Already grouped — nothing to add it to that it isn't in.
            return;
        }
        // Reuse the merge half of togglegroup.
        self.toggle_group();
    }

    /// Auto-group a freshly-mapped window when a `group:1` windowrule
    /// matched it. The new window joins the most-recent existing group
    /// of a same-app-id sibling on the same monitor+tag, or forms a new
    /// group with one if none is grouped yet. Called from
    /// `finalize_initial_map` after rules + placement settle.
    ///
    /// Keyed by app-id so "every terminal opens as a tab of the
    /// terminal group" works with a single
    /// `windowrule = group:1, appid:^kitty$`. Honours `groups_locked`.
    pub(crate) fn maybe_auto_group(&mut self, idx: usize) {
        if self.groups_locked || idx >= self.clients.len() {
            return;
        }
        if self.clients[idx].group_id.is_some() {
            return; // already grouped (e.g. session restore)
        }
        let wants_group = self
            .matching_window_rules(&self.clients[idx].app_id, &self.clients[idx].title)
            .iter()
            .any(|r| r.group == Some(true));
        if !wants_group {
            return;
        }
        let app_id = self.clients[idx].app_id.clone();
        let mon = self.clients[idx].monitor;
        let tags = self.clients[idx].tags;
        if app_id.is_empty() {
            return;
        }
        // Find a same-app sibling sharing this window's monitor + tags.
        let sibling = self.clients.iter().enumerate().find(|(i, c)| {
            *i != idx && c.app_id == app_id && c.monitor == mon && (c.tags & tags) != 0
        });
        let Some((sib_idx, sib)) = sibling else {
            return; // first instance — nothing to group with yet
        };
        let gid = match sib.group_id {
            Some(existing) => existing,
            None => {
                let new_id = self.next_group_id;
                self.next_group_id = self.next_group_id.wrapping_add(1).max(1);
                self.clients[sib_idx].group_id = Some(new_id);
                self.clients[sib_idx].group_active = true;
                new_id
            }
        };
        self.clients[idx].group_id = Some(gid);
        // The newly-opened window becomes the visible tab — matches the
        // "new tab is focused" expectation.
        self.set_group_active(idx);
        if mon < self.monitors.len() {
            self.arrange_monitor(mon);
        }
        self.mark_state_dirty();
        tracing::info!(
            client = idx,
            sibling = sib_idx,
            group = gid,
            app_id = %app_id,
            "auto-group: joined by windowrule group:1"
        );
    }

    /// `lockgroups [on|off|toggle]` — freeze group/ungroup operations.
    pub fn set_groups_locked(&mut self, mode: GroupLock) {
        self.groups_locked = match mode {
            GroupLock::On => true,
            GroupLock::Off => false,
            GroupLock::Toggle => !self.groups_locked,
        };
        tracing::info!(locked = self.groups_locked, "lockgroups");
        self.mark_state_dirty();
    }
}

/// Argument for [`MargoState::set_groups_locked`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupLock {
    On,
    Off,
    Toggle,
}
