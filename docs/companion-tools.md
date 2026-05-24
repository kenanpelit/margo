# Companion tools

Margo ships several binaries that share its workspace:

| Binary | Role |
|---|---|
| **`margo`** | the compositor itself |
| **`mctl`** | IPC + dispatch (Swiss-army CLI) |
| **`mlayout`** | named monitor profiles |
| **`mscreenshot`** | screen / region / window capture |
| **`mlogind`** | TUI login / display manager (matugen-themed) |

Run any of them with `--help` for the full command surface.

## `mctl`

Drives the compositor over the in-tree Wayland IPC plus a state.json sidecar.

### Inspection (no compositor side-effects)

```bash
mctl status                              # per-output: focused / tags / layout
mctl status --json                       # stable schema, version: 1
mctl clients --tag 2                     # every window on tag 2 (table)
mctl clients --json | jq '.[].app_id'
mctl outputs --json | jq '.[].name'
mctl focused                             # `app_id · title`, scriptable
mctl watch                               # streaming state on stdout
```

### Configuration validation (offline — no compositor needed)

```bash
mctl check-config                            # exit 1 on any error
mctl check-config ~/.config/margo/test.conf
mctl rules --appid X --title Y --verbose     # which rules match, which reject
mctl actions --verbose                       # the full dispatch catalogue
```

### Dispatch

Anything bindable from `config.conf` is also dispatchable from the shell. Argument shape matches `bind = …` lines:

```bash
mctl dispatch togglefullscreen
mctl dispatch view 4                     # tag bitmask 4 = tag 3
mctl dispatch togglefloating
mctl dispatch focus_next
mctl dispatch killclient
```

### Live ops

```bash
mctl reload                              # re-read config tree, re-apply
mctl run path/to/oneshot.rhai            # eval a script against the live engine
mctl spawn kitty                         # spawn a process under margo's session
```

### Migrate from another compositor

```bash
mctl migrate --from hyprland ~/.config/hypr/hyprland.conf > ~/.config/margo/config.conf
mctl migrate --from sway     ~/.config/sway/config         > ~/.config/margo/config.conf
mctl migrate ~/.config/sway/config                         > out.conf  # auto-detect
```

Translates the high-value subset (keybinds, spawn lines, workspace → tag bitmask, modifier names, key aliases). Window rules / animations / monitor topology stay manual — auto-translating them would invent semantics the source compositor doesn't actually mean. Niri's KDL is intentionally out-of-scope (workspaces+scrolling don't map onto tag-based without inventing wrong semantics). Unconvertible source lines emit warnings to stderr with line numbers; every translatable line still gets written.

## `mlayout`

Named monitor-topology profiles. Useful for laptops with frequent dock changes.

```bash
mlayout suggest                          # propose & activate a preset for the live setup
mlayout list                             # show saved profiles
mlayout set vertical-ext-top             # apply a saved profile
mlayout save my-desk                     # save the current topology under that name
mlayout edit my-desk                     # open the profile in $EDITOR
```

Profiles live at `~/.config/margo/layouts/<name>.conf`. Internally `mlayout set` re-positions outputs via `wlr-randr` (which routes through margo's `wlr-output-management-v1` handler — runtime mode + position changes apply live, no logout).

## `mscreenshot`

Wraps `grim` + `slurp` + `wl-copy` + an optional editor. Modes:

```bash
mscreenshot rec       # region → editor → file + clipboard
mscreenshot area      # region → file + clipboard (no editor)
mscreenshot screen    # focused output → file + clipboard
mscreenshot window    # focused window → file + clipboard
mscreenshot open      # open ~/Pictures/Screenshots in the file manager
mscreenshot dir       # print the screenshot dir
```

Editor preference: `swappy` if installed, else `satty`, else skip the editor pass and just save+copy. Files land at `~/Pictures/Screenshots/screenshot-YYYYMMDD-HHMMSS.png`.

### In-compositor region selector

When bound via `bind = NONE,Print,screenshot-region-ui`, margo dims the screen, lets you drag a rect (Enter to confirm, Esc to cancel) and spawns `mscreenshot <mode>` with `MARGO_REGION_GEOM="X,Y WxH"` set so `slurp` is skipped. Cursor stays visible while in selection mode (W2.1).

## `mlogind`

A first-party **TUI login / display manager**, forked from [lemurs](https://github.com/coastalwhite/lemurs) (MIT/Apache-2.0). It runs as a systemd service on a bare VT — no compositor needed to log in — draws a `ratatui` greeter (user + session switcher + password), authenticates through PAM, sets up the environment + utmpx, and launches the chosen X11 / Wayland session (margo included). margo appears as a session out of the box (`/usr/share/wayland-sessions/margo.desktop`).

```bash
mlogind --preview        # draw the greeter in the current session (no login, no root)
sudo mlogind sync-theme   # repaint /etc/mlogind/variables.toml from the active wallpaper
```

- **Theming.** Colours are `$`-variables resolved from `/etc/mlogind/variables.toml`, mapped from the margo **matugen** palette — the active session stands out in the accent colour. `mlogind sync-theme` copies the live wallpaper palette (margo writes it to `~/.config/margo/mlogind-variables.toml` on every theme change) into the greeter, so the login screen tracks the desktop.
- **Power controls.** `F1` Shutdown · `F2` Reboot · `F3` Suspend.
- **Fingerprint (opt-in).** Handled at the PAM level — uncomment `pam_fprintd.so` in `/etc/pam.d/mlogind` after `fprintd-enroll`.
- **Packaging.** Config + PAM + the systemd unit install to `/etc/mlogind/`, `/etc/pam.d/mlogind`, and `/usr/lib/systemd/system/mlogind.service`, but the package never enables it — switching login managers is a deliberate `systemctl disable --now <old-dm> && systemctl enable mlogind`. Defaults to `tty2`; for another VT add a drop-in under `/etc/systemd/system/mlogind.service.d/`.

## Shell completions

Bash, zsh, and fish completions ship under `/usr/share/{bash-completion,zsh,fish}/...` and pull dispatch action names from `mctl actions --names` at completion time. They auto-load — no rc-file work needed.

## See also

- The full per-action reference is generated from source: `mctl actions --verbose` always reflects what margo actually accepts.
- `mctl rules --verbose` is the right tool for "why didn't my windowrule fire?" — runs offline against the same rule engine.
