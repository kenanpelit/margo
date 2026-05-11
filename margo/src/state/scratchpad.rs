//! Scratchpad + summon dispatch methods on `MargoState`.
//!
//! Extracted from `state.rs` (roadmap Q1). Mango-style named
//! scratchpads — a regular toplevel the user keeps "in their pocket":
//! invisible by default, summoned onto the current tag with a single
//! keybind, dismissed back into hiding with the same keybind. Margo's
//! window rules already let the user mark a client
//! `isnamedscratchpad:1` and pin its float geometry; this module
//! provides the toggle / spawn-on-miss machinery that ties it
//! together.
//!
//! Mirrors mango-ext's `toggle_named_scratchpad` +
//! `apply_named_scratchpad` + `switch_scratchpad_client_state` +
//! `show_scratchpad` chain, simplified by skipping canvas-layout
//! per-tag offsets (margo doesn't carry those on `MargoClient` yet).
//!
//! The `summon` action — bring a window to the active monitor's
//! current tag, launching it if missing — lives here too because it
//! shares the same identity-matching helper. The anonymous
//! `toggle_scratchpad` mirrors the same flow against clients that
//! were promoted via the legacy "no name, no title" path.

use super::{matches_rule_text, FocusTarget, MargoState};
use crate::layout::Rect;

impl MargoState {
    /// Find the index of the first client whose `app_id` matches `name`
    /// (regex; same matcher as windowrule appid) and, if `title` is
    /// supplied, whose title also matches. Used by
    /// `toggle_named_scratchpad` to locate an already-running instance
    /// before deciding whether to spawn a new one.
    fn find_client_by_id_or_title(
        &self,
        name: Option<&str>,
        title: Option<&str>,
    ) -> Option<usize> {
        // Use the same regex matcher the windowrule machinery uses so
        // bind authors can write `clipse`, `^clipse$`, or `clip(se|board)`
        // and get consistent semantics. The earlier `.contains()`
        // substring match was a footgun: a user-typed bare `clipse`
        // matched any client whose app_id contained the substring
        // "clipse", which is fine right up until a different toolkit
        // happens to namespace itself with one of the scratchpad
        // names — at which point a regular window silently got
        // promoted to a scratchpad on the next toggle press, with no
        // way to escape short of restarting margo. Anchored or
        // word-boundary-aware patterns (`^clipse$`, `\bwiremix\b`)
        // protect against that.
        let name_pat = name.unwrap_or("");
        let title_pat = title.unwrap_or("");
        for (idx, c) in self.clients.iter().enumerate() {
            let app_match = if name_pat.is_empty() {
                true
            } else {
                matches_rule_text(name_pat, &c.app_id)
            };
            let title_match = if title_pat.is_empty() {
                true
            } else {
                matches_rule_text(title_pat, &c.title)
            };
            if app_match && title_match {
                return Some(idx);
            }
        }
        None
    }

