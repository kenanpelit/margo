//! Window-rule, tag-rule and layout-placement methods on `MargoState`.
//!
//! Extracted from `state.rs` (roadmap Q1 / state.rs split): the cluster that
//! decides *where and how* a client lands — `default_layout`, per-tag rule
//! application, window-rule matching + float geometry, and the Wayland
//! toplevel-identity refresh. Pure `MargoState` glue; no new types. Kept as a
//! sibling so editing rule logic doesn't recompile the whole state machine.

use super::*;

impl MargoState {
    pub fn default_layout(&self) -> LayoutId {
        LayoutId::from_name(&self.config.default_layout).unwrap_or(LayoutId::Tile)
    }

    /// Look up the "home monitor" for a given tag bitmask, by matching
    /// any single bit in the mask against `tagrule = id:N,monitor_name:X`
    /// entries. Returns the monitor index if exactly one tag is set in
    /// the mask AND a tagrule pins it. Used by `view_tag` and
    /// `new_toplevel` to route cross-monitor.
    pub fn tag_home_monitor(&self, tagmask: u32) -> Option<usize> {
        if tagmask == 0 {
            return None;
        }
        // Translate single-bit mask to 1-indexed tag id.
        let id = if tagmask.is_power_of_two() {
            (tagmask.trailing_zeros() + 1) as i32
        } else {
            // Multi-tag mask — use the lowest set bit.
            ((tagmask & tagmask.wrapping_neg()).trailing_zeros() + 1) as i32
        };
        let name = self
            .config
            .tag_rules
            .iter()
            .find(|r| r.id == id && r.monitor_name.is_some())
            .and_then(|r| r.monitor_name.clone())?;
        self.monitors.iter().position(|m| m.name == name)
    }

    pub fn apply_tag_rules_to_monitor(&mut self, mon_idx: usize) {
        // Tags with an explicit Settings → Tiling Layout (`taglayout`)
        // directive: that per-tag layout (already seeded into `ltidxs`)
        // wins over a tagrule's `layout_name`, so a layout picked in the
        // UI isn't clobbered by a blanket `tagrule = …,layout_name:…`.
        // The rule's mfact / nmaster / wallpaper still apply.
        let taglayout_tags: std::collections::HashSet<usize> = self
            .config
            .taglayouts
            .iter()
            .map(|(t, _)| *t as usize)
            .collect();
        let Some(mon) = self.monitors.get_mut(mon_idx) else {
            return;
        };

        for rule in &self.config.tag_rules {
            if rule.id <= 0 || rule.id as usize > crate::MAX_TAGS {
                continue;
            }
            if let Some(name) = &rule.monitor_name {
                if name != &mon.name {
                    continue;
                }
            }

            let tag = rule.id as usize;
            if let Some(layout_name) = &rule.layout_name {
                if !taglayout_tags.contains(&tag) {
                    if let Some(layout) = LayoutId::from_name(layout_name) {
                        mon.pertag.ltidxs[tag] = layout;
                    }
                }
            }
            if rule.mfact > 0.0 {
                mon.pertag.mfacts[tag] = rule.mfact.clamp(0.05, 0.95);
            }
            if rule.nmaster > 0 {
                mon.pertag.nmasters[tag] = rule.nmaster as u32;
            }
            if let Some(wp) = &rule.wallpaper {
                mon.pertag.wallpapers[tag] = wp.clone();
            }
        }
    }

