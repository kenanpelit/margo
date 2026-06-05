# mkeys

An on-screen keyboard for the **margo** Wayland compositor — a tap keyboard
that types into whatever window is focused, via `zwp_virtual_keyboard_v1`.

Ported from [ptazithos/wkeys](https://github.com/ptazithos/wkeys) (MIT) and
adapted to margo's stack (GTK4 + `gtk4-layer-shell` + `relm4`) with a config
file, an mshell bar pill, and an mshell Settings page.

## Usage

```sh
mkeys show     # show the keyboard (start it if not running)
mkeys hide     # hide it (quit the running instance)
mkeys toggle   # toggle — also the default with no subcommand
```

Visibility equals process lifetime: the first `show`/`toggle` starts a resident
process that draws the keyboard; `hide`/`toggle` tells it to quit. The config is
re-read on every `show`.

Bind it in margo's `binds.conf`:

```
bind = super,F2,spawn,mkeys toggle
```

…or add the **On-Screen Keyboard** pill to a bar slot in mshell (Settings →
Bar), which runs `mkeys toggle` on click.

## Config — `~/.config/margo/mkeys.toml`

Written by **Settings → On-Screen Keyboard**; every field has a default, so a
missing or partial file is valid.

```toml
layout    = "en"      # "en" (US QWERTY) | "tr" (Turkish-Q) | a path to a .toml
scale     = 1.0       # key size multiplier
position  = "bottom"  # "bottom" | "top"
opacity   = 0.95      # 0.0–1.0
margin    = 8         # px gap from the anchored screen edge
show_pill = true      # hint for the keyboard bar pill
```

> **Turkish-Q note:** mkeys emits physical keycodes; the glyph each key
> produces depends on margo's *active xkb layout*. For the `tr` faces to match
> the output, set margo's xkb layout to `tr` (e.g. `xkb_rules_layout = tr` in
> `config.conf`).

## Requirements

margo (or any compositor implementing `zwp_virtual_keyboard_v1` +
`wlr-layer-shell`). Under margo both are shipped.

## License

MIT — see [LICENSE](LICENSE). Original work © ptazithos (wkeys); ported into
margo by Kenan Pelit.
