# mpicker

A **native colour picker** for margo. Freezes the screen with a
wlr-screencopy capture, overlays a zoom lens, and copies the colour under the
cursor.

## Usage

```bash
mpicker              # freeze, pick, copy to clipboard
mpicker --help
```

Click to pick the pixel under the lens; the colour is placed on the clipboard.
Bind it:

```
bind = SUPER, p, spawn, mpicker
```

## How it works

A `wlr-screencopy` grab is frozen into a full-screen overlay so the pointer can
hover without the content changing underneath; a magnified lens shows the exact
pixel before you commit. No screenshot file is written — it's a pick, not a
capture.

## Build

```bash
cargo build --release -p mpicker
sudo install -m755 target/release/mpicker /usr/bin/mpicker
```

## License

GPL-3.0-or-later.
