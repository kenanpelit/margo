# Companion tools

Margo ships several binaries that share its workspace:

| Binary | Role |
|---|---|
| **`margo`** | the compositor itself |
| **`mctl`** | IPC + dispatch (Swiss-army CLI) |
| **`mlayout`** | named monitor profiles |
| **`mscreenshot`** | screen / region / window capture |
| **`mplay`** | mpv companion — window control, video wallpaper, media keys |
| **`mpower`** | automatic power-profile daemon + manual `cycle` / `set` |
| **`mlogind`** | TUI login / display manager (matugen-themed) |

Run any of them with `--help` for the full command surface.

## `mctl`

Drives the compositor over its Unix control socket (`$MARGO_SOCKET`,
`get` / `watch` / `dispatch`). The old `dwl-ipc-unstable-v2` Wayland protocol
and the polled `state.json` sidecar were removed in favour of this single
socket — see [the IPC protocol](ipc.md).

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

## Screenshots & recording — `mshellctl screenshot` / `screenrecord`

The single front door. Keybinds, the CLI, and the GUI menu
(`mshellctl menu screenshot`, Super+Shift+S) all drive the **shell's own
capture engine** (rich in-shell selectors + save / clipboard / editor /
notify) — one engine, one tool.

```bash
mshellctl screenshot region          # in-shell selector → file + clipboard
mshellctl screenshot window          # focused window
mshellctl screenshot output          # pick a monitor
mshellctl screenshot full            # whole layout (all outputs)
#   flags: --save (file only) · --copy (clipboard only) · --edit (editor) · -d N (delay)

mshellctl screenrecord start full    # start recording (region|window|output|full)
mshellctl screenrecord toggle region --audio <pw-source>
mshellctl screenrecord stop
```

Bind them however you like, e.g. `bind = NONE,Print,spawn,mshellctl screenshot region`. The recording-indicator bar pill shows an active recording and stops it on click.

## `mscreenshot`

A standalone `grim` + `slurp` + `wl-copy` + editor CLI. The screenshot
keybinds no longer route through it (they use the shell engine above), but
it stays installed as a self-contained capture tool and a `slurp`-free
region fallback. Modes:

```bash
mscreenshot rec       # region → editor → file + clipboard
mscreenshot area      # region → file + clipboard (no editor)
mscreenshot screen    # focused output → file + clipboard
mscreenshot full      # all outputs (whole layout)
mscreenshot window    # focused window → file + clipboard
mscreenshot open      # open ~/Pictures/Screenshots in the file manager
mscreenshot dir       # print the screenshot dir
```

Editor preference: `swappy` if installed, else `satty`, else skip the editor pass and just save+copy. Files land at `~/Pictures/Screenshots/screenshot-YYYYMMDD-HHMMSS.png`. For region capture it reuses the shell's selector via `mshellctl screenshot select-region` (falling back to `slurp`).

## `mplay`

margo's native mpv companion — replaces the old `margo-mpv.sh` / `osc-media.sh`
scripts. Three jobs:

```sh
# Window control (talks to the mpv JSON IPC socket + mctl)
mplay start            # launch mpv (pseudo-gui) with an IPC socket
mplay toggle           # play / pause
mplay play [URL]       # play a file/URL (or the clipboard; ytdl auto)
mplay download [URL]   # yt-dlp → ~/Downloads
mplay snap             # cycle the floating mpv window across corners
mplay pin              # pin to all tags (sticky)
mplay focus            # focus the mpv window (hops monitor/tag)
mplay stop             # quit mpv

# Smart media control across players (MPRIS via playerctl, MPD via mpc, mpv)
mplay media toggle           # auto-detect the best active player
mplay media next|prev [PLAYER]   # PLAYER: spotify|vlc|mpv|mpd|browser

# Native video wallpaper (in-tree mpvpaper port: wlr-layer-shell + EGL + libmpv)
mplay wallpaper start <SRC> [--output N] [--mute] [--no-loop] [--scale fit|fill|stretch]
mplay wallpaper stop [--output N]
```

The embedded yt-dlp shim (anti-bot client fallback + cookie file + browser
user-agent) is built in — no external `yt-dlp-mpv` script. Optional deps:
`yt-dlp`, `playerctl`, `mpc`.

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

## `mpower`

A small **automatic power-profile manager** — a long-lived `systemd --user` daemon that picks the [power-profiles-daemon](https://gitlab.freedesktop.org/upower/power-profiles-daemon) profile (`performance` / `balanced` / `power-saver`) from live CPU load and AC/battery state. It replaces an external auto-profile script: the mechanism now ships with margo, and every knob is exposed in the shell under **Settings → Power → Automatic Power Profile**.

```bash
mpower status        # live state: power source, current profile, CPU now, thresholds
mpower cycle         # manually switch to the next profile (perf → balanced → saver)
mpower set balanced  # manually pick a profile (performance | balanced | power-saver)
mpower pause         # suspend auto-switching (leaves the current profile)
mpower resume        # resume + clear a manual `set`/`cycle` back-off
mpower auto          # back to fully automatic now (alias of resume)
```

A manual `cycle` / `set` (handy on a keybind — e.g. `ctrl+alt,p`) counts as a
manual override: the daemon honours it until the next AC transition (below).

Each tick (default 5 s) it reads the active profile, AC/battery from `/sys`, and CPU busy% (aggregate **and** the hottest single core) from `/proc/stat`, then:

- **On AC** — climbs to **performance** on sustained high load (the aggregate *or* one pegged core), drops back to **balanced** when calm; streaks + a cooldown damp flapping.
- **On battery** — **balanced**, or **power-saver** at/under a configurable charge floor. Performance is never selected on battery.
- **Manual override** — a profile you set by hand (the bar pill, the Settings dropdown, `powerprofilesctl`) is honoured until the next AC transition, then auto resumes.

- **Config.** `~/.config/margo/mpower.toml`, re-read every tick — edits (from the settings page or by hand) go live with no restart. A missing or partial file is filled from the defaults, so you only write the keys you want to change. The full key table is in [`mpower/README.md`](https://github.com/kenanpelit/margo/blob/main/mpower/README.md).
- **margo-only.** The shipped `mpower.service` carries `ConditionEnvironment=XDG_CURRENT_DESKTOP=margo`, so it only runs under a margo session and never fights another compositor's auto-profile tool over `powerprofilesctl`. Other compositors can keep their own daemon with the inverse condition.
- **Lean.** No D-Bus/UPower client — sysfs polling + a `powerprofilesctl` shell-out, with in-memory state (no state file).

## Shell completions

Bash, zsh, and fish completions ship under `/usr/share/{bash-completion,zsh,fish}/...` and pull dispatch action names from `mctl actions --names` at completion time. They auto-load — no rc-file work needed.

## See also

- The full per-action reference is generated from source: `mctl actions --verbose` always reflects what margo actually accepts.
- `mctl rules --verbose` is the right tool for "why didn't my windowrule fire?" — runs offline against the same rule engine.
