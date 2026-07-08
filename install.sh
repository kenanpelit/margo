#!/usr/bin/env bash
# ==============================================================================
# margo — cross-distro install / uninstall
# ==============================================================================
# One self-contained installer for the margo Wayland compositor + mshell
# desktop shell. Detects the distro from /etc/os-release and dispatches:
#
#   * Arch / CachyOS (and arch-family) → builds + installs via the repo
#     PKGBUILD with makepkg (a `-git` VCS package; installs what is
#     PUSHED to the GitHub remote). Mirrors ~/.kod/margo_build/rebuild.sh
#     and is incremental on repeat runs. Uninstall = `pacman -R`.
#
#   * Debian / Ubuntu → installs build deps via apt, bootstraps a
#     recent Rust via rustup if needed (margo is edition 2024), builds
#     the LOCAL working tree with cargo, and installs the binaries +
#     assets to /usr — mirroring the PKGBUILD package() layout — while
#     recording every path in a manifest. Uninstall = remove the
#     manifested files.
#
# Requirements (Debian/Ubuntu): margo's gtk4-rs needs GTK >= 4.19, i.e.
# GTK 4.20+. Ubuntu 24.04 LTS ships GTK 4.14 and is NOT supported (apt
# upgrades stay on 4.14 for the LTS lifetime). Use Ubuntu 25.10+ /
# 26.04 LTS or a rolling distro with GTK 4.20+. The installer checks the
# GTK version up front and stops with a clear message on older releases.
# gtk4-layer-shell is built from source when it isn't in apt. Arch /
# CachyOS already track current GTK, so no version gate is needed there.
#
# Usage:
#   ./install.sh [install]   build + install (default)
#   ./install.sh uninstall   remove margo
#   ./install.sh deps        (Debian/Ubuntu) install build deps only
#   ./install.sh --help
#
# Design: docs/install-script.md
# ==============================================================================
set -euo pipefail

# ── Paths / constants ─────────────────────────────────────────────────────────
REPO_ROOT="$(cd -- "$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")" && pwd)"
ARCH_PKGNAME="margo-git"
DEB_DOCNAME="margo"                       # /usr/share/doc/<name> on Debian
MANIFEST="/usr/local/share/margo/install-manifest.txt"
MARGO_BUILD_DIR="${MARGO_BUILD_DIR:-${HOME}/.kod/margo_build}"
MIN_RUST_MINOR=85                         # margo is Rust edition 2024 → ≥ 1.85

# ── Logging ───────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  C_B=$'\e[1m'; C_G=$'\e[32m'; C_Y=$'\e[33m'; C_R=$'\e[31m'; C_0=$'\e[0m'
else
  C_B=''; C_G=''; C_Y=''; C_R=''; C_0=''
fi
log()  { printf '%s==>%s %s\n' "${C_G}${C_B}" "$C_0" "$*"; }
step() { printf '%s ::%s %s\n' "${C_B}" "$C_0" "$*"; }
warn() { printf '%s==> warning:%s %s\n' "${C_Y}${C_B}" "$C_0" "$*" >&2; }
die()  { printf '%s==> error:%s %s\n' "${C_R}${C_B}" "$C_0" "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

need_sudo() {
  if [[ $EUID -eq 0 ]]; then
    SUDO=""
  elif have sudo; then
    SUDO="sudo"
  else
    die "need root for this step but 'sudo' is not installed (run as root, or install sudo)"
  fi
}

# ── Distro detection ──────────────────────────────────────────────────────────
detect_distro() {
  [[ -r /etc/os-release ]] || die "cannot read /etc/os-release"
  # shellcheck disable=SC1091
  . /etc/os-release
  local hay=" ${ID:-} ${ID_LIKE:-} "
  case "$hay" in
    *" arch "*|*" cachyos "*|*" manjaro "*|*" endeavouros "*) echo "arch" ;;
    *" debian "*|*" ubuntu "*) echo "debian" ;;
    *) echo "unsupported" ;;
  esac
}

