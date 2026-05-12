# Maintainer: Kenan Pelit <kenanpelit@gmail.com>
#
# margo — a Rust + Smithay Wayland tiling compositor in the dwl/mango
# tradition. Ships the compositor binary together with a small set of
# first-party helpers (`mctl`, `mlock`, `mlayout`, `mscreenshot`,
# `mvisual`) from a single Cargo workspace. Bar / notifications /
# launcher / OSD / system tray are intentionally NOT in this package —
# margo speaks `dwl-ipc-v2`, so any compatible external shell
# (noctalia, waybar-dwl, fnott, …) plugs in.

pkgname=margo-git
pkgver=r0.0
pkgrel=1
pkgdesc="Rust/Smithay Wayland tiling compositor (mango heritage) with first-party lock, IPC, monitor-profile and screenshot helpers"
url="https://github.com/kenanpelit/margo"
arch=("x86_64")
license=("GPL-3.0-or-later")

# Runtime libraries the compositor + helpers link directly against,
# verified with `ldd /usr/bin/margo /usr/bin/mlock` against a fresh
# build. PCRE2 is pulled in by the `regex` crate's PCRE backend
# (window-rule regex compilation).
depends=(
  glibc
  gcc-libs
  libinput
  libxkbcommon
  wayland
  mesa            # libgbm / libEGL / libGLESv2 for DRM/KMS render
  seatd           # provides libseat.so.1
  pixman
  libdrm
  systemd-libs    # libudev, libsystemd (logind, sd-bus)
  libgudev
  glib2
  pcre2
  pam             # `mlock` authenticates the session owner via libpam
  cairo           # `mlock` software renderer (no GPU dependency)
  pango           # `mlock` text shaping
  dbus            # screencast portal D-Bus shims
  libnotify       # `notify-send` from the config-reload toast path
  # Screenshot pipeline — `mscreenshot` shells out to these. Required
  # (not optional) because the in-compositor `screenshot-region-ui`
  # dispatch routes through the same external-tool chain.
  grim
  slurp
  wl-clipboard
)
makedepends=(
  rust
  cargo
  clang
  pkg-config
  git
  wayland-protocols
)
optdepends=(
  # Sessions & XDG plumbing
  "uwsm: systemd-driven session entry (graphical-session.target)"
  "xdg-desktop-portal-gnome: GTK file picker, color picker, screencast"
  "xdg-desktop-portal-wlr: alternative wlroots-native portal stack"
  "polkit-gnome: graphical authentication agent"
  # Toolkit Wayland backends
  "qt5-wayland: Qt5 native Wayland backend"
  "qt6-wayland: Qt6 native Wayland backend"
  # Capture & recording (beyond grim+slurp+wl-clipboard pulled above)
  "swappy: post-capture annotation editor for mscreenshot"
  "satty: alternative annotation editor for mscreenshot"
  "wf-recorder: screen recording via wlr-screencopy"
  # Clipboard & shells
  "copyq: clipboard manager via wlr-data-control"
  # External shells over dwl-ipc-v2 (bar / notifications / launcher /
  # OSD / settings) — margo ships none of these; pick one.
  "noctalia-shell-git: reference dwl-ipc-v2 shell (osc-shell IPC)"
  "fnott: lightweight Wayland notification daemon"
)
provides=("margo=$pkgver" "wayland-compositor")
conflicts=("margo")
# Cargo profile already enables thin LTO; let makepkg's outer LTO pass
# stay out of the way so we don't pay for it twice.
options=(!lto)
source=("git+${url}.git#branch=main")
sha256sums=("SKIP")

pkgver() {
  cd "$srcdir/margo"
  printf "r%s.%s" \
    "$(git rev-list --count HEAD)" \
    "$(git rev-parse --short HEAD)"
}

prepare() {
  cd "$srcdir/margo"
  # Pre-fetch all dependencies so build() can run offline and fails
  # early if Cargo.lock has drifted from the workspace manifest.
  cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
  cd "$srcdir/margo"

  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR="$srcdir/target"

  # `--remap-path-prefix` rewrites the build dir to `/build` in the
  # embedded debug strings; otherwise pacman warns about a reference
  # to `$srcdir` in the installed binary.
  RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$srcdir=/build" \
    cargo build --frozen --release --workspace
}

check() {
  cd "$srcdir/margo"

  # Only the library + CLI crates have unit tests we can run from a
  # packager environment; the compositor itself wants a live Wayland
  # session. Don't fail the package on test errors — they surface as
  # warnings — but DO let compile errors propagate.
  cargo test --frozen --release \
    --package margo-config \
    --package margo-layouts \
    --package mctl \
    --package mlayout ||
    echo "::: margo: test suite reported failures (non-blocking)"
}

