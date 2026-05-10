//! Structured catalogue of every dispatch action margo accepts over
//! the dwl-ipc-v2 dispatch channel (and from `bind = …,…,<action>` in
//! the config file). The list mirrors the match arms in
//! `margo/src/dispatch/mod.rs`; keep them in sync — the consumer
//! tooling here (`mctl actions`, the bash/zsh/fish completion
//! generators in `contrib/completions/`) reads this single source so
//! a stale entry shows up in user-facing help immediately.
//!
//! Aliases are concrete strings the dispatcher accepts as synonyms
//! (e.g. `tag-view` and `tag_view` for `tagview`); they're emitted
//! into completion scripts so users can tab-complete whichever
//! spelling they prefer.
//!
//! Why a separate crate-internal module instead of pulling the list
//! straight out of `margo/dispatch/mod.rs`? `margo` is a Wayland
//! compositor binary — depending on it from `margo-ipc` would be a
//! workspace cycle, and pulling the strings out of source via
//! `build.rs` parsing is fragile. A hand-maintained list with one
//! entry per logical action (with aliases attached) is small enough
//! to keep accurate.

#[derive(Debug, Clone, Copy)]
pub struct Action {
    /// Canonical name (the spelling shown in help / completion
    /// menus). The dispatcher accepts this plus any of `aliases`.
    pub name: &'static str,
    /// Alternative spellings that route to the same handler.
    pub aliases: &'static [&'static str],
    /// Argument-shape hint, displayed alongside the description.
    /// Empty string for actions that take no args.
    pub args: &'static str,
    /// Logical grouping — used by `mctl actions` to print sections
    /// instead of one giant alphabetic list.
    pub group: Group,
    /// One-line description.
    pub summary: &'static str,
    /// Optional longer explanation / example, rendered after the
    /// summary when the user runs `mctl actions --verbose`.
    pub detail: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Tag,
    Focus,
    Layout,
    Scroller,
    Window,
    Scratchpad,
    Overview,
    System,
}

impl Group {
    pub const fn label(self) -> &'static str {
        match self {
            Group::Tag => "Tag / Workspace",
            Group::Focus => "Focus",
            Group::Layout => "Layout",
            Group::Scroller => "Scroller",
            Group::Window => "Window",
            Group::Scratchpad => "Scratchpad",
            Group::Overview => "Overview",
            Group::System => "System",
        }
    }
}

