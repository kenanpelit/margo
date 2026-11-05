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
| **`mtune`** | folder-first music player — GTK4 + GStreamer, MPRIS, tray, own bar pill + menu |
| **`mpower`** | automatic power-profile daemon + manual `cycle` / `set` |
| **`mkeys`** | on-screen keyboard (layer-shell, virtual-keyboard protocol) |
| **`mlogind`** | login / display manager (matugen-themed) — TUI or GTK4 greeter |
| **`mgreet`** | GTK4 multi-monitor login greeter for `mlogind` (the default host) |
| **`mdots`** | declarative Arch package + dotfiles manager (pacman/AUR/Flatpak/Nix, Lua modules, SOPS/age secrets) |
| **`mpicker`** | native screen colour picker (frozen overlay + zoom lens) |
| **`mlock`** | lock screen (PAM + `ext-session-lock-v1`) |
| **`start-margo`** | TTY launcher for a margo session |
| **`mtm`** | tmux session / layout / buffer / clipboard manager |
| **`mvisual`** | GTK4 layout previewer — every layout side by side, per-tag pinning rehearsal |
| **`mwizard`** | opens the in-shell setup wizard (compatibility shim) |
| **`margo-portal`** | margo's own xdg-desktop-portal backend (ScreenCast, Screenshot, GlobalShortcuts) |
| **`mshellshare`** | window/monitor picker helper for portal screen sharing |

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

mshellctl screenshot ocr             # region → OCR (tesseract) → text on the clipboard
mshellctl screenshot ocr --lang tur  #   override the languages for one run
mshellctl screenshot qr              # region → decode QR / barcode → clipboard
mshellctl screenshot palette -n 6    # region → 6 dominant colours (hex) → clipboard

mshellctl screenrecord start full    # start recording (region|window|output|full)
mshellctl screenrecord toggle region --audio <pw-source>
mshellctl screenrecord start full --audio both           # desktop + mic mixed
mshellctl screenrecord start output --backend gsr        # gpu-screen-recorder (VAAPI)
mshellctl screenrecord stop
```

Bind them however you like, e.g. `bind = NONE,Print,spawn,mshellctl screenshot region`. The recording-indicator bar pill shows an active recording and stops it on click.

Recording audio (`--audio`, or the Screen Record menu's audio row): a
single PipeWire source, `desktop`, `mic`, or `both` / *Desktop +
microphone* — desktop audio and the default microphone mixed into one
track. Two recorders: **wf-recorder** (default, CPU x264) and
**gpu-screen-recorder** (hardware encoding: VAAPI on Intel/AMD, NVENC on
NVIDIA; `-fallback-cpu-encoding` keeps it recording without a usable
encoder). The menu's recorder row appears when `gpu-screen-recorder` is
installed; `--backend wf|gsr` overrides it per run. gpu-screen-recorder
mixes the sources itself; for wf-recorder (one source only) mshell loads a
private PipeWire null sink plus one loopback per source with `pactl`,
records its monitor and unloads the modules when the clip ends (leftovers
from a crash are swept at the next mixed recording).

### Webcam overlay

`mshellctl screenrecord start <area> --webcam [small|medium|large]` (or the
screen-record menu's **Webcam** row, shown when a camera exists) puts a
click-through camera bubble inside the bottom-right of the recorded area; the
recorder captures it with the screen, so it is in the clip with no second
encode. The recorder starts once the first camera frame shows (≤ 2.5 s); a
camera that can't start never blocks the recording (it records without and
says why). Needs `ffmpeg`; `v4l2-ctl` (v4l-utils) picks a colour capture node
and a 30 fps mode. The saved-recording notification's **View** opens the clip
in the default player.

### Region tools

The screenshot menu's **Extract from area** row and the matching commands
select an area with the in-shell selector, run a tool on it and put the
result on the clipboard with a notification preview. The capture is a
temporary file that is deleted as soon as the tool has read it.

| Tool | Command | Compositor action | Needs |
|---|---|---|---|
| Text (OCR) | `mscreenshot ocr` · `mshellctl screenshot ocr` | `screenshot-ocr [LANGS]` | `tesseract` + `tesseract-data-<lang>` |
| QR / barcode | `mscreenshot qr` · `mshellctl screenshot qr` | `screenshot-qr` | `zbarimg` (zbar) |
| Palette | `mscreenshot palette [-n N]` · `mshellctl screenshot palette` | `screenshot-palette` | — (native median cut) |
| Text → translation | `mscreenshot ocr --translate [--to LANG]` · `mshellctl screenshot ocr --translate` | — | `tesseract` + Settings → Translate (network) |
| Measure (ruler) | `mscreenshot measure` · `mshellctl screenshot measure` | — | — (`slurp -d` only as fallback) |
| Pin on screen | `mshellctl screenshot region --pin` (any area) · screenshot menu → save option **Pin on screen** | — | — |

OCR languages: `--lang` / the action argument, else
`MSCREENSHOT_OCR_LANG` (set it once with `env = MSCREENSHOT_OCR_LANG,eng+tur`
in the margo config), else `eng+tur` narrowed to the installed tessdata
packs (plain `eng` without `tesseract-data-tur`). Asking for a language
whose pack is missing fails with the package to install instead of
silently falling back to English. `MSCREENSHOT_OCR_PSM` / `--psm` sets
tesseract's page segmentation (default 6, one block of text);
`mscreenshot ocr --screen` reads the focused output instead of a region.

QR / barcode: every decoded payload is copied, one per line (multi-line
payloads stay intact). When exactly one code was found and it is a plain
`http(s)://` link, the notification offers **Open link**; other schemes
(`file:`, `javascript:`, Wi-Fi configs, …) are only copied — a code on
screen is untrusted input.

