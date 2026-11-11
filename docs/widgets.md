# Bar widgets

The mshell bar is a row of **widgets** ("pills"). Some are *indicators*
(render-only — a clock, a CPU chip, a Wi-Fi glyph); others are *openers* that
toggle a layer-shell menu (the dashboard, the network panel, the VPN controls);
a few are *actions* (lock, screenshot, dark-mode toggle).

Every bar has three slots — **start / center / end** — and each holds an ordered
list of widgets. You almost never edit these by hand:

- **Settings → Bar → Top / Bottom bar** is the visual editor: add, remove, and
  reorder widgets per slot, and toggle the bar itself.
- **Settings → Bar pill** tunes per-pill placement and behaviour for the
  passive pills.

Under the hood each slot is a list in the shell profile
(`bars.widgets.{top,bottom}.{start,center,end}`). In that list a widget is its
**token** — the unit name (`Clock`, `Network`, `Vpn`, …). Three widgets take a
parameter and use YAML tag syntax:

```yaml
- !Custom my_button        # a user-defined pill (see Custom, below)
- !Spacer 8                # an 8 px gap
- !HiddenBarNamed media    # a named drawer
```

!!! tip "Live reload"
    Bar changes from Settings apply immediately. There is no compositor
    re-login needed — widgets are a shell concern.

---

## Time, dashboard & quick settings