pub const ACTIONS: &[Action] = &[
    // ── Tag / Workspace ─────────────────────────────────────────────
    Action {
        name: "view",
        aliases: &[],
        args: "<MASK>",
        group: Group::Tag,
        summary: "Switch to tag(s) by bitmask.",
        detail: "Pressing the same tag twice toggles to the previously \
                 active tag when `view_current_to_back = 1` (dwl/mango \
                 alt-tab-for-tags pattern). Mask is 1<<(tag-1) → tag 1=1, tag 8=128.",
    },
    Action {
        name: "toggleview",
        aliases: &[],
        args: "<MASK>",
        group: Group::Tag,
        summary: "Add or remove a tag from the active set (multi-tag view).",
        detail: "",
    },
    Action {
        name: "tag",
        aliases: &["tagsilent"],
        args: "<MASK>",
        group: Group::Tag,
        summary: "Move the focused window to tag(s); user stays on the current tag.",
        detail: "dwm/dwl style. Use `tagview` to follow the window to its new tag.",
    },
    Action {
        name: "tagview",
        aliases: &["tag_view", "tag-view", "movetagview"],
        args: "<MASK>",
        group: Group::Tag,
        summary: "Move the focused window AND switch to that tag (Hyprland follow).",
        detail: "",
    },
    Action {
        name: "toggletag",
        aliases: &[],
        args: "<MASK>",
        group: Group::Tag,
        summary: "Add or remove a tag from the focused window's mask.",
        detail: "",
    },
    Action {
        name: "tagall",
        aliases: &[],
        args: "",
        group: Group::Tag,
        summary: "Show every tag at once (mask = 2^32-1).",
        detail: "",
    },
    Action {
        name: "viewtoleft",
        aliases: &["viewtoleft_have_client"],
        args: "",
        group: Group::Tag,
        summary: "Cycle view to the previous occupied tag on the focused monitor.",
        detail: "",
    },
    Action {
        name: "viewtoright",
        aliases: &["viewtoright_have_client"],
        args: "",
        group: Group::Tag,
        summary: "Cycle view to the next occupied tag on the focused monitor.",
        detail: "",
    },
    Action {
        name: "tagtoleft",
        aliases: &[],
        args: "",
        group: Group::Tag,
        summary: "Move the focused window to the previous tag.",
        detail: "",
    },
    Action {
        name: "tagtoright",
        aliases: &[],
        args: "",
        group: Group::Tag,
        summary: "Move the focused window to the next tag.",
        detail: "",
    },

    // ── Focus ───────────────────────────────────────────────────────
    Action {
        name: "focusstack",
        aliases: &["focusdir"],
        args: "<DIRECTION>",
        group: Group::Focus,
        summary: "Move keyboard focus next/previous (1 / -1) or directional (left/right/up/down).",
        detail: "`focusdir` and `focusstack` share the dispatcher; the \
                 direction argument may be a signed integer or one of \
                 `left right up down prev next`.",
    },
    Action {
        name: "exchange_client",
        aliases: &["smartmovewin"],
        args: "<DIRECTION>",
        group: Group::Focus,
        summary: "Swap the focused window with its neighbour in the given direction.",
        detail: "",
    },
    Action {
        name: "focusmon",
        aliases: &[],
        args: "<DIRECTION>",
        group: Group::Focus,
        summary: "Move keyboard focus to another monitor.",
        detail: "Direction: left/right/up/down or 1/-1.",
    },
    Action {
        name: "zoom",
        aliases: &[],
        args: "",
        group: Group::Focus,
        summary: "Promote the focused window to the layout's master slot (dwm zoom).",
        detail: "",
    },

    // ── Layout ──────────────────────────────────────────────────────
    Action {
        name: "setlayout",
        aliases: &[],
        args: "<NAME>",
        group: Group::Layout,
        summary: "Switch the current tag's layout by name.",
        detail: "Names: tile, scroller, grid, monocle, deck, center_tile, \
                 right_tile, vertical_tile, vertical_scroller, vertical_grid, \
                 vertical_deck, tgmix, canvas, dwindle.",
    },
    Action {
        name: "switch_layout",
        aliases: &[],
        args: "",
        group: Group::Layout,
        summary: "Cycle through the `circle_layout` config list.",
        detail: "",
    },
    Action {
        name: "incnmaster",
        aliases: &[],
        args: "<DELTA>",
        group: Group::Layout,
        summary: "Change the master-slot count (+1 / -1) for the current tag.",
        detail: "",
    },
    Action {
        name: "setmfact",
        aliases: &[],
        args: "<DELTA>",
        group: Group::Layout,
        summary: "Adjust the master factor for the current tag (e.g. 0.05 / -0.05).",
        detail: "Floating-point delta — clamped to [0.05, 0.95].",
    },
    Action {
        name: "togglegaps",
        aliases: &[],
        args: "",
        group: Group::Layout,
        summary: "Toggle layout gaps on/off.",
        detail: "",
    },
    Action {
        name: "incgaps",
        aliases: &[],
        args: "<DELTA>",
        group: Group::Layout,
        summary: "Resize gaps by `delta` pixels (positive widens, negative tightens).",
        detail: "",
    },

    // ── Scroller ────────────────────────────────────────────────────
    Action {
        name: "set_proportion",
        aliases: &[],
        args: "<RATIO>",
        group: Group::Scroller,
        summary: "Set the focused window's scroller width ratio (0.1 – 1.0).",
        detail: "",
    },
    Action {
        name: "switch_proportion_preset",
        aliases: &[],
        args: "",
        group: Group::Scroller,
        summary: "Cycle through `scroller_proportion_preset` values.",
        detail: "",
    },

    // ── Window ──────────────────────────────────────────────────────
    Action {
        name: "togglefloating",
        aliases: &[],
        args: "",
        group: Group::Window,
        summary: "Toggle the focused window between tiled and floating.",
        detail: "",
    },
    Action {
        name: "togglefullscreen",
        aliases: &[],
        args: "",
        group: Group::Window,
        summary: "Toggle work-area fullscreen (bar stays visible).",
        detail: "Window resizes to the monitor's work_area — i.e. everything below the bar / above an overlay. Layer-shell surfaces (noctalia bar, notifications) keep rendering on top. Standard `F11` feel.",
    },
    Action {
        name: "togglefullscreen_exclusive",
        aliases: &["togglefullscreen-exclusive", "togglefullscreenexclusive"],
        args: "",
        group: Group::Window,
        summary: "Toggle exclusive fullscreen (bar hidden, full panel).",
        detail: "Window covers the entire output (`monitor_area`) and the render path suppresses every layer-shell surface for that monitor — the bar literally disappears while exclusive fullscreen is active. Right behaviour for mpv / browser fullscreen movie / fullscreen games.",
    },
    Action {
        name: "sticky_window",
        aliases: &["togglesticky", "toggle_sticky", "sticky"],
        args: "",
        group: Group::Window,
        summary: "Pin the focused window to every tag on its monitor.",
        detail: "Saves the current tag mask, then sets tags = u32::MAX so the \
                 window appears across every tag of its current monitor. \
                 A second press restores the saved tag set. Equivalent to \
                 niri-float-sticky's per-window sticky, built into the \
                 compositor — no external daemon needed. Skipped silently \
                 for windows currently in scratchpad state.",
    },
    Action {
        name: "killclient",
        aliases: &[],
        args: "",
        group: Group::Window,
        summary: "Close the focused window (xdg `close` / X11 `WM_DELETE_WINDOW`).",
        detail: "",
    },
    Action {
        name: "movewin",
        aliases: &[],
        args: "<DX> <DY>",
        group: Group::Window,
        summary: "Move the focused window by `dx` × `dy` pixels.",
        detail: "Forces the window to floating. Bind multiple binds to step \
                 around (e.g. super+ctrl+h/j/k/l, ±40 px).",
    },
    Action {
        name: "resizewin",
        aliases: &[],
        args: "<DW> <DH>",
        group: Group::Window,
        summary: "Resize the focused window by `dw` × `dh` pixels.",
        detail: "Forces the window to floating; min size 50 × 50.",
    },
    Action {
        name: "moveresize",
        aliases: &[],
        args: "<curmove|curresize>",
        group: Group::Window,
        summary: "Start an interactive pointer grab to move (`curmove`) or resize (`curresize`).",
        detail: "Intended for `mousebind` lines. Example: \
                 `mousebind = super,lmb,moveresize,curmove`.",
    },
    Action {
        name: "tagmon",
        aliases: &[],
        args: "<DIRECTION>",
        group: Group::Window,
        summary: "Move the focused window to an adjacent monitor.",
        detail: "",
    },
    Action {
        name: "disable_output",
        aliases: &["disable-output"],
        args: "<NAME>",
        group: Group::Window,
        summary: "Soft-disable an output (migrates clients off it).",
        detail: "Marks the named output (e.g. eDP-1) as disabled — \
                 clients on it are migrated to the first remaining \
                 enabled output, arrange + render skip it from then \
                 on. The smithay Output stays alive so a later \
                 `enable_output` resumes without a hotplug. The DRM \
                 panel itself is NOT powered down here (follow-up). \
                 Refused if it would leave zero active outputs.",
    },
    Action {
        name: "enable_output",
        aliases: &["enable-output"],
        args: "<NAME>",
        group: Group::Window,
        summary: "Re-enable a previously soft-disabled output.",
        detail: "Reverse of `disable_output`. Existing clients aren't \
                 automatically pulled back; they stay on whichever \
                 output the disable pass migrated them to.",
    },
    Action {
        name: "toggle_output",
        aliases: &["toggle-output"],
        args: "<NAME>",
        group: Group::Window,
        summary: "Toggle disable/enable on an output by name.",
        detail: "Convenience for dock/undock keybinds: \
                 `bind = super+ctrl,F12,toggle_output,eDP-1`.",
    },

    // ── Scratchpad ─────────────────────────────────────────────────
    Action {
        name: "toggle_named_scratchpad",
        aliases: &["togglenamedscratchpad", "toggle-named-scratchpad"],
        args: "<APPID> <TITLE|none> <SPAWN>",
        group: Group::Scratchpad,
        summary: "Show / hide a named scratchpad (mango pattern); spawn if absent.",
        detail: "First press launches the spawn-cmd if no client matches the \
                 appid+title regex; subsequent presses toggle hide/show. \
                 Works with windowrule `isnamedscratchpad:1`.",
    },
    Action {
        name: "summon",
        aliases: &["taghere", "tag_here", "tag-here", "bring_here", "bringhere"],
        args: "<APPID> <TITLE|none> <SPAWN>",
        group: Group::Scratchpad,
        summary: "Bring an app to the current tag, or launch it if not running.",
        detail: "mango-here equivalent. Searches every monitor/tag for a window \
                 matching the appid (and optional title) regex. If found, moves \
                 it to the focused monitor's active tag and focuses it. If not, \
                 spawns the SPAWN command. Hidden scratchpads are skipped — use \
                 `toggle_named_scratchpad` for those. Bind: \
                 `bind = alt,1,summon,^firefox$,none,firefox`.",
    },
    Action {
        name: "toggle_scratchpad",
        aliases: &["togglescratchpad"],
        args: "",
        group: Group::Scratchpad,
        summary: "Toggle every anonymous scratchpad on the focused monitor.",
        detail: "",
    },
    Action {
        name: "unscratchpad",
        aliases: &["unscratchpad_focused", "exit_scratchpad"],
        args: "",
        group: Group::Scratchpad,
        summary: "Reset the focused window's scratchpad / floating / fullscreen state.",
        detail: "Emergency recovery — clears in_scratchpad, scratchpad_show, \
                 named_scratchpad, minimized, floating, fullscreen, \
                 maximized_screen so the next arrange treats it as a normal tile.",
    },

    // ── Overview ────────────────────────────────────────────────────
    Action {
        name: "toggle_overview",
        aliases: &[],
        args: "",
        group: Group::Overview,
        summary: "Enter / leave the zoom-out overview of all tags.",
        detail: "Geometric Grid arrangement of every visible client across all tags inside the zoomed work area (`overview_zoom`, default 0.5). Each window keeps a deterministic spot — Mango/Hypr-style spatial continuity. Trigger via keybind, hot corner (`hot_corner_top_left = toggle_overview`), or 4-finger touchpad swipe up (`gesture = swipe, 4, up, toggle_overview`). Pair with `overview_focus_next/_prev` for niri-style alt+Tab keyboard MRU cycling.",
    },
    Action {
        name: "overview_focus_next",
        aliases: &[],
        args: "",
        group: Group::Overview,
        summary: "alt+Tab next thumbnail (opens overview if closed).",
        detail: "Cycles forward through visible thumbnails. If overview is closed, opens it first and lands on the first thumbnail. Each step calls `focus_surface` so the border + smithay keyboard focus track the selection — overview stays open until `overview_activate` (or close keybind) commits the choice.",
    },
    Action {
        name: "overview_focus_prev",
        aliases: &[],
        args: "",
        group: Group::Overview,
        summary: "alt+shift+Tab previous thumbnail (opens overview if closed).",
        detail: "Reverse direction of `overview_focus_next`. Same auto-open behaviour and same focus-follows-cycle semantics.",
    },
    Action {
        name: "overview_activate",
        aliases: &[],
        args: "",
        group: Group::Overview,
        summary: "Close overview keeping the currently-selected thumbnail focused.",
        detail: "Bind to Enter (or Esc) to commit the keyboard-cycle's selection. Without a hover/focus target, falls through to `close_overview(None)` and restores the pre-overview tag.",
    },

    // ── System ──────────────────────────────────────────────────────
    Action {
        name: "spawn",
        aliases: &[],
        args: "<COMMAND>",
        group: Group::System,
        summary: "Run a shell command (passes through `sh -c`).",
        detail: "",
    },
    Action {
        name: "run_script",
        aliases: &["run-script", "rhai-eval"],
        args: "<PATH>",
        group: Group::System,
        summary: "Evaluate a Rhai script against the live compositor (W3.2).",
        detail: "Same binding surface + sandbox as ~/.config/margo/init.rhai; \
                 works whether or not init.rhai exists. Hooks registered \
                 inside the script persist after the run. User-facing \
                 wrapper: `mctl run <file>`.",
    },
    // ── Screenshot ──────────────────────────────────────────────────
    // All screenshot actions delegate to the `mscreenshot`
    // shell helper, which orchestrates grim + slurp + wl-copy +
    // an optional editor (swappy / satty). Required tools live in
    // PKGBUILD `depends`; the editor is `optdepends`.
    Action {
        name: "screenshot",
        aliases: &["screenshot-screen", "screenshot_screen"],
        args: "[window]",
        group: Group::System,
        summary: "Capture the focused output → editor → file.",
        detail: "Spawns `mscreenshot screen` (or `window` when arg is \
                 `window`). Saves under $SCREENSHOT_SAVE_DIR or \
                 $XDG_PICTURES_DIR/Screenshots as screenshot_TIMESTAMP.png. \
                 If swappy/satty is installed, the saved file opens for \
                 quick edits before being committed.",
    },
    Action {
        name: "screenshot-window",
        aliases: &["screenshot_window"],
        args: "",
        group: Group::System,
        summary: "Capture the focused window → editor → file.",
        detail: "Spawns `mscreenshot window`.",
    },
    Action {
        name: "screenshot-region",
        aliases: &["screenshot_region"],
        args: "",
        group: Group::System,
        summary: "Drag a region → editor → file.",
        detail: "Spawns `mscreenshot area`. Uses `slurp` for selection.",
    },
    Action {
        name: "screenshot-region-ui",
        aliases: &["screenshot_region_ui"],
        args: "",
        group: Group::System,
        summary: "Drag a region → editor → file + clipboard.",
        detail: "Spawns `mscreenshot rec`. Region selection via `slurp`, \
                 capture via `grim`, clipboard via `wl-copy`.",
    },
    Action {
        name: "screenshot-output",
        aliases: &["screenshot_output"],
        args: "",
        group: Group::System,
        summary: "Alias of `screenshot` — capture the focused output.",
        detail: "Spawns `mscreenshot screen`. The `[NAME]` arg from \
                 earlier revisions is no longer honoured; the helper \
                 captures whatever is focused.",
    },
    Action {
        name: "reload",
        aliases: &["reload_config"],
        args: "",
        group: Group::System,
        summary: "Reload `~/.config/margo/config.conf`.",
        detail: "Re-reads keybinds, animation params, window rules, layer rules. Also invalidates the runtime theme baseline so `theme default` resets to the freshly-parsed values.",
    },
    Action {
        name: "theme",
        aliases: &["set_theme"],
        args: "<preset>",
        group: Group::System,
        summary: "Live-swap the visual theme preset (no config reload).",
        detail: "Built-in presets: `default` (restore the values from config.conf), `minimal` (no shadows/blur, thin square borders), `gaudy` (chunky rounded borders, deep drop shadows). Borders, shadows, blur all re-render on the next frame.",
    },
    Action {
        name: "session_save",
        aliases: &["save_session"],
        args: "",
        group: Group::System,
        summary: "Save per-monitor tag/layout state to disk.",
        detail: "Writes a JSON snapshot to `$XDG_STATE_HOME/margo/session.json` (defaults to `~/.local/state/margo/session.json`). Captures every monitor's seltags, tagset, and per-tag layout/mfact/nmaster/canvas-pan. Open windows are NOT captured — those belong to user-space spawn lines.",
    },
    Action {
        name: "session_load",
        aliases: &["load_session"],
        args: "",
        group: Group::System,
        summary: "Restore per-monitor tag/layout state from disk.",
        detail: "Reads `$XDG_STATE_HOME/margo/session.json` and re-applies it to whatever monitors are present today (matched by output name). Snapshot entries for monitors that aren't connected get skipped (logged, not an error). Triggers an arrange + repaint so the new state is visible on the next frame.",
    },
    Action {
        name: "quit",
        aliases: &[],
        args: "",
        group: Group::System,
        summary: "Exit the compositor cleanly.",
        detail: "",
    },
    Action {
        name: "setkeymode",
        aliases: &[],
        args: "<MODE>",
        group: Group::System,
        summary: "Switch keymode (per-mode bind sets, like vim's modes).",
        detail: "",
    },
    Action {
        name: "force_unlock",
        aliases: &["force-unlock"],
        args: "",
        group: Group::System,
        summary: "Tear down a stuck `ext_session_lock` from the compositor side.",
        detail: "Useful when noctalia / Quickshell can't accept keyboard \
                 input on the lock screen and the user would otherwise have \
                 to switch to a TTY.",
    },
    Action {
        name: "debug_dump",
        aliases: &["debug-dump", "diagnose"],
        args: "",
        group: Group::System,
        summary: "Dump compositor state (clients, monitors, layouts) to the log.",
        detail: "",
    },
];

/// Every spelling the dispatcher accepts (canonical names + aliases),
/// flattened. Used by completion scripts to feed both names and
/// aliases into the user's tab-completion pool.
pub fn all_names() -> Vec<&'static str> {
    let mut out = Vec::with_capacity(ACTIONS.len() * 2);
    for action in ACTIONS {
        out.push(action.name);
        for alias in action.aliases {
            out.push(alias);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Static list of layout names accepted by `setlayout`.
pub const LAYOUT_NAMES: &[&str] = &[
    "tile",
    "scroller",
    "grid",
    "monocle",
    "deck",
    "center_tile",
    "right_tile",
    "vertical_tile",
    "vertical_scroller",
    "vertical_grid",
    "vertical_deck",
    "tgmix",
    "canvas",
    "dwindle",
];