# ══════════════════════════════════════════════════════════════════════════════
# Arch / CachyOS
# ══════════════════════════════════════════════════════════════════════════════
arch_install() {
  have makepkg || die "makepkg not found — install base-devel"
  [[ -f "${REPO_ROOT}/PKGBUILD" ]] || die "no PKGBUILD at repo root: ${REPO_ROOT}"

  mkdir -p "$MARGO_BUILD_DIR"
  local cache_repo="${MARGO_BUILD_DIR}/margo" src_repo="${MARGO_BUILD_DIR}/src/margo"

  log "syncing canonical PKGBUILD into ${MARGO_BUILD_DIR}"
  cp "${REPO_ROOT}/PKGBUILD" "${MARGO_BUILD_DIR}/PKGBUILD"

  cd "$MARGO_BUILD_DIR"
  if [[ ! -d "$cache_repo" || ! -d "$src_repo" ]]; then
    log "no existing checkout — full build (makepkg -fsi)"
    exec makepkg -fsi --noconfirm
  fi

  step "refreshing VCS cache from origin"
  git -C "$cache_repo" fetch origin
  step "fast-forwarding build checkout to origin/main"
  git -C "$src_repo" fetch origin
  git -C "$src_repo" merge --ff-only origin/main

  log "makepkg -efsi (incremental)"
  exec makepkg -efsi --noconfirm
}

arch_uninstall() {
  need_sudo
  if pacman -Qq "$ARCH_PKGNAME" >/dev/null 2>&1; then
    log "removing ${ARCH_PKGNAME}"
    $SUDO pacman -R --noconfirm "$ARCH_PKGNAME"
  else
    warn "${ARCH_PKGNAME} is not installed — nothing to do"
  fi
}

# ══════════════════════════════════════════════════════════════════════════════
# Debian / Ubuntu
# ══════════════════════════════════════════════════════════════════════════════
DEBIAN_DEPS=(
  # toolchain
  clang libclang-dev pkg-config git meson ninja-build curl ca-certificates
  # wayland
  libwayland-dev wayland-protocols libinput-dev libxkbcommon-dev
  # drm / gl
  libgbm-dev libegl-dev libgles-dev libdrm-dev
  # seat / session
  libseat-dev seatd
  # core libs
  libpixman-1-dev libsystemd-dev libudev-dev libgudev-1.0-dev
  libglib2.0-dev libpcre2-dev libpam0g-dev libcairo2-dev libpango1.0-dev
  libdbus-1-dev
  # gtk / shell
  libgtk-4-dev libgdk-pixbuf-2.0-dev libgraphene-1.0-dev
  libfontconfig1-dev libfreetype-dev
  # gstreamer
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good
  # audio
  libasound2-dev libpulse-dev libpipewire-0.3-dev
  # runtime tools
  grim slurp wl-clipboard libnotify-bin pipewire
  xdg-desktop-portal xdg-desktop-portal-gtk
  # mlogind greeter host — cage+foot render the login greeter at each
  # monitor's native KMS resolution (`[display] host = "cage"`, the
  # default). mlogind falls back to the classic TTY greeter without them.
  cage foot
  # session manager — the installed wayland-session entry launches margo
  # via uwsm, which activates graphical-session.target (that target is
  # what starts mshell + the rest of the user session). Available on the
  # GTK >= 4.19 releases this installer already requires (Debian trixie+,
  # Ubuntu 25.10+, rolling).
  uwsm
)

debian_check_release() {
  # shellcheck disable=SC1091
  . /etc/os-release
  if [[ "${ID:-}" == "ubuntu" && "${VERSION_ID:-}" != "24.04" ]]; then
    warn "tuned for Ubuntu 24.04; you're on ${VERSION_ID:-unknown} — proceeding best-effort"
  fi
}

# margo's gtk4-rs (0.10) requires GTK ≥ 4.19 at build time. Many LTS
# releases ship older GTK (Ubuntu 24.04 = 4.14), where the shell build
# fails deep in `gdk4-sys` with a cryptic pkg-config error after a long
# compile. Fail fast here with the real reason instead. Run AFTER deps
# are installed so gtk4.pc exists to query.
MIN_GTK_MINOR=19
debian_check_gtk4() {
  local ver minor
  ver="$(pkg-config --modversion gtk4 2>/dev/null || echo 0.0)"
  minor="$(printf '%s' "$ver" | sed -n 's/^4\.\([0-9]\+\).*/\1/p')"
  minor="${minor:-0}"
  if (( minor < MIN_GTK_MINOR )); then
    die "GTK ${ver} is too old: margo's gtk4-rs needs GTK >= 4.${MIN_GTK_MINOR} (GTK 4.20+).
       Ubuntu 24.04 LTS ships GTK 4.14 and cannot build the mshell/mpicker
       shell. Use a release with GTK >= 4.20 (e.g. Ubuntu 25.10+ or a
       rolling distro), or install on Arch/CachyOS."
  fi
  log "gtk4: ${ver} (>= 4.${MIN_GTK_MINOR}, ok)"
}

