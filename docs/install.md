# Install

Three supported paths: an Arch PKGBUILD, build-from-source, and a Nix flake.

## Arch (PKGBUILD)

```bash
git clone https://github.com/kenanpelit/margo_build ~/.kod/margo_build
cd ~/.kod/margo_build && makepkg -si
```

This installs `margo`, `mctl`, `mlayout`, `mscreenshot`, the Wayland-session entry, and the example layouts. Required runtime tools (`grim`, `slurp`, `wl-clipboard`) come in as dependencies; `swappy` / `satty` are optional editors picked up at runtime.

## From source

```bash
git clone https://github.com/kenanpelit/margo
cd margo && cargo build --release --workspace
sudo install -Dm755 target/release/margo        /usr/bin/margo
sudo install -Dm755 target/release/mctl         /usr/bin/mctl
sudo install -Dm755 target/release/mlayout      /usr/bin/mlayout
sudo install -Dm755 target/release/mscreenshot  /usr/bin/mscreenshot
sudo install -Dm644 margo.desktop /usr/share/wayland-sessions/margo.desktop
```

### System dependencies

| Required | Used for |
|---|---|
| `wayland`, `libinput`, `libxkbcommon`, `seatd`, `mesa`, `libdrm`, `pixman`, `pcre2` | core |
| `xorg-xwayland` | X11 client support (optional but recommended) |
| `grim`, `slurp`, `wl-clipboard` | screenshot pipeline (`mscreenshot`) |
| `wlr-randr` | live monitor re-layout via `mlayout` |

### Cargo features

```bash
# default = full feature set
cargo build --release

# disable screencast (drop pipewire dep)
cargo build --release --no-default-features --features dbus

# headless / pure compositor (no D-Bus, no screencast)
cargo build --release --no-default-features

# accessibility + tracy profiler (off by default)
cargo build --release --features a11y,profile-with-tracy
```

## Nix flake

```bash
nix run github:kenanpelit/margo
```

The flake exposes `packages.default`, a `devShells.default` with `rust-analyzer` + `clippy`, plus `nixosModules.margo` and `hmModules.margo`.

## First run

After installation, log out, pick **margo** in your display manager's session menu, log in. If you don't have a display manager:

```bash
# launch from a TTY via UWSM (recommended — gets the full systemd
# graphical-session.target wiring)
uwsm start margo-uwsm.desktop
```

A blank screen with a cursor means margo is running but no client / bar is mapped yet. Spawn one:

```bash
super + Return        # default kitty
mctl spawn alacritty  # any other terminal
```

Drop a config:

```bash
mkdir -p ~/.config/margo
cp /usr/share/doc/margo-git/config.example.conf ~/.config/margo/config.conf
mctl reload
```

Validate it:

```bash
mctl check-config
```

For the full smoke-test pass after first install (lock screen, screenshots, multi-monitor, idle, etc.), see [the manual checklist](manual-checklist.md).
