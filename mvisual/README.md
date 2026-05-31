# mvisual

A **renderer visual debugger** for margo — a helper for inspecting what the
compositor's scene graph / render path is doing. Development tooling, not a
user-facing daily-driver binary.

## Usage

```bash
mvisual --help
```

It overlays/visualises renderer state to make damage tracking, scene
composition, and layout geometry observable while hacking on margo's render
code.

## Build

```bash
cargo build --release -p mvisual
sudo install -m755 target/release/mvisual /usr/bin/mvisual
```

## License

GPL-3.0-or-later.
