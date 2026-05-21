# `install.sh` — cross-distro installer design

Status: approved 2026-05-21. This is the design record for the
repo-root `install.sh` that builds, installs, and uninstalls margo on
Arch-family and Debian/Ubuntu systems.

## Goal

One self-contained script, shipped in the repo, that anyone who clones
margo can run to get a working install — before any Rust toolchain
exists — and to cleanly remove it later. It knows exactly what it puts
where, so uninstall is exact.

## CLI

```
./install.sh [install]     # detect distro, build + install (default)
./install.sh uninstall     # remove margo
./install.sh deps          # Debian/Ubuntu: install build deps only
./install.sh --help
```

Exit non-zero on any failure (`set -euo pipefail`). Colored, prefixed
log lines (`==>`), no silent steps.

## Distro detection

Parse `/etc/os-release`:

- `ID`/`ID_LIKE` contains **arch** (arch, cachyos, manjaro,
  endeavouros) → **Arch path**.
- `ID`/`ID_LIKE` contains **debian** (debian, ubuntu, pop, mint) →
  **Debian path**. Tuned for Ubuntu 24.04; warns (does not abort) on
  other releases.
- Otherwise → error with a clear "unsupported distro" message.

## Arch / CachyOS path

Mirrors the existing `~/.kod/margo_build/rebuild.sh` workflow, made
self-contained:

- Build dir defaults to `~/.kod/margo_build` (override via
  `MARGO_BUILD_DIR`).
- Copy the repo's canonical `PKGBUILD` into the build dir (source-tree
  PKGBUILD is authoritative).
- First run (no checkout): `makepkg -fsi`. Subsequent runs: `git fetch`
  + `--ff-only` merge of the VCS cache + build checkout, then
  `makepkg -efsi` (keeps cargo's `target/` cache → incremental).
- The `-git` PKGBUILD clones from the GitHub **remote**, so it installs
  what is **pushed** (matches the existing workflow; commit + push
  first).
- pacman installs the resulting `*.pkg.tar.zst` (makepkg `-i`).

**Uninstall:** `sudo pacman -R margo-git`.

## Debian / Ubuntu 24.04 path

### 1. Build dependencies (`apt-get install -y`)

Curated list mapped from the PKGBUILD `depends` + `makedepends`. Enable
the `universe` component first (`add-apt-repository universe` /
`apt-get update`) for grim/slurp/wl-clipboard/portals.

| Purpose | apt packages |
|---|---|
| Toolchain | `clang libclang-dev pkg-config git meson ninja-build` |
| Wayland | `libwayland-dev wayland-protocols libinput-dev libxkbcommon-dev` |
| DRM/GL | `libgbm-dev libegl-dev libgles-dev libdrm-dev` |
| Seat/session | `libseat-dev seatd` |
| Core libs | `libpixman-1-dev libsystemd-dev libudev-dev libgudev-1.0-dev libglib2.0-dev libpcre2-dev libpam0g-dev libcairo2-dev libpango1.0-dev libdbus-1-dev` |
| GTK/shell | `libgtk-4-dev libgdk-pixbuf-2.0-dev libgraphene-1.0-dev libfontconfig1-dev libfreetype-dev` |
| GStreamer | `libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good` |
| Audio | `libasound2-dev libpulse-dev libpipewire-0.3-dev` |
| Runtime tools | `grim slurp wl-clipboard libnotify-bin xdg-desktop-portal xdg-desktop-portal-gtk pipewire` |

**`gtk4-layer-shell` is the fragile one.** Try `libgtk4-layer-shell-dev`
via apt; if unavailable on the host's release, clone + build
`gtk4-layer-shell` from source with meson/ninja and install it (hence
`meson ninja-build` above) rather than aborting the whole run.

### 2. Rust toolchain

margo is edition 2024 → needs Rust ≥ 1.85. If `rustc` is missing or
older, bootstrap rustup non-interactively
(`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`),
source `$HOME/.cargo/env`, and use the freshly-installed stable. If a
recent enough `rustc`/`cargo` is already on PATH, use it as-is.

### 3. Build

Two separate `cargo build --release` invocations, matching the PKGBUILD
split exactly (so feature unification does not contaminate the
compositor build):

```
cargo build --release -p margo -p start-margo \
  -p mctl -p mlock -p mlayout -p mscreenshot -p mvisual
cargo build --release -p mshell -p mshellctl -p mshellshare \
  -p mpicker -p mwizard -p margo-portal
```

(Built from the **local working tree** on Debian — unlike the Arch
path — so a developer's checkout is what installs.)

### 4. Install + manifest

Mirror the PKGBUILD `package()` layout, installing to **`/usr`** (not
`/usr/local`): systemd **user** units are only searched in
`/usr/lib/systemd/user`, and xdg-desktop-portal's `.portal` /
`portals.conf` rely on the `/usr/share` fallback — `/usr/local` would
silently break margo-portal autostart and portal discovery. Since margo
is not apt-managed and the manifest gives exact removal, the `/usr`
layout is the robust choice.

Layout:

- Binaries → `/usr/bin/`: `margo start-margo mctl mlock mlayout
  mscreenshot mvisual mshell mshellctl mshellshare mpicker mwizard`
- `margo-portal` → `/usr/lib/margo/margo-portal`
- `margo.desktop` → `/usr/share/wayland-sessions/`
- Portal assets → `/usr/share/xdg-desktop-portal/margo-portals.conf`,
  `/usr/share/xdg-desktop-portal/portals/margo.portal`
- D-Bus activation → `/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.margo.service`
- systemd user unit → `/usr/lib/systemd/user/margo-portal.service`
- Icons (`margo.svg`, MargoMaterial tree), default wallpaper, shell
  SCSS + sounds, shell-completions, example configs, layouts, licenses
  — same destinations as `package()`.

Every installed path is appended to a **manifest** at
`/usr/local/share/margo/install-manifest.txt` (kept outside the removed
trees). A single `install_file <mode> <src> <dst>` helper does the
`install -D` and records `<dst>`.

**Uninstall (Debian):** read the manifest → `rm -f` each path → prune
now-empty margo-owned dirs (`/usr/lib/margo`, `/usr/share/margo`,
`/usr/share/icons/MargoMaterial`, `/usr/share/mshell`) → remove the
manifest. If the manifest is missing, fall back to removing the known
fixed paths and warn.

## Common

- Privilege: build runs as the user; install/uninstall file ops use
  `sudo` per command (script is run as a normal user, not via `sudo`).
- Idempotent: re-running `install` overwrites in place and rewrites the
  manifest.
- `post-install` hint: tells the user to log out / pick the "margo"
  session, and (Debian) that `systemctl --user daemon-reload` picks up
  the portal unit.

## Risks / known limitations

- **GTK ≥ 4.20 hard requirement (Debian/Ubuntu).** margo's gtk4-rs
  (0.10) needs GTK ≥ 4.19. **Ubuntu 24.04 LTS ships GTK 4.14 and is
  unsupported** — confirmed on a live 24.04.3 VM, where the whole noble
  archive (incl. backports) caps at 4.14, so `apt upgrade` cannot help.
  The installer gates on `pkg-config --modversion gtk4` and aborts early
  with a clear message. Supported: Ubuntu 25.10+ / 26.04 LTS or any
  distro with GTK 4.20+.
- `gtk4-layer-shell` is not packaged on Ubuntu; the installer builds it
  from source with `-Dintrospection=false -Dvapi=false` (verified on the
  VM).
- Arch path installs the pushed GitHub HEAD, not local uncommitted work
  (by design, matching `rebuild.sh`).
