//! Layout dispatch actions: set/switch/adaptive layouts, master count
//! and factor, column proportions and gaps.

use super::{MAX_GAP, MAX_NMASTER, layout_toast};
use crate::layout::LayoutId;
use crate::state::MargoState;

impl MargoState {
    pub fn set_layout(&mut self, name: &str) {
        if let Some(layout) = LayoutId::from_name(name) {
            let mon_idx = self.focused_monitor();
            if mon_idx >= self.monitors.len() {
                return;
            }
            let curtag = self.monitors[mon_idx].pertag.curtag;
            self.monitors[mon_idx].pertag.ltidxs[curtag] = layout;
            // User explicitly picked a layout — adaptive auto-layout
            // must back off on this tag so its choice survives every
            // subsequent arrange pass. Reset by `view_tag` switching
            // to a tag that's never been touched by `setlayout` and
            // letting auto-layout pick again.
            self.monitors[mon_idx].pertag.user_picked_layout[curtag] = true;
            self.arrange_monitor(mon_idx);
        }
    }

    /// Adaptive layout heuristic: pick the most ergonomic layout for
    /// the current tag based on visible-client count + monitor aspect
    /// ratio. Called from `arrange_monitor` when `Config::auto_layout`
    /// is on. Skipped on tags where the user has explicitly called
    /// `setlayout` (sticky `pertag.user_picked_layout` flag) so a
    /// deliberate user choice is never overridden.
    pub(crate) fn maybe_apply_adaptive_layout(&mut self, mon_idx: usize) {
        let curtag = self.monitors[mon_idx].pertag.curtag;
        if self.monitors[mon_idx]
            .pertag
            .user_picked_layout
            .get(curtag)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        let mon_area = self.monitors[mon_idx].monitor_area;

        // Count tile-eligible visible clients (skip floating /
        // fullscreen / scratchpad / minimised — they don't take up a
        // tile slot).
        let count = self
            .clients
            .iter()
            .filter(|c| c.is_visible_on(mon_idx, tagset) && c.is_tiled())
            .count();
        if count == 0 {
            // Empty tag — keep whatever's set so the user sees the
            // *next* arrival land in a sensible layout for one.
            return;
        }

        let aspect = if mon_area.height > 0 {
            mon_area.width as f32 / mon_area.height as f32
        } else {
            16.0 / 9.0
        };
        let very_wide = aspect >= 2.4; // ultrawide / 32:9
        let wide = aspect >= 1.5; // 16:9 / 16:10
        let portrait = aspect <= 0.9; // rotated panels

        // Heuristic. Tuned for the user's two-monitor setup
        // (DP-3 2560x1440 → wide; eDP-1 1920x1200 → wide-ish):
        //
        //   1 client  → monocle    (no point splitting space for one)
        //   2 clients → tile        (master/stack ratio classic)
        //   3-5 wide  → scroller    (niri-style horizontal tracks)
        //   3-5 portrait → tile
        //   6+  wide / ultrawide → grid / scroller (long horizontal track)
        //   6+  portrait → grid
        //
        // The thresholds and choices are deliberately conservative —
        // adaptive should "feel right" 90% of the time, never wrong.
        // A user who wants a different mapping toggles it off and
        // bumps `setlayout` directly per tag.
        let chosen = match (count, very_wide, wide, portrait) {
            (1, _, _, _) => crate::layout::LayoutId::Monocle,
            (2, _, _, _) => crate::layout::LayoutId::Tile,
            (3..=5, _, true, _) => crate::layout::LayoutId::Scroller,
            (3..=5, _, _, _) => crate::layout::LayoutId::Tile,
            (_, true, _, _) => crate::layout::LayoutId::Scroller,
            (_, _, true, _) => crate::layout::LayoutId::Grid,
            (_, _, _, true) => crate::layout::LayoutId::Grid,
            _ => crate::layout::LayoutId::Tile,
        };

        if self.monitors[mon_idx].pertag.ltidxs[curtag] != chosen {
            tracing::info!(
                "auto_layout: tag={} clients={} aspect={:.2} → {:?}",
                curtag,
                count,
                aspect,
                chosen,
            );
            self.monitors[mon_idx].pertag.ltidxs[curtag] = chosen;
        }
    }

    pub fn switch_layout(&mut self) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let current = self.monitors[mon_idx].current_layout().name();
        let layouts: Vec<String> = if self.config.circle_layouts.is_empty() {
            vec![
                "tile", "scroller", "grid", "monocle", "deck", "floating", "mosaic",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        } else {
            self.config.circle_layouts.clone()
        };
        if layouts.is_empty() {
            return;
        }
        let current_pos = layouts.iter().position(|name| name == current).unwrap_or(0);
        let next = layouts[(current_pos + 1) % layouts.len()].clone();
        self.set_layout(&next);
        self.notify_layout(&next);
    }

