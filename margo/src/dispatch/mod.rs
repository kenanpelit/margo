//! Action dispatcher.
//!
//! Each `match` arm below maps an action name (`view`, `setlayout`,
//! `spawn`, …) to its compositor-side implementation. The action
//! name comes from one of three sources:
//!
//!   1. **Keybinds** parsed from `config.conf` — the parser produces
//!      a `KeyBinding { action, arg, .. }` and the keyboard handler
//!      calls this function directly.
//!   2. **dwl-ipc dispatch requests** from `mctl dispatch <name> [args…]`.
//!      The IPC handler decodes the 5 string slots into an `Arg`
//!      and calls in.
//!   3. **Gesture / mouse / axis binds** — same handler shape, just
//!      a different action name pool.
//!
//! ## Reading `arg`
//!
//! The slot-to-field mapping is documented in detail on the
//! `margo_config::Arg` struct (see `margo-config/src/types.rs`).
//! TL;DR for handler authors:
//!
//! * `arg.i` / `arg.i2` — numeric args 1 / 2 (i32, default 0).
//! * `arg.f`            — numeric arg 3 (f32, default 0.0).
//! * `arg.v` / `arg.v2` — primary / secondary string. `Some` when
//!   the wire slot was non-empty.
//! * `arg.v3` / `arg.ui` / `arg.ui2` / `arg.f2` — bind-only,
//!   populated by the config parser. NOT on the IPC wire.
//!
//! ## Footgun
//!
//! When passing a string from `mctl` (slot 4 → `arg.v`), the three
//! preceding slots **must be empty strings** — they're numeric-
//! parsed and dropped on a non-numeric value, so the right behavior
//! is to leave them blank. Three CLI bugs (`mctl theme default`,
//! `mctl session load <path>`, `mctl run <script>`) landed in
//! 0.1.5 / 0.1.6 with the string in slot 1; their fix was to move
//! the payload to slot 4. Look at `mctl::Command::Theme` /
//! `Command::Run` / `Command::SessionLoad` for the canonical
//! "empty,empty,empty,STRING,empty" pattern.

#![allow(dead_code)]
use margo_config::Arg;
use tracing::debug;

use crate::state::MargoState;

