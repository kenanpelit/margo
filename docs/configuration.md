# Configuration

`~/.config/margo/config.conf` — plain `key = value`, hot-reloadable. A complete annotated example lives in [`margo/src/config.example.conf`](https://github.com/kenanpelit/margo/blob/main/margo/src/config.example.conf) and is installed at `/usr/share/doc/margo-git/config.example.conf`.

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
bind = super,       space,  spawn, qs -c noctalia-shell ipc call launcher toggle
bind = super+ctrl,  s,      sticky_window
bind = super+ctrl,  r,      reload_config

# Screenshot dispatch — uses mscreenshot under the hood.
bind = NONE,Print,screenshot-region-ui    # region → editor → file + clip
bind = alt, Print,screenshot-window
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

# Per-tag wallpaper hint (state.json exposes; daemon reads on tag-switch)
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
- Action: any of the 40+ dispatch actions; see `mctl actions --verbose` for the full enumerated list with examples.

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

## See also

- [Companion tools](companion-tools.md) — `mctl`, `mlayout`, `mscreenshot`.
- [Scripting](scripting.md) — `~/.config/margo/init.rhai`.
- [Manual checklist](manual-checklist.md) — what to verify after a fresh install.
- The annotated [`config.example.conf`](https://github.com/kenanpelit/margo/blob/main/margo/src/config.example.conf) on GitHub — every option documented inline.
