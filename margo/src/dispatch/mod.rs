#![allow(dead_code)]
use margo_config::Arg;
use tracing::debug;

use crate::state::MargoState;

pub fn dispatch_action(state: &mut MargoState, action: &str, arg: &Arg) {
    debug!("action: {action}");
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
                    tracing::warn!("theme: {e}");
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
            Err(e) => {
                tracing::error!("reload config: {e:?}");
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
                    tracing::error!("spawn '{cmd}': {e}");
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
                tracing::error!("spawn mscreenshot: {e}");
            }
        }
        "screenshot-window" | "screenshot_window" => {
            if let Err(e) = crate::utils::spawn_shell("mscreenshot window") {
                tracing::error!("spawn mscreenshot: {e}");
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
                tracing::error!("spawn mscreenshot: {e}");
            }
        }
        "screenshot-output" | "screenshot_output" => {
            if let Err(e) = crate::utils::spawn_shell("mscreenshot screen") {
                tracing::error!("spawn mscreenshot: {e}");
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
                debug!("key mode -> {mode}");
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
        "toggleoverview" => state.toggle_overview(),
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
