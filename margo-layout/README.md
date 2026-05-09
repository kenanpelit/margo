# margo-layout

Switch margo's monitor layout between named profiles. Useful for laptop users who plug into different setups (office monitor, home dock, projector) and don't want to keep rewriting `monitorrule` lines by hand.

## How it works

Drop one file per setup into the margo config directory:

```
~/.config/margo/
├── config.conf
├── layout_solo.conf
├── layout_dock.conf
└── layout_meeting.conf
```

Each `layout_<name>.conf` contains the `monitorrule` lines for that arrangement, plus `#@` meta-directives describing the layout to `margo-layout`'s picker. The active layout is published as a symlink:

```
~/.config/margo/margo-layout.conf  →  layout_dock.conf
```

Your `config.conf` does `source = margo-layout.conf` once, and a `mctl reload` re-reads it on every switch.

## Quick start

1. Add `source = margo-layout.conf` to `~/.config/margo/config.conf` (anywhere; ordering matters only if you also have other `monitorrule` lines).
2. Create one or more `layout_<name>.conf` files (see the examples below).
3. Run `margo-layout pick` and choose a layout.

## Layout file format

A layout file is a regular margo config snippet. Lines beginning with `#@` are picked up by `margo-layout` itself (margo treats them as comments).

### Top-level directives

| Directive | Example | Effect |
|---|---|---|
| `#@ name = "..."` | `#@ name = "Office Dock"` | Display title in the picker. Defaults to the file slug. |
| `#@ shortcut = ...` | `#@ shortcut = d` | Keyboard shortcut for `set`. May appear multiple times. |

### Per-output directives

These attach to the next `monitorrule` line below them:

| Directive | Example | Effect |
|---|---|---|
| `#@ output_name = "..."` | `#@ output_name = "external"` | Label drawn inside the preview rectangle. |
| `#@ color = N` | `#@ color = 9` | Palette index 0..17 for the preview. If unset, hashed from the connector name. |

### Example

```
#@ name = "Office Dock"
#@ shortcut = d
#@ shortcut = D

#@ output_name = "external"
#@ color = 9
monitorrule = name:DP-3,width:2560,height:1440,refresh:60,x:0,y:0,scale:1

#@ output_name = "laptop"
#@ color = 11
monitorrule = name:eDP-1,width:1920,height:1200,refresh:60,x:2560,y:240,scale:1
```

## Subcommands

| Command | What it does |
|---|---|
| `margo-layout list` | Print every available layout with shortcuts and a colour summary. `--preview` adds a multi-line ASCII rectangle render under each. `--json` emits a stable schema for scripts. |
| `margo-layout current` | Print the active layout's name and shortcut. |
| `margo-layout set <name>` | Switch by file slug, `#@ name`, or any `#@ shortcut`. Triggers `mctl reload` unless `--no-reload`. |
| `margo-layout next` / `prev` | Cycle alphabetically, wrapping. |
| `margo-layout preview <name>` | Render the layout to stdout without activating it. |
| `margo-layout pick` | Interactive picker. Auto-detects `fuzzel` / `wofi` / `rofi` for a Wayland-native menu; falls back to a numbered TTY prompt. `--no-gui` forces the inline prompt. |

## Bind it to a key

Margo config:

```
bind = SUPER+SHIFT,L,spawn,margo-layout pick
bind = SUPER+SHIFT,N,spawn,margo-layout next
```

## Why a symlink, not in-place edits?

The `layout_*.conf` files are your hand-edited catalogue — they belong in version control. The active selection is per-machine state that should change without touching version-controlled files. Keeping the two cleanly separated is what the symlink achieves: the catalogue stays static, the symlink is the runtime switch.

## License

GPL-3.0-or-later, same as the rest of margo.
