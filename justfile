# margo — developer inner-loop tasks (`just <recipe>`).
#
# This is the *dev iteration* counterpart to ./install.sh (the full
# cross-distro / packaged installer). It mirrors the per-component
# build+install+restart commands from CLAUDE.md so you don't have to
# remember the right `-p` flag and `/usr/bin` target each time.
#
# Install just: `pacman -S just` / `cargo install just`. Run `just` (no
# args) to list recipes.
#
# ── Two worlds, two restart stories (the trap this codifies) ──────────
# margo (compositor) and mshell (shell) are SEPARATE binaries:
#   * `just shell`  rebuilds mshell and restarts the systemd --user unit
#     live — no logout needed.
#   * `just margo`  installs the new compositor, but the RUNNING margo is
#     not replaced until the Wayland session restarts (re-login / reboot).
#   * `just reload` runs `mctl reload`, which only re-reads config.conf —
#     it does NOT pick up a newly-installed margo binary. Installing a new
#     /usr/bin/margo and running `mctl reload` is the classic "my change
#     isn't showing up" trap.

bindir := "/usr/bin"

# List recipes (default).
default:
    @just --list

# Build + install the compositor. NOTE: the running margo is replaced only
# on the next Wayland session (re-login / reboot) — see header.
margo:
    cargo build --release -p margo
    sudo install -m755 target/release/margo {{bindir}}/margo
    @echo "installed margo → re-login or reboot to run the new compositor (mctl reload won't swap the binary)"

# Built with the WASM plugin panels on, like the package (PKGBUILD).
# Build + install the shell and restart it live (no logout needed).
shell:
    cargo build --release -p mshell --features wasm-plugins
    sudo install -m755 target/release/mshell {{bindir}}/mshell
    systemctl --user restart mshell
    @echo "mshell rebuilt + restarted"

# Build + install the small CLI / helper binaries.
cli:
    cargo build --release -p mctl -p mshellctl -p mscreenshot -p mpicker -p mcal
    sudo install -m755 target/release/mctl {{bindir}}/mctl
    sudo install -m755 target/release/mshellctl {{bindir}}/mshellctl
    sudo install -m755 target/release/mscreenshot {{bindir}}/mscreenshot
    sudo install -m755 target/release/mpicker {{bindir}}/mpicker
    sudo install -m755 target/release/mcal {{bindir}}/mcal

# Build + install mdots (declarative package/dotfiles manager).
dots:
    cargo build --release -p mdots
    sudo install -m755 target/release/mdots {{bindir}}/mdots

# Build + install mtune (folder-first music player) + its desktop/schema/icons.
mtune:
    cargo build --release -p mtune
    sudo install -m755 target/release/mtune {{bindir}}/mtune
    sudo install -Dm644 mtune/data/org.margo.Tune.desktop.in /usr/share/applications/org.margo.Tune.desktop
    sudo install -Dm644 mtune/data/org.margo.Tune.metainfo.xml /usr/share/metainfo/org.margo.Tune.metainfo.xml
    sudo install -Dm644 mtune/data/org.margo.Tune.service /usr/share/dbus-1/services/org.margo.Tune.service
    sudo install -Dm644 mtune/data/org.margo.Tune.gschema.xml /usr/share/glib-2.0/schemas/org.margo.Tune.gschema.xml
    sudo install -Dm644 mtune/data/icons/hicolor/scalable/apps/org.margo.Tune.svg /usr/share/icons/hicolor/scalable/apps/org.margo.Tune.svg
    sudo install -Dm644 mtune/data/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg /usr/share/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg
    sudo glib-compile-schemas /usr/share/glib-2.0/schemas
    sudo update-desktop-database /usr/share/applications || true
    sudo gtk-update-icon-cache -qtf /usr/share/icons/hicolor || true
    @echo "mtune installed"

# Regenerate the checked-in mctl + mshellctl + mdots shell completions
# (bash/zsh/fish) from clap. Run after adding/renaming a subcommand or
# argument, then commit contrib/completions/ (CI fails on drift).
completions:
    ./contrib/completions/generate.sh

