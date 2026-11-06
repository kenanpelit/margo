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
windowrule = isfloating:1, width:640, height:260, title:^(Authentication Required|Unlock Keyring)$

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

`include` and `source` are the same directive. A `*` or `?` in the **file name** is a wildcard: every matching regular file in that directory loads, in lexical order (`*` skips dot-files; a wildcard in a directory name is not expanded; no match is not an error). A plain path that doesn't exist is reported by `mctl check-config` (E003) and skipped. Relative paths resolve against the file that contains the line; `~/` and `$var`s work. Re-applying via `mctl reload` re-reads the entire tree.

## Configuration variables

```ini
var = term, kitty
var = mod,  super

bind = $mod, Return, spawn, $term
bind = $mod, e,       spawn, $term -e nvim
```

Define with `var = name, value` (names: `[A-Za-z_][A-Za-z0-9_]*`), reference with `$name` or `${name}` — anywhere a value appears, including in `source`/`include`d files (define before use). Only names you've actually defined this way get substituted; `$HOME`, `$(date …)`, `$1`, `$$` and an undefined `$foo` are left exactly as written, so shell commands in `spawn`/`spawn_shell` keep working untouched. Pick distinctive names — `var = HOME, …` would also expand `$HOME` inside `spawn_shell` commands, before the shell ever sees it.

## Cursor theme and size

```ini
cursor_theme = Bibata-Modern-Ice   # a theme dir in ~/.icons, ~/.local/share/icons or /usr/share/icons
cursor_size  = 32                  # logical px, 8–256
```

Both keys apply at startup **and on `mctl reload`**: margo reloads the theme
and redraws its own pointer, sets `XCURSOR_THEME` / `XCURSOR_SIZE` for every
app it starts from then on (and pushes them to the systemd / D-Bus activation
environment), and tells XWayland (XSETTINGS `Gtk/CursorThemeName` /
`Gtk/CursorThemeSize` plus the root-window cursor), so a cursor picker only
has to write the two keys and run `mctl reload`. Apps that are already
running keep the cursor they loaded, except Wayland apps that use the
cursor-shape protocol (margo draws those) and GTK apps on XWayland.

