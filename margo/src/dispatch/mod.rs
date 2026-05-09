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
        "reload" | "reload_config" => match state.reload_config() {
            Ok(()) => {
                tracing::info!("config reloaded");
                let _ = crate::utils::spawn(&[
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
                let _ = crate::utils::spawn(&[
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
        // ── Native screenshot dispatch ──────────────────────
        // `arg.v` interpretations for "screenshot":
        //   (none)               → focused output, save+clip
        //   "window"             → focused window, save+clip
        //   "clipboard" / "clip" → focused output, clipboard ONLY
        //   "output:DP-3"        → specific output, save+clip
        // The other entries are convenience aliases.
        "screenshot" | "screenshot-screen" | "screenshot_screen" => {
            use crate::screenshot::{ScreenshotRequest, ScreenshotSource};
            let (source, save_to_disk, copy_clipboard) = match arg.v.as_deref() {
                Some("window") => (ScreenshotSource::FocusedWindow, true, true),
                Some("clipboard") | Some("clip") => {
                    (ScreenshotSource::FocusedOutput, false, true)
                }
                Some(s) if s.starts_with("output:") => (
                    ScreenshotSource::Output(s.trim_start_matches("output:").to_string()),
                    true,
                    true,
                ),
                _ => (ScreenshotSource::FocusedOutput, true, true),
            };
            crate::screenshot::queue(
                state,
                ScreenshotRequest {
                    source,
                    include_pointer: false,
                    save_to_disk,
                    save_path: None,
                    copy_clipboard,
                },
            );
        }
        "screenshot-window" | "screenshot_window" => {
            crate::screenshot::queue(
                state,
                crate::screenshot::ScreenshotRequest {
                    source: crate::screenshot::ScreenshotSource::FocusedWindow,
                    include_pointer: false,
                    save_to_disk: true,
                    save_path: None,
                    copy_clipboard: true,
                },
            );
        }
        // Phase 3: in-compositor region selector. Captures
        // every output to a frozen GLES texture, dims everything
        // outside a user-drawn rectangle, lets the user drag
        // out a selection on top of the frozen scene, then
        // queues the capture on Return. Esc cancels.
        // No `slurp` dependency.
        "screenshot-region-ui" | "screenshot_region_ui" => {
            let (save_to_disk, copy_clipboard) = match arg.v.as_deref() {
                Some("clipboard") | Some("clip") => (false, true),
                Some("no-clip") | Some("noclip") => (true, false),
                _ => (true, true),
            };
            state.pending_region_selector_open = Some(
                crate::screenshot_region_ui::PendingOpen {
                    save_to_disk,
                    save_path: None,
                    copy_clipboard,
                    include_pointer: false,
                },
            );
            state.request_repaint();
        }
        "screenshot-region" | "screenshot_region" => {
            let (save_to_disk, copy_clipboard) = match arg.v.as_deref() {
                Some("clipboard") | Some("clip") => (false, true),
                Some("no-clip") | Some("noclip") => (true, false),
                _ => (true, true),
            };
            crate::screenshot::queue_region(
                state,
                save_to_disk,
                None, // auto path
                copy_clipboard,
                false, // include_pointer — flip later when interactive UI lands
            );
        }
        "screenshot-output" | "screenshot_output" => {
            let source = match arg.v.as_deref() {
                Some(name) if !name.is_empty() => {
                    crate::screenshot::ScreenshotSource::Output(name.to_string())
                }
                _ => crate::screenshot::ScreenshotSource::FocusedOutput,
            };
            crate::screenshot::queue(
                state,
                crate::screenshot::ScreenshotRequest {
                    source,
                    include_pointer: false,
                    save_to_disk: true,
                    save_path: None,
                    copy_clipboard: true,
                },
            );
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
        "toggleoverview" => state.toggle_overview(),
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