debian_install_deps() {
  need_sudo
  debian_check_release
  log "enabling 'universe' (best effort) and refreshing apt"
  if have add-apt-repository; then
    $SUDO add-apt-repository -y universe >/dev/null 2>&1 || true
  fi
  $SUDO apt-get update
  log "installing build dependencies (${#DEBIAN_DEPS[@]} packages)"
  $SUDO apt-get install -y "${DEBIAN_DEPS[@]}"
  ensure_gtk4_layer_shell
}

# gtk4-layer-shell is the one dep that may be absent on a given 24.04
# host. Prefer the distro package; otherwise build + install it from
# source so the cargo build can link against it.
ensure_gtk4_layer_shell() {
  if $SUDO apt-get install -y libgtk4-layer-shell-dev >/dev/null 2>&1; then
    log "gtk4-layer-shell: installed from apt"
    return
  fi
  if pkg-config --exists gtk4-layer-shell-0 2>/dev/null; then
    log "gtk4-layer-shell: already present"
    return
  fi
  warn "gtk4-layer-shell-dev not in apt — building from source"
  local build; build="$(mktemp -d)"
  git clone --depth 1 https://github.com/wmww/gtk4-layer-shell "${build}/gtk4-layer-shell"
  # introspection/vapi are language-binding artifacts (GIR/typelib,
  # Vala) margo's Rust build never links against — disabling them drops
  # the gobject-introspection + vala build deps entirely.
  ( cd "${build}/gtk4-layer-shell"
    meson setup -Dexamples=false -Ddocs=false -Dtests=false \
      -Dintrospection=false -Dvapi=false --prefix=/usr build
    ninja -C build
    $SUDO ninja -C build install )
  $SUDO ldconfig
  rm -rf "$build"
  log "gtk4-layer-shell: built + installed from source"
}

# Ensure a Rust toolchain new enough for edition 2024 (≥ 1.85). Use an
# existing one if recent; otherwise bootstrap rustup non-interactively.
ensure_rust() {
  local minor=0
  if have rustc; then
    minor="$(rustc --version | sed -n 's/^rustc 1\.\([0-9]\+\).*/\1/p')"
    minor="${minor:-0}"
  fi
  if have cargo && (( minor >= MIN_RUST_MINOR )); then
    log "rust: using existing toolchain ($(rustc --version))"
    return
  fi
  if have rustup; then
    log "rust: rustup present — installing/using stable"
    rustup toolchain install stable
    rustup default stable
  else
    log "rust: bootstrapping rustup (need ≥ 1.${MIN_RUST_MINOR}, found 1.${minor})"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  fi
  # shellcheck disable=SC1091
  [[ -r "${HOME}/.cargo/env" ]] && . "${HOME}/.cargo/env"
  have cargo || die "cargo still not on PATH after rustup bootstrap"
}

# Two separate cargo invocations, matching the PKGBUILD split, so the
# tokio/zbus shell stack doesn't contaminate the compositor build via
# feature unification.
debian_build() {
  cd "$REPO_ROOT"
  # Ship with the `dist` profile (fat LTO + single codegen unit) — the
  # installer is not the dev inner loop, so the longer compile buys a
  # faster, smaller installed binary. `just` / CI stay on `release`.
  log "building compositor group (dist)"
  cargo build --profile dist -p margo -p start-margo \
    -p mctl -p mlock -p mlayout -p mscreenshot -p mvisual -p mplay -p mdots \
    -p mlogind -p mpower -p mcal
  log "building shell group (dist)"
  cargo build --profile dist -p mshell -p mshellctl -p mshellshare \
    -p mpicker -p mwizard -p mkeys -p mvpn -p margo-portal
}

# install_file <mode> <src> <dst> — install one file and record <dst>
# in the manifest (so uninstall is exact).
install_file() {
  local mode="$1" src="$2" dst="$3"
  [[ -e "$src" ]] || { warn "skip (missing source): $src"; return; }
  $SUDO install -Dm"$mode" "$src" "$dst"
  printf '%s\n' "$dst" | $SUDO tee -a "$MANIFEST" >/dev/null
}