pub fn dispatch_action(state: &mut MargoState, action: &str, arg: &Arg) {
    debug!(action = %action, "dispatch");
    match action {
        "quit" => state.should_quit = true,
        "debug_dump" | "debug-dump" | "diagnose" => state.debug_dump(),
        // Emergency escape from a stuck lock screen. Tears down the
        // current ext_session_lock from the compositor side without
        // requiring the locker client to cooperate — useful when
        // noctalia/Quickshell can't accept keyboard input and the user
        // would otherwise have to switch to a TTY (or reboot) to
        // recover. This action is whitelisted in handle_keyboard so
        // it works *even while* `session_locked` is true.
        "force_unlock" | "force-unlock" => state.force_unlock(),
        "moveresize" => {
            // mango legacy: arg `curmove` → start move grab; `curresize`
            // → start resize grab. Anything else falls back to move.
            match arg.v.as_deref().unwrap_or("curmove") {
                "curresize" | "resize" => state.start_interactive_resize(),
                _ => state.start_interactive_move(),
            }
        }
        "session_save" | "save_session" => {
            let path = match crate::session::session_path() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(error = ?e, "session_save: resolve path");
                    return;
                }
            };
            let snap = crate::session::SessionSnapshot::capture(state);
            let n = snap.monitors.len();
            match crate::session::save_to(&path, &snap) {
                Ok(()) => {
                    tracing::info!(target: "session", "saved {n} monitors to {path:?}");
                    let _ = crate::utils::spawn([
                        "notify-send",
                        "-a",
                        "margo",
                        "-i",
                        "document-save",
                        "-t",
                        "1500",
                        "Margo session",
                        &format!("Saved {n} monitor(s)"),
                    ]);
                }
                Err(e) => {
                    tracing::error!(target: "session", "save failed: {e:?}");
                    let _ = crate::utils::spawn([
                        "notify-send",
                        "-a",
                        "margo",
                        "-i",
                        "dialog-error",
                        "-t",
                        "3000",
                        "Margo session save failed",
                        &format!("{e}"),
                    ]);
                }
            }
        }
        "session_load" | "load_session" => {
            let path = match crate::session::session_path() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(error = ?e, "session_load: resolve path");
                    return;
                }
            };
            let snap = match crate::session::load_from(&path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(target: "session", "load failed: {e:?}");
                    let _ = crate::utils::spawn([
                        "notify-send",
                        "-a",
                        "margo",
                        "-i",
                        "dialog-warning",
                        "-t",
                        "3000",
                        "Margo session load failed",
                        &format!("{e}"),
                    ]);
                    return;
                }
            };
            let applied = crate::session::apply_to_state(state, &snap);
            tracing::info!(target: "session", "loaded {applied} monitors from {path:?}");
            let _ = crate::utils::spawn([
                "notify-send",
                "-a",
                "margo",
                "-i",
                "document-open",
                "-t",
                "1500",
                "Margo session",
                &format!("Restored {applied} monitor(s)"),
            ]);
        }
        "theme" | "set_theme" => {
            let name = arg.v.as_deref().unwrap_or("default");
            match state.apply_theme_preset(name) {
                Ok(()) => {
                    let _ = crate::utils::spawn([
                        "notify-send",
                        "-a",
                        "margo",
                        "-i",
                        "preferences-desktop-theme",
                        "-t",
                        "1500",
                        "Margo theme",
                        name,
                    ]);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "theme preset failed");
                    let _ = crate::utils::spawn([
                        "notify-send",
                        "-a",
                        "margo",
                        "-i",
                        "dialog-warning",
                        "-t",
                        "3000",
                        "Margo theme",
                        &e,
                    ]);
                }
            }
        }
        "reload" | "reload_config" => match state.reload_config() {
            Ok(()) => {
                // Reload succeeded — but warnings from the validator
                // pass survive into `last_reload_diagnostics` (the
                // compositor only refused to apply on *errors*).
                // Surface them so the user knows the reload technically
                // worked but with N typos / orphan keys silently
                // defaulted. Body text routes the user to `mctl
                // config-errors` for the full list.
                let warn_count = state
                    .last_reload_diagnostics
                    .iter()
                    .filter(|d| {
                        matches!(
                            d.severity,
                            margo_config::diagnostics::Severity::Warning,
                        )
                    })
                    .count();
                if warn_count > 0 {
                    tracing::info!(
                        "config reloaded with {warn_count} warning(s)"
                    );
                    let body = format!(
                        "Reload OK but {warn_count} warning{} — run `mctl config-errors`",
                        if warn_count == 1 { "" } else { "s" }
                    );
                    let _ = crate::utils::spawn([
                        "notify-send",
                        "-a",
                        "margo",
                        "-i",
                        "dialog-warning",
                        "-u",
                        "normal",
                        "-t",
                        "5000",
                        "Margo: config reloaded with warnings",
                        body.as_str(),
                    ]);
                } else {
                    tracing::info!("config reloaded");
                    let _ = crate::utils::spawn([
                        "notify-send",
                        "-a",
                        "margo",
                        "-i",
                        "preferences-system",
                        "-t",
                        "2500",
                        "Margo",
                        "Config reloaded ✓",
                    ]);
                }
            }
            Err(e) => {
                tracing::error!(error = ?e, "reload config failed");
                let _ = crate::utils::spawn([
                    "notify-send",
                    "-a",
                    "margo",
                    "-i",
                    "dialog-error",
                    "-u",
                    "critical",
                    "-t",
                    "5000",
                    "Margo: config reload failed",
                    &format!("{e}"),
                ]);
            }
        },
        "spawn" => {
            if let Some(cmd) = &arg.v {
                if let Err(e) = crate::utils::spawn_shell(cmd) {
                    tracing::error!(cmd = %cmd, error = ?e, "spawn failed");
                }
            }
        }

        // ── Twilight (built-in blue-light filter) ───────────────────
        // All four follow the same shape: mutate `state.twilight`
        // and then force a resample via `tick_twilight()` so the
        // change lands on the very next frame instead of waiting
        // for the calloop timer.
        "twilight_preview" => {
            // arg.i = Kelvin, arg.i2 = gamma % (0 ⇒ keep current 100)
            let k = if arg.i > 0 { arg.i as u32 } else { 4000 };
            let g = if arg.i2 > 0 { arg.i2 as u32 } else { 100 };
            state.twilight.set_preview(k, g);
            state.force_tick_twilight();
            tracing::info!(temp_k = k, gamma_pct = g, "twilight preview");
        }
        "twilight_test" => {
            // arg.i = duration seconds (clamped 1–60 in the CLI)
            let dur_s = if arg.i > 0 { arg.i as u64 } else { 5 };
            state.twilight.start_test(dur_s.saturating_mul(1000));
            state.force_tick_twilight();
            tracing::info!(duration_s = dur_s, "twilight test: sweeping day→night");
        }
        "twilight_reset" => {
            state.twilight.reset();
            state.force_tick_twilight();
            tracing::info!("twilight reset to schedule");
        }
        "twilight_toggle" => {
            state.config.twilight = !state.config.twilight;
            state.twilight.reset();
            state.force_tick_twilight();
            tracing::info!(enabled = state.config.twilight, "twilight toggled");
        }
        "twilight_set" => {
            // arg.v = "field=value" (e.g. "day_temp=5500"). Live
            // config tweak — survives until next reload, not
            // persisted to disk.
            if let Some(spec) = arg.v.as_deref() {
                if let Some((field, raw_val)) = spec.split_once('=') {
                    let field = field.trim();
                    let val = raw_val.trim();
                    let applied = match field {
                        "day_temp" => {
                            val.parse::<u32>().ok().map(|v| {
                                state.config.twilight_day_temp = v.clamp(1000, 25000)
                            })
                        }
                        "night_temp" => val.parse::<u32>().ok().map(|v| {
                            state.config.twilight_night_temp = v.clamp(1000, 25000)
                        }),
                        "day_gamma" => val
                            .parse::<u32>()
                            .ok()
                            .map(|v| state.config.twilight_day_gamma = v.clamp(10, 200)),
                        "night_gamma" => val.parse::<u32>().ok().map(|v| {
                            state.config.twilight_night_gamma = v.clamp(10, 200)
                        }),
                        "enabled" | "twilight" => val
                            .parse::<u32>()
                            .ok()
                            .map(|v| state.config.twilight = v != 0),
                        "transition_s" => val.parse::<u32>().ok().map(|v| {
                            state.config.twilight_transition_s = v.clamp(30, 7200)
                        }),
                        "mode" => {
                            // Accept the same lowercase tokens as the
                            // on-disk `twilight_mode` config key so the
                            // CLI / GUI stay symmetrical with the file.
                            match val.to_ascii_lowercase().as_str() {
                                "geo" => {
                                    state.config.twilight_mode =
                                        margo_config::TwilightMode::Geo;
                                    Some(())
                                }
                                "manual" => {
                                    state.config.twilight_mode =
                                        margo_config::TwilightMode::Manual;
                                    Some(())
                                }
                                "static" => {
                                    state.config.twilight_mode =
                                        margo_config::TwilightMode::Static;
                                    Some(())
                                }
                                "schedule" => {
                                    state.config.twilight_mode =
                                        margo_config::TwilightMode::Schedule;
                                    Some(())
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if applied.is_some() {
                        state.twilight.reset();
                        state.force_tick_twilight();
                        tracing::info!(field = %field, value = %val, "twilight_set");
                    } else {
                        tracing::warn!(
                            "twilight_set: unknown field or bad value: {spec:?}"
                        );
                    }
                }
            }
        }
        // W3.2 — one-shot Rhai script eval. `mctl run <file>` is
        // the user-facing wrapper; it just calls dispatch with
        // this action + path arg. Fully sandboxed via the
        // existing scripting engine; same recursion guards as
        // init.rhai.
        "run_script" | "run-script" | "rhai-eval" => {
            if let Some(path) = arg.v.as_deref() {
                let p = std::path::Path::new(path);
                crate::scripting::run_script_file(state, p);
            }
        }
        // ── Screenshot dispatch ─────────────────────────────
        // All screenshot actions delegate to the `mscreenshot`
        // companion binary (a workspace sibling of `mctl` and
        // `mlayout`). It orchestrates grim / slurp /
        // wl-copy + an optional editor (swappy / satty).
        //
        //   screenshot              → focused output → editor → file
        //   screenshot-screen       → alias of `screenshot`
        //   screenshot-window       → focused window → editor → file
        //   screenshot-region       → drag-region → save (no edit)
        //   screenshot-region-ui    → drag-region → editor → file + clipboard
        //
        // Required runtime tools (PKGBUILD `depends`): grim, slurp,
        // wl-clipboard. Optional editor: swappy / satty (optdep).
        "screenshot" | "screenshot-screen" | "screenshot_screen" => {
            let mode = match arg.v.as_deref() {
                Some("window") => "window",
                _ => "screen",
            };
            if let Err(e) = crate::utils::spawn_shell(&format!("mscreenshot {}", mode)) {
                tracing::error!(error = ?e, "spawn mscreenshot failed");
            }
        }
        "screenshot-window" | "screenshot_window" => {
            if let Err(e) = crate::utils::spawn_shell("mscreenshot window") {
                tracing::error!(error = ?e, "spawn mscreenshot failed");
            }
        }
        "screenshot-region-ui" | "screenshot_region_ui" => {
            // W2.1 in-compositor selection. Lights up the
            // ActiveRegionSelector at the cursor's current
            // position; subsequent pointer + key events route
            // through the selector until confirm / cancel. On
            // confirm, mscreenshot is spawned with the chosen
            // mode + MARGO_REGION_GEOM env var so it skips its
            // own slurp invocation.
            //
            // Optional arg picks the delivery mode (rec / area /
            // ri / rc / rf — same names as mscreenshot
            // subcommands). Bare action defaults to `rec` to
            // preserve the previous keybind contract.
            let mode = crate::screenshot_region::SelectorMode::parse(arg.v.as_deref());
            state.open_region_selector(mode);
        }
        "screenshot-region" | "screenshot_region" => {
            // `area` mode: region → save to disk only.
            if let Err(e) = crate::utils::spawn_shell("mscreenshot area") {
                tracing::error!(error = ?e, "spawn mscreenshot failed");
            }
        }
        "screenshot-output" | "screenshot_output" => {
            if let Err(e) = crate::utils::spawn_shell("mscreenshot screen") {
                tracing::error!(error = ?e, "spawn mscreenshot failed");
            }
        }
        "killclient" => state.kill_focused(),
        "focusstack" | "focusdir" => state.focus_stack(direction_arg(arg)),
        "exchange_client" | "smartmovewin" => state.exchange_stack(direction_arg(arg)),
        "view" => state.view_tag(tag_arg(arg)),
        "toggleview" => state.toggle_view_tag(tag_arg(arg)),
        "tag" | "tagsilent" => state.tag_focused(tag_arg(arg)),
        // `tagview` = move the focused window to <tag> AND switch the
        // current monitor to that tag, so you follow the window. This
        // is the behaviour the user usually wants when they think
        // "send this window to tag N and take me there." Plain `tag`
        // stays dwm-/dwl-style: window goes, user stays put.
        "tagview" | "tag_view" | "tag-view" | "movetagview" => {
            let mask = tag_arg(arg);
            state.tag_focused(mask);
            state.view_tag(mask);
        }
        "toggletag" => state.toggle_client_tag(tag_arg(arg)),
        "tagall" => state.view_tag(u32::MAX),
        "viewtoleft" | "viewtoleft_have_client" => state.view_relative(-1),
        "viewtoright" | "viewtoright_have_client" => state.view_relative(1),
        "tagtoleft" => state.tag_relative(-1),
        "tagtoright" => state.tag_relative(1),
        "setlayout" => {
            if let Some(name) = &arg.v {
                state.set_layout(name);
                // Mirror switch_layout's OSD: explicit-pick is also
                // a user-triggered change, deserves the same hint.
                state.notify_layout(name);
            }
        }
        "switch_layout" => state.switch_layout(),
        "togglefloating" => state.toggle_floating(),
        "togglefullscreen" => state.toggle_fullscreen(),
        "togglefullscreen_exclusive" | "togglefullscreen-exclusive" | "togglefullscreenexclusive" => {
            state.toggle_fullscreen_exclusive()
        }
        // niri-float-sticky equivalent — pin the focused client to
        // every tag on its monitor. Toggle via the same action;
        // second press restores the previous tag set.
        "sticky_window" | "togglesticky" | "toggle_sticky" | "sticky" => {
            state.toggle_sticky()
        }
        // Mango-style named scratchpad. Three args (mapped from the
        // bind line):
        //   v  → app_id pattern (e.g. `dropdown-terminal`)
        //   v2 → optional title pattern (use `none` to skip)
        //   v3 → spawn command run when no matching client exists
        // Together: `bind = super,Return,toggle_named_scratchpad,
        //           dropdown-terminal,none,kitty --class dropdown-terminal`
        "toggle_named_scratchpad"
        | "togglenamedscratchpad"
        | "toggle-named-scratchpad" => {
            let name = arg.v.as_deref();
            let title = arg.v2.as_deref().filter(|s| {
                let t = s.trim();
                !t.is_empty() && !t.eq_ignore_ascii_case("none")
            });
            let spawn = arg.v3.as_deref().filter(|s| !s.trim().is_empty());
            state.toggle_named_scratchpad(name, title, spawn);
        }
        "toggle_scratchpad" | "togglescratchpad" => state.toggle_scratchpad(),
        // mango-here equivalent — bring a window to the current tag,
        // launch it if it isn't running. Three args mirror
        // toggle_named_scratchpad:
        //   v  → app_id regex
        //   v2 → optional title regex (`none` to skip)
        //   v3 → spawn command if no instance exists
        // Bind example:
        //   bind = alt,1,summon,^Kenp$,none,start-kkenp
        //   bind = alt,2,summon,^firefox$,,firefox
        "summon" | "taghere" | "tag_here" | "tag-here" | "bring_here"
        | "bringhere" => {
            let name = arg.v.as_deref();
            let title = arg.v2.as_deref().filter(|s| {
                let t = s.trim();
                !t.is_empty() && !t.eq_ignore_ascii_case("none")
            });
            let spawn = arg.v3.as_deref().filter(|s| !s.trim().is_empty());
            state.summon(name, title, spawn);
        }
        // Recovery action: pull the focused client back out of any
        // scratchpad state. Useful when a regular window
        // accidentally got promoted to scratchpad (typo bind, fuzzy
        // app_id match) and the user wants it back as a normal
        // tile / float.
        "unscratchpad" | "unscratchpad_focused" | "exit_scratchpad" => {
            state.unscratchpad_focused()
        }
        "incnmaster" => state.inc_nmaster(arg.i),
        "setmfact" => state.set_mfact(arg.f),
        "togglegaps" => state.toggle_gaps(),
        "incgaps" => state.inc_gaps(arg.i),
        "set_proportion" => state.set_focused_proportion(float_arg(arg)),
        "switch_proportion_preset" => state.switch_focused_proportion_preset(),
        "movewin" => state.move_focused(arg.i, arg.i2),
        "resizewin" => state.resize_focused(arg.i, arg.i2),
        "zoom" => state.zoom(),
        "setkeymode" => {
            if let Some(mode) = &arg.v {
                debug!(mode = %mode, "key mode change");
                state.input_keyboard.mode = mode.clone();
            }
        }
        "focusmon" => state.focus_mon(direction_arg(arg)),
        "tagmon" => state.tag_mon(direction_arg(arg)),
        // Soft-disable / enable an output by name, mirroring the
        // wlr_output_management protocol path. Useful for keybind-
        // driven multi-monitor workflows ("toggle the laptop panel
        // when I dock"). Arg is the connector name (DP-3, eDP-1).
        // Last enabled monitor is protected — disabling it is
        // refused with a warn log.
        //   bind = super+ctrl,F1,disable_output,eDP-1
        //   bind = super+ctrl,F2,enable_output,eDP-1
        //   bind = super+ctrl,F3,toggle_output,eDP-1
        "disable_output" | "disable-output" => {
            if let Some(name) = arg.v.as_deref() {
                if let Some(idx) = state.monitors.iter().position(|m| m.name == name) {
                    state.disable_monitor(idx);
                }
            }
        }
        "enable_output" | "enable-output" => {
            if let Some(name) = arg.v.as_deref() {
                if let Some(idx) = state.monitors.iter().position(|m| m.name == name) {
                    state.enable_monitor(idx);
                }
            }
        }
        "toggle_output" | "toggle-output" => {
            if let Some(name) = arg.v.as_deref() {
                if let Some(idx) = state.monitors.iter().position(|m| m.name == name) {
                    if state.monitors[idx].enabled {
                        state.disable_monitor(idx);
                    } else {
                        state.enable_monitor(idx);
                    }
                }
            }
        }
        "toggle_overview" => state.toggle_overview(),
        // Keyboard navigation while overview is open. The action
        // handlers are no-ops outside overview, but the keybinding
        // dispatcher still intercepts the keystroke — pick combos
        // (alt+Tab, mod+J/K, ...) that don't collide with normal
        // text input. Binding bare Return would swallow Enter
        // everywhere, including terminals.
        "overview_focus_next" => state.overview_focus_next(),
        "overview_focus_prev" => state.overview_focus_prev(),
        "overview_activate" => state.overview_activate(),
        // Spatial-canvas pan (PaperWM-ish). Two integer args:
        // dx and dy logical-pixel deltas. Stored per-tag so each
        // tag remembers its viewport offset.
        "canvas_pan" => state.canvas_pan(arg.i, arg.i2),
        "canvas_reset" => state.canvas_reset(),
        _ => debug!("unhandled action: {action}"),
    }
}

fn tag_arg(arg: &Arg) -> u32 {
    if arg.ui != 0 {
        arg.ui
    } else if arg.i > 0 {
        arg.i as u32
    } else {
        0
    }
}

fn float_arg(arg: &Arg) -> f32 {
    if arg.f != 0.0 {
        arg.f
    } else {
        arg.i as f32
    }
}

fn direction_arg(arg: &Arg) -> i32 {
    if arg.i != 0 {
        return arg.i.signum();
    }
    let value = arg
        .v
        .as_deref()
        .or(arg.v2.as_deref())
        .unwrap_or("next")
        .to_ascii_lowercase();

    match value.as_str() {
        "left" | "up" | "prev" | "previous" | "-1" => -1,
        "right" | "down" | "next" | "1" => 1,
        _ => 1,
    }
}
