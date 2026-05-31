# mlock

The **screen locker** for margo — `ext-session-lock-v1` + PAM. Because it uses
the session-lock protocol, the compositor cooperates: a locked session stays
locked across mlock crashes, and only margo's `force_unlock` keybind can break
out.

## What it draws

A cairo/pango software-rendered lock screen (the [nlock](https://github.com/OldUser101/nlock)
stack, not GTK):

- blurred wallpaper backdrop, large clock, time-of-day greeting,
- avatar (`~/.face` or AccountsService),
- a frosted password card with shake-on-fail and an attempt counter,
- battery indicator and `F1`/`F2`/`F3` power keys with a two-press
  confirmation banner.

Authenticates the session owner via PAM. The password buffer is zeroized after
auth so it never lingers in memory. Theming follows the margo **matugen**
palette.

## Usage

```bash
mlock                 # lock now
mlock --help
```

Bind it in `config.conf`:

```
bind = alt, l, spawn, mlock
```

## Build

```bash
cargo build --release -p mlock
sudo install -m755 target/release/mlock /usr/bin/mlock
```

## Configure

`~/.config/margo/mlock.conf` (hand-edited). The shell can also drive locking
via `mshellctl lock`.

## License

GPL-3.0-or-later. Architecture follows [nlock](https://github.com/OldUser101/nlock)
and [waylock](https://codeberg.org/ifreund/waylock).