# install_tree <src_dir> <dst_dir> — copy a directory's contents and
# record every resulting file in the manifest.
install_tree() {
  local src="$1" dst="$2"
  [[ -d "$src" ]] || { warn "skip (missing dir): $src"; return; }
  $SUDO install -d "$dst"
  $SUDO cp -a "$src"/. "$dst"/
  # Record each copied file (relative paths under dst).
  ( cd "$src" && find . -type f -printf '%P\n' ) | while IFS= read -r rel; do
    printf '%s\n' "${dst%/}/${rel}" | $SUDO tee -a "$MANIFEST" >/dev/null
  done
}

debian_install_files() {
  local tgt="$1"   # target build dir (e.g. target/dist)
  log "installing to /usr (recorded in ${MANIFEST})"
  $SUDO install -d "$(dirname "$MANIFEST")"
  $SUDO rm -f "$MANIFEST"
  $SUDO touch "$MANIFEST"

  # ── binaries ──
  local bin
  for bin in margo start-margo mctl mlock mlayout mscreenshot mvisual mplay mdots \
             mlogind mpower mshell mshellctl mshellshare mpicker mwizard mkeys mvpn mcal; do
    install_file 755 "${tgt}/${bin}" "/usr/bin/${bin}"
  done
  # margo-portal lives under /usr/lib (D-Bus-activated, not a CLI)
  install_file 755 "${tgt}/margo-portal" "/usr/lib/margo/margo-portal"

  # ── man pages (margo / mctl / mshellctl, section 1) ──
  local manpage
  for manpage in "${REPO_ROOT}"/man/*.1; do
    [[ -e "$manpage" ]] || continue
    install_file 644 "$manpage" "/usr/share/man/man1/$(basename "$manpage")"
  done

  # ── wayland session entry ──
  # Only the uwsm-managed session is offered at the login chooser. uwsm
  # (a runtime dep above) runs the compositor in a transient systemd
  # scope and activates graphical-session.target — and it is that target
  # which pulls in mshell.service + the rest of the user session. The
  # entry chains margo-uwsm-session → margo-session → start-margo/margo;
  # both wrapper scripts resolve each other via PATH, so /usr/bin is
  # enough. The contrib .desktop's Exec= uses the manual-install
  # /usr/local/bin path; rewrite it to the packaged /usr/bin location.
  local uwsm_desktop_tmp
  uwsm_desktop_tmp="$(mktemp)"
  sed 's|/usr/local/bin/|/usr/bin/|' \
    "${REPO_ROOT}/contrib/sessions/margo-uwsm.desktop" > "$uwsm_desktop_tmp"
  install_file 644 "$uwsm_desktop_tmp" "/usr/share/wayland-sessions/margo-uwsm.desktop"
  rm -f "$uwsm_desktop_tmp"
  install_file 755 "${REPO_ROOT}/contrib/sessions/margo-uwsm-session" "/usr/bin/margo-uwsm-session"
  install_file 755 "${REPO_ROOT}/contrib/sessions/margo-session" "/usr/bin/margo-session"
  # uwsm env file: restores the standard XDG user-bin dirs (~/.local/bin)
  # that uwsm's login-shell env rebuild drops, so keybind/autostart
  # launches can resolve user-local tools. Per-user overrides still go in
  # ~/.config/uwsm/env.
  install_file 644 "${REPO_ROOT}/contrib/sessions/uwsm-env-margo" "/etc/xdg/uwsm/env-margo"
  # The plain `Exec=margo` entry is deliberately NOT a session: picked
  # from a DM it brings up a bare compositor with no shell (nothing
  # activates graphical-session.target). Kept under doc/ for manual /
  # no-systemd launching only.
  install_file 644 "${REPO_ROOT}/margo.desktop" \
    "/usr/share/doc/${DEB_DOCNAME}/sessions/margo-bare.desktop"

  # ── icons ──
  install_file 644 "${REPO_ROOT}/docs/assets/margo-icon.svg" \
    "/usr/share/icons/hicolor/scalable/apps/margo.svg"
  install_file 644 "${REPO_ROOT}/docs/assets/margo-icon.svg" "/usr/share/pixmaps/margo.svg"
  install_tree "${REPO_ROOT}/assets/icons/MargoMaterial" "/usr/share/icons/MargoMaterial"

  # ── example config + layouts ──
  install_file 644 "${REPO_ROOT}/margo/src/config.example.conf" \
    "/usr/share/doc/${DEB_DOCNAME}/config.example.conf"
  local layout
  for layout in "${REPO_ROOT}"/mlayout/examples/layout_*.conf; do
    [[ -e "$layout" ]] || continue
    install_file 644 "$layout" "/usr/share/doc/${DEB_DOCNAME}/layouts/$(basename "$layout")"
  done

  # ── default wallpaper (mlock fallback) ──
  install_file 644 "${REPO_ROOT}/assets/wallpapers/default.jpg" \
    "/usr/share/margo/wallpapers/default.jpg"

  # ── bundled default desktop wallpaper (margo brand) ──
  # Shown when no wallpaper dir is configured + first tile in the
  # Wallpaper menu; resolved from /usr/share/margo/wallpapers/.
  install_file 644 "${REPO_ROOT}/assets/wallpapers/margo-hero.png" \
    "/usr/share/margo/wallpapers/margo-hero.png"

  # ── xdg-desktop-portal: native margo backend (gnome-free) ──
  install_file 644 "${REPO_ROOT}/assets/margo-portals.conf" \
    "/usr/share/xdg-desktop-portal/margo-portals.conf"
  install_file 644 "${REPO_ROOT}/assets/margo.portal" \
    "/usr/share/xdg-desktop-portal/portals/margo.portal"
  install_file 644 "${REPO_ROOT}/assets/dbus/org.freedesktop.impl.portal.desktop.margo.service" \
    "/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.margo.service"
  install_file 644 "${REPO_ROOT}/assets/margo-portal.service" \
    "/usr/lib/systemd/user/margo-portal.service"

  # ── mshell runtime assets ──
  local snd
  for snd in "${REPO_ROOT}"/mshell-crates/mshell-sounds/assets/*.ogg; do
    [[ -e "$snd" ]] || continue
    install_file 644 "$snd" "/usr/share/mshell/sounds/$(basename "$snd")"
  done
  install_tree "${REPO_ROOT}/mshell-crates/mshell-style/scss" "/usr/share/mshell/scss"
  install_file 644 "${REPO_ROOT}/mshell-crates/mshell-config/profiles/default.yaml" \
    "/usr/share/doc/${DEB_DOCNAME}/mshell/profile.example.yaml"

  # ── shell completions for mctl ──
  install_file 644 "${REPO_ROOT}/contrib/completions/mctl.bash" \
    "/usr/share/bash-completion/completions/mctl"
  install_file 644 "${REPO_ROOT}/contrib/completions/_mctl" \
    "/usr/share/zsh/site-functions/_mctl"
  install_file 644 "${REPO_ROOT}/contrib/completions/mctl.fish" \
    "/usr/share/fish/vendor_completions.d/mctl.fish"

  # ── shell completions for mshellctl ──
  install_file 644 "${REPO_ROOT}/contrib/completions/mshellctl.bash" \
    "/usr/share/bash-completion/completions/mshellctl"
  install_file 644 "${REPO_ROOT}/contrib/completions/_mshellctl" \
    "/usr/share/zsh/site-functions/_mshellctl"
  install_file 644 "${REPO_ROOT}/contrib/completions/mshellctl.fish" \
    "/usr/share/fish/vendor_completions.d/mshellctl.fish"

  # ── shell completions for mdots ──
  install_file 644 "${REPO_ROOT}/contrib/completions/mdots.bash" \
    "/usr/share/bash-completion/completions/mdots"
  install_file 644 "${REPO_ROOT}/contrib/completions/_mdots" \
    "/usr/share/zsh/site-functions/_mdots"
  install_file 644 "${REPO_ROOT}/contrib/completions/mdots.fish" \
    "/usr/share/fish/vendor_completions.d/mdots.fish"

  # ── license ──
  install_file 644 "${REPO_ROOT}/LICENSE" "/usr/share/licenses/${DEB_DOCNAME}/LICENSE"
  # Upstream attributions (mango/dwl/dwm/OkShell/niri/noctalia).
  local lic
  for lic in "${REPO_ROOT}"/licenses/*; do
    [ -f "$lic" ] || continue
    install_file 644 "$lic" "/usr/share/licenses/${DEB_DOCNAME}/$(basename "$lic")"
  done
}

debian_install() {
  debian_install_deps
  debian_check_gtk4
  ensure_rust
  debian_build
  need_sudo
  debian_install_files "${CARGO_TARGET_DIR:-${REPO_ROOT}/target}/dist"
  $SUDO systemctl daemon-reload >/dev/null 2>&1 || true
  log "done."
  cat <<EOF

  margo is installed. Next:
    • Log out and pick the "margo" session in your display manager.
    • The portal backend (margo-portal) is D-Bus-activated; a fresh
      login (or 'systemctl --user daemon-reload') registers it.
EOF
}

debian_uninstall() {
  need_sudo
  if [[ ! -f "$MANIFEST" ]]; then
    warn "no manifest at ${MANIFEST} — removing known fixed paths only"
    local p
    for p in /usr/bin/{margo,start-margo,mctl,mlock,mlayout,mscreenshot,mvisual,mplay,mdots,mshell,mshellctl,mshellshare,mpicker,mwizard,mcal} \
             /usr/bin/margo-uwsm-session /usr/bin/margo-session \
             /usr/lib/margo/margo-portal \
             /usr/share/wayland-sessions/margo-uwsm.desktop \
             /etc/xdg/uwsm/env-margo \
             /usr/share/xdg-desktop-portal/margo-portals.conf \
             /usr/share/xdg-desktop-portal/portals/margo.portal \
             /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.margo.service \
             /usr/lib/systemd/user/margo-portal.service; do
      [[ -e "$p" ]] && $SUDO rm -f "$p"
    done
  else
    log "removing files listed in ${MANIFEST}"
    # Reverse order isn't required for files; just remove each path.
    while IFS= read -r path; do
      [[ -n "$path" && -e "$path" ]] && $SUDO rm -f "$path"
    done < "$MANIFEST"
    $SUDO rm -f "$MANIFEST"
  fi

  # Prune now-empty margo-owned directories.
  local d
  for d in /usr/lib/margo /usr/share/margo/wallpapers /usr/share/margo \
           /usr/share/icons/MargoMaterial /usr/share/mshell/sounds \
           /usr/share/mshell/scss /usr/share/mshell \
           /usr/share/doc/${DEB_DOCNAME}/layouts /usr/share/doc/${DEB_DOCNAME}/mshell \
           /usr/share/doc/${DEB_DOCNAME} /usr/share/licenses/${DEB_DOCNAME} \
           /usr/local/share/margo; do
    [[ -d "$d" ]] && $SUDO rmdir --ignore-fail-on-non-empty "$d" 2>/dev/null || true
  done
  $SUDO systemctl daemon-reload >/dev/null 2>&1 || true
  log "margo removed."
}

# ══════════════════════════════════════════════════════════════════════════════
# Main
# ══════════════════════════════════════════════════════════════════════════════
usage() {
  cat <<EOF
margo installer — build, install, and uninstall margo

Usage:
  ./install.sh [install]   detect distro, build + install (default)
  ./install.sh uninstall   remove margo
  ./install.sh deps        (Debian/Ubuntu) install build deps only
  ./install.sh --help

Env:
  MARGO_BUILD_DIR   Arch build dir (default: ~/.kod/margo_build)

Arch/CachyOS uses the repo PKGBUILD (installs the pushed GitHub HEAD).
Debian/Ubuntu builds the local tree and installs to /usr, tracked in
${MANIFEST}. See docs/install-script.md.
EOF
}

main() {
  local cmd="${1:-install}"
  case "$cmd" in
    -h|--help|help) usage; exit 0 ;;
  esac

  local distro; distro="$(detect_distro)"
  [[ "$distro" == "unsupported" ]] && die "unsupported distro (need arch-family or debian/ubuntu)"
  log "detected: ${distro}"

  case "$cmd" in
    install)
      case "$distro" in arch) arch_install ;; debian) debian_install ;; esac ;;
    uninstall)
      case "$distro" in arch) arch_uninstall ;; debian) debian_uninstall ;; esac ;;
    deps)
      [[ "$distro" == "debian" ]] || die "'deps' is Debian/Ubuntu only (Arch resolves deps via makepkg)"
      debian_install_deps ;;
    *)
      die "unknown command: ${cmd} (try --help)" ;;
  esac
}

main "$@"
