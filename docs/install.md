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

The binaries are stripped; their symbols and line tables are built into a
separate `margo-git-debug` package (`/usr/lib/debug`, ~0.9 GB). Install it
when you want symbolized crash reports — `coredumpctl info` / gdb and
margo's own panic backtraces find it automatically. Without it a panic
still names its source file and line, but the backtrace frames are bare
addresses. AUR helpers build it alongside the main package (paru keeps it
in `~/.cache/paru/clone/margo-git/`); `./install.sh` leaves it in its build
directory (or your `PKGDEST`):
`sudo pacman -U ~/.kod/margo_build/margo-git-debug-*.pkg.tar.zst`.

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

The Ubuntu path installs the build dependencies via `apt`, installs the
Rust toolchain the repo pins (`rust-toolchain.toml`) through `rustup` —
bootstrapping rustup when the system toolchain is missing or too old, and
never changing your rustup default — builds `gtk4-layer-shell` from source when it isn't
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
           mpower mlayout mscreenshot mkeys mvpn mcal mpicker mdots mvisual mwizard; do
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
nix run github:kenanpelit/margo     # run the compositor
nix build github:kenanpelit/margo   # the whole stack in ./result
```

The package builds the same three cargo groups as the PKGBUILD (one
invocation would leak zbus's `tokio` feature into the compositor) and
installs what the Arch package does, with `/usr` paths pointed at the
Nix store.

**NixOS.** The module installs the package, offers the *Margo (UWSM)*
session to the display manager (and enables `programs.uwsm`), registers
the `margo` portal next to the gtk one, starts `mshell` with the session
(and `mpower` when power-profiles-daemon is enabled), and declares the
`mlock` PAM service the lock screen authenticates with:

```nix
{
  inputs.margo.url = "github:kenanpelit/margo";
  outputs = { nixpkgs, margo, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      modules = [
        margo.nixosModules.margo
        { programs.margo.enable = true; }
        # programs.margo.shell.enable = false;   # bring your own bar
      ];
    };
  };
}
```

**Home Manager.** `margo.homeManagerModules.margo` writes
`~/.config/margo/config.conf` from Nix and rejects it at build time when
`mctl check-config` finds an error (`checkConfig = false` when it
`source`s files that only exist in your home):

```nix
wayland.windowManager.margo = {
  enable = true;
  settings = {
    border_radius = 6;
    bind = [ "SUPER,Return,spawn,foot" "ALT,R,setkeymode,resize" ];
    keymode.resize.bind = [ "NONE,Escape,setkeymode,default" ];
  };
};
```

The session entry, the portal and the PAM service are system-level: they
come from the NixOS module (or your distribution's package), not from
Home Manager. `nix flake check` evaluates a NixOS system with the module
enabled.

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
