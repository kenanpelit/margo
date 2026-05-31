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

margo ships **15 tiling layouts**; each tag remembers its own. Set one with
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
| `vertical_tile` | Vertical tile | tile with the master on top |
| `vertical_scroller` | Vertical scroller | column scroller stacked vertically |
| `vertical_grid` | Vertical grid | grid laid out column-major |
| `vertical_deck` | Vertical deck | deck stacked vertically |
| `tgmix` | TG-mix | tile / grid hybrid |
| `canvas` | Canvas | free-form pan/zoom canvas (PaperWM-meets-Excalidraw) |
| `dwindle` | Dwindle | recursive split (Hyprland default) |
| `overview` | Overview | zoom-out of all tags — usually via `toggle_overview` |

```ini
default_layout = scroller          # any tag without a tagrule uses this
circle_layout  = scroller, tile, center_tile, grid, deck, monocle, vertical_grid

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

### Canvas options

| Key | Default | Meaning |
| --- | --- | --- |
| `canvas_tiling` | `0` | `1` = auto-arrange new windows in a grid |
| `canvas_tiling_gap` | `10` | gap between auto-arranged windows |
| `canvas_pan_on_kill` | `1` | re-centre after closing a window |
| `canvas_anchor_animate` | `0` | animate manual anchor changes |

## Dispatch actions

Bind these to a key (`bind`), button (`mousebind`), or gesture
(`gesturebind`), or run them live with `mctl dispatch <action> [args]`.
Comma-separated aliases are interchangeable. Generated from the same source as
`mctl actions --verbose`.

### Tags / workspaces

| Action | Arg | Description |
| --- | --- | --- |
| `view` | `<MASK>` | Switch to tag(s) by bitmask. Same tag twice toggles back when `view_current_to_back = 1`. Mask = `1<<(tag-1)`. |
| `toggleview` | `<MASK>` | Add/remove a tag from the active set. |
| `tag`, `tagsilent` | `<MASK>` | Move the focused window to tag(s); stay on the current tag. |
| `tagview`, `movetagview` | `<MASK>` | Move the focused window and follow it. |
| `toggletag` | `<MASK>` | Add/remove a tag from the window's mask. |
| `tagall` | | Show every tag at once. |
| `viewtoleft` / `viewtoright` | | Cycle to the previous / next occupied tag. |
| `tagtoleft` / `tagtoright` | | Move the window to the previous / next tag. |

### Focus

| Action | Arg | Description |
| --- | --- | --- |
| `focusstack`, `focusdir` | `<DIR>` | Move focus next/prev (`1`/`-1`) or directional. |
| `exchange_client`, `smartmovewin` | `<DIR>` | Swap the focused window with its neighbour. |
| `focusmon` | `<DIR>` | Focus another monitor. |
| `zoom` | | Promote the focused window to master. |

### Layout

| Action | Arg | Description |
| --- | --- | --- |
| `setlayout` | `<NAME>` | Switch the current tag's layout by name. |
| `switch_layout` | | Cycle the `circle_layout` list. |
| `incnmaster` | `<DELTA>` | Change the master-slot count. |
| `setmfact` | `<DELTA>` | Adjust the master factor (clamped 0.05–0.95). |
| `togglegaps` | | Toggle layout gaps. |
| `incgaps` | `<DELTA>` | Resize gaps by px. |
| `set_proportion` | `<RATIO>` | Scroller: set the focused column width (0.1–1.0). |
| `switch_proportion_preset` | | Scroller: cycle `scroller_proportion_preset`. |

### Window

| Action | Arg | Description |
| --- | --- | --- |
| `togglefloating` | | Tiled ↔ floating. |
| `togglefullscreen` | | Work-area fullscreen (bar visible). |
| `togglefullscreen_exclusive` | | Exclusive fullscreen (bar hidden) — mpv / games. |
| `sticky_window`, `togglesticky` | | Pin to every tag on the monitor; press again to restore. |
| `killclient` | | Close the focused window. |
| `movewin` | `<DX> <DY>` | Move by px (forces floating). |
| `resizewin` | `<DW> <DH>` | Resize by px (forces floating; min 50×50). |
| `moveresize` | `curmove`\|`curresize` | Interactive pointer move/resize — for `mousebind`. |
| `tagmon` | `<DIR>` | Move the window to an adjacent monitor. |
| `disable_output` / `enable_output` / `toggle_output` | `<NAME>` | Soft-disable / re-enable / toggle an output (dock/undock). |

### Scratchpad

| Action | Arg | Description |
| --- | --- | --- |
| `toggle_named_scratchpad` | `<APPID> <TITLE\|none> <SPAWN>` | Show/hide a named scratchpad; spawn if absent. |
| `summon`, `bring_here` | `<APPID> <TITLE\|none> <SPAWN>` | Bring an app to the current tag, or launch it. |
| `toggle_scratchpad` | | Toggle anonymous scratchpads on the focused monitor. |
| `unscratchpad`, `exit_scratchpad` | | Reset the window's scratchpad/floating/fullscreen state. |

### Overview

| Action | Arg | Description |
| --- | --- | --- |
| `toggle_overview` | | Enter/leave the zoom-out overview (also hot corner / 4-finger swipe). |
| `overview_focus_next` / `overview_focus_prev` | | alt+Tab / alt+shift+Tab thumbnails. |
| `overview_activate` | | Commit the selection (Enter/Esc). |

### System

| Action | Arg | Description |
| --- | --- | --- |
| `spawn` | `<COMMAND>` | Run a shell command (`sh -c`). |
| `run_script`, `rhai-eval` | `<PATH>` | Evaluate a Rhai script live. |
| `screenshot` / `screenshot-window` / `screenshot-region` / `screenshot-region-ui` | | Capture output / window / region (last also copies) → editor → file. |
| `reload`, `reload_config` | | Reload `config.conf` in place. |
| `theme`, `set_theme` | `<preset>` | Live-swap preset: `default` / `minimal` / `gaudy`. |
| `session_save` / `session_load` | | Save / restore per-monitor tag + layout state. |
| `setkeymode` | `<MODE>` | Switch keymode (vim-like mode sets). |
| `force_unlock` | | Tear down a stuck `ext_session_lock`. |
| `quit` | | Exit the compositor cleanly. |
| `debug_dump`, `diagnose` | | Dump compositor state to the log. |

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
layerrule = noanim:1, namespace:^waybar$
layerrule = animation_type_open:slide_left, animation_type_close:slide_left, namespace:^rofi$
```

## See also

- **Full annotated reference:** <https://kenanpelit.github.io/margo/config-reference/>
- **Configuration walkthrough (site):** <https://kenanpelit.github.io/margo/configuration/>
- **Companion tools (`mctl`/`mlayout`/`mscreenshot`/`mlogind`/`mpower`):**
  <https://kenanpelit.github.io/margo/companion-tools/>
- **Scripting (`init.rhai`):** <https://kenanpelit.github.io/margo/scripting/>
