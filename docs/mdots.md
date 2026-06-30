# mdots — declarative dotfiles and package manager

`mdots` is a declarative Arch Linux package + dotfiles manager that lives in the margo workspace. It lets you describe the exact set of packages (pacman/AUR, Flatpak, Nix) and SOPS/age-encrypted secrets you want on each machine, then drives `paru`/`yay`/`flatpak`/`home-manager` to make reality match the declaration.

Key properties:

- **Host files** declare what a specific machine should have.
- **Modules** group logically related packages (e.g. `window-managers/margo`, `dev/rust`) that can be toggled per host.
- **Lua modules** add runtime intelligence — query CPU vendor, detect battery, test whether a service is running — and conditionally add packages from the same file.
- **SOPS/age secrets** are decrypted at sync time and copied (never symlinked) to their plaintext targets outside the repo.
- **Nix / home-manager** runs as a first-class sync step when enabled.

## Config home

Default: `~/.config/mdots`  
Override: `MDOTS_CONFIG_DIR=/path/to/repo mdots …`

Legacy fallback: `~/.config/arch-config` (also accepted via `ARCH_CONFIG_DIR`). A fresh `mdots init` always creates `~/.config/mdots`.

## Directory layout

```
~/.config/mdots/
  config.yaml          # top-level pointer: names the active host
  config.lua           # alternative: single-file Lua config (or pointer)
  hosts/
    <hostname>.yaml    # full host config (or .lua / .nix)
  modules/
    base.yaml          # packages common to every host (or base.lua / base.nix)
    dev/rust.yaml      # per-topic modules
    dev/rust.lua       # Lua module with runtime detection
  services/            # service profile Lua files
  home-manager/        # home.nix and related files (when nix.home_manager_enabled)
  secrets/             # SOPS-encrypted blobs (committed; never plaintext)
  .sops.yaml           # age recipient list (SOPS meta-file)
  state/               # mdots-managed runtime state (do not hand-edit)
```

## Config model

### Host file

A host file (YAML, Lua, or Nix) is the root of the declaration for one machine. The key fields (actual Rust field names, not aliases):

```yaml
host: myhostname          # required — drives module/host file resolution
description: "workstation"

# Package management
packages:
  - vim
  - flatpak:com.spotify.Client   # Flatpak prefix syntax
  - { name: neovim, type: native }
exclude:
  - gedit                        # remove a package pulled in by a module

enabled_modules:
  - base
  - dev/rust
  - window-managers/margo

flatpak_scope: user              # "user" (default) or "system"
auto_prune: false                # remove untracked packages on sync

# AUR / pacman
package_manager: pacman          # auto-detected; only "pacman" supported today
aur_helper: paru                 # paru, yay, …

# Multi-host reuse
import:
  - hosts/shared/laptop-common.yaml

# Git
auto_commit: false               # commit after a successful sync

# Nix / home-manager
nix:
  enabled: false
  home_manager_enabled: false
  flake_enabled: false
  nixpkgs_channel: nixpkgs-unstable
  home_manager_channel: release-25.05

# SOPS secrets (see Secrets section)
sops_key_path: ~/.config/sops/age/keys.txt
secrets:
  - source: secrets/ssh.conf.sops
    target: ~/.ssh/config
    mode: "0600"
    name: ssh-config

# Systemd services
services:
  enabled: [NetworkManager, bluetooth]
  disabled: [avahi-daemon]
  scope: system   # or "user"
```

### Modules

A module is a reusable package group. Three formats are supported:

**YAML module** (`modules/dev/rust.yaml`):

```yaml
description: Rust development toolchain
packages:
  - rustup
  - cargo-edit
pre_install_hook: scripts/setup-rust.sh
hook_behavior: once          # "ask" | "always" | "once" | "skip"
conflicts: [dev/go]
```

**Directory module** (`modules/dev/rust/`):

```
modules/dev/rust/
  module.yaml        # manifest (description, hooks, conflicts, …)
  packages.yaml      # package list(s)
  scripts/           # hook scripts
  dotfiles/          # optional dotfiles to sync
```

The manifest (`module.yaml`) supports the same fields as a YAML module plus `package_files` (explicit list), `dotfiles_sync`, `dotfiles` (explicit source/target pairs), and metadata (`author`, `version`, `category`, `tags`).