OCR → translate: the recognised text goes through the same engine and
settings as `mshellctl translate` (Settings → Translate: provider, target,
secondary language for the auto-flip; `--to LANG` forces a target and
disables the flip). Text that looks like a token or key is never sent — it
is copied untranslated and the notification says why.

Measure: drag a box (the in-shell selector labels its size and aspect while
you drag) and its size lands on the clipboard as `WxH` in the output's
device pixels; the notification adds the position relative to the output,
the aspect ratio and, on a scaled output, the logical size.

Pin: the capture floats above every window on an overlay layer — a region
pin opens exactly where the region was, 1:1. Drag to move, scroll to zoom,
Ctrl+scroll for opacity, Ctrl+C copy, Ctrl+S save, `0` resets, Esc or
right-click closes. Nothing is written to disk unless you save.

Palette: up to N (default 8, max 16) dominant colours by median cut over
the region's opaque pixels, most common first, copied as `#RRGGBB` lines;
the notification lists each colour's share and shows a swatch strip.

```
bind = SUPER,Print,screenshot-ocr
bind = SUPER+SHIFT,Print,screenshot-ocr,tur
```

## `mscreenshot`

A standalone `grim` + `slurp` + `wl-copy` + editor CLI. The screenshot
keybinds no longer route through it (they use the shell engine above), but
it stays installed as a self-contained capture tool and a `slurp`-free
region fallback. Modes:

```bash
mscreenshot rec       # region → editor → file + clipboard (alias: region)
mscreenshot area      # region → file (no editor, no clipboard)
mscreenshot screen    # focused output → editor → file (alias: output)
mscreenshot all       # all outputs (whole layout) → editor → file (alias: full)
mscreenshot window    # focused window → editor → file
mscreenshot ocr       # region → OCR → text on the clipboard (see Region tools; --translate)
mscreenshot qr        # region → QR / barcode → clipboard
mscreenshot palette   # region → dominant colours → clipboard
mscreenshot measure   # region → size (WxH, device px) → clipboard
mscreenshot open      # open ~/Pictures/Screenshots in the file manager
mscreenshot dir       # open the screenshot dir
```

