# mshellctl

The **shell IPC CLI** for [`mshell`](../mshell/). Toggle menus, query shell
state, and drive audio / wallpaper / lock — over zbus on the session bus
(service `com.mshell.Shell`).

## Usage

```bash
mshellctl menu dashboard         # open/toggle a menu (dashboard, control-center, …)
mshellctl menu control-center    # open/toggle the control center
mshellctl audio ...              # volume / mute / device control
mshellctl wallpaper ...          # rotate / apply wallpaper
mshellctl lock                   # lock the session
```

`mshellctl --help` lists the full verb set. The IPC verbs live in
`mshell-crates/mshell-core/src/ipc.rs`.

> **Different daemon from `mctl`.** `mshellctl` talks to the **shell**
> (`com.mshell.Shell`); [`mctl`](../mctl/) talks to the **compositor**
> (`dwl-ipc-v2`). They are not interchangeable.

## Build

```bash
cargo build --release -p mshellctl
sudo install -m755 target/release/mshellctl /usr/bin/mshellctl
```

## License

GPL-3.0-or-later.
