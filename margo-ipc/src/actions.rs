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
        summary: "Toggle the focused window's fullscreen state.",
        detail: "",
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
        name: "toggleoverview",
        aliases: &[],
        args: "",
        group: Group::Overview,
        summary: "Enter / leave the tag-overview grid (zoom-out of all tags).",
        detail: "",
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
    // ── Screenshot ──────────────────────────────────────────────────
    Action {
        name: "screenshot",
        aliases: &["screenshot-screen", "screenshot_screen"],
        args: "[output|window|clipboard|output:NAME]",
        group: Group::System,
        summary: "Capture the focused output (default), focused window, or specific output.",
        detail: "Native compositor capture — no `grim`/`slurp` subprocess. \
                 Saves to $SCREENSHOT_SAVE_DIR or $XDG_PICTURES_DIR/Screenshots, \
                 named `screenshot_TIMESTAMP.png`, and copies the same PNG to \
                 the clipboard via `wl-copy` (requires `wl-clipboard`). The \
                 `clipboard` mode skips disk and only sets the selection.",
    },
    Action {
        name: "screenshot-window",
        aliases: &["screenshot_window"],
        args: "",
        group: Group::System,
        summary: "Capture the focused window — content only, no decoration.",
        detail: "",
    },
    Action {
        name: "screenshot-output",
        aliases: &["screenshot_output"],
        args: "[NAME]",
        group: Group::System,
        summary: "Capture a specific output by connector name (defaults to focused).",
        detail: "Useful in multi-monitor setups: `screenshot-output DP-3` shoots \
                 the external monitor regardless of which one currently has focus.",
    },
    Action {
        name: "screenshot-region",
        aliases: &["screenshot_region"],
        args: "[clipboard|no-clip]",
        group: Group::System,
        summary: "Region screenshot via slurp + native capture.",
        detail: "Spawns `slurp` for rectangle selection (drag-to-select, Esc \
                 cancels), then captures + PNG-encodes that rect natively in \
                 the compositor. Default: save to disk + clipboard. \
                 `clipboard` = clipboard only. `no-clip` = save only.",
    },
    Action {
        name: "screenshot-region-ui",
        aliases: &["screenshot_region_ui"],
        args: "[clipboard|no-clip]",
        group: Group::System,
        summary: "In-compositor region selector — no `slurp` dependency.",
        detail: "Captures every output to a frozen GLES texture, dims the \
                 scene, lets the user drag out a selection on top of the \
                 frozen image, and confirms on Return (Esc cancels). Pointer \
                 + keyboard are intercepted while the selector is open — no \
                 client gets stray input. Cross-output drags are not \
                 supported (the active rectangle stays on one monitor).",
    },
    Action {
        name: "reload",
        aliases: &["reload_config"],
        args: "",
        group: Group::System,
        summary: "Reload `~/.config/margo/config.conf`.",
        detail: "Re-reads keybinds, animation params, window rules, layer rules.",
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