**Lua module** (`modules/dev/rust.lua`): dynamic — see the Lua API section below.

### Package entry formats

```yaml
packages:
  - vim                                 # native (pacman)
  - flatpak:com.spotify.Client          # Flatpak via prefix
  - nix:ripgrep                         # Nix package
  - { name: code, type: native }        # explicit type
  - { name: com.visualstudio.code, type: flatpak }
```

## Lua API

Lua modules (`.lua` files for host configs and modules) receive a pre-populated global `mdots.*` table hierarchy. Each sub-table groups related detection functions:

| Table | Purpose |
|---|---|
| `mdots.hardware` | CPU vendor, GPU list, laptop/battery/chassis detection |
| `mdots.security` | Secure boot, TPM, disk encryption status |
| `mdots.package` | Query whether a package is installed |
| `mdots.service` | Query systemd service state |
| `mdots.power` | Power management profiles and AC state |
| `mdots.desktop` | Running desktop environment / compositor detection |
| `mdots.boot` | Bootloader detection (systemd-boot, GRUB, …) |
| `mdots.network` | Network interface / connectivity state |
| `mdots.audio` | PipeWire / PulseAudio detection |
| `mdots.storage` | Filesystem and disk info |
| `mdots.file` | Filesystem helpers (existence, content, globs) |
| `mdots.system` | OS, kernel, hostname, uptime |
| `mdots.log` | Logging from within a Lua module |
| `mdots.env` | Read environment variables |
| `mdots.util` | String / path utilities |

Example Lua module:

```lua
-- modules/dev/nvidia-drivers.lua
local m = {}
m.description = "NVIDIA proprietary drivers"

if mdots.hardware.has_nvidia() then
    m.packages = { "nvidia", "nvidia-utils", "cuda" }
else
    m.packages = {}
end

return m
```

A Lua host file (`hosts/myhostname.lua`) returns a table with the same fields as a YAML host file: `host`, `enabled_modules`, `packages`, `exclude`, `secrets`, `nix`, etc.

## Secrets workflow

mdots delegates all crypto to `sops`; it only manages file placement and key lookup.

### 1. Generate an age key

```bash
mdots secrets keygen
# or manually:
age-keygen -o ~/.config/sops/age/keys.txt
```

Print the public key:

```bash
age-keygen -y ~/.config/sops/age/keys.txt
# → age1xxxxxxx…
```

### 2. Register the recipient in `.sops.yaml`

```yaml
# ~/.config/mdots/.sops.yaml
creation_rules:
  - path_regex: secrets/.*
    age: age1xxxxxxx…
```

### 3. Encrypt a file

```bash
sops --encrypt --age age1xxxxxxx… ~/.ssh/config > \
    ~/.config/mdots/secrets/ssh.conf.sops
git -C ~/.config/mdots add secrets/ssh.conf.sops
```

### 4. Declare the secret in the host file

```yaml
sops_key_path: ~/.config/sops/age/keys.txt   # where mdots finds the private key
secrets:
  - source: secrets/ssh.conf.sops    # relative to the config repo root
    target: ~/.ssh/config            # MUST be outside the config repo
    mode: "0600"                     # octal; default is "0600"
    name: ssh-config                 # display name; defaults to target filename
```

**Critical guard:** the plaintext `target` must be outside `~/.config/mdots` (or wherever `MDOTS_CONFIG_DIR` points). mdots refuses to write decrypted content inside the config repo to prevent leaking plaintext into git.

### 5. Operate on secrets

```bash
mdots secrets status      # check which secrets are decrypted / pending / broken
mdots secrets list        # list declared secrets (no filesystem access)
mdots secrets edit ssh-config   # open the encrypted file in $EDITOR via sops
mdots secrets sync        # decrypt all declared secrets into place (without a full sync)
```

`mdots sync` runs `secrets sync` automatically as part of the sync pipeline.

Secrets are **decrypted and copied** at the target path — never symlinked. The copy is atomic (write to a sibling temp file, rename over the target) and starts `0600` before the requested `mode` is applied, so the plaintext is never world-readable at any point.

If `sops_key_path` points to a missing file, every secret in that host is skipped with a warning. Per-secret failures are warned and skipped — never fatal — so a broken secret cannot abort `mdots sync`.

