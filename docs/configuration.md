# Configuration

`~/.config/margo/config.conf` — plain `key = value`, hot-reloadable.

This page is a **curated walkthrough** of the high-traffic options. For the complete annotated reference (every knob, every default, with inline commentary), see [Full reference](config-reference.md). The same content ships at `/usr/share/doc/margo-git/config.example.conf` after installation.

## A minimal config

```ini
# Look
borderpx          = 3
border_radius     = 12
gappih            = 12
gappiv            = 12
focused_opacity   = 1.0
unfocused_opacity = 0.9

# Tags 1–6 home on DP-3, 7–9 home on eDP-1
tagrule = id:1, layout_name:scroller, monitor_name:DP-3
tagrule = id:7, layout_name:scroller, monitor_name:eDP-1

# Pin Helium to tag 1; float password prompts
windowrule = tags:1, appid:^Kenp$
windowrule = isfloating:1, width:640, height:260, \
             title:^(Authentication Required|Unlock Keyring)$

# Animations — niri-style
animation_clock_move = spring     # mid-flight velocity preserved
animation_clock_tag  = bezier     # snap-stop, no overshoot
animation_curve_open = 0.16,1.0,0.30,1.0   # ease-out-expo

# Keys
bind = super,       Return, spawn, kitty
bind = super,       q,      killclient
bind = super,       space,  spawn, mshellctl menu app-launcher
bind = super+ctrl,  s,      sticky_window
bind = super+ctrl,  r,      reload_config

# Screenshots — one front door through the shell's capture engine.
bind = NONE,Print,spawn,mshellctl screenshot region   # region → file + clip
bind = alt, Print,spawn,mshellctl screenshot window
```

Validate before reloading:

```bash
mctl check-config
```

## Includes & multi-file configs

```ini
# main config.conf
include = keybinds.conf
include = rules.conf
source  = ~/.config/margo/conf.d/*.conf
```

Both `include = …` (single file) and `source = …` (glob; matches in lex order) are supported. Re-applying via `mctl reload` re-reads the entire tree.

## Startup apps on background tags

Windows on hidden tags normally receive no frame callbacks until their tag is
visited. That is intentional compositor throttling, but Electron / CEF apps
launched from session start scripts can treat this as "backgrounded" and stall
half-drawn until you switch to their tag.

`warmup_hidden_ms` keeps sending frame callbacks to a freshly mapped hidden-tag
window for a short startup window, then returns to strict visible-window
throttling:

```ini
# 10 seconds is enough for Spotify, Webcord, Ferdium, Discord-style clients.
# Set 0 to disable and only frame currently visible windows.
warmup_hidden_ms = 10000
```

## Layouts

margo ships **11 tiling layouts**. Each tag remembers its own layout choice. Set one by name with the `setlayout` action, or cycle with `switch_layout`.

| `setlayout` name | Layout | Description |
| --- | --- | --- |
| `tile` | Tile | dwm classic — master + stack |
| `scroller` | Scroller | PaperWM-style horizontal column scroller (the default) |
| `grid` | Grid | equal-area grid |
| `monocle` | Monocle | one window fills the area; siblings hidden |
| `deck` | Deck | master + stacked tabs |
| `center_tile` | Center tile | master centred, stacks to the left + right |
| `right_tile` | Right tile | master pane on the right |
| `tgmix` | TG-mix | tile / grid hybrid |
| `canvas` | Canvas | free-form pan/zoom canvas (PaperWM-meets-Excalidraw) |
| `dwindle` | Dwindle | recursive split (Hyprland default) |
| `overview` | Overview | zoom-out of every tag — usually entered via `toggle_overview`, not set directly |

### Choosing and cycling

```ini
default_layout = scroller          # any tag without a tagrule uses this
# What `switch_layout` cycles through, in order:
circle_layout  = scroller, tile, center_tile, grid, deck, monocle, dwindle
```

```ini
bind = super,       t, setlayout, tile     # set the current tag's layout by name
bind = super,       e, switch_layout       # cycle through circle_layout
tagrule = id:9, layout_name:monocle        # pin a layout to a specific tag
```

Per-tag layout precedence: **`taglayout` > `tagrule layout_name` > `default_layout`**. A manual `setlayout` sets the tag's live layout and is never clobbered by a reload.

### Master / stack tuning

| Key | Default | Meaning |
| --- | --- | --- |
| `default_mfact` | `0.55` | master pane fraction (0.05 – 0.95) |
| `default_nmaster` | `1` | how many windows live in the master area |
| `new_is_master` | `0` | `1` = new windows take the master slot |
| `center_master_overspread` | `0` | center_tile: master may overrun the stacks |
| `center_when_single_stack` | `1` | center_tile: centre the master when only one stack pane |