    /// Move keyboard focus + cursor "home" onto the given monitor. Does
    /// NOT change the monitor's current tagset — the caller (view_tag,
    /// focus_mon) is responsible for that. Used by view_tag's tag-home
    /// redirect: if the user presses super+N for a tag pinned to another
    /// monitor, we warp here first so the upcoming view operation
    /// happens in the right place.
    pub fn warp_focus_to_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        let area = self.monitors[mon_idx].monitor_area;
        // Center the pointer on the target monitor so subsequent
        // sloppy-focus / focus-under lookups land on this output.
        self.input_pointer.x = (area.x + area.width / 2) as f64;
        self.input_pointer.y = (area.y + area.height / 2) as f64;
        self.focus_first_visible_or_clear(mon_idx);
    }

    pub(crate) fn focus_first_visible_or_clear(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            self.focus_surface(None);
            return;
        }

        let tagset = self.monitors[mon_idx].current_tagset();
        if let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.is_visible_on(mon_idx, tagset))
        {
            self.monitors[mon_idx].selected = Some(idx);
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
        } else {
            self.monitors[mon_idx].selected = None;
            self.focus_surface(None);
        }
    }

    pub(crate) fn update_pertag_for_tagset(&mut self, mon_idx: usize, tagmask: u32) {
        let Some(mon) = self.monitors.get_mut(mon_idx) else {
            return;
        };

        mon.pertag.prevtag = mon.pertag.curtag;
        mon.pertag.curtag = if tagmask.count_ones() == 1 {
            tagmask.trailing_zeros() as usize + 1
        } else {
            0
        };
    }

    /// Why a window-rule reapply is happening. Lets the single
    /// reapply path log meaningfully and (in future) skip rule subsets
    /// that don't make sense for a given trigger (e.g. `tags:`
    /// shouldn't move a client on `Reload`).
    pub(crate) fn apply_window_rules(&self, client: &mut MargoClient) {
        // Pre-mount path (X11 + initial XDG before the client is in
        // `self.clients`). The post-mount equivalent is
        // [`reapply_rules`].
        let rules = self.matching_window_rules(&client.app_id, &client.title);
        Self::apply_matched_window_rules(&self.monitors, client, &rules);
    }

    pub(crate) fn apply_matched_window_rules(
        monitors: &[MargoMonitor],
        client: &mut MargoClient,
        rules: &[WindowRule],
    ) {
        // Placement (`tags` / `monitor`) is a *one-time* decision, applied the
        // first time a rule matches (initial map, or the first time a late
        // app_id/title settles). After that the window's location belongs to
        // the user — `summon`, `tag`, `toggletag`, a drag, etc. Re-asserting it
        // on every later reapply (e.g. a browser updating its title on each
        // click) would yank a summoned window straight back to its rule tag.
        // Visual rules below keep applying on every reapply.
        let place = !client.rule_placement_done;
        let mut placed = false;
        for rule in rules {
            if place && rule.tags != 0 {
                client.tags = rule.tags;
                placed = true;
            }
            if place
                && let Some(monitor_name) = &rule.monitor
                && let Some(mon_idx) = monitors.iter().position(|mon| &mon.name == monitor_name)
            {
                client.monitor = mon_idx;
                placed = true;
            }

            if let Some(value) = rule.is_floating {
                client.is_floating = value;
            }
            if let Some(value) = rule.is_fullscreen {
                client.is_fullscreen = value;
            }
            if let Some(value) = rule.is_fake_fullscreen {
                client.is_fake_fullscreen = value;
            }
            if let Some(value) = rule.no_border {
                client.no_border = value;
            }
            if let Some(value) = rule.no_shadow {
                client.no_shadow = value;
            }
            if let Some(value) = rule.no_radius {
                client.no_radius = value;
            }
            if let Some(value) = rule.no_animation {
                client.no_animation = value;
            }
            if let Some(value) = rule.border_width {
                client.border_width = value;
            }
            if let Some(value) = rule.open_silent {
                client.open_silent = value;
            }
            if let Some(value) = rule.tag_silent {
                client.tag_silent = value;
            }
            if let Some(value) = rule.is_named_scratchpad {
                client.is_named_scratchpad = value;
            }
            if let Some(value) = rule.is_unglobal {
                client.is_unglobal = value;
            }
            if let Some(value) = rule.is_global {
                client.is_global = value;
            }
            if let Some(value) = rule.is_overlay {
                client.is_overlay = value;
            }
            if let Some(value) = rule.no_focus {
                client.no_focus = value;
            }
            if let Some(value) = rule.no_fade_in {
                client.no_fade_in = value;
            }
            if let Some(value) = rule.no_fade_out {
                client.no_fade_out = value;
            }
            if let Some(value) = rule.is_term {
                client.is_term = value;
            }
            if let Some(value) = rule.allow_csd {
                client.allow_csd = value;
            }
            if let Some(value) = rule.force_fake_maximize {
                client.force_fake_maximize = value;
            }
            if let Some(value) = rule.force_tiled_state {
                client.force_tiled_state = value;
                if value {
                    client.is_floating = false;
                }
            }
            if let Some(value) = rule.no_swallow {
                client.no_swallow = value;
            }
            if let Some(value) = rule.no_blur {
                client.no_blur = value;
            }
            if let Some(value) = rule.canvas_no_tile {
                client.canvas_no_tile = value;
            }
            if let Some(value) = rule.scroller_proportion {
                client.scroller_proportion = value.clamp(0.1, 1.0);
            }
            if let Some(value) = rule.scroller_proportion_single {
                client.scroller_proportion_single = value.clamp(0.1, 1.0);
            }
            if let Some(value) = rule.focused_opacity {
                client.focused_opacity = value.clamp(0.0, 1.0);
            }
            if let Some(value) = rule.unfocused_opacity {
                client.unfocused_opacity = value.clamp(0.0, 1.0);
            }
            // Per-window animation-type overrides. The rule's
            // `animation_type_open` / `animation_type_close` win over
            // the global config when the window opens or closes —
            // `finalize_initial_map` and `toplevel_destroyed` already
            // read these per-client fields and only fall back to the
            // global `Config::animation_type_*` when they're `None`.
            if let Some(value) = rule.animation_type_open.as_ref() {
                client.animation_type_open = Some(value.clone());
            }
            if let Some(value) = rule.animation_type_close.as_ref() {
                client.animation_type_close = Some(value.clone());
            }
            // Niri-style additions.
            if rule.min_width > 0 {
                client.min_width = rule.min_width;
            }
            if rule.min_height > 0 {
                client.min_height = rule.min_height;
            }
            if rule.max_width > 0 {
                client.max_width = rule.max_width;
            }
            if rule.max_height > 0 {
                client.max_height = rule.max_height;
            }
            if let Some(focused) = rule.open_focused {
                // open_focused=false → equivalent to no_focus=true
                client.no_focus = !focused;
            }
            if let Some(value) = rule.block_out_from_screencast {
                client.block_out_from_screencast = value;
            }
            if rule.width > 0
                || rule.height > 0
                || rule.width_fraction.is_some()
                || rule.height_fraction.is_some()
                || rule.offset_x != 0
                || rule.offset_y != 0
            {
                client.is_floating = true;
                client.float_geom = Self::rule_float_geometry_for(monitors, client.monitor, rule);
            }
        }
        // Lock placement after the first time a rule actually set a tag /
        // monitor, so later reapplies (title changes) leave the window where
        // the user has since put it.
        if placed {
            client.rule_placement_done = true;
        }
        // Fallback: if a rule flagged `isfloating:1` but didn't
        // give any size / offset hint, `client.float_geom` stays
        // at the (0,0,0,0) default. The arrange path then sees
        // `float_geom.width == 0` and *skips* applying it, leaving
        // the toplevel sized to 0×0 → invisible. Synthesize a
        // sensible default geometry from the empty rule so the
        // window gets the same 60 %-of-work-area treatment as a
        // size-bearing rule. Same code path, just sourced from
        // the empty rule's defaults (offsets = 0, no fractions).
        if client.is_floating && client.float_geom.width == 0 {
            let empty_rule = margo_config::WindowRule::default();
            client.float_geom =
                Self::rule_float_geometry_for(monitors, client.monitor, &empty_rule);
        }
        // After all matched rules are applied, clamp the floating geometry
        // to any size constraints picked up.
        clamp_size(
            &mut client.float_geom.width,
            &mut client.float_geom.height,
            client.min_width,
            client.min_height,
            client.max_width,
            client.max_height,
        );
    }

    fn rule_float_geometry(&self, mon_idx: usize, rule: &WindowRule) -> Rect {
        Self::rule_float_geometry_for(&self.monitors, mon_idx, rule)
    }

    fn rule_float_geometry_for(
        monitors: &[MargoMonitor],
        mon_idx: usize,
        rule: &WindowRule,
    ) -> Rect {
        let area = monitors
            .get(mon_idx)
            .map(|mon| mon.work_area)
            .unwrap_or_else(|| Rect::new(0, 0, 1280, 720));
        // Monitor-fraction (`width:50%`) wins over absolute pixels
        // when both are set on the same rule — mango 0.13's
        // flexible-window-rules. Falls back to the legacy 60 %
        // default when neither key is present.
        let width = if let Some(frac) = rule.width_fraction {
            ((area.width as f32) * frac).round() as i32
        } else if rule.width > 0 {
            rule.width.min(area.width)
        } else {
            (area.width as f32 * 0.6) as i32
        };
        let height = if let Some(frac) = rule.height_fraction {
            ((area.height as f32) * frac).round() as i32
        } else if rule.height > 0 {
            rule.height.min(area.height)
        } else {
            (area.height as f32 * 0.6) as i32
        };

        Rect::new(
            area.x + (area.width - width) / 2 + rule.offset_x,
            area.y + (area.height - height) / 2 + rule.offset_y,
            width,
            height,
        )
    }

    pub(crate) fn refresh_wayland_toplevel_identity(
        &mut self,
        window: &Window,
        toplevel: &ToplevelSurface,
    ) {
        let (app_id, title) = read_toplevel_identity(toplevel);
        let Some(idx) = self
            .clients
            .iter()
            .position(|client| client.window == *window)
        else {
            return;
        };

        let (app_id_changed, title_changed, old_monitor, handle) = {
            let client = &mut self.clients[idx];
            let app_id_changed = client.app_id != app_id;
            let title_changed = client.title != title;
            if !app_id_changed && !title_changed {
                return;
            }

            let old_monitor = client.monitor;
            let handle = client.foreign_toplevel_handle.clone();
            client.app_id = app_id.clone();
            client.title = title.clone();
            (app_id_changed, title_changed, old_monitor, handle)
        };

        if let Some(handle) = handle {
            if app_id_changed {
                handle.send_app_id(&app_id);
            }
            if title_changed {
                handle.send_title(&title);
            }
            handle.send_done();
        }

        // Cached at config-load time (`config_has_title_rules`) instead
        // of re-scanning every window rule on each title commit.
        let should_reapply_rules = (app_id_changed && !app_id.is_empty())
            || (title_changed && !title.is_empty() && self.title_rules_exist);

        if should_reapply_rules && self.reapply_rules(idx, WindowRuleReason::AppIdSettled) {
            let new_monitor = self.clients[idx].monitor;
            if old_monitor != new_monitor {
                self.arrange_monitor(old_monitor);
            }
            self.arrange_monitor(new_monitor);
            self.mark_state_dirty();
        } else if title_changed || app_id_changed {
            // Even when no rule reapply was needed (the client just
            // changed its title — e.g. browser tab switch — and no
            // title-keyed rules exist), noctalia / waybar-dwl still
            // care about the new title / app_id for their focused-
            // window indicator. Mango broadcasts on every title
            // commit; without this the bar would freeze on the
            // previous title until something else triggered a
            // broadcast.
            self.mark_state_dirty();
        }
    }
}