Each value comes from the first of: the key; an `env = XCURSOR_THEME,…` /
`env = XCURSOR_SIZE,…` line; the `XCURSOR_*` margo was started with; no theme
(libXcursor's `default`) and 24 px. A key left empty counts as unset.

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

margo ships **11 layouts** — 9 partition-style tiling algorithms plus **floating** and **mosaic** for non-partition desktops — and a separate zoom-out **overview** mode. Each tag remembers its own layout choice. Set one by name with the `setlayout` action, or cycle with `switch_layout`.

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
| `dwindle` | Dwindle | recursive split (Hyprland default) |
| `floating` | Floating | classic stacking desktop — windows auto-float with a cascade |
| `mosaic` | Mosaic | [GNOME's content-aware self-arranging concept](https://blogs.gnome.org/tbernard/2023/07/26/rethinking-window-management/) — windows keep their own size, drag one onto another to swap, overflow moves to an empty tag (or stacks in the corner, see below) |
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

## Window rules

Match by `app_id` regex, `title` regex, or both. Negation via `exclude_appid` / `exclude_title`. Common shapes:

```ini
# Float small toolboxes
windowrule = isfloating:1, appid:^pavucontrol$
windowrule = isfloating:1, width:800, height:560, appid:^org\.keepassxc\.KeePassXC$

# Pin browsers to tag 1, audio apps to tag 8
windowrule = tags:1,   appid:^firefox$
windowrule = tags:8,   appid:^spotify$     # tags:N is the tag number, not a bitmask

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

Match the layer-shell namespace with `layer_name:` (regex) — used by bars, OSDs, launchers.
Options: `layer_name`, `noanim`, `noblur`, `noshadow`, `animation_type_open`, `animation_type_close`.
A rule is one line; there are no continuation lines.

```ini
# Disable open/close animations on the bar — slide-in jitters
layerrule = noanim:1, layer_name:^waybar$

# Custom open animation for the launcher
layerrule = animation_type_open:slide_left, animation_type_close:slide_left, layer_name:^rofi$
```

An option margo doesn't know is reported by `mctl check-config` (W008). A window or
layer rule whose selector is misspelled (`class:`, `namespace:`) and so has no
recognised selector is ignored (W009) — it is not applied to every window.

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
animation_spring_stiffness     = 800
animation_spring_damping_ratio = 1.0
animation_spring_mass          = 1.0
```

## Keybindings

Format: `bind = MODIFIERS, KEY, ACTION[, ARG[, ARG…]]`

- Modifiers: `super`, `ctrl`, `alt`, `shift`, or compound (`super+ctrl`). `NONE` for unmodified keys.
- Key: keysym name (`q`, `Return`, `Print`, `XF86AudioPlay`).
- Action: any entry from the [Dispatch actions](#dispatch-actions) catalogue below (or `mctl actions --verbose` for the always-current list).

```ini
bind = super,       Return,        spawn, kitty
bind = super,       q,             killclient
bind = super,       j,             focusstack, 1
bind = super,       k,             focusstack, -1
bind = super,       space,         togglefloating
bind = super+shift, space,         togglefullscreen
bind = super,       1,             view, 1
bind = super+shift, 1,             tag,  1
bind = super,       Tab,           focuslast
bind = super+ctrl,  r,             reload_config

# Mouse binds
mousebind = super, btn_left,  moveresize, curmove
mousebind = super, btn_right, moveresize, curresize
```

Duplicate-bind detection runs offline:

```bash
mctl check-config            # lists shadowed binds (W004, first one wins)
mctl check-config --strict   # …and exits 1 on any warning (CI)
```

### Key modes and the lock screen

`keymode = NAME` starts a block of binds that are live only after
`setkeymode, NAME` (vim-style modes such as `resize`, or `passthrough` for a
VM); binds after `keymode = common` are live in every mode, and
`setkeymode, default` returns to the normal binds:

```ini
bind = super, r, setkeymode, resize
keymode = resize
bind = NONE, h, resizewin, -20, 0
bind = NONE, Escape, setkeymode, default
keymode = default
```

`setkeymode` ignores (and logs) a mode no bind declares, so a typo can't
strand the keyboard in a mode where no key leads back; a reload that removes
the current mode returns to `default`. While the session is locked, only
`bindl` binds (the `l` suffix, e.g. volume keys) and `force_unlock` run, and
they are looked up in the current mode first and then in `default`, so they
keep working over the lock screen whatever mode was active when it locked.

### When the lock screen crashes

If the locker (mlock) dies while the session is locked — OOM-killed,
crashed — the session **stays locked** and margo starts `mlock` again on its
own, which asks for the password as usual. Margo retries up to 3 times within
a minute (giving each start 5 s to take the lock), then waits a minute and
tries again; until a locker is back the screen stays black. A `force_unlock`
bind (allowed over the lock screen) restarts the locker right away; it never
unlocks the session without your password, whatever state the locker is in.

If no locker can start at all, unlock from outside the locked seat — switch
to a TTY (`ctrl+alt+F3`) or ssh in, log in, and run:

```bash
loginctl list-sessions          # find the graphical session's id
loginctl unlock-session <ID>    # or: loginctl unlock-sessions
```

margo listens for logind's `Unlock` signal on its session and ends the lock;
a still-running locker is told to exit.

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

On a touchscreen every finger goes to the app under it (taps, drags, pinch
zoom), and a first finger focuses the window it lands on. Once as many fingers
are down as some `gesturebind` uses, margo takes the touch for the gesture:
the app's touch sequence is cancelled and the swipe runs its bind when the
last finger lifts. Without a matching bind (two-finger pinch with only 3/4
finger binds) the touches stay the app's. The touchscreen maps to the built-in
panel (`eDP-*`, `LVDS-*`, `DSI-*`), or the first output when there is none,
following its rotation.

Mouse buttons bind with `mousebind`, and the scroll wheel with `axisbind`:

```ini
mousebind = super, btn_left,  moveresize, curmove
mousebind = super, btn_right, moveresize, curresize
axisbind  = super, UP,        focusdir,   left      # Super + wheel → roam columns
```

## Dispatch actions

Every action below can be bound to a key (`bind`), a mouse button (`mousebind`), a gesture (`gesturebind`), or run live with `mctl dispatch <action> [args]`. Comma-separated aliases are interchangeable. The tables are generated from the action catalogue (`margo-config/src/actions.rs`) — the same source as **`mctl actions --verbose`** and the shell completions — and a test fails when they fall behind it, so every action margo accepts is listed here.

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

## See also

- [**Full config reference**](config-reference.md) — every option with inline commentary, the complete `config.example.conf` rendered in-page.
- [Companion tools](companion-tools.md) — `mctl`, `mlayout`, `mscreenshot`.
- [Scripting](scripting.md) — `~/.config/margo/init.rhai` for what window rules and keybinds can't express.
- [Manual checklist](manual-checklist.md) — what to verify after a fresh install.
