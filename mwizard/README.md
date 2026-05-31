# mwizard

The **first-launch setup wizard** for margo. Runs on a fresh install to seed a
working configuration — a shell profile, sensible defaults — so the desktop is
usable before you start hand-editing `config.conf`.

## Usage

```bash
mwizard              # launch the setup wizard
mwizard --help
```

It writes the initial `mshell` profile (via `mshell-config`) and the starter
config the rest of the stack expects, then hands off to the live session.

## Build

```bash
cargo build --release -p mwizard
sudo install -m755 target/release/mwizard /usr/bin/mwizard
```

## License

GPL-3.0-or-later.