    /// Bring a scratchpad client onto the active tagset and centre it
    /// at its `float_geom` (already populated by the windowrule's
    /// width/height/offsetx/offsety, or falls back to the
    /// `scratchpad_*_ratio` config defaults).
    fn show_scratchpad_client(&mut self, idx: usize) {
        if idx >= self.clients.len() {
            return;
        }

        // Migrate the scratchpad to the *cursor's* monitor when
        // `scratchpad_cross_monitor` is on. Without this, a scratchpad
        // first opened on eDP-1 would always re-show on eDP-1 even
        // after the user moved their cursor to DP-3 — the
        // "imlecin olduğu ekranda değil eDP-1'de açılıyor" symptom.
        //
        // We deliberately use `pointer_monitor()` rather than
        // `focused_monitor()` for the migration target. A scratchpad
        // summon is a "bring it *here*" gesture; if the user is
        // reading docs on DP-3 with cursor there but their last
        // keyboard focus happened to land on eDP-1, they'd still want
        // clipse / wiremix to drop down where the cursor is. Falls
        // back to focused-monitor → client's stored monitor if the
        // pointer hasn't entered any output yet.
        let target_mon_idx = if self.config.scratchpad_cross_monitor {
            self.pointer_monitor()
                .or_else(|| {
                    let f = self.focused_monitor();
                    (f < self.monitors.len()).then_some(f)
                })
                .filter(|i| *i < self.monitors.len())
                .unwrap_or(self.clients[idx].monitor)
        } else {
            self.clients[idx].monitor
        };
        if target_mon_idx >= self.monitors.len() {
            return;
        }
        // Apply the migration before reading work_area / tagset so
        // the scratchpad rect is centred on its new home.
        self.clients[idx].monitor = target_mon_idx;
        let work_area = self.monitors[target_mon_idx].work_area;
        let active_tagset = self.monitors[target_mon_idx].current_tagset();

        // Re-centre the float_geom on the target monitor's work
        // area while preserving the rule-supplied size+offset. The
        // windowrule's offsetx/offsety is a hint about *where on
        // the active monitor* the scratchpad should sit, not an
        // absolute screen position — using the absolute coords
        // baked at first-map time would put the panel on whichever
        // monitor it was originally arranged for.
        let c = &mut self.clients[idx];
        c.is_in_scratchpad = true;
        c.is_scratchpad_show = true;
        c.is_minimized = false;
        c.is_floating = true;
        c.is_fullscreen = false;
        c.is_maximized_screen = false;

        // Decide width/height: prefer windowrule values if they
        // were set, fall back to `scratchpad_*_ratio * work_area`.
        let (w, h) = if c.float_geom.width > 0 && c.float_geom.height > 0 {
            (
                c.float_geom.width.min(work_area.width.max(1)),
                c.float_geom.height.min(work_area.height.max(1)),
            )
        } else {
            (
                (work_area.width as f32 * self.config.scratchpad_width_ratio).round() as i32,
                (work_area.height as f32 * self.config.scratchpad_height_ratio).round() as i32,
            )
        };
        // Recentre on the active monitor's work area, then layer
        // the rule's offset on top so user-tuned positioning still
        // applies (the user has e.g. `offsety:-100` on
        // dropdown-terminal so it docks near the top of whatever
        // monitor it lands on).
        let center_x = work_area.x + (work_area.width - w) / 2;
        let center_y = work_area.y + (work_area.height - h) / 2;
        c.float_geom = Rect {
            x: center_x,
            y: center_y,
            width: w.max(100),
            height: h.max(100),
        };
        c.geom = c.float_geom;
        c.tags = active_tagset; // join the current tagset

        let window = c.window.clone();
        // Re-map at the float position. `map_element(_, _, true)`
        // raises to the top of the scene, which is what we want for a
        // toggled-up scratchpad.
        self.space.map_element(window.clone(), (c.float_geom.x, c.float_geom.y), true);
        self.enforce_z_order();
        self.arrange_monitor(target_mon_idx);
        self.focus_surface(Some(FocusTarget::Window(window)));
    }

    /// Tuck a scratchpad client away. We unmap from the scene so it
    /// doesn't render anywhere, and clear `is_scratchpad_show` so the
    /// next `toggle_named_scratchpad` flips it back on.
    fn hide_scratchpad_client(&mut self, idx: usize) {
        if idx >= self.clients.len() {
            return;
        }
        let window = self.clients[idx].window.clone();
        self.clients[idx].is_scratchpad_show = false;
        self.clients[idx].is_minimized = true;
        self.space.unmap_elem(&window);
        // If this was the focused window, drop focus to the next
        // visible client on the same monitor so the keyboard isn't
        // stranded on a hidden surface.
        let mon_idx = self.clients[idx].monitor;
        if mon_idx < self.monitors.len() {
            self.focus_first_visible_or_clear(mon_idx);
        }
        self.request_repaint();
    }

    /// Toggle the show/hide state of a single scratchpad client.
    fn switch_scratchpad_state(&mut self, idx: usize) {
        if idx >= self.clients.len() {
            return;
        }
        if self.clients[idx].is_scratchpad_show {
            self.hide_scratchpad_client(idx);
        } else {
            self.show_scratchpad_client(idx);
        }
    }

