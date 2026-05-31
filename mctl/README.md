# mctl

The **compositor IPC + control CLI** for margo. Inspect compositor state,
validate config offline, dispatch any keybind action, and drive live ops —
all over margo's `dwl-ipc-v2` + JSON state surface.

## Usage

```bash
mctl reload                  # re-read config.conf in the running compositor
mctl actions --verbose       # every dispatchable action (generated from source)
mctl actions --names         # bare action names (used by shell completions)
mctl check-config            # validate config.conf offline — no compositor needed
mctl rules --verbose         # explain which window rules match (offline)
mctl twilight preset list    # blue-light-filter presets
```

`mctl --help` lists the full surface. The dispatch verbs match margo's keybind
function names where possible (`killclient`, `togglefullscreen`, …), so what
you bind in `config.conf` is what you can run by hand.

### Categories

- **Inspection** — read outputs, tags, clients, layout state (no side effects).
- **Config validation** — `check-config` / `rules` run offline against the same
  parser + rule engine the compositor uses, so they answer "why didn't this
  fire?" without a live session.
- **Dispatch** — invoke any action the compositor accepts.
- **Live ops** — reload, twilight, output management, and more.
- **Migration** — helpers for porting config from another compositor.

See [Companion tools](https://kenanpelit.github.io/margo/companion-tools/) for
worked examples.

## Build

```bash
cargo build --release -p mctl
sudo install -m755 target/release/mctl /usr/bin/mctl
```

## License

GPL-3.0-or-later.