## Nix / home-manager integration

Enable in the host file:

```yaml
nix:
  enabled: true                  # nix is installed on this machine
  home_manager_enabled: true     # run `home-manager switch` during sync
  flake_enabled: false           # use channels (default) or flakes
  nixpkgs_channel: nixpkgs-unstable
  home_manager_channel: release-25.05
```

```bash
mdots nix install        # install Nix and Home Manager (one-time setup)
mdots nix switch         # run Home Manager switch
mdots nix update         # update channels + run Home Manager switch
mdots nix search <pkg>   # search Nixpkgs
mdots nix status         # show Nix and Home Manager status
```

`home-manager` configuration lives in `~/.config/mdots/home-manager/home.nix` (or the directory mdots points at). When `home_manager_enabled: true`, `mdots sync` runs `home-manager switch` as part of the sync pipeline.

## Operator commands

| Command | What it does |
|---|---|
| `mdots init` | Create the `~/.config/mdots` directory skeleton and a starter `config.yaml`. |
| `mdots status` | Show current config, active host, enabled modules, package counts, and a drift summary (how many packages are declared-but-not-installed vs installed-but-not-declared). |
| `mdots diff` | Compute declared-vs-installed package diff and print it — read-only, no changes applied. Equivalent to the drift section of `status` but always machine-readable with `--json`. |
| `mdots doctor` | Run environment health checks (age key, sops, AUR helper, nix, home-manager, git repo) and report each item as PASS / WARN / FAIL. |
| `mdots validate [--check-packages]` | Parse and validate the config structure and all enabled modules. `--check-packages` additionally queries the package repos (slower). |
| `mdots sync [--dry-run]` | Install declared packages, enable/disable services, decrypt secrets, run home-manager — everything in one pass. `--dry-run` previews changes without applying. Other flags: `--prune` (remove untracked packages), `--force` (skip confirmation prompts), `--no-backup`, `--no-hooks`, `--auto-commit`. |
| `mdots find <PACKAGE>` | Search all host files, modules, and the base set for where a package name is declared. |
| `mdots module list\|enable\|disable\|run-hook\|create` | List available modules, toggle them in the host file, manually run a module's hook, or scaffold a new module. |
| `mdots service list\|enable\|disable\|show` | Manage systemd service profiles (from the `services/` directory). |
| `mdots secrets status\|sync\|list\|edit\|keygen` | SOPS/age secrets management — see the Secrets workflow section. |
| `mdots nix install\|switch\|update\|search\|status` | Nix / home-manager integration — see the Nix section. |
| `mdots tui` | Launch the interactive ratatui TUI (overview, modules, packages, sync-preview screens). |
| `mdots install <PACKAGE>` | Install a package with the AUR helper and add it to the declared package list. |
| `mdots remove <PACKAGE>` | Remove a package and untrack it from mdots. |
| `mdots forget <PACKAGE>` | Remove a package from mdots tracking without uninstalling it. |
| `mdots update` | Update the system (respects version constraints; runs pre/post hooks if configured). |
| `mdots search` | Interactive TUI package search. |
| `mdots edit` | Interactive TUI config-file selector (opens the chosen file in `$EDITOR`). |
| `mdots merge` | Add currently installed but untracked packages into `system-packages.yaml`. |
| `mdots generate` | Generate derived config files (completions, man page, declared-packages snapshot). |
| `mdots completion <SHELL>` | Print a shell completion script (bash, zsh, fish, …). |
| `mdots man` | Print the man page to stdout (roff format). |

All commands accept `-j` / `--json` for machine-readable output.

## Quick-start example

```bash
# 1. Initialise
mdots init

# 2. Edit the host file for this machine
mdots edit

# 3. Validate
mdots validate

# 4. Preview what sync would do
mdots sync --dry-run

# 5. Apply
mdots sync

# 6. Check drift after the fact
mdots diff
mdots doctor
```

## See also

- [Config conventions](config-conventions.md) — covers the `margo-config` (compositor) and `mshell-config` (shell YAML profiles) config worlds that mdots sits alongside.
- `mdots --help` and `mdots <subcommand> --help` always reflect the actual CLI.
- `mdots man` prints the generated man page.
