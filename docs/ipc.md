# margo IPC

margo's control IPC is a single **Unix-domain socket**. It replaces the
old `dwl-ipc-unstable-v2` Wayland protocol and the polled `state.json`
file — both removed.

## Socket

- Path: `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`
  (fallback `/run/user/<uid>/margo/margo-ipc.sock`).
- margo exports `MARGO_SOCKET=<path>` into its own environment and into
  every child it spawns, so clients and scripts find it without
  re-deriving the path.
- `SOCK_STREAM`. Requests are UTF-8, **one request per `\n`-terminated
  line**. Replies are **one JSON object per `\n`-terminated line**.

## Verbs

| Request | Reply | Connection |
|---|---|---|
| `get <topic> [args…]` | one JSON frame | stays open, reusable |
| `dispatch <action> [a1..a5]` | `{"ok":true}` / `{"ok":false,"error":…}` | stays open |
| `watch <topic> [args…]` | initial frame, then a frame whenever the topic's document changes (at most 60 a second; an unchanged document is not re-sent) | stays open until the client disconnects |

## Topics (`get` and `watch`)

| Topic | Args | Payload |
|---|---|---|
| `state` | — | full snapshot (outputs, clients, layouts, focused_idx, keyboard_layout, keymode, session_locked, lock_keys, twilight, …) |
| `clients` | — | `{"clients":[…]}` |
| `client` | `<id>` | one client object, or `{"error":…}` |
| `monitors` | — | `{"monitors":[…]}` |
| `monitor` | `<name>` | one monitor object |
| `tags` | `<monitor>` | active/occupied tag masks + layout idx |
| `focused` | — | `{"focused":{…}|null}` |
| `layouts` | — | `{"layouts":[…]}` |
| `keyboard-layout` | — | `{"keyboard_layout":"…"}` |
| `twilight` | — | twilight state object |
| `mirror` | — | `active` (`{source,target}` or null) and `error` (why the last mirror request was refused, or null — for display; it is last-writer-wins across clients, and the requester gets the refusal in its own `dispatch` reply) — output mirroring. Each `outputs[]` entry also carries `mirroring` (true on the target, which then reads disabled in wlr-output-management). |
| `config-errors` | — | `{"config_errors":[…]}` |
| `layers` | — | `{"layers":[{"monitor","layer","name"},…]}` — every layer-shell surface (`layer` = background/bottom/top/overlay, `name` = its namespace). `get` only. |
| `wallpaper` | — | `{"supported", "fits", "entries":[{"outputs","path","size","fit","bg","owner","serial","fading"},…], "pending", "error"}` — wallpapers set with `wallpaper_set` (see below). `supported` is false on a backend that draws no compositor wallpaper (nested winit), where `wallpaper_set` is refused. |

Each `outputs[]` entry carries `scroller`: `null` unless the output is in
the scroller layout, else `{"viewport_width", "columns":[{"id","app_id",
"x","width","focused"}]}` — the strip's tiled columns in order, each
column's target rect with `x` relative to the work area (negative = off
the left edge, `x >= viewport_width` = off the right).

Each `clients[]` entry carries `scratchpad` (in the scratchpad set),
`scratchpad_shown` (on screen now) and `named_scratchpad`, plus `xwayland`
(an X11 client running through XWayland). `minimized` is true for a hidden
scratchpad and for a minimized window; a minimized (non-scratchpad) window
also has `minimized_order` > 0, larger = minimized later.

Error frames are `{"error":"<message>"}`. Unknown verb/topic returns an
error frame; the connection stays open.

## dispatch

`dispatch <action> [args…]` runs the same actions as `bind = …` lines.
Positional args map onto margo's `Arg`: args 1–3 parse as numbers
(`i` / `i2` / `f`), arg 4 → `v` (primary string), arg 5 → `v2`. A single
non-numeric first arg also fills `v` (the `spawn` / `theme` / `run_script`
shape). Run `mctl actions --verbose` for the full catalogue.

The reply is `{"ok":true}` once the action ran. `{"ok":false,"error":…}`
means it did not: an unknown action, a `reload` margo refused (the running
config stays), a mirror request it turned down, or an output action
(`disable_output`, `enable_output`, `toggle_output`, `dpms … <name>`,
`resetmfact <name>`) naming no output — or switching off the last output
that is on. `mctl dispatch` exits non-zero on it.

## Runtime wallpaper

A shell can hand margo its wallpaper instead of painting it in a
full-screen layer surface (a whole swapchain for a static image: four
output-sized buffers under GTK's Vulkan renderer). margo draws it from one
texture where the configured `wallpaper` would be, in screenshots and
casts, and in every scroller-overview cell.

```text
dispatch wallpaper_set    <*|OUTPUT[,…]> [fit=<fit>] [bg=#rrggbb[aa]|none] [fade=<ms>] [owner=<tag>] <path>
dispatch wallpaper_update <*|OUTPUT[,…]> [fit=<fit>] [bg=#rrggbb[aa]|none]
dispatch wallpaper_clear  [*|OUTPUT[,…]] [owner=<tag>]
```

`*` is every output without an entry of its own. `<path>` is absolute; a
headed `.raw` (`[u32 LE width][u32 LE height][RGBA]`) is read before the
reply, so `{"ok":true}` means margo holds the pixels and the file can be
deleted. Other formats decode in the background (`ok` = queued; watch
`pending` / `error` in `get wallpaper`). `fit` is
`cover|contain|fill|center|scale-down` (the `wallpaper_fit` names); `bg`
fills the output under the image; `fade` crossfades from the previous
image (GTK's ease-out cubic); `owner` scopes a later `wallpaper_clear`.
A missing `fit` / `bg` / `owner` keeps the entry's previous one. mshell
uses this (owner `mshell`) and keeps its own windows when the reply is not
`ok` — an older margo answers `unknown action`.

## Examples

```sh
# one-shot query from any language / shell
printf 'get state\n' | socat - "UNIX-CONNECT:$MARGO_SOCKET"

# via the reference client
mctl get clients
mctl watch tags eDP-1          # streams until Ctrl-C
mctl dispatch view 4           # switch to tag 3
```

The desktop shell (`mshell`) consumes `watch state` and mirrors each
frame into its reactive store; `mctl` is the CLI reference client.
