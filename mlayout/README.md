# mlayout

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

Each `layout_<name>.conf` contains the `monitorrule` lines for that arrangement, plus `#@` meta-directives describing the layout to `mlayout`'s picker. The active layout is published as a symlink:

```
~/.config/margo/mlayout.conf  →  layout_dock.conf
```

Your `config.conf` does `source = mlayout.conf` once, and a `mctl reload` re-reads it on every switch.

## Quick start

```sh
mlayout init
```

That's it. The `init` subcommand runs `wlr-randr`, captures every active output, writes the result as `layout_default.conf`, asks once whether it should add `source = mlayout.conf` to your `config.conf`, and activates the new layout. Idempotent — safe to re-run.

Need a different starting name? `mlayout init --name vertical` writes `layout_vertical.conf` instead.

To bookmark another setup later: rearrange your monitors physically, then

```sh
mlayout new dock
```

This snapshots the new geometry as `layout_dock.conf` without activating it. From now on `mlayout set dock` flips between them.

## Layout file format

A layout file is a regular margo config snippet. Lines beginning with `#@` are picked up by `mlayout` itself (margo treats them as comments).

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
| `mlayout init [--name SLUG] [--yes] [--force]` | First-time setup. Captures the current monitor configuration, wires `config.conf`, activates. |
| `mlayout new <name> [--title T] [--shortcut S] [--activate]` | Snapshot the live monitor configuration as a new named layout. No activation by default — pass `--activate` to switch immediately. |
| `mlayout list` | Print every available layout with shortcuts and a colour summary. `--preview` adds a multi-line ASCII rectangle render under each. `--json` emits a stable schema for scripts. |
| `mlayout current` | Print the active layout's name and shortcut. |
| `mlayout set <name>` | Switch by file slug, `#@ name`, or any `#@ shortcut`. Triggers `mctl reload` unless `--no-reload`. |
| `mlayout next` / `prev` | Cycle alphabetically, wrapping. |
| `mlayout preview <name>` | Render the layout to stdout without activating it. |
| `mlayout pick` | Interactive picker. Auto-detects `fuzzel` / `wofi` / `rofi` for a Wayland-native menu; falls back to a numbered TTY prompt. `--no-gui` forces the inline prompt. |

## Bind it to a key

Margo config:

```
bind = SUPER+SHIFT,L,spawn,mlayout pick
bind = SUPER+SHIFT,N,spawn,mlayout next
```

## Why a symlink, not in-place edits?

The `layout_*.conf` files are your hand-edited catalogue — they belong in version control. The active selection is per-machine state that should change without touching version-controlled files. Keeping the two cleanly separated is what the symlink achieves: the catalogue stays static, the symlink is the runtime switch.

## License

GPL-3.0-or-later, same as the rest of margo.