    /// Public action: toggle the visibility of a named scratchpad.
    ///
    /// `name`  — appid pattern (substring match, case-insensitive).
    /// `title` — optional title pattern. Both must match if supplied.
    /// `spawn` — shell command to launch if no running client matches;
    ///           the next call after the spawn picks up the new client
    ///           (its windowrule should set `isnamedscratchpad:1` so
    ///           it lands hidden, ready to be toggled).
    pub fn toggle_named_scratchpad(
        &mut self,
        name: Option<&str>,
        title: Option<&str>,
        spawn: Option<&str>,
    ) {
        let target = self.find_client_by_id_or_title(name, title);
        let Some(idx) = target else {
            // No matching client — spawn the launcher command if the
            // user supplied one. The just-launched client will land
            // tagged `isnamedscratchpad:1` per its windowrule and the
            // next bind press will toggle it visible.
            if let Some(cmd) = spawn.filter(|s| !s.trim().is_empty())
                && let Err(e) = crate::utils::spawn_shell(cmd)
            {
                tracing::error!(cmd = %cmd, error = ?e, "toggle_named_scratchpad spawn failed");
            }
            return;
        };

        // Mark as named scratchpad (the windowrule may already have
        // set it, but this is idempotent and lets bare keybindings
        // turn arbitrary running clients into scratchpads).
        self.clients[idx].is_named_scratchpad = true;

        // Single-scratchpad enforcement: when this config is on, only
        // ONE named scratchpad may be visible at a time. Hide every
        // other shown scratchpad on the same monitor before switching
        // the target's state.
        if self.config.single_scratchpad {
            let mon_idx = self.clients[idx].monitor;
            let to_hide: Vec<usize> = self
                .clients
                .iter()
                .enumerate()
                .filter(|(i, c)| {
                    *i != idx
                        && c.is_in_scratchpad
                        && c.is_scratchpad_show
                        && (self.config.scratchpad_cross_monitor || c.monitor == mon_idx)
                })
                .map(|(i, _)| i)
                .collect();
            for i in to_hide {
                self.hide_scratchpad_client(i);
            }
        }

        // First-time toggle: mark the client as `is_in_scratchpad` so
        // future toggles see it as a scratchpad. Then flip the
        // visibility.
        if !self.clients[idx].is_in_scratchpad {
            self.clients[idx].is_in_scratchpad = true;
            // Start the client hidden so the very first toggle reveals
            // it (mirrors mango's "set_minimized then switch_state"
            // dance).
            self.clients[idx].is_scratchpad_show = false;
            self.clients[idx].is_minimized = true;
            let window = self.clients[idx].window.clone();
            self.space.unmap_elem(&window);
        }
        self.switch_scratchpad_state(idx);
    }