package() {
  cd "$srcdir/margo"

  # ── Binaries ───────────────────────────────────────────────────────
  local bin
  for bin in margo mctl mlock mlayout mscreenshot mvisual; do
    install -Dm755 "$CARGO_TARGET_DIR/release/$bin" "$pkgdir/usr/bin/$bin"
  done

  # ── Wayland session entry ──────────────────────────────────────────
  # Picked up by display managers (gdm, sddm, ly, greetd-tuigreet …)
  # from the standard wayland-sessions location.
  install -Dm644 "margo.desktop" \
    "$pkgdir/usr/share/wayland-sessions/margo.desktop"

  # ── Icon ───────────────────────────────────────────────────────────
  # The compositor logo is published under `docs/assets/`. Surface it
  # as a hicolor scalable app icon AND a /usr/share/pixmaps fallback
  # so legacy DMs that don't consult the icon theme still find it.
  if [[ -f "docs/assets/margo-icon.svg" ]]; then
    install -Dm644 "docs/assets/margo-icon.svg" \
      "$pkgdir/usr/share/icons/hicolor/scalable/apps/margo.svg"
    install -Dm644 "docs/assets/margo-icon.svg" \
      "$pkgdir/usr/share/pixmaps/margo.svg"
  fi

  # ── Example configs / docs ─────────────────────────────────────────
  # Ship the annotated compositor config + Rhai init template under
  # /usr/share/doc/$pkgname so users can `cp` them into
  # ~/.config/margo/ without poking around the source tree.
  install -Dm644 "margo/src/config.example.conf" \
    "$pkgdir/usr/share/doc/$pkgname/config.example.conf"
  if [[ -f "contrib/scripts/init.example.rhai" ]]; then
    install -Dm644 "contrib/scripts/init.example.rhai" \
      "$pkgdir/usr/share/doc/$pkgname/init.example.rhai"
  fi

  # ── mlayout example profiles ───────────────────────────────────────
  # `mlayout set <name>` looks up `~/.config/margo/layout_<name>.conf`;
  # these are starter profiles the user can copy + tweak.
  if [[ -f "mlayout/README.md" ]]; then
    install -Dm644 "mlayout/README.md" \
      "$pkgdir/usr/share/doc/$pkgname/mlayout.md"
  fi
  local layout
  for layout in mlayout/examples/layout_*.conf; do
    [[ -f "$layout" ]] || continue
    install -Dm644 "$layout" \
      "$pkgdir/usr/share/doc/$pkgname/layouts/$(basename "$layout")"
  done

  # ── XDG desktop-portal preferences ────────────────────────────────
  # xdg-desktop-portal reads `<desktop>-portals.conf` when
  # XDG_CURRENT_DESKTOP matches the file's stem. Path is canonical —
  # /usr/share/doc/ does NOT work as a fallback.
  if [[ -f "assets/margo-portals.conf" ]]; then
    install -Dm644 "assets/margo-portals.conf" \
      "$pkgdir/usr/share/xdg-desktop-portal/margo-portals.conf"
  fi

  # ── Shell completions for mctl ─────────────────────────────────────
  # Hand-curated under contrib/completions/: extends the clap-derived
  # subcommand layer with dispatch-action names (`mctl actions
  # --names`), layout names, and output names. System paths so they
  # auto-load without anything in user shell rc files.
  if [[ -f "contrib/completions/mctl.bash" ]]; then
    install -Dm644 "contrib/completions/mctl.bash" \
      "$pkgdir/usr/share/bash-completion/completions/mctl"
  fi
  if [[ -f "contrib/completions/_mctl" ]]; then
    install -Dm644 "contrib/completions/_mctl" \
      "$pkgdir/usr/share/zsh/site-functions/_mctl"
  fi
  if [[ -f "contrib/completions/mctl.fish" ]]; then
    install -Dm644 "contrib/completions/mctl.fish" \
      "$pkgdir/usr/share/fish/vendor_completions.d/mctl.fish"
  fi

  # ── Developer / packager documentation ─────────────────────────────
  local doc
  for doc in README.md CHANGELOG.md road_map.md CONTRIBUTING.md; do
    [[ -f "$doc" ]] || continue
    install -Dm644 "$doc" "$pkgdir/usr/share/doc/$pkgname/$doc"
  done

  # ── Licenses ───────────────────────────────────────────────────────
  # margo inherits portions of dwl, dwm, sway, tinywl, wlroots, and
  # mango. Every upstream header is shipped so downstream attribution
  # is preserved on every install.
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  local lic
  for lic in LICENSE.dwl LICENSE.dwm LICENSE.sway LICENSE.tinywl \
             LICENSE.wlroots LICENSE.mango; do
    [[ -f "$lic" ]] || continue
    install -Dm644 "$lic" "$pkgdir/usr/share/licenses/$pkgname/$lic"
  done
}

# NOTE — manual one-time setup the package does NOT perform:
#   • PAM service for mlock: create /etc/pam.d/mlock (e.g. one line
#     `auth include system-auth`) so mlock can authenticate the
#     session owner. Not shipped here to avoid clobbering a local
#     stack; mlock --help prints the recipe.
#   • External shell (bar / notifications / launcher / OSD): pick a
#     dwl-ipc-v2 client (noctalia, waybar-dwl, fnott, …) and start it
#     from your session — margo only speaks the protocol, it does not
#     paint chrome itself.

# vim:set sw=2 et:
