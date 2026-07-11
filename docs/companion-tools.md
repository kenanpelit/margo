# Companion tools

Margo ships several binaries that share its workspace:

| Binary | Role |
|---|---|
| **`margo`** | the compositor itself |
| **`mctl`** | IPC + dispatch (Swiss-army CLI) |
| **`mlayout`** | named monitor profiles |
| **`mscreenshot`** | screen / region / window capture |
| **`mvpn`** | native Mullvad VPN control (CLI + the DNS/VPN bar menu) |
| **`mcal`** | calendar — local + remote ICS (read-only), CLI + Settings → Calendar |
| **`mplay`** | mpv companion — window control, video wallpaper, media keys |
| **`mpower`** | automatic power-profile daemon + manual `cycle` / `set` |
| **`mkeys`** | on-screen keyboard (layer-shell, virtual-keyboard protocol) |
| **`mlogind`** | login / display manager (matugen-themed) — TUI or GTK4 greeter |
| **`mgreet`** | GTK4 multi-monitor login greeter for `mlogind` (the default host) |
| **`mdots`** | declarative Arch package + dotfiles manager (pacman/AUR/Flatpak/Nix, Lua modules, SOPS/age secrets) |

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
mshellctl screenshot region satty    # force satty (positional editor ⇒ implies --edit)
#   flags: --save (file only) · --copy (clipboard only) · --edit (editor) · -d N (delay)
#   positional EDITOR: satty | swappy | gimp | krita — overrides the default
#   chain ($SCREENSHOT_EDITOR env, then satty → swappy → gimp → krita)

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

