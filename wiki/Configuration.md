# Configuration

margo reads `~/.config/margo/config.conf` — plain `key = value`, hot-reloadable
with `mctl reload` (or `Super+Ctrl+R`). Everything has a sane built-in default,
so you only set what you want to change.

```bash
mctl reload          # re-read the config in place — no logout
mctl check-config    # validate offline; exits 1 on any error (unknown keys,
                     # regex errors, duplicate/shadowed binds, include loops)
mctl actions --verbose   # the always-current dispatch-action catalogue
```

> **Full annotated reference:** every knob with inline commentary lives at
> <https://kenanpelit.github.io/margo/config-reference/>, rendered from
> [`margo/src/config.example.conf`](https://github.com/kenanpelit/margo/blob/main/margo/src/config.example.conf)
> (also installed at `/usr/share/doc/margo-git/config.example.conf`). This page
> is the structured guide to the high-traffic options.

## Includes & multi-file configs

```ini
include = keybinds.conf                 # single file
include = rules.conf
source  = ~/.config/margo/conf.d/*.conf # glob, lex order
```

`mctl reload` re-reads the entire tree.

## Layouts

margo ships **11 layouts** — 9 partition-style tiling algorithms plus
**floating** and **mosaic** for non-partition desktops — and a separate
zoom-out **overview** mode; each tag remembers its own choice. Set one with
`setlayout <name>`, or cycle with `switch_layout`.

| `setlayout` name | Layout | Description |
| --- | --- | --- |
| `tile` | Tile | dwm classic — master + stack |
| `scroller` | Scroller | PaperWM-style horizontal column scroller (the default) |
| `grid` | Grid | equal-area grid |
| `monocle` | Monocle | one window fills the area; siblings hidden |
| `deck` | Deck | master + stacked tabs |
| `center_tile` | Center tile | master centred, stacks left + right |
| `right_tile` | Right tile | master pane on the right |
| `tgmix` | TG-mix | tile / grid hybrid |
| `dwindle` | Dwindle | recursive split (Hyprland default) |
| `floating` | Floating | classic stacking desktop — windows auto-float with a cascade |
| `mosaic` | Mosaic | [GNOME's content-aware self-arranging concept](https://blogs.gnome.org/tbernard/2023/07/26/rethinking-window-management/) — windows keep their own size, drag one onto another to swap, overflow moves to an empty tag |
| `overview` | Overview | zoom-out of all tags — usually via `toggle_overview` |

```ini
default_layout = scroller          # any tag without a tagrule uses this
circle_layout  = scroller, tile, center_tile, grid, deck, monocle, dwindle

bind = super, t, setlayout, tile   # set the current tag's layout
bind = super, e, switch_layout     # cycle circle_layout
tagrule = id:9, layout_name:monocle
```

Per-tag precedence: **`taglayout` > `tagrule layout_name` > `default_layout`**.
A manual `setlayout` is never clobbered by a reload.

### Master / stack tuning

| Key | Default | Meaning |
| --- | --- | --- |
| `default_mfact` | `0.55` | master pane fraction (0.05–0.95) |
| `default_nmaster` | `1` | windows in the master area |
| `new_is_master` | `0` | `1` = new windows take the master slot |
| `center_master_overspread` | `0` | center_tile: master may overrun the stacks |
| `center_when_single_stack` | `1` | center_tile: centre master with one stack |

```ini
bind = super,       i, incnmaster, +1
bind = super,       d, incnmaster, -1
bind = super,       h, setmfact,   -0.05
bind = super,       l, setmfact,   +0.05
bind = super,       g, togglegaps
bind = super+shift, g, incgaps,    +4
```

### Scroller options

| Key | Default | Meaning |
| --- | --- | --- |
| `scroller_default_proportion` | `0.800` | column width when ≥2 columns are visible |
| `scroller_default_proportion_single` | `0.800` | width when one column shows |
| `scroller_ignore_proportion_single` | `0` | ignore the `*_single` value |
| `scroller_focus_center` | `1` | auto-centre the focused column |
| `scroller_prefer_center` | `1` | open new columns near the focused one |
| `scroller_prefer_overspread` | `0` | let columns exceed the monitor width |
| `edge_scroller_pointer_focus` | `1` | edge pointer scrolls + re-focuses |
| `scroller_proportion_preset` | `1.000, 0.800, 0.618, 0.500` | `switch_proportion_preset` cycle (Φ = 0.618) |

```ini
bind = super, r, set_proportion, 0.618
bind = super, p, switch_proportion_preset
```

## Dispatch actions

Bind these to a key (`bind`), button (`mousebind`), or gesture
(`gesturebind`), or run them live with `mctl dispatch <action> [args]`.
Comma-separated aliases are interchangeable. Generated from the action
catalogue — the same source as `mctl actions --verbose` — and checked by a
test, so every action margo accepts is listed.

<!-- BEGIN dispatch-actions: generated from margo-config/src/actions.rs by `just docs-actions`; do not edit by hand -->

### Tag / Workspace

| Action | Arg | Description |
| --- | --- | --- |
| `view` | `<MASK>` | Switch to tag(s) by bitmask. Pressing the same tag twice toggles to the previously active tag when `view_current_to_back = 1` (dwl/mango alt-tab-for-tags pattern). Mask is 1&lt;&lt;(tag-1) → tag 1=1, tag 8=128. |
| `toggleview` | `<MASK>` | Add or remove a tag from the active set (multi-tag view). |
| `tag`, `tagsilent` | `<MASK>` | Move the focused window to tag(s); user stays on the current tag. dwm/dwl style. Use `tagview` to follow the window to its new tag. |
| `tagview`, `tag_view`, `tag-view`, `movetagview` | `<MASK>` | Move the focused window AND switch to that tag (Hyprland follow). |
| `toggletag` | `<MASK>` | Add or remove a tag from the focused window's mask. |
| `tagall` |  | Show every tag at once (mask = 2^32-1). |
| `viewtoleft`, `viewtoleft_have_client` |  | Cycle view to the previous occupied tag on the focused monitor. |
| `viewtoright`, `viewtoright_have_client` |  | Cycle view to the next occupied tag on the focused monitor. |
| `view_insert` | <code>&lt;prev&#124;next&gt;</code> | View the adjacent tag if empty, else insert a fresh empty tag and switch to it. Ported from mango 0.16.1. If the neighbouring tag already has windows, a new empty tag is inserted before (`prev`) or after (`next`) the current one — every later tag's clients and per-tag layout state shift up by one — and you land on it. No-op if the current view isn't a single tag, or there's no room to shift into (the last tag is already occupied). |
| `tagtoleft` |  | Move the focused window to the previous tag. |
| `tagtoright` |  | Move the focused window to the next tag. |
| `settagset` | `<MASK>` | Set the focused monitor's active tagset to a raw bitmask (arg 1). Tag N is bit `1 << (N-1)`. Arg 2 nonzero writes the *inactive* tagset slot instead. Low-level — `view` / `toggleview` are the usual verbs. |
| `setclienttags` | `<AND-MASK> <XOR-MASK>` | Rewrite the focused window's tags as `(tags & arg1) ^ arg2`. Low-level bit surgery behind `tag` / `toggletag`. No-op if the result would be 0 (a window on no tag). |

### Focus

| Action | Arg | Description |
| --- | --- | --- |
| `focusstack` | `<DIRECTION>` | Cycle keyboard focus through the stack: next (1) / previous (-1). The direction may be a signed integer or one of `left right up down prev next` (left/up = previous). |
| `focusdir` | <code>&lt;left&#124;right&#124;up&#124;down&gt;</code> | Focus the nearest window in a direction. Spatial, like mango: monocle steps the stack on left/right. At the edge `focus_cross_tag` jumps to the nearest occupied tag, else `focus_cross_monitor` to the adjacent monitor (which also lets the search include other monitors). A numeric argument cycles the stack like `focusstack`. |
| `focuslast`, `focus_last`, `focus-last` |  | Toggle focus back to the previously-focused window on this monitor. dwl's `focuslast`: jumps to the window you were on before the current one; press again to return. No-op when there is no prior window, or it has since closed or left the tag. |
| `focus_window_or_workspace`, `focuswindoworworkspace` | `<DIRECTION>` | Focus the window in a direction, or switch to the adjacent workspace if there is none that way. Spatial directional focus with an edge fallback (mango 0.15.5): if a window lies `left`/`right`/`up`/`down` of the focused one it gains focus; otherwise the compositor views the previous tag (left/up) or next tag (right/down). Unlike `focusdir` (which cycles the stack and never runs out), this uses true geometry so it can fall through at a tag edge. |
| `exchange_client`, `smartmovewin` | `<DIRECTION>` | Swap the focused window with the next/previous one in stacking order. Despite the `<DIRECTION>` argument this is list-order cycling (`direction_arg` collapses left/up to prev, right/down to next), not true spatial neighbour selection — see `move_client` for that. |
| `move_client` | `<DIRECTION>` | Move the focused window toward a direction, displacing its neighbour. True spatial neighbour selection (same geometry `focus_window_or_workspace` uses), unlike `exchange_client`'s list-order cycling. Removes the focused window from its slot in the layout's client order and re-inserts it next to the neighbour in that direction, shifting whatever was in between — every margo layout (dwindle included) derives its geometry purely from that order, so one reorder covers all of them. No-op on a floating/fullscreen focus or when there's no neighbour that way (mango 0.17 port, c844a107; no cross-monitor edge case — margo has no directional monitor-selection primitive to jump through). |
| `focusmon` | `<DIRECTION>` | Move keyboard focus to another monitor. Direction: left/right/up/down or 1/-1. |
| `zoom` |  | Promote the focused window to the layout's master slot (dwm zoom). |
| `focuswindow`, `activatewindow`, `focusclient` | `<INDEX>` | Focus a window by its positional index (arg 1). The index is the client's slot in the state snapshot's `clients` array. Racy if a window closes between the snapshot and the dispatch — prefer `focuswindowid`. |
| `focuswindowid` | `<ID>` | Focus a window by its stable, session-monotonic id (arg 1). Race-free variant of `focuswindow` — the id doesn't shift when another window closes. |
| `mru_next`, `mruwindow` | `<SCOPE> <FILTER>` | Cycle forward through the most-recently-used window stack (niri Super+Tab). Arg 1 = scope (`all` / `output` / `workspace`), arg 2 = filter (`all` / `appid`). Release the modifier to commit the pick. |
| `mru_prev` | `<SCOPE> <FILTER>` | Cycle backward through the most-recently-used window stack. Same scope / filter args as `mru_next`. |

### Layout

| Action | Arg | Description |
| --- | --- | --- |
| `setlayout` | `<NAME>` | Switch the current tag's layout by name. Names: tile, scroller, grid, monocle, deck, center_tile, right_tile, tgmix, dwindle, floating, mosaic. |
| `switch_layout` |  | Cycle through the `circle_layout` config list. |
| `setlayoutindex` | `<INDEX>` | Switch the current tag's layout by 0-based index (arg 1). Same index space as `mctl layout <N>` and the `layouts` array in the state snapshot. `setlayout <name>` is more stable across config changes. |
| `incnmaster` | `<DELTA>` | Change the master-slot count (+1 / -1) for the current tag. |
| `setmfact` | `<DELTA>` | Adjust the master factor for the current tag (e.g. 0.05 / -0.05). Floating-point delta — clamped to [0.05, 0.95]. |
| `resetmfact`, `reset_mfact`, `resetsplit`, `reset_split` | `[MONITOR]` | Reset the current tag's master factor (split ratio) to its configured value. Restores the tag rule's `mfact` for this tag, else `default_mfact`. An optional output name targets that monitor instead of the focused one (what the shell's layout menu sends). |
| `togglegaps` |  | Toggle layout gaps on/off. |
| `incgaps` | `<DELTA>` | Resize gaps by `delta` pixels (positive widens, negative tightens). |

### Scroller

| Action | Arg | Description |
| --- | --- | --- |
| `set_proportion` | `<RATIO>` | Set the focused window's scroller width ratio (0.1 – 1.0). |
| `switch_proportion_preset` |  | Cycle through `scroller_proportion_preset` values. |

### Window

| Action | Arg | Description |
| --- | --- | --- |
| `togglefloating` |  | Toggle the focused window between tiled and floating. |
| `togglegroup`, `toggle_group`, `togglegroupmerge` |  | Group/ungroup the focused window with its layout neighbour (tabbed group). Hyprland-style window groups. Grouping collapses the windows into ONE tile that shows one member at a time, with a tab strip across the top (`group_bar_height`). If the focused window is already grouped, this pulls it back out; the group dissolves when only one member remains. No-op while `lockgroups` is on. Bind: `bind = super,g,togglegroup`. |
| `changegroupactive`, `change_group_active`, `groupcycle` | <code>&lt;next&#124;prev&gt;</code> | Cycle which member of the focused window's group is displayed. Wraps at the ends; moves keyboard focus to the newly-shown member. No-op if the focused window isn't grouped. Defaults to `next` when no arg. Binds: `bind = super,Tab,changegroupactive,next` / `bind = super+shift,Tab,changegroupactive,prev`. |
| `movegroupwindow`, `move_group_window`, `movewindowingroup` | <code>&lt;next&#124;prev&gt;</code> | Reorder the focused window within its group's tab strip (no wrap). Swaps the focused member with the adjacent one in tab order. No-op at the ends or if the focused window isn't grouped. |
| `movewindowtogroup`, `move_window_to_group` |  | Absorb the focused window into its layout neighbour's group. Like `togglegroup` but only ever ADDS to a group — never ungroups. Useful for 'pull the next window into my tabs'. |
| `lockgroups`, `lock_groups` | <code>&lt;on&#124;off&#124;toggle&gt;</code> | Freeze group/ungroup operations (existing groups still cycle). Hyprland `lockgroups`. While locked, `togglegroup` / `movewindowtogroup` / auto-group windowrules are no-ops, so you can rearrange windows without accidentally merging them. Defaults to `toggle`. Bind: `bind = super+alt,g,lockgroups,toggle`. |
| `togglefullscreen` |  | Toggle work-area fullscreen (bar stays visible). Window resizes to the monitor's work_area — i.e. everything below the bar / above an overlay. Layer-shell surfaces (noctalia bar, notifications) keep rendering on top. Standard `F11` feel. |
| `togglefullscreen_exclusive`, `togglefullscreen-exclusive`, `togglefullscreenexclusive` |  | Toggle exclusive fullscreen (bar hidden, full panel). Window covers the entire output (`monitor_area`) and the render path suppresses every layer-shell surface for that monitor — the bar literally disappears while exclusive fullscreen is active. Right behaviour for mpv / browser fullscreen movie / fullscreen games. |
| `sticky_window`, `togglesticky`, `toggle_sticky`, `sticky` |  | Pin the focused window to every tag on its monitor. Saves the current tag mask, then sets tags = u32::MAX so the window appears across every tag of its current monitor. A second press restores the saved tag set. Equivalent to niri-float-sticky's per-window sticky, built into the compositor — no external daemon needed. Skipped silently for windows currently in scratchpad state. |
| `killclient` |  | Close the focused window (xdg `close` / X11 `WM_DELETE_WINDOW`). |
| `smartkillclient` |  | Remove the focused window from the current tag(s) if it's on more than one tag; otherwise close it. A window pinned across several tags (e.g. `sticky_window`, or manually tagged onto more than one) disappears from just the tag(s) you're viewing instead of closing outright — it's still open on its other tags. Falls back to a normal `killclient` when it's on a single tag, or when removing the viewed tag(s) would leave it on none. |
| `movewin` | `<DX> <DY>` | Move the focused window by `dx` × `dy` pixels. Forces the window to floating. Bind multiple binds to step around (e.g. super+ctrl+h/j/k/l, ±40 px). |
| `resizewin` | `<DW> <DH>` | Resize the focused window by `dw` × `dh` pixels. Forces the window to floating; min size 50 × 50. |
| `moveresize` | <code>&lt;curmove&#124;curresize&gt;</code> | Start an interactive pointer grab to move (`curmove`) or resize (`curresize`). Intended for `mousebind` lines. Example: `mousebind = super,lmb,moveresize,curmove`. |
| `tagmon` | `<DIRECTION>` | Move the focused window to an adjacent monitor. |
| `disable_output`, `disable-output` | `<NAME>` | Soft-disable an output (migrates clients off it). Marks the named output (e.g. eDP-1) as disabled — clients on it are migrated to the first remaining enabled output, arrange + render skip it from then on. The smithay Output stays alive so a later `enable_output` resumes without a hotplug. The DRM panel itself is NOT powered down here (follow-up). Refused if it would leave zero active outputs. |
| `enable_output`, `enable-output` | `<NAME>` | Re-enable a previously soft-disabled output. Reverse of `disable_output`. Existing clients aren't automatically pulled back; they stay on whichever output the disable pass migrated them to. |
| `toggle_output`, `toggle-output` | `<NAME>` | Toggle disable/enable on an output by name. Convenience for dock/undock keybinds: `bind = super+ctrl,F12,toggle_output,eDP-1`. |
| `mirror_output`, `mirror-output`, `mirror` | `[SOURCE] [TARGET]` | Mirror one output onto another (presentation mode). Shows SOURCE's picture on TARGET, scaled to fit and letterboxed; TARGET leaves the layout (its windows move, the pointer can't enter it) until `stop_mirror` or either output unplugs. No args: the laptop panel onto the first external output. `mctl mirror eDP-1 DP-3`. |
| `toggle_mirror`, `toggle-mirror` | `[SOURCE] [TARGET]` | Stop mirroring, or start it like `mirror_output`. `bind = super,p,toggle_mirror`. |
| `stop_mirror`, `stop-mirror`, `unmirror` |  | Stop output mirroring; the target returns to the layout. No-op when nothing is mirrored. |

### Scratchpad

| Action | Arg | Description |
| --- | --- | --- |
| `toggle_named_scratchpad`, `togglenamedscratchpad`, `toggle-named-scratchpad` | <code>&lt;APPID&gt; &lt;TITLE&#124;none&gt; &lt;SPAWN&gt;</code> | Show / hide a named scratchpad (mango pattern); spawn if absent. First press launches the spawn-cmd if no client matches the appid+title regex; subsequent presses toggle hide/show. Works with windowrule `isnamedscratchpad:1`. |
| `togglescratchpadid`, `toggle_scratchpad_id` | `<ID>` | Show / hide one scratchpad window by its stable id (arg 1). What the shell's scratchpad chips send. Only acts on a window that is already a scratchpad; honours `single_scratchpad`. |
| `minimize`, `minimize_window`, `minimizewindow` | `[ID]` | Minimize the focused window, or the window with stable id ID. The window leaves every tag until restored (`restore_last`, `restore_all`, `restore_window`, a click in the Scratchpads pill or a dock, or any activation). A shown scratchpad is tucked away instead. Apps' own minimize buttons and taskbars (wlr-foreign-toplevel) do the same. |
| `restore_last`, `restorelast`, `restore-last`, `unminimize` |  | Restore the most recently minimized window. Onto the focused monitor's current tag, or back to its own tag with `minimize_restore_in_place = 1`. No-op when nothing is minimized. |
| `restore_all`, `restoreall`, `restore-all` |  | Restore every minimized window. Oldest first, so the most recently minimized one ends up focused. |
| `restore_window`, `restorewindow`, `restore-window` | `<ID>` | Restore one minimized window by its stable id (arg 1). What the shell's Scratchpads pill sends. No-op unless that window is minimized. |
| `summon`, `taghere`, `tag_here`, `tag-here`, `bring_here`, `bringhere` | <code>&lt;APPID&gt; &lt;TITLE&#124;none&gt; &lt;SPAWN&gt; [OP]</code> | Bring an app to the current tag, or launch it if not running. mango-here equivalent. Searches every monitor/tag for a window matching the appid (and optional title) regex. If found, moves it to the focused monitor's active tag and focuses it. If not, spawns the SPAWN command. When several windows match, repeated presses cycle through them (run-or-raise style). The optional 4th field OP combines appid+title: `and` (default), `or`, or `difference` (appid but not title). Hidden scratchpads are skipped — use `toggle_named_scratchpad` for those. Bind: `bind = alt,1,summon,^firefox$,none,firefox`. |
| `focusapp`, `raiseapp`, `runorraise`, `run_or_raise`, `focus_app` | <code>&lt;APPID&gt; &lt;TITLE&#124;none&gt; &lt;SPAWN&gt; [OP]</code> | Focus an app where it is (switch to its tag), or launch it. Run-or-raise: the counterpart to `summon`. Instead of bringing the window to the current tag, it goes TO the window — switches to the window's tag/monitor and focuses it. Same matching as `summon` (appid/title regex, cycling on repeat, optional OP operator); spawns SPAWN if no instance is running. Bind: `bind = super,b,focusapp,^firefox$,none,firefox`. |
| `toggle_scratchpad`, `togglescratchpad` |  | Toggle every anonymous scratchpad on the focused monitor. |
| `unscratchpad`, `unscratchpad_focused`, `exit_scratchpad` |  | Reset the focused window's scratchpad / floating / fullscreen state. Emergency recovery — clears in_scratchpad, scratchpad_show, named_scratchpad, minimized, floating, fullscreen, maximized_screen so the next arrange treats it as a normal tile. |

### Overview

| Action | Arg | Description |
| --- | --- | --- |
| `toggle_overview` |  | Enter / leave the zoom-out overview of all tags. Geometric Grid arrangement of every visible client across all tags inside the zoomed work area (`overview_zoom`, default 0.5). Each window keeps a deterministic spot — Mango/Hypr-style spatial continuity. Trigger via keybind, hot corner (`hot_corner_top_left = toggle_overview`), or 4-finger touchpad swipe up (`gesture = swipe, 4, up, toggle_overview`). Pair with `overview_focus_next/_prev` for niri-style alt+Tab keyboard MRU cycling. |
| `overview_focus_next` |  | alt+Tab next thumbnail (opens overview if closed). Cycles forward through visible thumbnails. If overview is closed, opens it first and lands on the first thumbnail. Each step calls `focus_surface` so the border + smithay keyboard focus track the selection — overview stays open until `overview_activate` (or close keybind) commits the choice. |
| `overview_focus_prev` |  | alt+shift+Tab previous thumbnail (opens overview if closed). Reverse direction of `overview_focus_next`. Same auto-open behaviour and same focus-follows-cycle semantics. |
| `overview_activate` |  | Close overview keeping the currently-selected thumbnail focused. Bind to Enter (or Esc) to commit the keyboard-cycle's selection. Without a hover/focus target, falls through to `close_overview(None)` and restores the pre-overview tag. |
| `toggle_grid_overview` |  | Enter / leave the grid overview specifically (ignores `overview_style`). `toggle_overview` picks grid or scroller per the config; this always drives the grid one. The `grid_overview_*` navigation verbs pair with it. |
| `open_grid_overview` |  | Open the grid overview (no-op if already open). |
| `close_grid_overview` |  | Close the grid overview (no-op if already closed). |
| `grid_overview_focus_next` |  | Grid overview: cycle to the next thumbnail. Grid-only counterpart of `overview_focus_next`. |
| `grid_overview_focus_prev` |  | Grid overview: cycle to the previous thumbnail. Grid-only counterpart of `overview_focus_prev`. |
| `grid_overview_activate` |  | Grid overview: commit the selected thumbnail and close. Grid-only counterpart of `overview_activate`. |
| `toggle_scroller_overview` |  | Enter / leave the niri-style scroller overview. Separate surface from the grid overview. `open_scroller_overview` / `close_scroller_overview` are the one-shot halves for press-to-open / Esc-to-close binds. |
| `open_scroller_overview` |  | Open the scroller overview (no-op if already open). |
| `close_scroller_overview` |  | Close the scroller overview (no-op if already closed). |
| `scroller_overview_focus_next` |  | Scroller overview: select the next column. |
| `scroller_overview_focus_prev` |  | Scroller overview: select the previous column. |
| `scroller_overview_activate` |  | Scroller overview: commit the selected column and close. |

### System

| Action | Arg | Description |
| --- | --- | --- |
| `spawn` | `<COMMAND>` | Run a shell command (passes through `sh -c`). |
| `sendkey` | `<COMBO>[,<APPID-REGEX>][,<FALLBACK>]` | Inject a synthetic key combo into the focused window. COMBO is `+`-joined modifiers + one key (xkb names): `ctrl+Tab`, `ctrl+shift+Tab`, `ctrl+Page_Up`. With an optional anchored APPID-REGEX the combo is sent only when the focused window's app_id matches; otherwise the optional FALLBACK action (`action[:arg]`, e.g. `focusdir:up`) runs. Handy on a touchpad gesture for browser tab switching. Layout-independent keys only (Tab, Page_Up/Down, arrows, F-keys, Home/End, Esc, Return, Space, digits). |
| `run_script`, `run-script`, `rhai-eval` | `<PATH>` | Evaluate a Rhai script against the live compositor (W3.2). Same binding surface + sandbox as ~/.config/margo/init.rhai; works whether or not init.rhai exists. Hooks registered inside the script persist after the run. User-facing wrapper: `mctl run <file>`. |
| `dpms` | <code>&lt;on&#124;off&#124;toggle&gt; [OUTPUT]</code> | Real panel power off / on via DPMS. Arg 1 defaults to `toggle`; arg 2 is an optional connector name (`eDP-1`), otherwise all outputs. Any input afterwards wakes the screen, so an off is always recoverable. `mctl dispatch dpms off eDP-1`. |
| `loglevel`, `log_level` | `<LEVEL>` | Set the file-log level live (error / warn / info / debug / trace). Wrapper: `mctl log level <lvl>`. Defaults to `info`. |
| `logenabled`, `log_enabled` | `<BOOL>` | Turn file logging on / off live. Accepts 1/true/on/yes/enable(d) or their negatives. Wrapper: `mctl log enable` / `mctl log disable`. |
| `twilight_toggle` |  | Toggle the built-in blue-light filter (twilight) on / off. Resets to the configured schedule and resamples on the next frame. |
| `twilight_preview` | `<KELVIN> [GAMMA%]` | Preview a fixed twilight colour temperature until reset. Arg 1 = Kelvin (default 4000), arg 2 = gamma % (default keep current). |
| `twilight_test` | `[SECONDS]` | Sweep twilight day→night over a few seconds, then restore. Arg 1 = duration in seconds (default 5, CLI clamps 1–60). |
| `twilight_reset` |  | Drop any twilight preview/test and return to the schedule. |
| `twilight_set` | `<field=value>` | Live-tweak a twilight config field (e.g. `day_temp=5500`). Not persisted — lasts until the next `mctl reload`. |
| `global_shortcuts_bind` | `<SPEC>` | Register an xdg-desktop-portal GlobalShortcuts binding. Sent by margo-portal over the control socket — not meant for a `bind =` line. |
| `global_shortcuts_unbind` | `<SPEC>` | Drop an xdg-desktop-portal GlobalShortcuts binding. Portal-internal, like `global_shortcuts_bind`. |
| `screenshot`, `screenshot-screen`, `screenshot_screen` | `[window]` | Capture the focused output → editor → file. Spawns `mscreenshot screen` (or `window` when arg is `window`). Saves under $SCREENSHOT_SAVE_DIR or $XDG_PICTURES_DIR/Screenshots as screenshot_TIMESTAMP.png. If swappy/satty is installed, the saved file opens for quick edits before being committed. |
| `screenshot-window`, `screenshot_window` |  | Capture the focused window → editor → file. Spawns `mscreenshot window`. |
| `screenshot-region`, `screenshot_region` |  | Drag a region → editor → file. Spawns `mscreenshot area`. Uses `slurp` for selection. |
| `screenshot-output`, `screenshot_output` |  | Alias of `screenshot` — capture the focused output. Spawns `mscreenshot screen`. The `[NAME]` arg from earlier revisions is no longer honoured; the helper captures whatever is focused. |
| `screenshot-ocr`, `screenshot_ocr` | `[LANGS]` | Drag a region → OCR (tesseract) → copy the text. Spawns `mscreenshot ocr`. Optional arg = tesseract languages joined with `+` (e.g. `eng+tur`); default is $MSCREENSHOT_OCR_LANG, else eng+tur narrowed to the installed tesseract-data-* packs. |
| `screenshot-qr`, `screenshot_qr` |  | Drag a region → decode QR codes / barcodes → copy. Spawns `mscreenshot qr` (needs `zbarimg`, package zbar). A single http(s) link gets an "Open link" notification button; other payloads are only copied. |
| `screenshot-palette`, `screenshot_palette` |  | Drag a region → extract its dominant colours → copy. Spawns `mscreenshot palette`: 8 colours by median cut, copied as hex codes (one per line) with a swatch strip in the notification. |
| `reload`, `reload_config` | `[force]` | Reload `~/.config/margo/config.conf`. Re-reads keybinds, animation params, window rules, layer rules. Also invalidates the runtime theme baseline so `theme default` resets to the freshly-parsed values. A config with errors (`mctl check-config`) is refused and the running one kept; `force` applies it anyway (not saved as the last-good generation, banner stays up). A file that can't be read is always refused. |
| `theme`, `set_theme` | `<preset>` | Live-swap the visual theme preset (no config reload). Built-in presets: `default` (restore the values from config.conf), `minimal` (no shadows/blur, thin square borders), `gaudy` (chunky rounded borders, deep drop shadows). Borders, shadows, blur all re-render on the next frame. |
| `session_save`, `save_session` |  | Save per-monitor tag/layout state and per-tag app lists to disk. Writes a JSON snapshot to `$XDG_STATE_HOME/margo/session.json` (defaults to `~/.local/state/margo/session.json`). Captures every monitor's seltags, tagset, and per-tag layout/mfact/nmaster, plus every open window's app id, output, tags and launch command (`/proc/<pid>/cmdline`) — the app lists `session_restore_windows` restores. |
| `session_load`, `load_session` |  | Restore per-monitor tag/layout state from disk. Reads `$XDG_STATE_HOME/margo/session.json` and re-applies it to whatever monitors are present today (matched by output name). Snapshot entries for monitors that aren't connected get skipped (logged, not an error). Triggers an arrange + repaint so the new state is visible on the next frame. With `session_restore_windows = 1` it also moves open windows back to their saved output and tag and relaunches saved apps that aren't open (each window they open lands on its saved tag). Never closes windows. `session_restore_at_login = 1` runs this at startup. |
| `session_end` | <code>&lt;reboot&#124;poweroff&#124;logout&#124;cancel&gt;</code> | Close every window gracefully, then reboot / power off / log out. Windows get a normal close request so browsers exit cleanly. Windows still open after `session_end_grace_ms` get a notification with a Cancel button; the action runs after `session_end_countdown_ms` or as soon as they close. `cancel` aborts a pending one. |
| `quit` |  | Exit the compositor cleanly. |
| `setkeymode` | `<MODE>` | Switch keymode (per-mode bind sets, like vim's modes). |
| `cyclekblayout`, `cycle_kb_layout`, `cycle-kb-layout` |  | Cycle the keyboard to the next configured xkb layout. No-op when a single `xkb_rules_layout` is configured. Wraps at the end of the list. The active layout name is published in the state snapshot (`keyboard_layout`) for the shell's keyboard-layout pill. |
| `internal_keyboard`, `internalkeyboard`, `toggle_internal_keyboard` | <code>[on&#124;off&#124;toggle]</code> | Turn the built-in keyboard off / on (default: toggle). For typing on an external keyboard over a laptop's own. The device is `internal_keyboard = <name>` (default: the i8042 keyboard). Turning it off is refused while it is the only keyboard, and unplugging the last other keyboard turns it back on. `internal_keyboard_auto = 1` turns it off when an external keyboard is plugged in. TTY consoles are not affected. |
| `force_unlock`, `force-unlock` |  | Restart a crashed lock screen (never unlocks). With the session locked and the locker gone, starts `mlock` again; the session stays locked until the password is typed. Ignored while a locker lives and while unlocked. margo also restarts a dead locker on its own. The only unlock without the locker is `loginctl unlock-session` from a TTY or ssh. Not bound by default. |
| `chvt` | `<vt>` | Switch to virtual terminal &lt;vt> (default bind: Ctrl+Alt+F1…F12). Bound automatically on every config; the session pauses and resumes through logind/seatd. Ignored when margo runs nested (winit). |
| `debug_dump`, `debug-dump`, `diagnose` |  | Dump compositor state (clients, monitors, layouts) to the log. |
| `wallpaper_set` | <code>&lt;*&#124;OUTPUT[,…]&gt; [fit=…] [bg=#rrggbb[aa]&#124;none] [fade=MS] [owner=TAG] &lt;PATH&gt;</code> | Draw an image as the compositor wallpaper on some or every output. Replaces the configured `wallpaper` there until `wallpaper_clear`. PATH is absolute: a headed `.raw` is read before the reply, other images decode in the background. fit is cover\|contain\|fill\|center\|scale-down; bg paints around a contained image; fade crossfades from the previous image. `get wallpaper` shows what is set. |
| `wallpaper_update` | <code>&lt;*&#124;OUTPUT[,…]&gt; [fit=…] [bg=#rrggbb[aa]&#124;none]</code> | Change the fit or background of a runtime wallpaper. Fails when nothing was set there with `wallpaper_set`. |
| `wallpaper_clear` | <code>[*&#124;OUTPUT[,…]] [owner=TAG]</code> | Drop runtime wallpapers; the configured `wallpaper` shows again. No argument or `*` clears every entry; owner= only those set with that owner. |

<!-- END dispatch-actions -->

## Keybindings

```
bind = MODIFIERS, KEY, ACTION[, ARG[, ARG…]]
```

- **Modifiers:** `super`, `ctrl`, `alt`, `shift`, or compound (`super+ctrl`);
  `NONE` for unmodified keys.
- **Key:** keysym name (`q`, `Return`, `Print`, `XF86AudioPlay`).
- **Action:** any entry from the catalogue above.

```ini
bind = super,       Return, spawn, kitty
bind = super,       q,      killclient
bind = super,       space,  togglefloating
bind = super,       1,      view, 1
bind = super+shift, 1,      tag,  1
bind = super+ctrl,  r,      reload_config

mousebind = super, btn_left,  moveresize, curmove
mousebind = super, btn_right, moveresize, curresize
```

`mctl check-config` flags shadowed/duplicate binds offline.

## Window rules

Match by `appid` regex, `title` regex, or both (`exclude_appid` /
`exclude_title` negate). Regexes are anchored `^…$` by convention; prefix
`(?i)` for case-insensitive.

```ini
windowrule = isfloating:1, appid:^pavucontrol$
windowrule = isfloating:1, width:800, height:560, appid:^org\.keepassxc\.KeePassXC$
windowrule = tags:1,   appid:^firefox$
windowrule = block_out_from_screencast:1, appid:^(KeePassXC|1Password)$
windowrule = isnamedscratchpad:1, appid:^kitty-scratch$
```

Debug without running margo:

```bash
mctl rules --appid firefox --title "" --verbose   # Match / Reject(reason) per rule
```

## Tag rules

```ini
tagrule = id:1, layout_name:scroller, monitor_name:DP-3   # layout + home monitor
tagrule = id:9, layout_name:monocle
tagrule = id:5, wallpaper:/home/kenan/wall/lake.jpg       # per-tag wallpaper hint
```

`monitor_name:` pins the tag to an output — `view N` warps focus there, and
windowrules targeting `tags:N` inherit that monitor.

## Layer rules

Match the layer-shell `namespace` (regex) — bars, OSDs, launchers.

```ini
layerrule = noanim:1, layer_name:^waybar$
layerrule = animation_type_open:slide_left, animation_type_close:slide_left, layer_name:^rofi$
```

## See also

- **Full annotated reference:** <https://kenanpelit.github.io/margo/config-reference/>
- **Configuration walkthrough (site):** <https://kenanpelit.github.io/margo/configuration/>
- **Companion tools (`mctl`/`mlayout`/`mscreenshot`/`mlogind`/`mpower`):**
  <https://kenanpelit.github.io/margo/companion-tools/>
- **Scripting (`init.rhai`):** <https://kenanpelit.github.io/margo/scripting/>