Editor preference: `$SCREENSHOT_EDITOR`, else `satty`, `swappy`, `gimp`, `krita` (first installed), else skip the editor pass and just save+copy. Files land at `~/Pictures/Screenshots/screenshot-YYYYMMDD-HHMMSS.png`. For region capture it reuses the shell's selector via `mshellctl screenshot select-region` (falling back to `slurp`).

## Media & mpv companion — `mshellctl media` / `play`

Another single front door: the mpv companion (window control + playback +
yt-dlp downloads + a native video-wallpaper engine) and smart cross-player
media control both live natively in `mshellctl` — no separate binary, no
`playerctl`/`mpc` subprocess shellouts. Media picking talks to MPRIS players
and native MPD directly; the mpv companion talks to margo via `mctl`'s IPC
library and to mpv's own JSON IPC socket.

```sh
# Smart media control — scores every active player (MPRIS + native MPD),
# picks the best one (playing beats paused, app/backend bonuses, last-used
# tie-break), and remembers it. An explicit target skips scoring.
mshellctl media toggle              # auto-detect the best active player
mshellctl media next|prev [TARGET]  # TARGET: spotify | vlc | browser | mpd | mpc | …
mshellctl media status [--json]     # the player a target-less command would act on
mshellctl media list [--json]       # every player, MPRIS + MPD, with state

# mpv companion window control (talks to mpv's JSON IPC socket + mctl)
mshellctl play start            # launch mpv (pseudo-gui) with an IPC socket
mshellctl play toggle           # play / pause
mshellctl play play [URL]       # play a file/URL (or the clipboard; ytdl auto)
mshellctl play queue [URL]      # append to the playlist (same window; starts mpv if idle)
mshellctl play forget ID|--all  # drop a YouTube video's history entry + resume point
mshellctl play download [URL]   # yt-dlp → ~/Downloads
mshellctl play snap             # cycle the floating mpv window across corners
mshellctl play pin              # pin to all tags (sticky)
mshellctl play focus            # focus the mpv window (hops monitor/tag)
mshellctl play stop             # quit mpv

# Native video wallpaper (wlr-layer-shell + EGL + direct libmpv render-API FFI)
mshellctl play wallpaper start <SRC> [--output N] [--mute] [--no-loop] [--scale fit|fill|stretch]
mshellctl play wallpaper stop [--output N]
```

YouTube videos started with `play` / `queue` resume where you stopped
(unless the URL carries its own `t=`, or the video was watched to the end):
a small tracker (`mshellctl play __track`, started alongside mpv) records
the position every few seconds, on pause and when a video ends, into
`~/.local/state/margo/youtube/progress.json` — the history the launcher's
`yt` provider lists. `--title` / `--channel` / `--channel-id` /
`--duration` attach metadata to that history entry.

The embedded yt-dlp shim (anti-bot client fallback + cookie file + browser
user-agent) is built in — no external `yt-dlp-mpv` script. Optional dep:
`yt-dlp`, for `play`/`download`'s YouTube handling. `mpv` (libmpv) is
required, build- and runtime-side, for the companion + wallpaper commands.

## `mtune`

