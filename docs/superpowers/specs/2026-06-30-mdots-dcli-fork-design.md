# mdots — full-surface fork of `dcli` into the margo workspace

**Date:** 2026-06-30
**Status:** Approved (design); implementation pending

## Summary

Vendor the standalone `dcli` project (a declarative, NixOS-style package
manager for Arch — repo `kenanpelit/dcli`, ~37k LOC) into the margo
workspace as a new top-level crate `mdots/`, shipping a `mdots` binary.
The full functional surface is carried over verbatim; the fork only
adapts the project to margo's conventions (naming, licence, build,
packaging, CI gates). The standalone `dcli` repo is retired — margo
becomes mdots's sole home.

This is a **fork, not a refactor**: behaviour is preserved. The work is
mechanical (rename + rehome) plus whatever margo's CI gates force.

## Goals

- `mdots` builds as a member of the margo workspace and installs via
  the same `just` / `install.sh` / `PKGBUILD` paths as the other `m*`
  system tools.
- Every feature dcli has today works under the `mdots` name.
- The crate passes margo's full CI gate (`just check`).
- The user's existing daily workflow (`dcli sync`, …) keeps working
  through the transition.

## Non-goals (YAGNI for this pass)

- No compositor/shell integration: no `mctl` verb, no mshell Settings
  page, no D-Bus surface. `mdots` stays a standalone CLI that merely
  lives inside the workspace. (Possible later phase.)
- No behaviour changes or refactors beyond what CI forces.
- No dependency de-duplication / version harmonisation heroics.
- **Out of scope / follow-up job:** rewiring the user's existing
  symlinks under `~/.config` to the new `~/.config/mdots` layout. The
  user has explicitly deferred this to a separate, later task. The fork
  ships the new config home + a one-time migration; the broader
  symlink reorganisation is not touched here.

## Decisions (locked in brainstorming)