# PKGBUILD build groups (compositor-side crates apart from the tokio/GTK
# shell stack). mlogind/mgreet/mlock take effect at the next login / lock,
# start-margo and margo-portal at the next session.
# Build + install every other binary the package ships.
helpers:
    cargo build --release -p start-margo -p mlock -p mlayout -p mvisual -p mlogind -p mpower -p mtm
    cargo build --release -p mshellshare -p mwizard -p mkeys -p mvpn -p mgreet -p margo-portal
    for b in start-margo mlock mlayout mvisual mlogind mpower mtm mshellshare mwizard mkeys mvpn mgreet; do sudo install -m755 target/release/$b {{bindir}}/$b; done
    sudo install -Dm755 target/release/margo-portal /usr/lib/margo/margo-portal

# scripts/install-parity.sh fails if a packaged binary is missing here.
# Everything the package ships: compositor, shell, CLI tools, mtune, helpers.
all: margo shell cli dots mtune helpers

# Re-read the compositor config (config.conf + sourced fragments). Does NOT
# install or swap the margo binary — use `just margo` + re-login for that.
reload:
    mctl reload

# The pre-push CI gate set: the steps of ci.yml's `check` and `wasm` jobs,
# with the same `--locked`. Run this before pushing — a clean `cargo check`
# is NOT enough: CI fails on clippy --all-targets -D warnings, fmt, the panic
# ratchet, the file-size gate, the design lint, the packaging parity and
# completion-drift checks, the example-config check and the WASM plugin
# host tests against the real guest fixtures (needs `rustup target add
# wasm32-wasip2`). The package's build groups are each checked on their own
# (`cargo check`, cheaper than CI's `cargo build`): a crate that only builds
# because the workspace-wide feature union enables something for it fails
# there, as it would in CI and makepkg. Not run here: the separate machete /
# MSRV / cargo-deny / debian jobs.
check:
    cargo fmt --all -- --check
    ./scripts/build-groups.sh --check
    ./scripts/test-build-groups.sh
    ./scripts/build-groups.sh each cargo check --locked
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo clippy --locked --all-targets -p mshell -p mshell-frame -p mshell-plugin-ui -p mshell-plugin-host --features mshell/wasm-plugins,mshell-plugin-ui/wasm -- -D warnings
    ./scripts/build-wasm-fixtures.sh
    cmp mplugin-sdk/wit/world.wit mshell-crates/mshell-plugin-host/wit/world.wit
    cargo clippy --locked --target wasm32-wasip2 --manifest-path mplugin-sdk/Cargo.toml -- -D warnings
    cargo fmt --manifest-path mplugin-sdk/Cargo.toml -- --check
    MARGO_REQUIRE_WASM_FIXTURES=1 cargo test --locked -p mshell-plugin-host --features wasm
    MARGO_REQUIRE_WASM_FIXTURES=1 cargo test --locked -p mshell-frame -p mshell-plugin-ui --features mshell-frame/wasm-plugins,mshell-plugin-ui/wasm
    ./scripts/design-lint.sh
    ./scripts/test-panic-baseline-guard.sh
    ./scripts/panic-ratchet.sh
    ./scripts/test-file-size-gate.sh
    ./scripts/file-size-gate.sh
    ./scripts/install-parity.sh
    ./scripts/test-install-parity.sh
    ./scripts/test-install-arch.sh
    ./scripts/test-install-rust.sh
    ./scripts/test-install-pkgbuild.sh
    ./scripts/optdepends-drift.sh
    ./scripts/test-optdepends-drift.sh
    ./scripts/test-retire-old-sync-variables.sh
    ./scripts/mtune-debrand-gate.sh
    cargo build --locked -p mctl -p mshellctl -p mdots
    ./contrib/completions/generate.sh --check
    if cargo run --locked -q -p mctl -- check-config --help | grep -q -- --strict; then s=--strict; fi; cargo run --locked -q -p mctl -- check-config $s --config margo/src/config.example.conf
    cargo test --workspace --locked
    cargo check --manifest-path fuzz/Cargo.toml --all-targets
    cargo test --manifest-path fuzz/Cargo.toml --lib
    cargo fmt --manifest-path fuzz/Cargo.toml -- --check

# Format the whole workspace.
fmt:
    cargo fmt --all
