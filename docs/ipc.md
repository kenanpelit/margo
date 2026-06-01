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
| `watch <topic> [args…]` | initial frame, then a frame on every change | stays open until the client disconnects |

## Topics (`get` and `watch`)

| Topic | Args | Payload |
|---|---|---|
| `state` | — | full snapshot (outputs, clients, layouts, focused_idx, keyboard_layout, twilight, …) |
| `clients` | — | `{"clients":[…]}` |
| `client` | `<id>` | one client object, or `{"error":…}` |
| `monitors` | — | `{"monitors":[…]}` |
| `monitor` | `<name>` | one monitor object |
| `tags` | `<monitor>` | active/occupied tag masks + layout idx |
| `focused` | — | `{"focused":{…}|null}` |
| `layouts` | — | `{"layouts":[…]}` |
| `keyboard-layout` | — | `{"keyboard_layout":"…"}` |
| `twilight` | — | twilight state object |
| `config-errors` | — | `{"config_errors":[…]}` |

Error frames are `{"error":"<message>"}`. Unknown verb/topic returns an
error frame; the connection stays open.

## dispatch

`dispatch <action> [args…]` runs the same actions as `bind = …` lines.
Positional args map onto margo's `Arg`: args 1–3 parse as numbers
(`i` / `i2` / `f`), arg 4 → `v` (primary string), arg 5 → `v2`. A single
non-numeric first arg also fills `v` (the `spawn` / `theme` / `run_script`
shape). Run `mctl actions --verbose` for the full catalogue.

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