    /// Public action: bring a window matching <name>/<title> to the
    /// currently-focused monitor's active tag, launching it via
    /// <spawn> if no instance is open. The mango-here.sh script
    /// implements the same flow for the C compositor; this is the
    /// in-process Rust port — no `mmsg` round-trips, no view
    /// snapshot/restore dance.
    ///
    /// Three args (mapped from the bind line):
    ///   v  → app_id pattern (regex; same matcher as windowrule appid)
    ///   v2 → optional title pattern (use `none` to skip)
    ///   v3 → spawn command run when no matching client exists
    /// Together: `bind = alt,1,summon,^Kenp$,none,start-kkenp`
    ///
    /// Hidden scratchpads are skipped — they have their own
    /// `toggle_named_scratchpad` dispatch and summoning them here
    /// would bypass the single-scratchpad enforcement.
    pub fn summon(
        &mut self,
        name: Option<&str>,
        title: Option<&str>,
        spawn: Option<&str>,
    ) {
        let target = self.find_summonable_client(name, title);
        let Some(idx) = target else {
            if let Some(cmd) = spawn.filter(|s| !s.trim().is_empty())
                && let Err(e) = crate::utils::spawn_shell(cmd)
            {
                tracing::error!(cmd = %cmd, error = ?e, "summon spawn failed");
            }
            return;
        };

        let target_mon = self.focused_monitor();
        if target_mon >= self.monitors.len() {
            return;
        }
        let target_tagset = self.monitors[target_mon].current_tagset();
        if target_tagset == 0 {
            return;
        }

        // Fast path: window is already visible on the focused monitor's
        // active tag — just refocus it. Saves a needless tag-switch
        // animation and a re-arrange when the user presses summon while
        // the target is already in front of them.
        let already_here = self.clients[idx].monitor == target_mon
            && (self.clients[idx].tags & target_tagset) != 0
            && !self.clients[idx].is_minimized;
        if already_here {
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
            return;
        }

        let source_mon = self.clients[idx].monitor;
        let was_minimized = self.clients[idx].is_minimized;

        self.clients[idx].old_tags = self.clients[idx].tags;
        self.clients[idx].is_tag_switching = true;
        self.clients[idx].animation.running = false;
        self.clients[idx].tags = target_tagset;
        self.clients[idx].monitor = target_mon;
        if was_minimized {
            self.clients[idx].is_minimized = false;
        }

        if source_mon != target_mon && source_mon < self.monitors.len() {
            self.arrange_monitor(source_mon);
        }
        self.arrange_monitor(target_mon);

        let window = self.clients[idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window)));

        crate::protocols::dwl_ipc::broadcast_monitor(self, target_mon);
        if source_mon != target_mon && source_mon < self.monitors.len() {
            crate::protocols::dwl_ipc::broadcast_monitor(self, source_mon);
        }
    }

    /// Like `find_client_by_id_or_title` but skips hidden scratchpads —
    /// summoning them would conflict with the named-scratchpad toggle
    /// dispatch and bypass single_scratchpad enforcement.
    fn find_summonable_client(
        &self,
        name: Option<&str>,
        title: Option<&str>,
    ) -> Option<usize> {
        let name_pat = name.unwrap_or("");
        let title_pat = title.unwrap_or("");
        for (idx, c) in self.clients.iter().enumerate() {
            if c.is_in_scratchpad && !c.is_scratchpad_show {
                continue;
            }
            let app_match = if name_pat.is_empty() {
                true
            } else {
                matches_rule_text(name_pat, &c.app_id)
            };
            let title_match = if title_pat.is_empty() {
                true
            } else {
                matches_rule_text(title_pat, &c.title)
            };
            if app_match && title_match {
                return Some(idx);
            }
        }
        None
    }

    /// Public action: full reset of the focused client back to
    /// a normal tile. Bind this to an emergency-recovery key
    /// (the user has it on `super+ctrl,Escape`) for any time a
    /// window ends up in a state the standard binds can't get it
    /// out of — accidental scratchpad promotion, sticky floating
    /// because some popup left it that way, fullscreen
    /// stuck-on, the list goes on. Mirrors mango-ext's "exit
    /// scratchpad" but also drops the floating / fullscreen /
    /// minimised flags so the next arrange treats the window as a
    /// vanilla tiled toplevel. Cheaper and more reliable than
    /// chasing the specific flag that's misbehaving.
    pub fn unscratchpad_focused(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        let already_normal = !self.clients[idx].is_in_scratchpad
            && !self.clients[idx].is_named_scratchpad
            && !self.clients[idx].is_scratchpad_show
            && !self.clients[idx].is_floating
            && !self.clients[idx].is_fullscreen
            && !self.clients[idx].is_maximized_screen
            && !self.clients[idx].is_minimized;
        if already_normal {
            return;
        }
        let c = &mut self.clients[idx];
        let app_id = c.app_id.clone();
        let snapshot = (
            c.is_in_scratchpad,
            c.is_scratchpad_show,
            c.is_named_scratchpad,
            c.is_minimized,
            c.is_floating,
            c.is_fullscreen,
            c.is_maximized_screen,
        );
        c.is_in_scratchpad = false;
        c.is_scratchpad_show = false;
        c.is_named_scratchpad = false;
        c.is_minimized = false;
        c.is_floating = false;
        c.is_fullscreen = false;
        c.is_maximized_screen = false;
        let mon_idx = c.monitor;
        let window = c.window.clone();
        let geom = c.geom;

        // Re-map the surface (scratchpad hide had unmapped it from
        // the scene). Active tagset already covers the recovered
        // window since `is_visible_on`'s scratchpad-guard no longer
        // suppresses it.
        self.space.map_element(window.clone(), (geom.x, geom.y), true);
        self.arrange_monitor(mon_idx);
        self.focus_surface(Some(FocusTarget::Window(window)));
        tracing::info!(
            "unscratchpad: recovered app_id={} from \
             (in_scratch={}, scratch_show={}, named_scratch={}, \
              minimized={}, floating={}, fullscreen={}, max_screen={})",
            app_id,
            snapshot.0,
            snapshot.1,
            snapshot.2,
            snapshot.3,
            snapshot.4,
            snapshot.5,
            snapshot.6,
        );
    }

    /// Public action: toggle the *anonymous* scratchpad set — every
    /// client previously promoted to a scratchpad via the legacy
    /// `toggle_scratchpad` command (no name, no title, just "stash
    /// the current focused window"). Mirrors mango's implementation
    /// faithfully enough that the pattern carries over.
    pub fn toggle_scratchpad(&mut self) {
        if let Some(mon_idx) = self
            .focused_client_idx()
            .map(|i| self.clients[i].monitor)
        {
            // First pass: if any anonymous scratchpad is currently
            // shown, hide them all. (single_scratchpad makes this
            // mostly the same as toggle_named, just keyed off the
            // anonymous flag.)
            let mut hit = false;
            let to_toggle: Vec<usize> = (0..self.clients.len())
                .filter(|&i| {
                    let c = &self.clients[i];
                    !c.is_named_scratchpad
                        && (self.config.scratchpad_cross_monitor || c.monitor == mon_idx)
                })
                .collect();

            for i in to_toggle {
                let c = &self.clients[i];
                if self.config.single_scratchpad
                    && c.is_named_scratchpad
                    && !c.is_minimized
                {
                    self.clients[i].is_minimized = true;
                    let window = self.clients[i].window.clone();
                    self.space.unmap_elem(&window);
                    continue;
                }
                if c.is_named_scratchpad {
                    continue;
                }
                if hit {
                    continue;
                }
                if c.is_in_scratchpad {
                    self.switch_scratchpad_state(i);
                    hit = true;
                }
            }
        }
    }
}
