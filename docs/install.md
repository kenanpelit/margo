# Install

The repository ships a single installer, `install.sh`, that detects your
distribution and does the right thing — build, install, and uninstall.
Clone the repo and run it:

```bash
git clone https://github.com/kenanpelit/margo
cd margo
./install.sh            # build + install (detects distro)
./install.sh uninstall  # remove margo
./install.sh --help
```

It installs every binary (compositor + shell + helpers), the
`margo-portal` screencast/screenshot backend, the Wayland session entry,
example configs and layouts, and shell completions.

## Arch / CachyOS

**From the AUR (recommended).** [`margo-git`](https://aur.archlinux.org/packages/margo-git)
builds the full stack (compositor + shell + helpers) from GitHub HEAD; any
AUR helper resolves the build dependencies:

```bash
paru -S margo-git        # or: yay -S margo-git
```

It's a VCS (`-git`) package — re-running the same command rebuilds against
the latest `main`. Uninstall with `pacman -Rns margo-git`.

**From the repo.** `./install.sh` runs the bundled `PKGBUILD` through
`makepkg` + `pacman` — the same flow as `makepkg -si`, handy when working
on the tree locally. Build dependencies are resolved by `makepkg`.

```bash
./install.sh            # makepkg build + pacman install
./install.sh uninstall  # pacman -R margo-git
```

## Ubuntu / Debian

!!! warning "Requires GTK ≥ 4.20"
    margo's GTK4 bindings need GTK 4.19+, so **Ubuntu 24.04 LTS (GTK 4.14)
    is not supported** — and `apt upgrade` won't help, because an LTS keeps
    the same GTK for its lifetime. Use **Ubuntu 25.10+ / 26.04 LTS** (or any
    distro with GTK 4.20+). The installer verifies the GTK version up front
    and stops early with a clear message on older releases.

The Ubuntu path installs the build dependencies via `apt`, bootstraps a
current Rust toolchain with `rustup` if the system one is too old (margo is
Rust edition 2024), builds `gtk4-layer-shell` from source when it isn't
packaged, then compiles and installs to `/usr`. Every installed path is
recorded in `/usr/local/share/margo/install-manifest.txt`, so `uninstall`
removes exactly what was added.

```bash
./install.sh deps       # install build dependencies only (optional)
./install.sh            # deps + Rust + build + install
./install.sh uninstall  # remove (reads the install manifest)
```

## Manual (any distro)

```bash
git clone https://github.com/kenanpelit/margo
cd margo && cargo build --release --workspace
for bin in margo start-margo mctl mshell mshellctl mshellshare mlock mlogind mgreet \
           mpower mlayout mscreenshot mplay mkeys mvpn mcal mpicker mdots mvisual mwizard; do
  sudo install -Dm755 target/release/$bin /usr/bin/$bin
done
sudo install -Dm644 margo.desktop /usr/share/wayland-sessions/margo.desktop
```

### System dependencies

| Required | Used for |
|---|---|
| `wayland`, `libinput`, `libxkbcommon`, `seatd`, `mesa`, `libdrm`, `pixman`, `pcre2` | compositor core |
| `gtk4` (≥ 4.20), `gtk4-layer-shell`, `cairo`, `pango`, `pam` | shell + lock screen |
| `pipewire`, `gstreamer` | screencast + media |
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

The flake exposes `packages.default`, a `devShells.default` with
`rust-analyzer` + `clippy`, plus `nixosModules.margo` and `hmModules.margo`.

## First run

After installation, log out, pick **margo** in your display manager's
session menu, log in. If you don't have a display manager:

```bash
# launch from a TTY via UWSM (recommended — gets the full systemd
# graphical-session.target wiring)
uwsm start margo-uwsm.desktop
```

A blank screen with a cursor means margo is running but no client / bar is
mapped yet. Spawn one:

```bash
super + Return        # default kitty
mctl spawn alacritty  # any other terminal
```

Drop a config:

```bash
mkdir -p ~/.config/margo
cp /usr/share/doc/margo*/config.example.conf ~/.config/margo/config.conf
mctl reload
```

Validate it:

```bash
mctl check-config
```

For the full smoke-test pass after first install (lock screen, screenshots,
multi-monitor, idle, etc.), see [the manual checklist](manual-checklist.md).