```ini
bind = super,       i, incnmaster, +1      # grow / shrink the master count
bind = super,       d, incnmaster, -1
bind = super,       h, setmfact,   -0.05   # resize the master pane
bind = super,       l, setmfact,   +0.05
bind = super,       g, togglegaps          # gaps on/off
bind = super+shift, g, incgaps,    +4      # widen gaps (negative tightens)
```

### Tabbed window groups

Hyprland-style window groups merge several windows into **one tile** that shows
one member at a time, with a tab strip across the top — like browser tabs for
your windows. Groups are **purely opt-in**: no windows are grouped until you
bind the verbs or set a `group:1` windowrule, so existing setups are unchanged.

A grouped set occupies a single layout slot (it reuses the Deck rect); only the
active member is mapped full-size, the rest are hidden until you cycle to them.

| Action | Arg | Meaning |
| --- | --- | --- |
| `togglegroup` | — | group the focused window with its layout neighbour, or ungroup it if already grouped |
| `changegroupactive` | `next` \| `prev` | cycle which member is displayed (wraps) |
| `movegroupwindow` | `next` \| `prev` | reorder the focused window in the tab strip (no wrap) |
| `movewindowtogroup` | — | absorb the focused window into a neighbour's group (only ever adds) |
| `lockgroups` | `on` \| `off` \| `toggle` | freeze group/ungroup ops (existing groups still cycle) |

```ini
bind = super,        g,   togglegroup
bind = super,        Tab, changegroupactive, next
bind = super+shift,  Tab, changegroupactive, prev
bind = super+alt,    g,   lockgroups, toggle

# Auto-group every kitty window into one tab strip:
windowrule = group:1, appid:^kitty$
```

The tab strip is the **only** on-screen cue that a tile is a group, and it is
**hidden by default** (`group_bar_height = 0`). With height 0 `togglegroup` still
merges windows, but you see no strip and can't tell which tile is grouped — raise
`group_bar_height` to ~22 to draw it.

