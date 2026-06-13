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

# Build + install the shell and restart it live (no logout needed).
shell:
    cargo build --release -p mshell
    sudo install -m755 target/release/mshell {{bindir}}/mshell
    systemctl --user restart mshell
    @echo "mshell rebuilt + restarted"

# Build + install the small CLI / helper binaries.
cli:
    cargo build --release -p mctl -p mshellctl -p mscreenshot -p mpicker
    sudo install -m755 target/release/mctl {{bindir}}/mctl
    sudo install -m755 target/release/mshellctl {{bindir}}/mshellctl
    sudo install -m755 target/release/mscreenshot {{bindir}}/mscreenshot
    sudo install -m755 target/release/mpicker {{bindir}}/mpicker

# Everything: compositor + shell + CLI tools.
all: margo shell cli

# Re-read the compositor config (config.conf + sourced fragments). Does NOT
# install or swap the margo binary — use `just margo` + re-login for that.
reload:
    mctl reload

# The exact pre-push CI gate set (ci.yml). Run this before pushing — a clean
# `cargo check` is NOT enough: CI fails on clippy --all-targets -D warnings,
# fmt, the panic ratchet, and the design lint that a plain check never sees.
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    ./scripts/panic-ratchet.sh
    ./scripts/design-lint.sh
    cargo test --workspace

# Format the whole workspace.
fmt:
    cargo fmt --all
