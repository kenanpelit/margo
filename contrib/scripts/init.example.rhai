// Example ~/.config/margo/init.rhai
//
// Drop this at $XDG_CONFIG_HOME/margo/init.rhai (typically
// ~/.config/margo/init.rhai) and margo will evaluate it once at
// compositor startup, after exec_once and before the event loop.
//
// What you can do today (Phase 2):
//
//   * Inspect compositor state via read-only bindings.
//   * Invoke any registered margo dispatch action via dispatch().
//   * Combine several actions into one logical operation that runs
//     atomically without round-tripping through `mctl`.
//
// What's coming (Phase 3) — these registrations work today as
// no-ops, so any handler you write here will start firing
// automatically once the event sites are wired:
//
//   * on_focus_change(|| { … })           — query focused_*() inside
//   * on_tag_switch(|| { … })              — query current_tag() inside
//   * on_window_open(|| { … })             — fires after rules + focus
//   * on_window_close(|appid, title| { … }) — args carry the dying
//                                             window's identity
//
// Errors in this file are non-fatal — the compositor logs the
// Rhai line/col where things went wrong and keeps running.

// ── Inspect ───────────────────────────────────────────────────────────

// `print` lands in the compositor's journal at info level.
print("init.rhai: " + monitor_count() + " monitors, " +
      client_count() + " clients on startup");
print("init.rhai: focused = " + focused_appid() +
      " on " + focused_monitor_name());

// `monitor_names()` returns an array of strings like ["DP-3", "eDP-1"].
for name in monitor_names() {
    print("monitor: " + name);
}

// ── Dispatch ──────────────────────────────────────────────────────────
//
// Same actions you'd put in `bind = …` lines. The args array maps
// onto the action's expected argument layout — strings go into
// v/v2/v3, integers into i/ui (one int fills both), floats into f/f2.
//
// Tags are bitmasks; use `tag(n)` for a 1-based tag number.

// Examples (commented so this file doesn't actually do anything on
// fresh install — uncomment what you want):

// dispatch("setlayout", ["scroller"]);
// dispatch("view", [tag(1)]);
// dispatch("spawn", ["notify-send 'init.rhai ran'"]);
// dispatch("focusstack", [1]);          // direction = +1 (next)

// ── A real-world pattern ─────────────────────────────────────────────
//
// Combine several actions into one "focus mode" operation. With
// Phase 3 event hooks you can attach this to a tag-switch handler;
// today you can call it from a bind via `dispatch("spawn",
// ["mctl run focus-mode.rhai"])` once Phase 4 lands. For now,
// invoke directly during init for a "always start in focus mode"
// effect.

fn focus_mode_on() {
    dispatch("spawn", ["pkill -SIGUSR1 waybar"]);
    // Set gaps to 0 (assuming the user wants a clean focus mode).
    // setgaps takes outer_h, inner_h, outer_v, inner_v as four ints.
    dispatch("setgaps", [0, 0, 0, 0]);
}

// fn focus_mode_off() {
//     dispatch("spawn", ["pkill -SIGUSR2 waybar"]);
//     dispatch("setgaps", [10, 10, 10, 10]);
// }

// ── Phase 3 forward-compat (registers but does not fire today) ──────
//
// These calls are recognized but log-only until margo Phase 3 wires
// the event sites. Writing them today is harmless — when Phase 3
// ships, your handlers light up automatically.

on_window_open(|| {
    let appid = focused_appid();
    if appid == "spotify" || appid == "Spotify" {
        // Send Spotify to tag 8 and follow it there.
        dispatch("tagview", [tag(8)]);
    }
});

on_tag_switch(|| {
    if current_tag() == 9 {
        focus_mode_on();
    }
});

on_focus_change(|| {
    // Could update a status indicator, log focus history, etc.
    print("focus → " + focused_appid());
});

on_window_close(|appid, title| {
    // Fires AFTER the toplevel is gone — the window is no longer in
    // `client_count()`, focus has shifted to a sibling, and the
    // closing identity arrives as args (focused_*() can't reach a
    // dead window).
    print("closed: " + appid + " — " + title);
});
