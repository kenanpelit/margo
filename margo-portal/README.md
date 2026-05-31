# margo-portal

The **xdg-desktop-portal screencast / screenshot backend** for margo. It lets
`xdg-desktop-portal-gnome` serve the Window / Entire-Screen ScreenCast tabs and
Screenshot without gnome-shell, by implementing five Mutter D-Bus shims backed
by margo's own capture (compositor screencopy → PipeWire / PNG).

## How it runs

Not a user-facing CLI — it's **D-Bus-activated** and lives under `/usr/lib`:

```
/usr/lib/margo/margo-portal                                  # the binary
/usr/share/xdg-desktop-portal/portals/margo.portal           # portal registration
/usr/share/dbus-1/services/…impl.portal.desktop.margo.service # D-Bus activation
/usr/lib/systemd/user/margo-portal.service                   # systemd user unit
```

When an app requests screen sharing, xdg-desktop-portal routes the
`org.freedesktop.impl.portal.*` calls to margo-portal, which presents the
Window / Entire-Screen picker and streams the chosen source over PipeWire.

## Build

```bash
cargo build --release -p margo-portal
sudo install -m755 target/release/margo-portal /usr/lib/margo/margo-portal
```

(The package installs the `.portal`, D-Bus activation, and systemd unit
alongside it.) See [`mshellshare`](../mshellshare/) for the shell-side portal
screencast helper, and the [portal design notes](https://kenanpelit.github.io/margo/portal-design/)
for the architecture.

## License

GPL-3.0-or-later.