Each chip shows the member's **window title** (app-id fallback), rasterised with
[`fontdue`](https://crates.io/crates/fontdue) and truncated with an ellipsis to
fit. The active member's chip uses `group_active_color`, the rest
`group_inactive_color`, and the label colour auto-picks black/white for contrast.
Cycle members with `changegroupactive`. Colours default to the focus/border
palette and follow matugen via `colors.conf`. (If no system font is found under
`/usr/share/fonts` the labels are skipped, but the coloured chips still draw.)

| Key | Default | Meaning |
| --- | --- | --- |
| `group_bar_height` | `0` | tab-strip height in px; `0` hides it (groups still work by keybind) |
| `group_bar_gap` | `4` | px gap between tab chips |
| `group_active_color` | `focuscolor` | active member's chip fill |
| `group_inactive_color` | `bordercolor` | inactive members' chip fill |

> Clicking / scrolling the tab chips to switch members is not wired yet — use
> `changegroupactive` for now. The strip currently renders above the tile's top
> edge; on a window flush against the work-area top it can overlap the area
> above it.

### Scroller options

The scroller is the most option-heavy layout, so it has its own block:

| Key | Default | Meaning |
| --- | --- | --- |
| `scroller_default_proportion` | `0.800` | column width ratio for new windows when ≥2 columns are visible |
| `scroller_default_proportion_single` | `0.800` | width when only one column shows |
| `scroller_ignore_proportion_single` | `0` | ignore the `*_single` value above |
| `scroller_focus_center` | `1` | auto-centre the focused column |
| `scroller_prefer_center` | `1` | open new columns near the focused one rather than at the end |
| `scroller_prefer_overspread` | `0` | let columns exceed the monitor width |
| `edge_scroller_pointer_focus` | `1` | moving the pointer to the edge scrolls and re-focuses |
| `scroller_proportion_preset` | `1.000, 0.800, 0.618, 0.500` | the list `switch_proportion_preset` cycles through (Φ = 0.618) |

```ini
bind = super, r, set_proportion, 0.618        # set the focused column's width ratio
bind = super, p, switch_proportion_preset     # cycle scroller_proportion_preset
```

### Canvas options

| Key | Default | Meaning |
| --- | --- | --- |
| `canvas_tiling` | `0` | `1` = auto-arrange new windows in a grid |
| `canvas_tiling_gap` | `10` | gap between auto-arranged windows |
| `canvas_pan_on_kill` | `1` | re-centre after closing a window |
| `canvas_anchor_animate` | `0` | animate manual anchor changes |

## Window rules

Match by `app_id` regex, `title` regex, or both. Negation via `exclude_appid` / `exclude_title`. Common shapes:

```ini
# Float small toolboxes
windowrule = isfloating:1, appid:^pavucontrol$
windowrule = isfloating:1, width:800, height:560, appid:^org\.keepassxc\.KeePassXC$

# Pin browsers to tag 1, audio apps to tag 8
windowrule = tags:1,   appid:^firefox$
windowrule = tags:128, appid:^spotify$

# CSD only for one app, server-side everywhere else (default)
windowrule = allow_csd:1, appid:^firefox$

# Anti-screencast — these don't appear in screen-share frames
windowrule = block_out_from_screencast:1, appid:^(KeePassXC|1Password)$

# Scratchpad — toggleable hidden window
windowrule = isnamedscratchpad:1, appid:^kitty-scratch$
```

`mctl rules --appid X --title Y --verbose` — debug rules **without** running margo. Match / Reject(reason) per rule.

## Tag rules

```ini
# Layout per tag
tagrule = id:1, layout_name:scroller
tagrule = id:9, layout_name:monocle

# Pin tag to a specific monitor — view N → focus warps to that output
tagrule = id:1, monitor_name:DP-3
tagrule = id:7, monitor_name:eDP-1

# Per-tag wallpaper hint (exposed in the IPC `state` topic; the shell reads it on tag-switch)
tagrule = id:5, wallpaper:/home/kenan/wall/lake.jpg
```

## Layer rules

Match the layer-shell `namespace` string (regex) — used by bars, OSDs, launchers.

```ini
# Disable open/close animations on the bar — slide-in jitters
layerrule = noanim:1, namespace:^waybar$

# Custom open animation for the launcher
layerrule = animation_type_open:slide_left,
            animation_type_close:slide_left,
            namespace:^rofi$
```

## Animations

```ini
animations             = 1
animation_duration_open  = 220
animation_duration_close = 200
animation_duration_move  = 250
animation_duration_tag   = 240

# Per-type clock: bezier (default) or spring
animation_clock_open  = bezier
animation_clock_close = bezier
animation_clock_move  = spring     # only spring with mid-flight retarget
animation_clock_tag   = bezier
animation_clock_focus = bezier
animation_clock_layer = bezier

# Bezier control points: x1,y1,x2,y2
animation_curve_open  = 0.16,1.0,0.30,1.0    # ease-out-expo
animation_curve_close = 0.40,0.0,0.55,0.95
animation_curve_move  = 0.25,1.0,0.50,1.0
animation_curve_tag   = 0.20,0.7,0.20,1.0

# Spring tuning (used when clock = spring)
spring_stiffness  = 800
spring_damping    = 1.0
spring_mass       = 1.0
```

## Keybindings

Format: `bind = MODIFIERS, KEY, ACTION[, ARG[, ARG…]]`

- Modifiers: `super`, `ctrl`, `alt`, `shift`, or compound (`super+ctrl`). `NONE` for unmodified keys.
- Key: keysym name (`q`, `Return`, `Print`, `XF86AudioPlay`).
- Action: any entry from the [Dispatch actions](#dispatch-actions) catalogue below (or `mctl actions --verbose` for the always-current list).

```ini
bind = super,       Return,        spawn, kitty
bind = super,       q,             killclient
bind = super,       j,             focus_next
bind = super,       k,             focus_prev
bind = super,       space,         togglefloating
bind = super+shift, space,         togglefullscreen
bind = super,       1,             view, 1
bind = super+shift, 1,             tag,  1
bind = super,       Tab,           switch_tag_back
bind = super+ctrl,  r,             reload_config

# Mouse binds
mousebind = super, btn_left,  moveresize, curmove
mousebind = super, btn_right, moveresize, curresize
```

Duplicate-bind detection runs offline:

```bash
mctl check-config       # exits 1 if any bind is shadowed
```

## Gestures

Touchpad (and touchscreen) swipes map to dispatch actions with `gesturebind`,
exactly the way keys map with `bind`:

```
gesturebind = MODIFIERS, MOTION, FINGERS, ACTION[, ARG[, ARG…]]
```

- **Modifiers:** held keyboard modifiers (`super`, `ctrl`, …) or `NONE`.
- **Motion:** swipe direction — the four cardinals `up` / `down` / `left` /
  `right` and the four diagonals `up_left` / `up_right` / `down_left` /
  `down_right`. A swipe is treated as diagonal only when it is clearly at an
  angle (both axes carry > 40 % of the travel); otherwise it snaps to the
  nearest cardinal, so diagonal binds never steal a straight swipe.
- **Fingers:** `3` or `4`.
- **Action:** any [dispatch action](#dispatch-actions) below; trailing fields
  are its args. Swipes shorter than `swipe_min_threshold` px are ignored.

A typical Niri/Mango-style set — 3 fingers to move *within* a monitor, 4 fingers
to move *between* tags and monitors:

```ini
# 3 fingers — horizontal: focus windows; vertical: browser-tab cycle (native
# sendkey, app-gated) falling back to window focus
gesturebind = NONE, left,  3, focusdir, left
gesturebind = NONE, right, 3, focusdir, right
gesturebind = NONE, up,    3, sendkey, ctrl+shift+Tab, ^(firefox|chrome.*|brave.*)$, focusdir:up
gesturebind = NONE, down,  3, sendkey, ctrl+Tab,       ^(firefox|chrome.*|brave.*)$, focusdir:down

# 4 fingers — horizontal: tag nav; vertical: overview; up-diagonal: roam monitors
gesturebind = NONE, left,      4, viewtoleft
gesturebind = NONE, right,     4, viewtoright
gesturebind = NONE, up,        4, toggle_overview
gesturebind = NONE, down,      4, toggle_overview
gesturebind = NONE, up_left,   4, focusmon, left
gesturebind = NONE, up_right,  4, focusmon, right
```

Mouse buttons bind with `mousebind`, and the scroll wheel with `axisbind`:

```ini
mousebind = super, btn_left,  moveresize, curmove
mousebind = super, btn_right, moveresize, curresize
axisbind  = super, UP,        focusdir,   left      # Super + wheel → roam columns
```

## Dispatch actions

Every action below can be bound to a key (`bind`), a mouse button (`mousebind`), a gesture (`gesturebind`), or run live with `mctl dispatch <action> [args]`. Comma-separated aliases are interchangeable. This catalogue is generated from the same source as **`mctl actions --verbose`**, which is always current — run it if a build adds an action this page hasn't caught up with.

### Tags / workspaces

| Action | Arg | Description |
| --- | --- | --- |
| `view` | `<MASK>` | Switch to tag(s) by bitmask. Same tag twice toggles back when `view_current_to_back = 1`. Mask = `1<<(tag-1)` → tag 1 = 1, tag 8 = 128. |
| `toggleview` | `<MASK>` | Add or remove a tag from the active set (multi-tag view). |
| `tag`, `tagsilent` | `<MASK>` | Move the focused window to tag(s); you stay on the current tag. |
| `tagview`, `movetagview` | `<MASK>` | Move the focused window **and** follow it to that tag. |
| `toggletag` | `<MASK>` | Add or remove a tag from the focused window's mask. |
| `tagall` | | Show every tag at once. |
| `viewtoleft` / `viewtoright` | | Cycle the view to the previous / next occupied tag. |
| `tagtoleft` / `tagtoright` | | Move the focused window to the previous / next tag. |

### Focus

| Action | Arg | Description |
| --- | --- | --- |
| `focusstack`, `focusdir` | `<DIR>` | Move focus next/previous (`1` / `-1`) or directional (`left`/`right`/`up`/`down`). |
| `exchange_client`, `smartmovewin` | `<DIR>` | Swap the focused window with its neighbour in that direction. |
| `focusmon` | `<DIR>` | Move keyboard focus to another monitor (`left`/`right`/`up`/`down` or `1`/`-1`). |
| `zoom` | | Promote the focused window to the master slot (dwm zoom). |

### Layout

| Action | Arg | Description |
| --- | --- | --- |
| `setlayout` | `<NAME>` | Switch the current tag's layout by name (see the [Layouts](#layouts) table). |
| `switch_layout` | | Cycle through the `circle_layout` list. |
| `incnmaster` | `<DELTA>` | Change the master-slot count (`+1` / `-1`). |
| `setmfact` | `<DELTA>` | Adjust the master factor (e.g. `0.05` / `-0.05`); clamped to 0.05–0.95. |
| `togglegaps` | | Toggle layout gaps on/off. |
| `incgaps` | `<DELTA>` | Resize gaps by `delta` px (positive widens). |

### Scroller

| Action | Arg | Description |
| --- | --- | --- |
| `set_proportion` | `<RATIO>` | Set the focused column's width ratio (0.1 – 1.0). |
| `switch_proportion_preset` | | Cycle through `scroller_proportion_preset`. |

### Window

| Action | Arg | Description |
| --- | --- | --- |
| `togglefloating` | | Toggle the focused window between tiled and floating. |
| `togglefullscreen` | | Work-area fullscreen (bar stays visible) — standard `F11` feel. |
| `togglefullscreen_exclusive` | | Exclusive fullscreen — covers the whole output, hides every layer-shell (bar). Right for mpv / games. |
| `sticky_window`, `togglesticky` | | Pin the focused window to every tag on its monitor; press again to restore. |
| `killclient` | | Close the focused window. |
| `movewin` | `<DX> <DY>` | Move the focused window by px (forces floating). |
| `resizewin` | `<DW> <DH>` | Resize the focused window by px (forces floating; min 50×50). |
| `moveresize` | `curmove`\|`curresize` | Start an interactive pointer move/resize — for `mousebind`. |
| `tagmon` | `<DIR>` | Move the focused window to an adjacent monitor. |
| `disable_output` / `enable_output` / `toggle_output` | `<NAME>` | Soft-disable / re-enable / toggle an output by name (dock/undock). |

### Scratchpad

| Action | Arg | Description |
| --- | --- | --- |
| `toggle_named_scratchpad` | `<APPID> <TITLE\|none> <SPAWN>` | Show/hide a named scratchpad; spawn it if absent. Pairs with `windowrule isnamedscratchpad:1`. |
| `summon`, `bring_here` | `<APPID> <TITLE\|none> <SPAWN> [OP]` | Bring a matching app **to the current tag**, or launch it if not running. Repeated presses cycle through every match (run-or-raise). `OP` (optional 4th field) combines appid+title: `and` (default) / `or` / `difference` (appid but not title). |
| `focusapp`, `raiseapp` | `<APPID> <TITLE\|none> <SPAWN> [OP]` | Run-or-raise counterpart to `summon`: focus the app **where it is** (switch to its tag/monitor), cycling instances; launch if none match. Same matching + `OP` operator. |
| `toggle_scratchpad` | | Toggle every anonymous scratchpad on the focused monitor. |
| `unscratchpad`, `exit_scratchpad` | | Emergency reset of the focused window's scratchpad/floating/fullscreen state. |

### Overview

| Action | Arg | Description |
| --- | --- | --- |
| `toggle_overview` | | Enter/leave the zoom-out overview of all tags (also via hot corner / 4-finger swipe). |
| `overview_focus_next` / `overview_focus_prev` | | alt+Tab / alt+shift+Tab through thumbnails (opens overview if closed). |
| `overview_activate` | | Commit the keyboard cycle's selection (bind to Enter/Esc). |

### System

| Action | Arg | Description |
| --- | --- | --- |
| `spawn` | `<COMMAND>` | Run a shell command (through `sh -c`). |
| `sendkey` | `<COMBO>[,<APPID-REGEX>][,<FALLBACK>]` | Inject a synthetic key combo into the focused window. Optional app-id gate restricts it to matching windows; optional fallback action fires otherwise (e.g. `focusdir:up`). Great for gestures that send browser shortcuts. |
| `cyclekblayout`, `cycle_kb_layout` | | Cycle the keyboard to the next configured xkb layout. |
| `run_script`, `rhai-eval` | `<PATH>` | Evaluate a Rhai script against the live compositor. |
| `screenshot` / `screenshot-window` / `screenshot-region` / `screenshot-output` | | Capture output / window / region → editor → file (via `mscreenshot`). |
| `reload`, `reload_config` | | Reload `~/.config/margo/config.conf` in place. |
| `theme`, `set_theme` | `<preset>` | Live-swap the visual preset: `default` / `minimal` / `gaudy` (no config reload). |
| `session_save` / `session_load` | | Save / restore per-monitor tag + layout state (`session.json`). |
| `setkeymode` | `<MODE>` | Switch keymode (per-mode bind sets, like vim modes). |
| `force_unlock` | | Tear down a stuck `ext_session_lock` from the compositor side. |
| `quit` | | Exit the compositor cleanly. |
| `debug_dump`, `diagnose` | | Dump compositor state (clients, monitors, layouts) to the log. |

## See also

- [**Full config reference**](config-reference.md) — every option with inline commentary, the complete `config.example.conf` rendered in-page.
- [Companion tools](companion-tools.md) — `mctl`, `mlayout`, `mscreenshot`.
- [Scripting](scripting.md) — `~/.config/margo/init.rhai` for what window rules and keybinds can't express.
- [Manual checklist](manual-checklist.md) — what to verify after a fresh install.
