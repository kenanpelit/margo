//! MRU (most-recently-used) window switcher — niri-style Super+Tab.
//!
//! Holds the modifier, taps Tab to walk a most-recently-used list of windows,
//! and commits the selection when the modifier is released (the same "hold,
//! cycle, release to confirm" pattern the grid overview uses, but as its own
//! lightweight switcher rather than the zoomed grid). Phase 1 here is the
//! model + live-focus cycling + commit/cancel; the thumbnail overlay is a
//! separate render phase that reads [`MruSwitcher::candidates`] + `selected`.
//!
//! Recency comes from each client's `last_focus_serial` (bumped in
//! `focus_surface`), so the order survives clients opening/closing and isn't
//! tied to fragile `Vec` indices. Candidates are stored as stable `Window`
//! handles; we resolve them back to a live client index at each step.

use margo_config::Modifiers;
use smithay::desktop::Window;

use crate::state::MargoState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MruScope {
    /// Every window on every output/workspace.
    #[default]
    All,
    /// Windows on the active output.
    Output,
    /// Windows on the active workspace (output + visible tags).
    Workspace,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MruFilter {
    /// All windows.
    #[default]
    All,
    /// Only windows with the same app-id as the currently-focused one.
    AppId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MruDirection {
    /// Most-recently-used → least.
    Forward,
    /// Least-recently-used → most.
    Backward,
}

/// Live switcher state. Present only while the switcher is open.
pub struct MruSwitcher {
    /// Candidate windows in MRU order; `candidates[0]` is the window that was
    /// focused when the switcher opened.
    pub candidates: Vec<Window>,
    /// Index into `candidates` of the highlighted window.
    pub selected: usize,
    /// Modifiers that were held when the switcher opened; releasing all of
    /// them commits the selection (see the input handler).
    pub modifier_mask: Modifiers,
    /// Scope + filter the switcher opened with (for the overlay title).
    pub scope: MruScope,
    pub filter: MruFilter,
}

pub fn parse_scope(s: &str) -> MruScope {
    match s.trim().to_lowercase().as_str() {
        "output" => MruScope::Output,
        "workspace" => MruScope::Workspace,
        _ => MruScope::All,
    }
}

pub fn parse_filter(s: &str) -> MruFilter {
    match s.trim().to_lowercase().as_str() {
        "appid" => MruFilter::AppId,
        _ => MruFilter::All,
    }
}

impl MargoState {
    pub fn is_mru_open(&self) -> bool {
        self.mru_switcher.is_some()
    }

    /// Advance using bind args when given, else the configured defaults
    /// (`mru_scope` / `mru_filter`). The keybind/dispatch entry point.
    pub fn mru_advance_args(
        &mut self,
        scope: Option<&str>,
        filter: Option<&str>,
        dir: MruDirection,
    ) {
        let scope = parse_scope(scope.unwrap_or(&self.config.mru_scope));
        let filter = parse_filter(filter.unwrap_or(&self.config.mru_filter));
        self.mru_advance(scope, filter, dir);
    }

    /// Build the MRU-ordered candidate window list for a scope + filter.
    fn mru_candidate_windows(&self, scope: MruScope, filter: MruFilter) -> Vec<Window> {
        let focused = self.focused_client_idx();
        let active_mon = self.focused_monitor();
        let active_tagset = self
            .monitors
            .get(active_mon)
            .map(|m| m.current_tagset())
            .unwrap_or(!0);
        let want_app = focused.map(|i| self.clients[i].app_id.clone());

        let mut idxs: Vec<usize> = (0..self.clients.len())
            .filter(|&i| {
                let c = &self.clients[i];
                let scope_ok = match scope {
                    MruScope::All => true,
                    MruScope::Output => c.monitor == active_mon,
                    MruScope::Workspace => c.monitor == active_mon && (c.tags & active_tagset) != 0,
                };
                let filter_ok = match filter {
                    MruFilter::All => true,
                    MruFilter::AppId => want_app.as_deref() == Some(c.app_id.as_str()),
                };
                scope_ok && filter_ok
            })
            .collect();
        // Most-recently-used first. The focused window (highest serial) leads.
        idxs.sort_by(|&a, &b| {
            self.clients[b]
                .last_focus_serial
                .cmp(&self.clients[a].last_focus_serial)
        });
        idxs.into_iter()
            .map(|i| self.clients[i].window.clone())
            .collect()
    }

    /// Advance the switcher, opening it first if needed. Opening snapshots the
    /// held modifier (`mru_open_mask`, set by the input handler) so the release
    /// handler knows when to commit. Each step live-focuses the selection so
    /// the user sees where they are even before the overlay lands.
    pub fn mru_advance(&mut self, scope: MruScope, filter: MruFilter, dir: MruDirection) {
        if self.mru_switcher.is_none() {
            let candidates = self.mru_candidate_windows(scope, filter);
            if candidates.len() < 2 {
                return; // nothing to switch to
            }
            self.mru_switcher = Some(MruSwitcher {
                candidates,
                selected: 0,
                modifier_mask: self.mru_open_mask,
                scope,
                filter,
            });
        }
        let len = self
            .mru_switcher
            .as_ref()
            .map(|s| s.candidates.len())
            .unwrap_or(0);
        if len == 0 {
            return;
        }
        if let Some(sw) = self.mru_switcher.as_mut() {
            sw.selected = match dir {
                MruDirection::Forward => (sw.selected + 1) % len,
                MruDirection::Backward => (sw.selected + len - 1) % len,
            };
        }
        self.mru_focus_selected();
    }

    /// Focus the currently-selected candidate (preview while cycling).
    fn mru_focus_selected(&mut self) {
        let Some(win) = self
            .mru_switcher
            .as_ref()
            .and_then(|s| s.candidates.get(s.selected).cloned())
        else {
            return;
        };
        if let Some(idx) = self.clients.iter().position(|c| c.window == win) {
            self.activate_window_idx(idx);
        }
    }

    /// Commit: close the switcher, then focus the selected window for real so
    /// it records a fresh `last_focus_serial` (becomes most-recently-used).
    pub fn mru_confirm(&mut self) {
        let win = self
            .mru_switcher
            .as_ref()
            .and_then(|s| s.candidates.get(s.selected).cloned());
        self.mru_switcher = None;
        if let Some(win) = win
            && let Some(idx) = self.clients.iter().position(|c| c.window == win)
        {
            self.activate_window_idx(idx);
        }
        self.request_repaint();
    }

    /// Cancel: re-focus the window that was active when we opened, then close.
    pub fn mru_cancel(&mut self) {
        if let Some(sw) = self.mru_switcher.take() {
            if let Some(orig) = sw.candidates.first().cloned() {
                if let Some(idx) = self.clients.iter().position(|c| c.window == orig) {
                    self.activate_window_idx(idx);
                }
            }
        }
        self.request_repaint();
    }

    /// A window closed — drop it from an open switcher (keeps the cycle valid).
    pub fn mru_remove_window(&mut self, win: &Window) {
        if let Some(sw) = self.mru_switcher.as_mut() {
            if let Some(pos) = sw.candidates.iter().position(|w| w == win) {
                sw.candidates.remove(pos);
                if sw.candidates.len() < 2 {
                    self.mru_switcher = None;
                    self.request_repaint();
                    return;
                }
                if sw.selected >= sw.candidates.len() {
                    sw.selected = 0;
                }
            }
        }
    }
}