A first-party **login / display manager**, forked from [lemurs](https://github.com/coastalwhite/lemurs) (MIT/Apache-2.0). It runs as a systemd service on a bare VT — no compositor needed to log in — presents a greeter (user + session switcher + password), authenticates through PAM, sets up the environment + utmpx, and launches the chosen X11 / Wayland session (margo included). The greeter renders either as a GTK4 multi-monitor surface (`mgreet`, the default) or a built-in `ratatui` TUI — see [`mgreet`](#mgreet) below. margo appears as a session out of the box (`/usr/share/wayland-sessions/margo.desktop`).

```bash
mlogind --preview        # draw the greeter in the current session (no login, no root)
sudo mlogind sync-theme   # repaint /etc/mlogind/variables.toml from the active wallpaper
```

- **Greeter host.** `[display] host` in `/etc/mlogind/config.toml` picks how the greeter reaches the screen: `gui` (the shipped default — the GTK4 [`mgreet`](#mgreet) greeter under a throwaway root `margo`, a login card on every monitor at its own native mode), `cage` (a `cage` wlroots kiosk hosting a terminal greeter), or `tty` (the in-process `ratatui` greeter straight to the VT). `gui` auto-falls back to `cage`, then `tty`, if `margo` / `mgreet` / `mctl` can't start — so it never locks you out.
- **Theming.** Colours are `$`-variables resolved from `/etc/mlogind/variables.toml`, mapped from the margo **matugen** palette — the active session stands out in the accent colour. `mlogind sync-theme` copies the live wallpaper palette (margo writes it to `~/.config/margo/mlogind-variables.toml` on every theme change) into the greeter, so the login screen tracks the desktop.
- **Power controls.** `F1` Shutdown · `F2` Reboot · `F3` Suspend.
- **Fingerprint (opt-in).** Handled at the PAM level — uncomment `pam_fprintd.so` in `/etc/pam.d/mlogind` after `fprintd-enroll`.
- **Packaging.** Config + PAM + the systemd unit install to `/etc/mlogind/`, `/etc/pam.d/mlogind`, and `/usr/lib/systemd/system/mlogind.service`, but the package never enables it — switching login managers is a deliberate `systemctl disable --now <old-dm> && systemctl enable mlogind`. Defaults to `tty2`; for another VT add a drop-in under `/etc/systemd/system/mlogind.service.d/`.

## `mgreet`

The GTK4 **graphical login greeter** for [`mlogind`](#mlogind), and its default
greeter host. Where the TUI greeter is a `ratatui` VT surface, `mgreet` renders
one `gtk4-layer-shell` card per output under a throwaway root `margo` instance —
so a login prompt lands on *every* connected monitor at its own native mode
(something the older `cage` host could not do). It wears the same **matugen**
palette as the desktop, shows the wallpaper as a blurred backdrop and the
`~/.face` avatar, and blanks itself after `[display] blank_timeout` seconds of
inactivity (any key wakes it without landing in the field).

```bash
mgreet --preview     # draw the greeter in the current session — no PAM, power keys inert
```

`mgreet` never authenticates or opens the session itself: it collects the
username + password and hands them to the privileged `mlogind` orchestrator over
a one-shot socket, which runs PAM and launches the session — that split keeps the
GTK4 front-end unprivileged. You don't invoke it directly; `mlogind` starts it
when `[display] host = "gui"` (the default). Use `--preview` to see it
non-destructively.

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

## `mvpn`

Native **Mullvad VPN** control — a GTK-free engine wrapping the `mullvad` CLI
plus a GTK4 layer-shell panel, replacing the old `osc-mullvad` script and the
`mullvad` WASM plugin. The daemon + `~/.mullvad/{favorites.txt,slot.state}` are
the source of truth, so the CLI and the shell both call straight into the same
engine (no extra service).

```bash
mvpn                       # connection status (relay · country, city · protocol)
mvpn status --json         # machine-readable; --pill for the bar feed, -v verbose
mvpn connect / disconnect / toggle / reconnect
mvpn de            ;  mvpn us nyc      # connect by country / country+city
mvpn random [cc]                       # a random relay (optionally in a country)
mvpn fastest [cc]                      # ping EVERY relay in <cc>, connect to the
                                       #   genuinely fastest (prints each ping)
mvpn fastest-fav [cc]                  # same sweep, but also save the winner
mvpn fav add | remove <relay> | list | connect | refresh [cc]
mvpn obf [auto|off|udp2tcp|shadowsocks|quic|cycle|hunt443]   # anti-censorship
mvpn lockdown on|off   ;  mvpn auto-connect on|off  ;  mvpn quantum
mvpn slot <recycle|status|whoami|list|revoke|disconnect>     # multi-machine slots
mvpn timer <start N|stop|status>       # auto-switch relay every N minutes
mvpn test          ;  mvpn split       # leak test · split-tunnel processes
mvpn ensure                            # drive the blocky DNS guard from VPN state
```

- **Notifications.** connect / disconnect / toggle / random / fastest raise a
  desktop toast with the resulting relay + location (silence with
  `MVPN_NO_NOTIFY=1`).
- **Bar pill + menu.** The **DNS / VPN** bar pill (`Vpn` widget) is accent-tinted
  when the tunnel is up; left-click opens the shell's native layer-shell VPN
  menu (connect / random / fastest / favourites + lockdown / auto-connect /
  quantum / anti-censorship, plus a collapsible **DNS** section for the Blocky
  guard and DNS presets), right-click toggles the tunnel. Open it from a
  terminal with `mshellctl menu vpn`. Full relay management also lives in
  **Settings → VPN**.
- **osc-mullvad compatible.** Reads the existing `favorites.txt` / `slot.state`
  unchanged and honours the `OSC_MULLVAD_*` env overrides.

## `mcal`

A read-only **calendar** viewer — a GTK-free engine + CLI. Reads local `.ics`
folders and remote iCalendar subscriptions, and connects **Google** calendars
over OAuth. The same engine backs the shell: **Settings → Calendar** manages the
sources, and the dashboard / clock calendar marks the days that have events.

```bash
mcal today                            # events happening today
mcal agenda 14                        # events over the next 14 days (default 7)
mcal on 2026-07-20                    # events on a specific date
mcal --ics https://…/feed.ics today  # add a remote subscription (repeatable)
mcal account setup google            # connect a Google account (OAuth)
mcal account list                    # list connected accounts (remove with: account remove <id>)
```

Local calendars live under `~/.config/margo/calendars` by default (`--dir`
overrides). It's a viewer for now — creating and editing events is not yet
supported.

## `mkeys`

A standalone **on-screen keyboard** (a wkeys-style port): a GTK4 layer-shell
surface that types into the focused window via `zwp_virtual_keyboard`. Toggled
over its own socket, themed from the matugen palette, with en/tr layouts.

```bash
mkeys            # show (or focus) the keyboard
mkeys toggle     # show / hide — bind this, or use the bar pill / Settings page
mkeys hide
```

Config lives in `~/.config/margo/mkeys.toml`; a bar pill and a **Settings →
On-screen keyboard** page expose the toggle + layout.

## `mpicker`

A native **screen colour picker**. Freezes the screen via `wlr-screencopy`,
overlays a zoom lens, and copies the pixel under the cursor as hex / rgb to the
clipboard — no external `hyprpicker`/`grim` pipeline.

```bash
mpicker           # pick a colour → clipboard
```

Bind it, or use the **ColorPicker** bar pill.

## `mlock`

The **lock-screen** binary — PAM authentication over `ext-session-lock-v1`
(the fail-secure protocol: a crash keeps the session locked). It's the locker
`loginctl lock-session`, the **Lock** bar pill, and the idle/lid triggers all
resolve to. Matugen-themed, with media-key + keyboard-layout support.

```bash
mlock             # lock now (usually invoked for you, not by hand)
```

## `start-margo`

The **TTY session launcher / supervisor**. Forks the compositor, forwards
signals, sets `PR_SET_PDEATHSIG` so nothing is orphaned if it dies, and pairs
with the systemd watchdog + restart backoff so a hung compositor is recovered
rather than left frozen. This is what a display manager (or `exec start-margo`
from a TTY) runs — not `margo` directly.

## Shell completions

Bash, zsh, and fish completions ship under `/usr/share/{bash-completion,zsh,fish}/...` and pull dispatch action names from `mctl actions --names` at completion time. They auto-load — no rc-file work needed.

## `mdots`

Declarative Arch package + dotfiles manager — declares which pacman/AUR/Flatpak/Nix packages, systemd services, and SOPS/age-encrypted secrets each machine should have, then drives the appropriate package managers to converge reality to the declaration. Supports static YAML modules, Lua modules with runtime hardware/service detection, and Nix/home-manager integration.

Full user guide: **[mdots](mdots.md)**

## See also

- The full per-action reference is generated from source: `mctl actions --verbose` always reflects what margo actually accepts.
- `mctl rules --verbose` is the right tool for "why didn't my windowrule fire?" — runs offline against the same rule engine.