| Topic | Decision |
|---|---|
| Scope | **Full surface**, verbatim — pacman, nix + nix_eval, flatpak, Lua scripting, ratatui TUI, services, SOPS/age secrets, AUR. Nothing trimmed. |
| Heavy deps | `mlua` (lua54, vendored) and `ratatui`+`crossterm` are load-bearing (Lua eval engine + TUI mode) and stay. All dcli deps carried over as-is. |
| Licence | Relicence to **GPL-3.0-or-later** (margo's licence). 0BSD permits this with no attribution carried in. Drop dcli's `LICENSE`, the "Don" author entry, and any 0BSD headers. |
| Crate placement | Top-level `mdots/` (peer of `mctl`/`mplay`/`mvpn`), **not** under `mshell-crates/`. |
| Config home | `~/.config/mdots` (a system manifest, conceptually separate from compositor/shell config — so **not** under `~/.config/margo/`). |
| `dcli` compat | Ship a `/usr/bin/dcli → mdots` compatibility symlink so muscle memory and `dcli sync` (the user's mshell pipewire-restart trigger) don't break. |
| dcli repo | Retire/archive `kenanpelit/dcli` (manual GitHub step done by the user, not by this work). |

## Architecture

`mdots/` is a self-contained binary crate. No margo crate depends on it
and it depends on no margo crate — it is a workspace member purely for
unified build/CI/packaging.

### Components (vendored verbatim from `dcli/src/`)

- `backend/` — pacman backend (`mod.rs`, `pacman.rs`)
- `commands/` — ~25 subcommands (sync, status, merge, search, service,
  secrets, generate, init, edit, find, migrate, nix, repo, selfupdate,
  validate, …)
- `config/` — YAML/manifest model
- `lua/` — Lua config surface (audio, boot, desktop, hardware, network,
  package, power, security, service, storage, sandbox, helpers)
- `module/`, `package/`, `source/`, `services.rs`, `service_profile.rs`
- `nix/`, `nix_eval/` — Nix-style evaluation
- `secrets.rs` — SOPS/age decrypt-on-sync
- `theming/`, `theming.rs`, `tui/`, `ui.rs`, `progress.rs`,
  `process.rs`, `dotfiles.rs`, `defaults.rs`, `main.rs`

### Vendored-but-replaced / dropped

- Dropped: prebuilt `dcli` binary, `target/`, dcli's own `install.sh`,
  `PKGBUILD`, `.SRCINFO`, `LICENSE`, `benchmark.sh`,
  `update-to-rust.sh`, `.opencode/`, `.github/` (margo owns CI).
- Replaced by margo's equivalents: packaging + completion/man
  generation move into margo's `install.sh` / `PKGBUILD`.

## Identity scrub: `dcli` → `mdots`

Mechanical rename across the vendored tree:

- Binary + crate name → `mdots`.
- Env vars `DCLI_*` → `MDOTS_*`.
- Config / state / cache / backup dirs `…/dcli` → `…/mdots`; config
  home resolves to `~/.config/mdots`.
- Shell-completion command name, man page (`dcli.1` → `mdots.1`).
- `selfupdate` / `repo` URLs and any GitHub references repointed (or
  neutralised if they no longer apply once standalone releases stop).
- Help/about strings, self-references in output ("run `dcli sync`" →
  "run `mdots sync`").

### Config migration

On first run: if `~/.config/dcli` exists and `~/.config/mdots` does not,
migrate (copy/move) so `mdots sync` works on day one against the user's
current manifest. Minimal, one-shot, no ongoing dual-read.

## margo CI conformance (`just check`)

This is where the real (non-mechanical) work lives.

- **panic-ratchet** (`scripts/panic-ratchet.sh`): dcli has ~116
  `unwrap()/expect()` sites (raw grep: 100 + 16). After vendoring,
  recount with the script's exact exclusions (test modules after the
  first `#[cfg(test)]`, `build.rs`, `tests/`, `benches/`) and raise
  `scripts/panic-baseline.txt` by that measured delta. Commit-message
  rationale: *mdots is a separate CLI binary — a panic cannot take down
  the compositor or shell; baseline raised by N, to be ratcheted down
  over time.* (The ratchet explicitly permits a justified raise.)
- **clippy** `--all-targets -D warnings`: green-up dcli's warnings under
  margo's stricter config. **Largest unknown** — size not yet measured.
- **fmt**: `cargo fmt`, commit the result.
- **test**: dcli's tests carried over and kept green.
- **design-lint**: shell-UI lint; a CLI crate should be a no-op, but
  `just check` runs it, so confirm it doesn't choke.

## Build & packaging wiring

- Root `Cargo.toml`: add `mdots` to `[workspace] members`.
- `justfile`: add a `just dots` recipe (build + `install -m755
  target/release/mdots /usr/bin/mdots` + the `dcli` symlink), and add
  `mdots` to `just all` / `just cli`'s `-p` lists.
- `install.sh`: install the `mdots` binary, shell completions, man page,
  and the `dcli → mdots` compat symlink.
- margo `PKGBUILD`: add `mdots` (with completion/man generation) to the
  packaged binary set.

## Verification

1. `cargo build -p mdots` clean.
2. `mdots --help`, `mdots --version` show the new identity.
3. After migration: `mdots status` and `mdots sync --dry-run` run
   correctly against the user's existing manifest.
4. `just check` green (fmt + clippy `-D warnings` + panic-ratchet +
   design-lint + test).
5. `dcli sync` (compat symlink) still works.

## Risks

- **clippy green-up is unbounded** until measured — could be the bulk of
  the effort.
- panic-site recount must use the script's exact rules, not a raw grep.
- Config migration must be correct so the user's daily flow isn't broken
  on the first `mdots`/`dcli` invocation after install.
- 37k LOC vendored in one move — keep the rename mechanical and
  reviewable; resist opportunistic edits.

## Phasing

Single implementation plan, but naturally ordered:

1. Vendor `src/` + `Cargo.toml`, add to workspace, get
   `cargo build -p mdots` green (deps + compile, no rename yet).
2. Identity scrub (`dcli` → `mdots`) including config home + migration +
   compat symlink.
3. CI green-up (fmt → clippy → panic baseline → test → `just check`).
4. Packaging (justfile / install.sh / PKGBUILD).
5. Manual follow-ups (user): archive the dcli repo; the `~/.config`
   symlink reorganisation is a separate later job.