| Token | Widget | What it does |
| --- | --- | --- |
| `Clock` | Clock | Date/time pill. Cycles through a list of strftime formats (`[tempo]`) on right-click; left-click opens the clock/calendar menu. **World clocks:** Settings → Widgets → Clock → "Show world clocks in the Clock menu" adds a World Clocks section (`MenuWidget::WorldClocks`) to the Clock menu: the device row, then one row per city in `tempo.world_clocks` with its time, "3h ahead · Tomorrow morning · Outside working hours" and an hour strip (−2…+9 h, now highlighted, the working day `tempo.work_day_start`–`work_day_end`, 9–17, shaded). Add cities with the + search (bundled IANA database, no network), move up, remove, or pin one as the device timezone (`timedatectl set-timezone`, asks polkit). `tempo.world_clocks_in_bar` (0–3, default 0) puts the first cities next to the bar time (`New York 07:05`); the tooltip lists them all. |
| `Mdash` | Mdash | The dashboard opener — same clock label as `Clock`, but a click opens **mdash**, the tabbed dashboard: a header (greeting, date and time, Settings, one power button), a status strip and four tabs — **Today**, **Controls**, **System**, **Tools**. See [mdash dashboard](#mdash-dashboard). |
| `ControlCenter` | Control Center | System-preferences glyph that opens the Control Center menu (header + sliders + a grid of quick toggles; opt-in tiles such as Hotspot are enabled from the grid's edit mode). The optional **Mirror** tile (off by default — tick it in the grid's edit mode) toggles margo's output mirroring, laptop panel onto the external display, and shows the running `source → target`. The optional **Internal Keyboard** tile (also off by default) turns the laptop's built-in keyboard off and on (margo `internal_keyboard`); it is highlighted while the keyboard is off, and margo refuses to turn it off while it is the only keyboard. The optional **Logo LED** tile (off by default; only on ThinkPads with `tpacpi::lid_logo_dot`) switches the red dot in the lid logo on or off: straight to sysfs when a udev rule made its `brightness` writable, otherwise through the `mshell-privileged led-set` polkit action (thinkpad_acpi LEDs only). **Logo LED automation** (Settings → Widgets → Control Center, `control_center.logo_led`, all off by default) turns the LED off while the screen is locked (`off_while_locked`) or during Do Not Disturb (`off_while_dnd`) and restores it afterwards (unless you switched it on meanwhile), and blinks it when a notification arrives (`blink_on_notification`, not during DND or while locked). It writes the LED directly, so it only runs when `brightness` is writable by you — never one password prompt per event; Settings shows the rule when it isn't: `ACTION=="add", SUBSYSTEM=="leds", KERNEL=="tpacpi::lid_logo_dot", RUN+="/bin/chgrp video /sys/class/leds/%k/brightness", RUN+="/bin/chmod g+w /sys/class/leds/%k/brightness"` in `/etc/udev/rules.d/90-thinkpad-logo-led.rules` (you must be in the `video` group), then reboot. The optional **Custom Shortcut** tile (off by default) is your own: label, icon and a command run on click (`sh -c`), set under Settings → Widgets → Control Center → Custom tile (`control_center.custom_shortcut_tile`); with a *state command* it becomes a toggle — exit status 0 lights the tile and its first output line is the subtitle (re-read when the Control Center opens and after each click). |

## System monitors

| Token | Widget | What it does |
| --- | --- | --- |
| `CpuDashboard` | CPU Dashboard | A system-monitor capsule: by default live CPU load + package temperature; Settings → Widgets → CPU Dashboard (`bars.widgets.cpu_dashboard`) adds Intel iGPU busy % (`show_gpu`, sampled only while on), RAM, swap, the usage of the filesystem holding `disk_path` (default `/`) and network down / up over the physical interfaces (`show_network`). Each metric tints on its own thresholds (accent when high, red when critical; `highlight`); a missing sensor shows "—", or is left out with `hide_unavailable`, never a fake 0. Hover shows a two-column table — used / total per metric, CPU frequency, load average, the top CPU and memory processes (`detailed_tooltip`; read only while the pointer is on the pill). Click opens the dashboard (per-core bars, RAM, load averages) plus Intel iGPU busy % / frequency per GT (xe / i915 sysfs) and the processes using the GPU (DRM fdinfo), and on ThinkPads the fan level / RPM with manual levels (Auto · 0–7 · Max) when `thinkpad_acpi fan_control=1` is set (otherwise a hint how to enable it), with a **Processes** tab: a `/proc` process table sortable by CPU / RAM / swap / disk IO, searchable by name, command or PID, optionally grouped under the app window each process belongs to, with a confirmed terminate / force-kill and a **Pause** / **Resume** button per row (SIGSTOP / SIGCONT; a paused process reads "Paused" and its button turns into Resume; the compositor, mshell and anything it runs under, the login / session processes, the D-Bus buses, polkit, PipeWire and Xwayland are never paused). Other users' processes go through the `mshell-privileged kill` polkit action, which re-checks the process start time — and, for a pause, the same protected list — as root. A **Ports** tab lists listening TCP/UDP sockets (`ss -tulpn`) grouped by exposure (all interfaces / specific address / this machine only), with search, copy-PID, a confirmed kill and, on TCP listeners, an **Open** button that launches `http://localhost:<port>` (the bound address when it is a specific one; `https` on 443) in the default browser. A **Drives** tab shows each NVMe drive's temperature (sysfs, no root) and, on request, full SMART health — verdict, wear, spare, error counters, data written, power-on hours — from `smartctl` behind one polkit prompt (`mshell-privileged smart-read`; needs `smartmontools`). Once SMART is loaded, a drive that supports it offers a **Short** / **Long** self-test (`mshell-privileged smart-selftest`, one prompt) with live progress and a notification with the result; with drive-health history on, each drive gets temperature (and wear) trend charts — see [Drive health](#drive-health). Right-click shows / hides RAM in the capsule for the session. |
| `AudioDashboard` | Audio Dashboard | Default output **and** input volume in one cluster (`🔊 42% · 🎙 5%`). Right-click cycles which side shows; click opens the mixer menu (sliders, mute, per-app streams, device + port pickers, and a Noise suppression switch — an RNNoise virtual mic via a private PipeWire filter-chain, needs `noise-suppression-for-voice`; the switch is remembered, survives a shell restart and comes back after a reboot). |
| `Shelf` | Shelf | A file shelf: drop files on the pill or into its menu (from a file manager, a browser download bar, …) to hold them, then drag them out into any app — a browser upload, a chat, a mail, a folder. The pill shows how many files are on the shelf. The menu (also `mshellctl menu shelf`) has *Copy all* (paste in a file manager copies the files) and *Clear* in its header, a drop zone with *Drag all*, and one row per file: click opens it, drag the row out, or use *Show in folder* (the file manager highlights it), *Copy path*, *Remove*. A file that is gone from disk stays listed, dimmed, until removed. The shelf keeps references only — it never copies, moves or deletes files, and a drop is accepted as copy/link, never as a move. Saved in `$XDG_STATE_HOME/margo/shelf.json` (a file that does not parse is kept with a `.bak` copy and not overwritten). |
| `DiscordVoice` | Discord Voice | Your current Discord voice call via Discord's local RPC socket: call glyph tinted while in a channel, channel name + how many are speaking. Click opens the call panel (one-time Authorize, the channel + server with a favourite star and the voice latency — ≤ 60 ms primary, ≤ 120 ms warning, else error —, Mute / Deafen / Leave, your microphone input volume 0–100 %, participants with speaking dot and per-user volume 0–200 %). Out of a call, *Saved channels* lists up to 5 favourite and 5 recent voice channels (`$XDG_STATE_HOME/margo/discord-channels.json`, mode 0600): *Join* rejoins one, or — while Discord isn't running — *Open* starts Discord on it; the panel also offers *Launch Discord*. Settings → Widgets → Discord Voice: *Launch Discord on click* (a pill click starts Discord while it isn't running), *Hide when not in a call*, *Discord program* (blank = `discord`, `discord-canary`, `discord-ptb`). Needs the Discord desktop app; also `mshellctl menu discord-voice`. |
| `AudioVisualizer` | Audio Visualizer | Live audio spectrum — a strip of bars driven by the `cava` CLI. Pulses with whatever is playing; flat strip on silence. Needs `cava` installed. |
| `Catwalk` | Catwalk | A CPU-reactive animated cat (noctalia / RunCat sprites). Idles below a CPU threshold, walks faster as load climbs. Click opens the CPU dashboard. |
| `Weather` | Weather | Current condition icon + temperature for the configured location. Click opens the Current / Hourly / Daily weather menu. Location is set in Settings → General. |
| `SystemUpdate` | System Updates | Count of pending updates across official repo / AUR / Flatpak. Click opens the package list; right-click re-probes. The panel shows the estimated download size (cached packages excluded) and flags Arch news posts published since the last full upgrade and asks you to read them before **Update** runs (`check_news`). Each package row can be hidden from the count (`hidden_packages`; it still upgrades); a **Hidden** group lists every configured hidden package (including “no pending update” rows), with Unhide also available in Settings → System Updates, plus pacman.conf's `IgnorePkg` (read-only). With `arch-audit` installed (package `arch-audit`; `check_audit`), the pill carries a severity-tinted advisory badge and a **Security** tab lists advisories by severity with links to security.archlinux.org. A **History** tab lists past upgrades (pacman.log, or the package DB when the log is root-only) with per-package rollback from the pacman cache (`pacman -U` through the `mshell-privileged rollback` polkit action, click twice to confirm). A **Config files** tab lists `.pacnew` / `.pacsave` / `.pacorig` files under /etc with a diff view — a 3-way merge when the previous default is still in a cached package — and merge / replace / delete through one polkit prompt that names the action (the root helper `mshell-privileged` refuses symlinks and keeps a `.mshell-bak` copy). **Update** runs in a terminal (default) or, with `update_mode: background`, detached through `pkexec` (the `mshell-privileged upgrade` action, a fixed `pacman -Syu --noconfirm`) with a live progress bar + log tail in the panel and a notification with the result. Background mode upgrades the official repos and Flatpak only: AUR packages are never built unattended — the AUR group gets a **Review in terminal** button that runs `paru -Sua` / `yay -Sua` interactively, PKGBUILD diffs included. `upgrade_backend: topgrade` runs `topgrade` (your topgrade.toml) in the terminal instead. Sources + interval in Settings → System Updates. |
| `Podman` | Podman | Running containers / pods / images summary from `podman ps`. Click opens the Podman menu: containers (lifecycle, shell, logs; a published TCP port chip opens `http://localhost:<port>` in the browser, right-click copies the URL), images, pods, and a **Host** tab listing your own listening TCP services (`ss -tlpn`, one row per process, named after its systemd unit) with the same open / copy chips. |
| `Agents` | AI Agents | Claude Code plan usage: the window closest to its limit (5-hour, weekly, per-model weekly) as a percentage, tinted calm/warn/danger by where the current burn rate takes it. Reads the real limits from `api.anthropic.com/api/oauth/usage` with Claude Code's saved sign-in (read-only, never refreshed or logged), and Codex limits when the Codex CLI is signed in. Click opens the AI Agents menu: per-window bars, forecast, reset time and per-hour budget, plus today's prompts / tokens / tokens per model from the local transcripts. One lamp per running Claude Code session sits beside the percentage (red = needs you, accent = working, dim = idle); the menu's **Sessions** tab lists them grouped Needs you / Working / Idle — click one to jump to its terminal (tmux pane selected, then the terminal window focused through margo's window pids) — and recent saved sessions to resume in a new terminal (`claude --resume`). Each running session shows what it is doing (the tool it runs, e.g. `Bash: npm test`, or its todo progress), its git branch, and chips for model, permission mode, context use (share of the model's window), estimated cost and **remote** (Remote Control on; a button opens it on claude.ai/code). Costs are estimates from LiteLLM's public price table (downloaded once a day, cached in `~/.cache/mshell/agents-prices.json`), in USD or EUR at the day's ECB rate (`currency`). While a session needs you the pill's icon turns into a red bell; `notify_waiting` / `notify_finished` add a desktop notification when one starts waiting for you or finishes a turn of 2 minutes or more (both off by default). **Several accounts**: sessions started with another `CLAUDE_CONFIG_DIR` are found from the running `claude` processes and shown with their account name, each account with its own plan-limit card; `claude_config_dirs` lists accounts to show before any session runs in them. The launcher's `cc <query>` searches running and recent sessions (Enter jumps to / resumes one). Unfold a running session (chevron) to read Claude's last reply without switching windows. The Sessions tab's search box searches every prompt you ever typed (all accounts' `history.jsonl`) and resumes the matching session; the header's **+** starts a new session in one of the recent projects (under that project's account). Below them, **All sessions** lists every saved session on disk (every account's `projects/` transcripts), grouped by project folder (click a folder to unfold it; 20 sessions at a time, **Show more** for the rest), each with its date and time, age and git branch; click one to resume it. A glyph marks a session that can't resume as it is, with the reason in its tooltip: **moved** (the folder was renamed or moved — found from the project folder's name, and the session resumes from the new folder), **orphan** (no folder matches; the tooltip says when its project folder still has memory files), **misplaced** (the transcript is in a project folder that matches no folder, but its recorded working directory exists) and **unknown** (the scan couldn't check). Recent and matching rows above show the same glyph, and orphan / misplaced / unknown ones don't resume. The chevron on a row unfolds **Copy command** (`cd <dir> && claude --resume <id>` to the clipboard), **Rename** (writes the same `custom-title` record Claude Code's `/rename` appends, plus its `custom-title.json`; not for a running or orphan session), **Lock** (protects it from deletion; kept in `~/.local/state/mshell/agents-session-locks.json`, never in `~/.claude`), **Repair** (a misplaced session: moves the transcript into the project folder of its working directory, after a confirm; never creates the working directory, never overwrites) and **Delete** (after a confirm: the transcript, its `<id>/` folder, and its `session-env` / `file-history` folders when no other transcript has that id; never a running or locked session). **Date** / **Name** / **Count** sort the folders and their sessions (the active one again reverses); **Moved**, **Orphaned** and **Empty** filter, with counts; the search box filters this list too. **Delete empty** removes every session nothing was ever sent in (running and locked ones kept), after a confirm; **Leftovers…** lists what deleted sessions left behind for a day or more — `session-env` / `file-history` folders with no transcript, project folders with no session — and removes them after a confirm. Every path is checked to stay inside the account's config dir; symlinks are never followed out of it. The Usage tab adds the last 30 days by project (Claude Code worktrees fold into their repo) and by hour of day, from every transcript — the first build reads them all once in the background, then only appended lines, cached in `~/.cache/mshell/agents-usage.json` (totals of deleted transcripts are kept). Above them, tokens and estimated cost for today, this week, this month and all time, all-time sessions/messages, and a Monday–Sunday chart of this week. `pill_pace` adds the pace to the pill (`64% ↑12` = twelve points over the elapsed share of the window, `↓` under; off by default). A **Memory** tab lists every CLAUDE.md (each account's global one and those of projects with sessions); click opens it in your editor. A **Plugins** tab lists the installed Claude Code plugins by marketplace (version, scope, skill count, an **off** chip when disabled, **update x.y** when the marketplace clone offers a newer version; click opens the plugin's page) and the bare skill folders in `~/.claude/skills` (click opens the SKILL.md) — read-only, from the config files; installing, updating and removing stay with `/plugin` in Claude Code. The launcher's `ai <question>` asks Claude Code (`claude -p`, locked down: `--safe-mode`, no MCP, no permission prompts, no tools — or only WebSearch/WebFetch with `ask_web` —, question on stdin, `ask_timeout_s`, model `ask_model`): Enter on **Ask Claude** streams the answer into the row and the preview pane, and Enter on the finished answer continues the same session (`claude --resume`, with approvals required after a web lookup) in a new terminal or, with `ask_target: tmux`, a new window of the most recently used tmux session. Settings → Widgets → AI Agents. Right-click refreshes the limits. Replaces the `model-usage` plugin. |
| `Github` | GitHub | Unread GitHub notifications: the GitHub mark with the unread dot and count, via the `gh` CLI and its saved login (`github-cli` package; a missing or logged-out `gh` is reported in the tooltip/menu). Polled every 2 min while shown. Click opens the GitHub menu — **Inbox** (unread threads; click opens one in the browser and marks it read, ✓ marks read, header ✓ marks all read) **PRs** (open pull requests grouped by repository, with CI and review chips; Settings → Widgets → GitHub → *Search rules* runs one GitHub search per line — default `author:@me`, add e.g. `review-requested:@me` or `org:acme label:bug` — merged and de-duplicated, up to 8 rules; *Excluded repositories* drops `owner/name` or bare-name globs; a failed or truncated rule is noted under the list) and **Activity** (the last year's contribution heat-map with today / current streak / best streak / year total, and *Open profile*; fetched with `gh api graphql` every 30 min, only while shown). Settings → Widgets → GitHub → *Pill shows* switches the pill's count from unread notifications to open pull requests (the PR set above), today's contributions, the current streak or the year total; *Hide when zero* hides the pill while its count is 0. Right-click refreshes. |
| `SshSessions` | SSH Sessions | Live count of active `ssh` clients. Click opens a searchable host list from `~/.ssh/config` and the files it `Include`s, read like the launcher's `ssh` provider reads it (click a host to connect in a new terminal), plus a **Forwards** tab of managed `ssh -L` port forwards; right-click re-polls. |
| `Usb` | USB Drives | Removable drives over udisks2 (D-Bus, no `udisksctl` parsing). Hidden while nothing is plugged in (or udisks2 isn't running); the icon tints while a volume is mounted, the count shows with 2+ drives. Click opens the drive panel (`mshellctl menu usb`): per volume Open (mounting first) / Unmount, LUKS Unlock (inline passphrase) / Lock; per drive Remove safely (unmount → lock → power off / eject). Right-click removes every drive safely. Toasts on plug / unplug (`toasts.removable_drives`, default on); unplugging a mounted drive always warns. |
| `Power` | Power Profile | Active power profile (performance / balanced / power-saver) + battery / power source, from power-profiles-daemon + UPower. Click opens the profile switcher panel, which also draws a battery-level history graph from UPower's own record (`GetHistory`; 6h / 24h / 7d chips, persisted as `power.history_hours`, default 6) and a USB-C POWER section per occupied port (charging / powering a device, live W when the firmware reports voltage, else the negotiated A, charger max from PD PDOs, PD flag; read from `/sys/class/typec` + `power_supply`, hidden when nothing is plugged) and a CHARGE LIMIT section (presets + custom; the battery's `charge_control_end_threshold` through the `mshell-privileged charge-limit` polkit action, hidden when the platform has none). |

## Network & connectivity

| Token | Widget | What it does |
| --- | --- | --- |
| `Network` | Network Console | Wi-Fi / wired link state + live throughput, from NetworkManager (no `nmcli` polling). Click opens the network menu (Wi-Fi list, connect/disconnect, rescan, radio toggle, an on-demand speed test — Ookla `speedtest` or `speedtest-cli` — with live gauge, ping/jitter/loss and server/ISP, Wi-Fi hotspot with its connected clients — `network.hotspot_*`, needs `nmcli` + `iw`; a QR button on the hero and on the hotspot row shows a code a phone joins from — see [QR codes](#qr-codes)); right-click flips between speed and icon display. |
| `Vpn` | VPN (Mullvad) | Native Mullvad pill driving the `mvpn` binary. Shield tints when the tunnel is up (relay + location in the tooltip). Click opens the VPN menu — an error header when Mullvad needs a fix (service stopped, kernel updated without a reboot, lockdown blocking, logged out, expired; each with its button), live tunnel rx/tx graph (wg0-mullvad), connect / random / fastest / my country, a searchable **Locations** list (each country opens to its cities), lockdown, auto-connect, quantum, multihop with an entry country / city, an Account section (devices with Revoke, voucher redeem, log in / out), plus a collapsible DNS section. |
| `VpnIndicator` | VPN Indicator | Lights up while a *generic* VPN tunnel is up — OpenVPN (`tun*`) or WireGuard (`wg*`: wg-quick, NetworkManager). Reads `/sys/class/net` (polled); the whole pill hides while disconnected, and the active interface name shows in the tooltip. Mullvad (`wg0-mullvad`) is excluded — the dedicated `Vpn` pill covers it. No menu. |
| `Dns` | DNS | Standalone DNS / Blocky panel (separate from the combined VPN pill). Polls DNS/VPN/Blocky state; click opens the DNS menu. |
| `HostMonitor` | Host Monitor | Pings your own list of hosts (a router, a home server, a VPN endpoint) on an interval: the server glyph is green while all answer and red with a count of the down hosts when any does not; a notification when a host goes down and when it is back. Click opens the Host Monitor menu ([details](#host-monitor)); right-click checks now. Needs `ping` (iputils). |
| `Ip` | Public IP | Public IP from ipinfo.io (polled). Click opens a detail panel (city, ASN, …). |
| `Bluetooth` | Bluetooth | Adapter state icon; tints when a paired device is connected (a "hooked up to my headphones" cue). Opens the Bluetooth panel. |
| `Valent` | Valent Connect | The main paired phone's connection state + battery level, from [Valent](https://valent.andyholmes.ca/) (KDE Connect protocol). Left-click opens the Valent Connect menu (phone status, actions, [screen mirroring](#phone-screen-mirroring-valent-menu)); right-click refreshes device discovery. |

## Media

| Token | Widget | What it does |
| --- | --- | --- |
| `MediaPlayer` | Media Player | Mirrors whichever MPRIS player is *currently playing* (Spotify, mpd, browsers, mpv, …) with transport controls. |
| `Mtune` | Tune | Dedicated pill for the built-in [`mtune`](companion-tools.md#mtune) music player: cover, title, queue position, and elapsed / total. Left-click opens the Tune menu (seek bar, transport, shuffle / repeat, speed, folder + playlist controls); the menu also shows the live queue with a filter box, and can play or remove any entry directly; right-click toggles play / pause; when mtune isn't running, a click launches it. Opt-in **scroll to skip** (Settings → Widgets → Tune → Behaviour, `scroll_skips`): scroll up = next, down = previous, one per gesture — with Tune's radio stations queued, it flips through stations. |
| `Lyrics` | Lyrics | Current synced lyric line of the now-playing track, scrolling in the bar; word-synced lyrics (enhanced LRC, NetEase YRC) fill word by word. Prefers the track's own embedded lyrics (if the file has any — mtune only; embedded LRC stays synced) over a lrclib.net lookup, with an opt-in NetEase fallback when lrclib has nothing synced. Click opens the full scrolling lyrics panel (disk-cached; a network failure retries with back-off) with a player footer: seek bar, prev / play / next, and a per-track timing stepper (`−` / `+` 0.25 s, click the value to reset) for an LRC that is off for one recording; click a synced line to seek there. Settings → Widgets → Lyrics: bar line on/off + width, **timing offset** (ms, + = earlier), **NetEase fallback**, **show translation** (a source's translation / romanization under each line), **local lyrics folder** (`Artist - Title.lrc`, `Artist/Title.lrc`, `Title.lrc`; checked first), and **only these / never these players** (which MPRIS players drive the lyrics). |

## Windows, tags & layout

| Token | Widget | What it does |
| --- | --- | --- |
| `ActiveWindow` | Active Window | Title of the globally focused window next to an app glyph; long titles marquee. An `X11` badge marks an XWayland window. |
| `MargoTags` | Margo Tags | 1–9 tag pills with focus / occupied / urgent states (urgent: a visible window asks for attention, filled `--error` capsule). Click to switch tags, scroll to cycle. Options under `bars.widgets.margo_tags` (Settings → Widgets → Margo Tags; every default keeps the plain digit row): `display: All \| ActiveOccupied \| OccupiedPlusNext` (every tag / only focused, occupied and urgent tags / those plus the first empty tag after the last occupied one); `glyph_focused` / `glyph_urgent` / `glyph_occupied` / `glyph_empty` (text or emoji per state, e.g. 🌕 🌟 🌗 🌙; empty = the number); `accent_focused` / `accent_urgent` / `accent_occupied: Primary \| Secondary \| Tertiary \| Error` (capsule fill of a focused / urgent tag, digit tint of an occupied one; defaults Primary / Error / Primary); `minimap: true` draws each occupied tag as a tiny proportional map of its windows (focused `--primary`, urgent `--error`, floating `--tertiary`, an outline of the screen when a scroller strip runs past it). With `window_icons: true` the active tag's windows follow as clickable app icons (click focuses the window). |
| `ScrollerRibbon` | Scroller Ribbon | One segment per column of the focused output's scroller strip — lit inside the viewport, accented for the focused column, dim when scrolled off — with `‹N` / `N›` counts of the columns hidden on each side. Click a segment to focus that column, scroll to move focus. Hidden outside the scroller layout. |
| `Scratchpads` | Scratchpads | One chip per scratchpad window (named or anonymous), labelled with its app; the filled accent capsule while it is on screen. Click toggles it (`togglescratchpadid`). Then one dim chip per minimized window, newest first: click restores it (`restore_window`); with two or more a restore-all button follows (`restore_all`). Hidden when there are no scratchpads and nothing is minimized. |
| `MargoLayoutSwitcher` | Margo Layout Switcher | Trigger button that opens the layout menu to pick one of the 11 layouts for the active tag. |
| `MargoDock` | Margo Dock | Per-app dock / taskbar — running + pinned apps as buttons; click focuses or launches. See [Dock (mdock)](mdock.md). |

## Indicators

| Token | Widget | What it does |
| --- | --- | --- |
| `LockKeys` | Lock Keys | Caps / Num / Scroll lock capsules (A / N / S) that light when engaged; the whole pill hides when all three are off. Read from `/sys/class/leds`. |
| `KeyboardLayout` | Keyboard Layout | Active xkb layout (e.g. US, TR). Click cycles to the next configured layout (`cyclekblayout`). |
| `Keymode` | Keymode | margo's active keymode (`setkeymode`, e.g. `resize`). Hidden in the default keymode; click returns to it. Settings → Toasts → Keymode (`toasts.keymode`, off by default) adds a toast on every change. |
| `Privacy` | Privacy | Lights up while the mic, camera, or screen is in use; click shows a per-sensor access log. |
| `RecordingIndicator` | Recording Indicator | Lights up while a screen recording is in progress; click stops the recording. |
| `Tray` | System Tray | StatusNotifierItem tray icons for apps that publish one. |
| `Notifications` | Notifications | Bell pill with unread count; click opens the notification center (history + Do Not Disturb). |

## Toggles & tools

| Token | Widget | What it does |
| --- | --- | --- |
| `DarkMode` | Dark Mode Toggle | One-click flip between Light and Dark matugen modes; the icon previews the mode you'd switch *to*. |
| `Twilight` | Twilight | margo's built-in blue-light filter. Click opens the panel (toggle + temperature + mode + schedule); right-click flips it on/off; the icon shows the live colour temperature while filtering. |
| `KeepAwake` | Keep Awake | Timed idle inhibitor. Left-click opens the duration grid + countdown; right-click ends a running session. Shows a live countdown while active. The menu's "Keep running with the lid closed" switch (`idle.inhibit_lid_switch`, off by default; also in Settings → Idle) holds a logind `handle-lid-switch` inhibitor while Keep Awake is on, so closing the lid on an external monitor doesn't suspend (the media hold never engages it). |
| `Keyboard` | On-Screen Keyboard | Toggles `mkeys`, margo's GTK on-screen keyboard. |
| `ColorPicker` | ColorPicker | Click picks a colour from the screen (`mpicker`) and copies its hex to the clipboard. Right-click opens the Color Picker menu (`mshellctl menu color-picker`): the selected colour as a swatch with editable HEX / RGB / HSL fields and an alpha slider that update each other (Enter applies a field, the copy button copies that form), and **Recent** — the last 12 colours picked anywhere in the shell, alpha included (`~/.cache/mshell/color_history.json`); click one to select and copy it again. The header's eyedropper picks a new one; the bin clears the list. Position / width in Settings → Widgets → Color Picker. |
| `Screenshot` | Screenshot | Takes a screenshot (region / window / output → file + clipboard). |
| `Wallpaper` | Wallpaper | Opens the wallpaper picker. Each mouse button is configurable in Settings → Theme → Wallpaper → Bar pill (`bars.widgets.wallpaper.left_click` / `middle_click` / `right_click`: `OpenPicker`, `Next`, `Previous`, `Random` or `None`; default: left opens the picker, the others do nothing), and `scroll: true` makes scrolling up / down over the pill switch to the next / previous wallpaper (off by default). The tooltip names the current wallpaper. When the wallpaper is today's daily image (Settings → Theme → Wallpaper → Daily Wallpaper), the picker shows its caption — Bing's headline, description and photo credit, or NASA's caption — with a **Learn more** button that opens Bing's page about the place or NASA's image article. In the picker, ↓ moves from the search into the thumbnails, the arrows browse, Enter applies, Space applies a random wallpaper, and Enter in the search applies the first match. While a theme bundle is active (Settings → Theme → Bundles, `theme.bundles` / `theme.active_bundle`), Next / Previous / Random, scrolling and the automatic rotation step through that bundle's wallpapers only. |
| `Clipboard` | Clipboard | Clipboard history (wl-clipboard); click opens the searchable history menu. |
| `Notes` | Notes Hub | Scratchpad + notes + todos with counts; click opens the Notes menu. Note cards take a colour (palette button — theme roles, so they re-tint with the wallpaper), pin to the top (star), carry a checklist under the body (with a done/total count), and are archived instead of deleted (the **Archive** toggle lists them read-only with Restore / Delete for good). Todos live on named pages (picker, new / rename / delete — a deleted page's todos move to **General**), each with optional details (chevron), drag-to-reorder by the grip (or Alt+↑/↓ in the text), a show/hide-completed toggle, and copy (Markdown / plain text) or save-to-`~/Documents` export. The **Files** tab works on a folder of Markdown notes (Settings → Widgets → Notes Hub → *Notes folder*; blank = off): search titles and text, capture a todo line to `Inbox.md`, create and edit notes inline (autosaved), open one in the default editor, and — through the AI provider set in Settings → AI — **Summarize** or **Rewrite** the open note (reviewed before it replaces the text) and **Ask your notes** (the best-matching notes are sent as context and cited). Optional *Git history* commits the folder after edits (never pushes); *AI capture* tidies captures into one todo line. |
| `AlarmClock` | Alarm Clock | Alarm-bell pill that opens the Alarm Clock menu (alarms, timers, stopwatch, countdowns, Pomodoro). Shows a running stopwatch inline and pulses while a tone rings. **Timers**: one-shot timers and reminders typed as one line — `10m tea`, `1h 30m`, `45s`, `14:30 meeting`, `11am standup`, `tomorrow 09:00 - coffee`, `in 10m tea`, `after 5m - stretch`, `25 Pomodoro` (a bare number is minutes), and Turkish `10dk`, `30 sn`, `1 saat 30 dakika`, `yarım saat`, `10 dk sonra çay`, `yarın 09:00 kahve`, `saat 15:30 toplantı`. A clock time already past today rings tomorrow (`today 09:00` after nine is an error instead); `at 3` is the next 3 o'clock, morning or afternoon; the label is the rest of the line or what follows ` - `. Set them from the menu's **Timers** tab (quick-entry field with a live “Rings at 14:35 (in 10 min) — tea” preview, `+5m` `+10m` `+15m` `+25m` `+1h` presets — text in the field becomes the preset's label —, countdown rows with progress and cancel), the launcher (`timer 10m tea`, or `t 10m tea`; bare `timer` lists them and Enter cancels one) or `mshellctl timer add "10m tea"` / `list` / `cancel <id>` / `clear`. A timer rings with the looping timer chime and the Alarm Clock's Stop / Snooze notification (the label is its title); Snooze brings it back after `alarm.snooze_minutes`. Timers are saved to `$XDG_STATE_HOME/margo/timers.json` (not the config) and survive a shell restart; one that came due while the machine slept or the shell was off fires once as “missed at HH:MM” (one chime, no loop). While a timer is pending the pill shows the soonest one's time left (`09:59`, `1h 05m`; a running stopwatch keeps precedence) and a click opens the Timers tab — `alarm.show_timer_on_pill`, on by default (with no timer pending the pill is unchanged). Ringing options live in Settings → Widgets → Alarm Clock → Timers & ringing. |
| `Countdown` | Countdown | Shows the soonest enabled Alarm Clock countdown (hourglass glyph + remaining time). Click opens the Alarm Clock menu on its Countdown tab. Hidden when no enabled, parseable target remains. |
| `Pomodoro` | Pomodoro | The Pomodoro timer: phase glyph + time left (`24:13`; minutes on a vertical bar), primary-tinted while a focus session runs, green on a break. Click opens the Alarm Clock menu on its Pomodoro tab (Start/Pause · Skip · Reset, progress, sessions done); right-click starts / pauses. Cycle rules (`pomodoro.work_minutes` 25, `short_break_minutes` 5, `long_break_minutes` 15, `sessions_before_long_break` 4, `auto_start_breaks` / `auto_start_work` off, `notifications` / `sound` on) live in Settings → Widgets → Alarm Clock. A phase end pops a notification and plays the timer chime. Scriptable: `mshellctl pomodoro toggle\|start\|pause\|skip\|reset\|reset-all\|status`. Only one timer wakes per phase; nothing ticks while it's paused. |
| `EyeCare` | Eye Care | The 20-20-20 break reminder: every 20 minutes of screen time, look about 6 m (20 ft) away for 20 seconds. Shows the minutes until the next break, a seconds countdown (primary tint) during one, a dim eye while you're away. Only time at the keyboard counts: `eyecare.away_seconds` (120) without input is a natural break and resets the count; a playing video keeps you present (it holds the idle inhibitor). Click takes a break now / ends a running one; right-click restarts the count. Off by default (`eyecare.enabled`; the pill's click, Settings → Widgets → Eye Care, or `mshellctl eyecare on` turns it on). A break start / end pops a notification and chimes (no chime under Do Not Disturb). `mshellctl eyecare break\|skip\|reset\|on\|off\|toggle\|status`. |
| `Ai` | AI | Opens the native streaming-chat menu. Provider / model / key live in Settings → AI. |
| `Translate` | Translate | Opens the Translate menu: translate the selection or clipboard (Google Translate by default, optional DeepL). For a keybind without the menu use `mshellctl translate selection`. |
| `Keybinds` | Keyboard Shortcuts | Opens a searchable cheatsheet of every shortcut, parsed live from `config.conf`. |
| `Ufw` | UFW Firewall | The UFW firewall's state, read without root or a prompt (`/etc/ufw/ufw.conf`, the rule files `/etc/ufw/user.rules` / `user6.rules`, `systemctl show ufw.service`; re-read every 15 s while shown and right after every change): a green shield when ufw is on and its service runs and starts at boot, red when it is off, **amber** when it is on but its service failed / isn't running (rules may not be enforced) or doesn't start at boot (not enforced after a reboot) — the tooltip says which and names the fix —, muted when ufw is missing or unreadable. `bars.widgets.ufw.show_count` (Settings → Widgets → UFW Firewall → *Show the rule count*, off by default) adds the number of incoming allow / limit rules next to the shield while the firewall is on. Click opens the firewall menu: an on/off switch (turning off asks first; on also starts `ufw.service` when it isn't running), a banner with one-click fixes (**Start ufw**, **Start at boot**, and **Load rules** where the rule files are root-only), a *Start at boot* switch (`systemctl enable/disable ufw.service`), the default policies and logging level, and the rule list (target, action, source, interface, comment); each rule ufw can delete exactly from here has a remove button behind an inline confirm (route, app-profile and source-port rules are left to `sudo ufw delete`). **+** opens the add form: Allow / Deny / Reject / Limit, a port, range (`8000:8100`) or list (`80,443`), TCP / UDP / Both (Both over a range adds a TCP and a UDP rule), an optional source address or network and comment, validated live (the field at fault is named; a rule already listed is caught), plus presets for SSH, HTTP, HTTPS, KDE Connect / Valent (1714:1764), LocalSend (53317) and mDNS. Every change is one polkit prompt through the root helper's `ufw-*` actions (`mshell-privileged`: ufw / systemctl run with an argv it builds from validated fields, never a shell); failures show as toasts. Also `mshellctl menu ufw`, and from a terminal `mshellctl ufw status\|enable\|disable\|allow\|deny\|reject\|limit\|delete`. The Control Center's Firewall tile shows the same state; right-click turns ufw on / off. |

## Session

| Token | Widget | What it does |
| --- | --- | --- |
| `Lock` | Lock | Locks the screen (mlock). |
| `Logout` | Logout | Logs out of the session (confirms with a dialog). |
| `Reboot` | Reboot | Reboots the system (confirms with a dialog). |
| `Shutdown` | Shutdown | Powers off the system (confirms with a dialog). |
| `Setup` | Setup | Opens the Settings panel straight to the in-shell setup wizard. |

## Layout helpers & custom

| Token | Widget | What it does |
| --- | --- | --- |
| `!Spacer <px>` | Spacer | A blank gap of the given pixel width, to push widgets apart. |
| `Separator` | Separator | A thin vertical divider line between widgets. |
| `HiddenBar` | Hidden Bar | A collapsible "drawer" pill that renders this bar's `hidden_widgets` behind a trigger — hover (auto-expand) or click to reveal, right-click to pin, auto-collapse on leave. |
| `!HiddenBarNamed <name>` | Hidden Bar (named) | An independently-addressable drawer with its own widget list + behaviour (entry under `bars.widgets.hidden_bars`). Target it with `mshellctl hidden-bar <verb> <name>`. |
| `!Custom <name>` | Custom Widget | A user-defined pill: icon/image + optional label, left/right click commands, and an optional `exec` poller whose stdout fills the label via `{output}`. Defined under `bars.widgets.custom_widgets`. |

---

## Adding a custom widget

A `Custom` pill is the escape hatch for anything not covered above — a button
that runs a command, or a tiny status readout from a script:

```yaml
bars:
  widgets:
    custom_widgets:
      - name: weather_alert
        icon: "weather-storm-symbolic"
        exec: "~/.config/margo/scripts/storm.sh"   # stdout → {output}
        template: "{output}"
        interval: 300
        on_click: "notify-send 'Storm watch' \"$(cat /tmp/storm)\""
    top:
      end:
        - !Custom weather_alert
```

Right-click an `exec` pill that has no `on_click_right` to re-run its command
now; `mshellctl custom-refresh [name]` does the same from a script or keybind
(no name = every custom pill).

The `exec` runs only while the pill is shown, and once per poll however many
monitors show it: copies of a pill share one run (pills whose `exec`,
`interval`, `template`, `max_chars`, `json` or `art` differ never do).

### JSON mode

With `json: true` the `exec` prints one JSON object per poll and drives the
whole pill:

```json
{"visible": true, "glyph": "input-mouse-symbolic", "text": "26%",
 "text_color": "primary", "glyph_color": "error",
 "tips": [["Name", "TX Track"], ["Paired", "yes"]]}
```

| Key | Meaning |
| --- | --- |
| `visible` | `false` hides the pill (default `true`). |
| `text` | Label (goes through `template` / `max_chars`). |
| `glyph` | Icon name (falls back to `icon`). |
| `text_color`, `glyph_color` | A theme role: `primary`, `secondary`, `tertiary`, `error`, `muted`. Hex colours are ignored so the pill follows the theme. |
| `class` | Extra CSS class on the pill (e.g. `active`). |
| `tooltip`, `tips` | Tooltip line, then one bold-label row per `["label", "value"]`. |

Output that is not a JSON object keeps the pill visible with its `icon` and a
tooltip saying so.

## mdash dashboard

`mshellctl menu mdash` (or the `Mdash` pill) opens the dashboard at the top
centre. The default is the tabbed dashboard (`MenuWidget::Dashboard`):

- **Header** — "Good evening, Kenan", the date and time, a Settings button
  and ONE power button whose popover holds Lock, Suspend, Log out, Reboot and
  Shut down (the last three ask once more; the Session menu's commands run
  them).
- **Status strip** — small chips for battery, network (SSID / VPN), volume,
  weather, unread notifications and pending updates; each opens its menu and
  hides when it doesn't apply (no battery on a desktop, nothing unread).
- **Today** — two columns that end level: Calendar over Weather (now, the
  next hours, three days) on the left; the **Today** card, Now playing and
  Timers on the right. The Today card shows the selected day's events, then
  the next seven days' grouped under day headings, then the newest unread
  notifications (app icon, summary, time — a click opens the notification
  centre; *Mark all read* clears the count). It takes the height the left
  column leaves and shows whole rows only: what doesn't fit folds into a
  "+N more" line (events → the calendar menu, notifications → the
  notification centre). Empty states are one line ("No events this week",
  "No new notifications"); leftover room goes to more saved-station rows on
  the idle media card, then to the *Earlier* (already read) notifications.
  When nothing plays the media card offers to resume the last Tune track or
  start a saved station. Timers shows the next alarm, the running timers
  (cancel in place) and one row with the quick entry (`10m tea`,
  `14:30 call`) and +5m / +10m / +25m / +1h — the same store as the Alarm
  Clock menu and `mshellctl timer`.
- **Controls** — output volume, microphone and brightness sliders, then
  every quick setting as a labelled tile in four equal columns, flowing in
  full rows through Connectivity, Display, Focus, Audio, Services, Hardware
  and Custom (a section can start mid-row under its own caption).
  These are the Control Center's own tiles (same state, same probes; a
  toggle here and there always agree, and tiles the hardware lacks are
  hidden the same way). A click toggles; the chevron or a right click opens
  the related menu.
- **System** — CPU, memory, temperature and disk with small sparklines;
  battery (time left) with the power-profile switch; network throughput;
  pending updates with *Review & update*; Claude / Codex usage limits. Each
  card opens its detailed menu. These poll only while the tab is on screen.
- **Tools** — a grid of every menu worth launching (Screenshot, Color Picker,
  Clipboard, Notes, Translate, Keybinds, Monitor, Network, VPN, Podman, SSH,
  USB, Theme, …). Opening one closes mdash.

The panel is 960 px wide (`minimum_width`) so its text reads at body size
(14 px; card titles 16, the greeting 18; only timestamps and captions at
12 px, never bold); its height follows the tab on screen and eases on a
switch. A tab taller than the screen (or than the
menu's `maximum_height`) scrolls inside the tab area.

Keys: <kbd>Tab</kbd> / <kbd>Shift</kbd>+<kbd>Tab</kbd> cycle the tabs
(wrapping), as do <kbd>Ctrl</kbd>+<kbd>Tab</kbd> /
<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd>; <kbd>1</kbd>–<kbd>4</kbd> pick
a tab. The arrow keys move keyboard focus inside a tab. While a text field
has focus (the timer entry) <kbd>Tab</kbd> and the digits keep their normal
meaning; <kbd>Esc</kbd> leaves the field and Tab cycles tabs again
(<kbd>Ctrl</kbd>+<kbd>Tab</kbd> works from inside the field too). The last tab is
remembered (`$XDG_STATE_HOME/margo/mdash-tab`) and the next open starts there.
`mshellctl menu mdash today|controls|system|tools` opens on that tab (the same
tab again closes it). Tab order, visibility and the cards of each tab are set
under **Settings → Widgets → Mdash → Tabs** (YAML: see
[Configuration](configuration.md#shell-dashboard-mdash)).

## Crashes menu

`mshellctl menu crashes` (or `MenuWidget::Crashes` in any menu) opens the
crash history: core dumps of **your own** processes from systemd-coredump,
newest first (program, signal, PID, time, whether the core file still
exists), each with a bell button that mutes crash notifications for that
program (`crashes.muted`). The **Journal errors** tab lists this boot's
journal at `crashes.journal_priority` (`err` by default) and above — set
it under **Settings → Notifications → Journal errors shown** (an unknown
value falls back to `err` with a warning in the log).

Turn on **Settings → Notifications → Crash notifications**
(`crashes.notify`) for a critical notification when one of your programs
dumps core. The very first run only records the crashes already on disk, so
turning it on never floods you with old ones.
Needs `coredumpctl` / `journalctl` (systemd).

## QR codes

`mshellctl qr` opens the **QR Code** menu (`MenuWidget::Qr`, menu
`qr_menu`, moved / resized under **Settings → Widgets → QR Code**) and shows
a code to scan with a phone camera:

| Command | Code |
|---|---|
| `mshellctl qr "https://…"` / `echo … \| mshellctl qr -` | the text / URL (up to 1000 bytes) |
| `mshellctl qr --clipboard` | the clipboard's text |
| `mshellctl qr --wifi` | the Wi-Fi network this machine is on (`WIFI:T:WPA;S:…;P:…;;`) |
| `mshellctl qr --hotspot` | this machine's hotspot (the running one, else the `margo-hotspot` profile) |

`mshellctl menu qr` just toggles the menu. In the menu, the **Text** tab
takes typed or pasted text (Enter or *Show code*; *From clipboard*; *Copy
image* puts the code on the clipboard as a PNG), **Wi-Fi** and **Hotspot**
read the network from NetworkManager. The Network menu's QR buttons show the
same Wi-Fi / hotspot codes inline.

A Wi-Fi code **is** the network password. The shell reads it from
NetworkManager over D-Bus only when you ask, hands it to `qrencode` on
standard input (never a command line), never copies a Wi-Fi code to the
clipboard (clipboard history would keep it) and forgets it as soon as the
menu closes. Treat a screenshot of it as the password. Enterprise (802.1X)
networks cannot be shared this way. Needs `qrencode`; the clipboard source
needs `wl-paste`.

## Confirming IP changes (Settings → Network)

A static address or gateway typo on the connection you are using takes the
machine offline. Turn on **Settings → Network → (a connection) → Edit →
General → Confirm IP changes** (`network.ip_confirm_seconds`, 30 s; `0` =
off, the default) and an IP / gateway / DNS / route change to a connection
that is **in use** asks *Keep these network settings?* with a countdown.
**Keep** keeps them; **Revert now**, or no answer in time, restores the saved
profile and re-activates it. The rollback is a transient systemd user timer
(`margo-net-rollback-<uuid>`), armed before anything is written, so it still
runs if the shell restarts in the meantime. Changes to inactive profiles and
non-IP settings (autoconnect, metered, Wi-Fi password) apply directly.

## Host monitor

Add the `HostMonitor` pill to a bar, open it and use **+** to add a host:
an IP (`192.168.1.1`), a host name (`nas.lan`) or a pasted link — scheme,
user, port and path are dropped and only the host is pinged. A label is
optional. Each row shows the host's state (up, down with the reason — *No
reply*, *Unknown host*, *Network unreachable* — or *Checking…*), the
round-trip time (green under 100 ms, amber under 300 ms, red above) and a
remove button; **⟳** checks now. The list lives in
`network.host_monitor.hosts` (`host`, `label`).

One check is three pings 0.2 s apart, so a single lost packet is not an
outage; a host is down when none of them is answered. Checks run every
`network.host_monitor.interval_seconds` (30 s, 5–3600) with a
`timeout_seconds` reply wait (1 s, 1–10), all hosts at once, while the pill
or the menu is shown. A host that goes down notifies (`notify_down`, on —
also for one already down when the shell starts) and its recovery replaces
that notification (`notify_recovery`, on). These four are under **Settings
→ Widgets → Host Monitor → Checks**; `mshellctl menu host-monitor` toggles
the menu. Hosts that drop ICMP (some firewalls and cloud hosts) always read
as down.

## SSH port forwards (SSH Sessions menu)

The SSH Sessions menu's **Hosts · Forwards** switch opens named local port
forwards (`ssh -L`), saved in `network.ssh_forwards` (name, SSH host,
local port, remote host, remote port, *Open as https://*). **Add forward**
takes an SSH host (an alias from `~/.ssh/config` or `user@host`), the local
port and the host:port to reach as the SSH host sees it (`localhost` = the
SSH host itself). The switch starts or stops a forward; each row shows its
state (*Connected*, *Connecting…*, *Failed · retrying*) and has **Open**
(`http(s)://localhost:<port>` in the browser), **Copy** (the URL),
**Logs** (its journal, followed in a terminal) and **Remove** (stops it
first). A forward runs as the transient systemd user service
`margo-ssh-forward-<name>`: it survives a shell restart, reconnects after
a dropped link, binds `127.0.0.1` only and never starts on its own. It uses
`BatchMode=yes`, so the host needs key / agent login and a known host key
(connect once from the Hosts tab first); the shell never asks for or stores
a password. Needs `openssh` and systemd user services.

**AI diagnosis.** Every crash and journal row has an agent button
(`ai-agents-symbolic`) that opens your terminal (`$TERMINAL`, else the first
installed emulator) running Claude Code or Codex on that row. The prompt —
the recorded facts, `coredumpctl info` (first 16 KiB), an evidence-first
investigation guide (rule out OOM / disk / updates, read the journal around
the crash, symbolise with gdb + debuginfod, report proven vs inferred,
change nothing) — is written to a private file
(`$XDG_RUNTIME_DIR/mshell/crash-diagnose/*.md`, mode 0600, deleted after a
day); only its path reaches the agent's command line. Agents are tried in
`crashes.agent_order` (**Settings → Notifications → Crash diagnosis
agent**: Claude Code then Codex by default). With `crashes.agent_probe`
on (**Skip an agent that can't answer**, default) each one is first asked a
one-word question, and one that fails or reports a usage / rate limit or
quota is skipped for the next — the terminal says which and why.
`crashes.agent_instructions` (**Diagnosis instructions**) is appended to
every prompt, e.g. “Answer in Turkish”. The button is disabled when neither
`claude` nor `codex` is found on PATH or in `~/.local/bin`, the npm or bun
bin directories. Nothing runs an agent without a click.

## Drive health

Settings → Widgets → CPU Dashboard → **Drive health** (`drive_health.*`,
everything off by default):

- **Drive alerts** (`alerts`): a minute scan of sysfs (no root; SATA
  temperatures only with the `drivetemp` module) watches the internal drives — NVMe controllers and
  non-removable SATA disks not behind USB — and notifies when one runs hot
  (**Hot at** / **Critical at**, `temp_warning_c` 70 / `temp_critical_c`
  85, with 3 °C hysteresis) or, with **Missing-drive alerts**
  (`missing_alerts`), when a drive that was established (seen in three
  scans) has been absent for **Missing-drive grace** minutes
  (`missing_grace_scans`, 3). Full SMART needs root and is never polled:
  every full read the Drives tab makes (**Read SMART health…**, or a
  self-test's progress reads) is compared with the last one, and a new or
  worse issue — failed self-assessment, critical-warning flags, media
  errors, spare below threshold, bad sectors, wear at **Wear warning**
  (`wear_warning_pct`, 80; then every further 5 points) — raises a
  notification. **Notify when an issue clears** (`notify_recovery`) also
  announces recoveries. The thresholds also colour the Drives tab.
- **Trend history** (`history`): a temperature sample every
  `history_interval_minutes` (60, 15–1440) plus every full SMART read,
  kept for `history_retention_days` (30, 1–365) and at most 20 000
  samples per drive; the Drives tab charts temperature (dashed line at the
  hot threshold) and wear once it has moved.
- **Self-tests**: in the Drives tab, after a full SMART read, **Short** /
  **Long** start `smartctl -t` through `mshell-privileged smart-selftest`
  (its own polkit action). The helper reports a `smartctl -a` read every
  15 s, so one password prompt gives a progress bar and the final result
  (notified — a long test on a hard disk takes hours); closing the shell
  stops the watching, never the test. A test the drive is already running
  shows its progress without buttons.

State and history live in `$XDG_STATE_HOME/mshell/drive-health/`
(`alerts.json`, `history.json`; a file that doesn't parse is kept as
`.bak` and never overwritten). Not ported from the plugin: a root
systemd-timer collector (SMART is only as fresh as your last read),
per-drive aliases / thresholds and mounted-space per drive.

## Reboot into… (session menu)

Turn on **Settings → Session → Reboot into… → Show “Reboot into…”**
(`session.reboot_into`) and the session menu gets a **Reboot into…** row
under its buttons. It opens the machine's UEFI boot entries (from
`efibootmgr`), marks the one this boot started from (*this boot*) and a
pending one-shot entry (*next boot*, ticked). Click an entry to boot it
**once** — another installed system, the firmware updater — and reboot:
the polkit prompt names the action (`mshell-privileged boot-next`, which
re-reads the entries as root and only sets one that is listed); the boot
order is not changed. **Clear next boot** drops a pending entry without
rebooting. The reboot is the menu's own Reboot, so `session.reboot_command`
and margo's clean window shutdown still apply.

By default the list offers the entries that boot a loader from a disk;
**Every firmware entry** (`session.reboot_into_all`) adds the firmware's
device shortcuts (USB HDD, network boot, …). Entries whose label contains a
word from **Hide entries containing** (`session.reboot_into_hidden`; network
boot, shells, setup and diagnostics apps by default) are left out.

From a script or keybind: `mshellctl session boot-entries` lists the
entries, `mshellctl session reboot-into <0003|label>` reboots into one.
Needs `efibootmgr`.

## RSS / Atom feed notifier

Add feeds under **Settings → Notifications → RSS feeds** (`rss.feeds`: a
URL and an optional name). Each feed (RSS 2.0, RSS 1.0 / RDF or Atom) is
checked every `rss.interval_minutes` (30 by default, 5–1440) with a
conditional request (`If-None-Match` / `If-Modified-Since`), so an
unchanged feed costs one `304`. A new item pops a notification: the item's
title, the feed's name (or its own title), and an **Open** action (or a
click) that opens the item in the browser — `http(s)` links only. At most
`rss.max_per_poll` item notifications per check (5); more are summed up in
one. `rss.notify: false` keeps tracking without notifications.
Adding a feed only marks what it already lists as seen, so it never floods
you with its backlog. Seen items (up to 300 per feed) and the HTTP
validators are kept in `$XDG_STATE_HOME/margo/rss-seen.json`. With no feeds
the notifier does no network I/O at all.

## Phone screen mirroring (Valent menu)

The Valent menu ends with a **Screen mirroring** card: every adb device
(USB, or Wi-Fi after wireless debugging) gets a **Mirror** button that opens
a scrcpy window you can control with mouse and keyboard, and **Stop** to
close it. Pair a phone once with the IP:port and 6-digit code from its
*Developer options → Wireless debugging → Pair device with pairing code*,
then **Connect** to the IP:port shown there. Under **Settings → Widgets →
Valent Connect → Screen mirroring**, *Turn the phone screen off*
(`valent.scrcpy_screen_off`) keeps the phone's own screen dark and *Extra
scrcpy arguments* (`valent.scrcpy_args`) adds scrcpy flags (e.g.
`--max-size=1280 --stay-awake`). Mirror windows run in their own scope, so
they stay open across a shell restart, and the restarted shell still lists
them with **Stop**. Needs `scrcpy` and `android-tools`.

See [Configuration](configuration.md) for the full profile layout, and
[Scripting](scripting.md) for driving the shell and compositor from scripts.