    /// Fire an OSD toast telling the user the active layout just
    /// changed. Called from `switch_layout` (cycle) and from the
    /// `setlayout` dispatch handler (explicit pick) — not from
    /// `set_layout` itself, because that's also called internally
    /// for window-rule application and we don't want to toast on
    /// every rule-driven re-arrangement.
    pub fn notify_layout(&self, name: &str) {
        // W3.5: enrich the toast with position-in-cycle context so
        // users navigating the layout catalogue see where they
        // are. Format: `<name> (<pos>/<total>) → <next>`. Falls
        // back to bare name when not in `circle_layout`.
        let cycle: Vec<String> = if self.config.circle_layouts.is_empty() {
            vec![]
        } else {
            self.config.circle_layouts.clone()
        };
        let body = if let Some(pos) = cycle.iter().position(|n| n == name) {
            let total = cycle.len();
            let next = &cycle[(pos + 1) % total];
            format!("{name}  ({}/{total}) → next: {next}", pos + 1)
        } else {
            name.to_string()
        };
        layout_toast(&body);
    }

    /// Toast for layout-adjacent actions (proportion preset,
    /// gap toggle). Same look-and-feel as `notify_layout` so the
    /// user can rely on the in-corner toast giving consistent
    /// state feedback for layout-cycle keybinds.
    pub fn notify_layout_state(&self, action: &str, value: &str) {
        layout_toast(&format!("{action}: {value}"));
    }

    pub fn set_focused_proportion(&mut self, proportion: f32) {
        if !proportion.is_finite() {
            return;
        }
        if let Some(idx) = self.focused_client_idx() {
            self.clients[idx].scroller_proportion = proportion.clamp(0.1, 1.0);
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn switch_focused_proportion_preset(&mut self) {
        if self.config.scroller_proportion_presets.is_empty() {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current = self.clients[idx].scroller_proportion;
        let presets = &self.config.scroller_proportion_presets;
        let current_pos = presets
            .iter()
            .position(|value| (*value - current).abs() < 0.01)
            .unwrap_or(0);
        let next_proportion = presets[(current_pos + 1) % presets.len()];
        self.clients[idx].scroller_proportion = next_proportion;
        let mon_idx = self.clients[idx].monitor;
        self.arrange_monitor(mon_idx);
        // W3.5: toast feedback for the cycling action.
        self.notify_layout_state("scroller proportion", &format!("{:.2}", next_proportion));
    }

    pub fn inc_nmaster(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        let current = self.monitors[mon_idx].pertag.nmasters[curtag];
        self.monitors[mon_idx].pertag.nmasters[curtag] =
            current.saturating_add_signed(delta).min(MAX_NMASTER);
        self.arrange_monitor(mon_idx);
    }

    pub fn set_mfact(&mut self, delta: f32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        let current = self.monitors[mon_idx].pertag.mfacts[curtag];
        let next = current + delta;
        if !next.is_finite() {
            return;
        }
        self.monitors[mon_idx].pertag.mfacts[curtag] = next.clamp(0.05, 0.95);
        self.arrange_monitor(mon_idx);
    }

    /// Reset the current tag's master factor on `mon_idx` to its configured
    /// value — the tag rule's `mfact` when one applies to this tag on this
    /// monitor, else `default_mfact` — the same precedence a fresh tag gets
    /// (`reset_pertag_slot` → `apply_tag_rules_to_monitor`).
    pub fn reset_mfact(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        let curtag = mon.pertag.curtag;
        let rule_mfact = self
            .config
            .tag_rules
            .iter()
            .filter(|r| r.id > 0 && r.id as usize == curtag && r.mfact > 0.0)
            .filter(|r| r.monitor_name.as_ref().is_none_or(|n| *n == mon.name))
            .map(|r| r.mfact)
            // Later rules override earlier ones, as in
            // `apply_tag_rules_to_monitor`.
            .next_back();
        let target = rule_mfact.unwrap_or(self.config.default_mfact);
        if !target.is_finite() {
            return;
        }
        self.monitors[mon_idx].pertag.mfacts[curtag] = target.clamp(0.05, 0.95);
        self.arrange_monitor(mon_idx);
    }

    pub fn toggle_gaps(&mut self) {
        self.enable_gaps = !self.enable_gaps;
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.notify_layout_state("gaps", if self.enable_gaps { "on" } else { "off" });
    }

    pub fn inc_gaps(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if let Some(mon) = self.monitors.get_mut(mon_idx) {
            let step = |gap: i32| gap.saturating_add(delta).clamp(0, MAX_GAP);
            mon.gappih = step(mon.gappih);
            mon.gappiv = step(mon.gappiv);
            mon.gappoh = step(mon.gappoh);
            mon.gappov = step(mon.gappov);
            self.arrange_monitor(mon_idx);
        }
    }
}
