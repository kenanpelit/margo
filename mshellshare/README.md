# mshellshare

The **portal screencast helper** for the margo shell. A small companion that
bridges the shell's screen-sharing flow to the desktop's screencast portal so
apps (browsers, meeting clients) can pick a window or screen to share.

## What it does

It coordinates with the xdg-desktop-portal screencast path (served natively by
[`margo-portal`](../margo-portal/)) to present the share picker and hand the
selected source to the requesting app over PipeWire — the shell-side glue that
makes "Share your screen" work under margo.

## Build

```bash
cargo build --release -p mshellshare
sudo install -m755 target/release/mshellshare /usr/bin/mshellshare
```

## See also

- [`margo-portal`](../margo-portal/) — the compositor-side portal backend.
- [Built-in portal design notes](https://kenanpelit.github.io/margo/portal-design/).

## License

GPL-3.0-or-later.