A native **folder-first** music player — a de-branded fork of
[Amberol](https://gitlab.gnome.org/World/amberol), vendored into the workspace
and re-themed to the margo look with a live matugen palette. Where `mshellctl
media` *controls* other players, `mtune` *is* one.

- **Folder library, not a database.** Point `[library] roots` in
  `~/.config/margo/mtune.toml` (or Settings → Tune) at one or more folders;
  recursive off-thread scan, an on-disk tag index, and a debounced inotify
  watcher keep the queue live as files come and go.
- **Interop.** MPRIS + MPRIS `TrackList`, a `StatusNotifierItem` tray icon, and
  a supplementary `org.margo.Tune` D-Bus interface for the library / queue that
  MPRIS can't express.
- **Shell surface.** A dedicated **Tune bar pill** (queue position + elapsed /
  total) and a menu with a draggable seek bar, transport, shuffle / repeat, a
  speed control, and the folder / playlist actions — distinct from the generic
  MPRIS media pill.
- **Playback.** Speed control with **pitch held constant** (`scaletempo`),
  playlists (m3u / m3u8 / pls, open + save — each saved playlist remembers
  and resumes at its own last-played track), resume the last track +
  position on launch, ReplayGain, compact **mini / strip** window skins
  (cycled with `Ctrl+M`), and desktop notifications on track / setting
  change. Repeat modes: off / all / one / **each** (repeat a track N times,
  then advance — `[playback] repeat_count`, default 3).
- **Radio & Podcasts** (main menu → *Radio & Podcasts…*, `Ctrl+Shift+R`).
  *Radio* searches the [radio-browser.info](https://www.radio-browser.info)
  directory by name, country and genre (most voted / trending / name);
  *My Stations* keeps starred stations and your own stream URLs (Icecast,
  Shoutcast, HLS); *Podcasts* finds shows through iTunes search or a pasted
  RSS feed URL, subscribes, and lists episodes with played state and resume
  points. Everything plays through the normal queue: a station plays with
  its list queued, so next / previous (window, MPRIS, the Tune pill) step
  through stations, and the stream's own *now playing* title (Icecast
  `StreamTitle`) reaches MPRIS — the shell's media widgets show the station,
  the programme and its logo. Episodes are added to the queue and resume
  where you left them; one counts as played in its last 30 s (or past 95 %).
  Stored in `~/.local/share/mtune/radio.json` and `podcasts.json` (a file
  that does not parse is kept, never overwritten); logos and covers are
  cached under `~/.cache/mtune/art/`.
- **Start hidden.** `mtune --hidden`, `[behaviour] start_hidden = true`, or the
  Settings toggle — runs in the tray with no window. Show / hide it with the
  tray icon or `mshellctl mtune toggle-window`.

```sh
mtune                          # open the player
mtune --hidden                 # start in the tray, no window
mtune ~/Music/album            # queue a folder / file / playlist and play

# CLI — talks straight to mtune's D-Bus (spawns it for actions)
mshellctl mtune play-pause
mshellctl mtune next | previous
mshellctl mtune seek 90        # absolute seconds; +30 / -30 for relative
mshellctl mtune rate 1.5       # 0.5–2.0, pitch preserved
mshellctl mtune repeat one     # off | all | one | each | cycle
mshellctl mtune repeat-count 5 # times to repeat under "each" (default 3)
mshellctl mtune jump 12        # play queue entry 12
mshellctl mtune playlist-save "focus"
mshellctl mtune status --json  # now-playing + queue state
mshellctl mtune toggle-window  # show / hide the window
```

Runtime deps: `gstreamer` + `gst-plugins-base` / `gst-plugins-good` (which
also carries the HTTP source radio and podcasts stream through); optional
`gst-libav` / `gst-plugins-bad` for AAC / M4A / ALAC / WMA, extra containers
and HLS radio streams.

## `mlogind`

A first-party **login / display manager**, forked from [lemurs](https://github.com/coastalwhite/lemurs) (MIT/Apache-2.0). It runs as a systemd service on a bare VT — no compositor needed to log in — presents a greeter (user + session switcher + password), authenticates through PAM, sets up the environment + utmpx, and launches the chosen X11 / Wayland session (margo included). The greeter renders either as a GTK4 multi-monitor surface (`mgreet`, the default) or a built-in `ratatui` TUI — see [`mgreet`](#mgreet) below. margo appears as a session out of the box (`/usr/share/wayland-sessions/margo.desktop`).

```bash
mlogind --preview        # draw the greeter in the current session (no login, no root)
sudo mlogind sync-theme   # publish the active wallpaper palette to /var/lib/mgreet/variables.toml
```

- **Greeter host.** `[display] host` in `/etc/mlogind/config.toml` picks how the greeter reaches the screen: `gui` (the shipped default — the GTK4 [`mgreet`](#mgreet) greeter under a throwaway root `margo`, a login card on every monitor at its own native mode), `cage` (a `cage` wlroots kiosk hosting a terminal greeter), or `tty` (the in-process `ratatui` greeter straight to the VT). `gui` auto-falls back to `cage`, then `tty`, if `margo` / `mgreet` / `mctl` can't start — so it never locks you out.
- **Theming.** Colours are `$`-variables resolved from `/etc/mlogind/variables.toml`, mapped from the margo **matugen** palette — the active session stands out in the accent colour. `mlogind sync-theme` (also run after every login) validates the live wallpaper palette (margo writes it to `~/.config/margo/mlogind-variables.toml` on every theme change) and publishes it as `/var/lib/mgreet/variables.toml`, which overlays the admin's file, so the login screen tracks the desktop.
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
mvpn status --json --follow  # a JSON line per change until stdin closes (mshell's VPN feed)
mvpn connect / disconnect / toggle / reconnect
mvpn de            ;  mvpn us nyc      # connect by country / country+city
mvpn locations                         # relay locations as a JSON country → city tree
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

## `mtm`

A native tmux manager (Rust port of `tm.sh`): `mtm session …`
(create / list / kill / attach / layout / term), `mtm buffer` (pick a tmux
buffer with fzf and copy it), `mtm clip` (clipboard history via cliphist or
clipse), `mtm plugin` (TPM plugins), `mtm speed` (a category-aware fzf command
launcher with pins and recency) and `mtm config` (config backup / restore).
Bare `mtm` attaches to the default session; any other word is taken as a
session name to create or attach to.

## `mvisual`

An interactive GTK4 design tool for the layouts: every layout renders side by
side at the current parameters, a tag rail rehearses per-tag layout pinning,
and every tweak (window count, `mfact`, `nmaster`, gaps, focus, scroller
proportion) re-runs the same `margo-layouts` arithmetic the compositor uses.

## `mwizard`

A thin shim kept for old keybinds and autostart entries: it asks the running
shell to open the in-shell setup wizard over D-Bus, exactly like
`mshellctl wizard`. Legacy flags such as `--force` are accepted and ignored.

## `margo-portal` and `mshellshare`

`margo-portal` is margo's own xdg-desktop-portal backend. It serves
**ScreenCast** (share a window or a monitor — capture is margo's own PipeWire
path), **Screenshot** and **GlobalShortcuts**, is D-Bus-activated on the first
portal call, and is selected in `margo-portals.conf`. `mshellshare` is the
picker helper behind screen sharing: it asks the shell (`com.mshell.Shell`)
for the window/monitor choice and prints it for the portal. Neither is run by
hand. Design notes: [Built-in portal](portal-design.md).

## Shell completions

Bash, zsh, and fish completions ship under `/usr/share/{bash-completion,zsh,fish}/...` and pull dispatch action names from `mctl actions --names` at completion time. They auto-load — no rc-file work needed.

## `mdots`

Declarative Arch package + dotfiles manager — declares which pacman/AUR/Flatpak/Nix packages, systemd services, and SOPS/age-encrypted secrets each machine should have, then drives the appropriate package managers to converge reality to the declaration. Supports static YAML modules, Lua modules with runtime hardware/service detection, and Nix/home-manager integration.

Full user guide: **[mdots](mdots.md)**

## See also

- The full per-action reference is generated from the action catalogue: [Configuration → Dispatch actions](configuration.md#dispatch-actions) and `mctl actions --verbose` both list everything margo accepts.
- `mctl rules --verbose` is the right tool for "why didn't my windowrule fire?" — runs offline against the same rule engine.
