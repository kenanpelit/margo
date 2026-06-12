# mshell Road Map — surpass noctalia / DankMaterialShell

**Last updated:** 2026-06-12 — **the catch-and-surpass goal is met.**
Every Tier A item, all remaining Tier B items except B3, and most of
Tier D have shipped (statuses below). mshell now has 50+ bar pills,
the control center, a native AI assistant, alarm clock, privacy
indicator, dock, keybinds editor, GNOME-parity Settings pages and a
WASM plugin tier — this document is kept as the shipped-ledger plus
the short tail of still-open items (B3 process list, D1, D2, D4, D10,
Tier C).
**Source audit:** `~/.kod/noctalia-shell/` and `~/.kod/DankMaterialShell/` walked
end-to-end (Modules/, Services/, Widgets/) on 2026-05-15; noctalia v5
re-walked for the 2026-06 port wave.

> mshell already had a strong baseline at audit time (24 bar pills, full
> menu/notification/wallpaper rotation/lockscreen stack, margo socket IPC,
> in-process polkit + PAM). This document was the **catch-and-surpass**
> plan against the two QML shells most often compared with us.

---

## Baseline at audit time (2026-05-15) — superseded, kept for context

> Today's surface is far larger (50+ pills incl. AI, alarm clock, privacy,
> lock-keys, keyboard-layout, sysstat, visualizer, dock, VPN, Valent,
> ssh-sessions, control center…). This section records what existed when
> the catch-and-surpass audit was made.

**Bar pills (24):** workspaces (tag pills), layout, dock, audio in/out,
battery, bluetooth, clipboard, clock, media player, active window, network
speed, dns (ndns), ufw (nufw), podman (npodman), power profile (npower),
notification count, recording indicator, system tray, volume, wallpaper, IP
(nip), notes (nnotes).

**Menus:** app launcher, quick actions, screenshot, screenshare, screen
record, weather, theme picker, audio in/out, bluetooth, network, ndns,
nufw, npodman, npower, nnotes, settings, session menu, notifications,
wallpaper, clipboard.

**Services / infrastructure:** matugen theming, polkit prompts, PAM auth,
idle manager (`mshell-idle`), notification daemon, OSD (mshell-osd), gamma
(now driven by margo's built-in twilight), wallpaper rotation, mctl IPC,
lockscreen, sound pack, settings UI window, mshellctl.

**Compositor-side advantages over noctalia / Dank:**
- Tight margo integration via the Unix control socket (`MARGO_SOCKET`,
  `watch state` push stream — tag-native, not workspace-emulated).
- Built-in twilight in the compositor (`mctl twilight`); no second gamma writer.
- In-process polkit + PAM; no DMS-style Go backend daemon required.
- Pills the others lack: ndns, nufw, npodman, nip, nnotes.
- Pure Rust monolith — no Quickshell / QML runtime dependency.

---

## Headline structural change — settings panel

> **Owner-flagged top priority.**

Both noctalia and Dank treat **Settings** as just another menu surface, not
a separate decorated window: it opens anchored to the bar (or centred over
it), inside the same layer-shell window every other menu lives in. mshell
today launches Settings as a `gtk::Window` — a decorated toplevel that the
compositor treats as a normal app. The owner wants the embedded form.

**S1 — Embed Settings in the frame menu stack.** ✅ **Shipped.**

Settings now mounts into the frame's layer-shell menu stack alongside
wallpaper / notifications / app-launcher. The old `gtk::Window`
toplevel is gone. Active-monitor routing through the IPC layer
(`ShellInput::ToggleSettingsMenu(Option<String>)`) sends the toggle
to the right Frame; panel size scales to the monitor's geometry
(`height = monitor_h * 3/4`, 4:3 aspect). Sidebar fully reorganised:

- Top-level (alphabetical after General): Bar, Display, Fonts, Idle,
  Theme, Wallpaper, Widgets.
- Widgets sub-sidebar (36 entries): Layout · 13 menu pages · 20
  bar-pill pages · Notifications · Session.
- Display has its own sub-sidebar with Twilight.

Reference commits: `05fee31` (initial embed), `7882044` (monitor
routing + sizing), `05f89e6` (Widgets group), `ac3a71f` (Bar
top-level, Widgets restructure), `bc43d23` (fill all pills + menus).

---

## TIER A — high daily value, low/medium cost

Target: ship the whole tier as one batch.

| # | Item | Source | Cost | Effort breakdown |
|---|---|---|---|---|
| ~~**A1**~~ | ~~**Privacy indicator** — bar pill that lights when cam / mic / screencast is active~~ | Dank `PrivacyService` | low | **SHIPPED.** `privacy_bar_widget` lives in `bars/bar_widgets/privacy.rs`. Mic detection rides wayle audio's already-filtered `recording_streams` property; camera state is polled every 3 s via `fuser /dev/video*` (cheap; only spawns when devices exist). Pill auto-hides when nothing is active; tooltip lists which apps are recording. Screencast detection is deliberately deferred — mshell's own recordings already surface in `RecordingIndicator`, and a portable "any app is screencasting" probe needs PipeWire client integration |
| ~~**A2**~~ | ~~**CPU / RAM / GPU / temp monitor widgets**~~ | Dank `CpuMonitor`, `RamMonitor`, `CpuTemperature`, `GpuTemperature` | low | **SHIPPED.** `sysstat` combined pill + the full CPU dashboard menu (`cpu_dashboard` pill + menu) cover CPU/RAM/temp; semantic temp colours follow the dashboard design system |
| ~~**A3**~~ | ~~**Lock-key indicator** — Caps / Num / Scroll lock status pill~~ | Noctalia `LockKeys` | low | **SHIPPED.** `lock_keys` bar pill |
| ~~**A4**~~ | ~~**Keyboard layout pill + cycle**~~ | Noctalia `KeyboardLayout`, Dank `KeyboardLayoutName` | low | **SHIPPED** (noctalia-v5 wave). Compositor-side runtime xkb-group switching + socket IPC + `keyboard_layout` pill |
| ~~**A5**~~ | ~~**Calendar grid in clock menu**~~ | Dank `CalendarService` + overview card | mid | **SHIPPED.** `calendar_grid` menu widget; dashboard calendar is native `gtk::Calendar` |
| **A6** ✅ | **Dark-mode toggle pill** — flips `matugen.mode` light↔dark | Noctalia `DarkMode` | low | **Shipped** `2e9a33b`. Reactive — picks up external config writes too. |
| **A7** ✅ | **KeepAwake (idle inhibit) pill** — bar toggle for idle-inhibit | Noctalia `KeepAwake`, Dank `IdleInhibitor` | low | **Shipped** `2e9a33b`. Subscribed to `IdleInhibitor::global().watch()` so `mctl idle inhibit` toggles update the pill. |
| ~~**A8**~~ | ~~**Setup wizard** — first-launch onboarding modal~~ | Noctalia `SetupWizard` | mid | **SHIPPED.** `wizard` menu widget (in-shell layer-shell menu): profile pick, theme, keyboard, touchpad, Wi-Fi, wallpaper, bar; `mwizard` shim binary reopens it on demand; pairs with margo's first-run config bootstrap |
| ~~**A9**~~ | ~~**Screen-corners overlay** — rounded display corners drawn by mshell~~ | Noctalia `ScreenCorners` | low | **SHIPPED.** Settings → Screen: rounded-corner mask + radius knob (`screen_settings.rs`) |
| ~~**A10**~~ | ~~**OSD coverage for brightness / keyboard layout / network state**~~ | Noctalia OSD pattern | low | **Mostly already shipped.** Volume + brightness + mic-mute OSDs live in `mshell-osd` and fire on the relevant hardware-key events. A4 (keyboard layout) has since shipped; a wifi-connect/disconnect popup remains niche enough to leave out unless someone asks |

**Cumulative cost estimate:** 7–10 days of focused work, ~1500 LOC across
mshell-services / mshell-frame.

**Tier A done = mshell at feature parity with noctalia's status cluster.**

---

## Bonus shipped (not originally on the roadmap)

Work that came up between sessions and shipped alongside the roadmap
items. Captured here so the audit is honest.

| # | Item | Commits |
|---|---|---|
| **X1** | **Twilight: built-in preset-schedule mode** — `TwilightMode::Schedule` reads `~/.config/margo/twilight/{schedule.conf,presets/*.toml}`, interpolates between consecutive presets in mired space. Bootstrap seeds 6 starter presets on first run. Settings UI mode dropdown gains a "Schedule" entry. | `29303df`, `2a2ccce`, `324ba32` |
| **X2** | **Twilight: multi-GPU gamma routing fix** — `pending_gamma` was drained unconditionally on every `BackendData` render, so the second GPU silently lost its ramps. Filter the drain by this device's outputs and re-park the rest. | (margo-side) |
| **X3** | **Margo theme + default** — new "Margo" colour scheme (Dracula-style surface + kitty Catppuccin Mocha foreground `#CDD6F4`). Owns its own CLUT (`cluts/margo.bin`). `Themes::Default` now aliases to Margo so a fresh install lands on the brand look. | `798083b`, `f4698c0` |
| **X4** | **Bar font scale** — `--font-bar: 1em` token, pill labels go from ~11 px to ~13 px to match noctalia. | `8a2233a` (and earlier) |
| **X5** | **Bar min-height crash fix** — `update_config` was firing on every SpinButton arrow click, triggering a write storm that took mshell down. 350 ms debounce. | (mshell-settings) |
| **X6** | **Session menu keyboard nav** — Tab / Shift+Tab / Ctrl+N / Ctrl+P / Ctrl+J / Ctrl+K walk focus between the five power-menu buttons. Took four attempts (Bubble → Capture phase → ShortcutController → Capture ShortcutController) — `road_map.md` §B9 closed. | `c86828b` |
| **X7** | **mshell-settings reorganisation** — Bar moved back to top-level; Notifications and Session moved into Widgets; sub-sidebar now has 36 entries (Layout + 13 menu pages + 20 bar-pill pages + Notifications + Session). | `ac3a71f`, `bc43d23` |
| **X8** | **mlayout symlink-follow fix** — `gather_layouts` used `entry.file_type().is_file()` which returns `false` for symlinks. Users who keep their margo configs in a dotfiles repo and symlink them in had `mlayout list` return "no layouts" even though the file existed. Now `metadata()` traverses the link. | (mlayout) |

## TIER B — meaningful, mid-to-large effort

Each item below is a single-session goal. Don't batch.

| # | Item | Source | Notes |
|---|---|---|---|
| ~~**B4**~~ | ~~**Overview dashboard** — full-screen overview with cards~~ | Dank `DankDash/Overview` | **SHIPPED** as the combined dashboard hub (`mshellctl menu dashboard`: clock / weather / media / calendar / system / audio cards, per the dashboard design system) plus the control-center menu. The separate `mshelldash` experiment was built and then **removed** (2026-06-01, owner: poor design) — the menu hub is the surviving form |
| ~~**B6**~~ | ~~**System update indicator**~~ | Dank `SystemUpdateService` | **SHIPPED.** `system_update` bar pill polls every 30 min. Backend auto-detects yay / paru / pikaur (preferred — covers AUR) → pacman+checkupdates → dnf → apt. Auto-hides when zero updates available. Click spawns terminal (kitty → alacritty → foot → wezterm → konsole → gnome-terminal → xterm) running the matching upgrade command |
| ~~**B9**~~ | ~~**Window-rule editor in Settings**~~ | Dank `WindowRuleModal` | **SHIPPED.** Settings → Window Rules page (list + add/edit over margo's `windowrule` config, written through the shared `compositor_conf` patch + `mctl reload` pipeline). The same wave also shipped the **Keybinds editor** (owns `binds.conf`) and the compositor Appearance/Effects/Behaviour pages — beyond the original scope |
| ~~**B10**~~ | ~~**Output management** — display arrangement panel: position, resolution, scale, rotation under Display → Layout~~ | `mlayout` (in-tree) | **SHIPPED.** New `LayoutSettingsModel` in mshell-settings drives `mlayout list --json` / `set` / `init` / `suggest` / `new` as Settings backend. Lives under Display → Layout sub-sidebar alongside Twilight. Also fixed an `mlayout` parser bug that skipped symlinked layout files (see X8). Outstanding nice-to-haves: a visual preview rectangle per row (today we show "label (WxH) · …"); per-output scale / rotation knobs as a Capture form (today only slug name is editable, geometry is read from `wlr-randr`) |

_B3 (process list) and B7 (hooks) remain in Tier D — see below. B5 (audio visualizer) has since **shipped** (`audio_visualizer` pill, PipeWire FFT / cava-style)._

---

## TIER C — niche / opt-in

Track these but don't prioritise.

| # | Item | Notes |
|---|---|---|
| **C1** | Tmux / Zellij session manager (Dank `MuxService`) — terminal-heavy users only |
| **C2** | Tailscale integration (Dank `TailscaleService`) — VPN niche |
| **C3** | Printer management (Dank `CupsService`) — print queue UI |
| **C4** | Greeter (Dank `Greetd`) — login screen reusing lockscreen theme |
| **C5** | Color picker tool (Noctalia `NColorPicker`) — pick colour from screen |
| **C6** | Workspace rename modal + switch overlay (Dank) — name your tags |
| **C7** | Performance mode toggle (Noctalia `noctaliaPerformanceMode`) — disables animations |
| **C8** | File browser modal (Dank `FileBrowser`) — for picker contexts |
| **C9** | Theme browser UI with built-in palettes (Dank `ThemeBrowser`) — extend our Theme picker |
| **C10** | Sound pack selection UI (Dank `SoundsTab`) — choose click / notification sound bundles |

---

## TIER D — back-burner

Worth doing eventually, but de-prioritised explicitly by the owner.
Owner notes recorded inline so the rationale doesn't get lost.

| # | Item | Owner note |
|---|---|---|
| **D1** | **Desktop widgets** — clock / media / system stat / visualizer / weather as draggable overlays on the wallpaper layer | Not used much — defer until the bar+menu surface is fully built out |
| **D2** | **Wallpaper search (Wallhaven)** — search + download from inside the wallpaper menu | Defer — rotation + manual pick + the shipped **Daily Wallpaper** (Bing/NASA image-of-the-day) cover the daily case |
| ~~**D3**~~ | ~~**Plugin system**~~ | **SHIPPED — and bigger than planned:** WASM plugin tier (`mshell-plugin-host` capability/path sandbox + `mshell-plugin-ui` renderer + `mplugin-sdk`), `plugins.toml` with per-plugin keybind overrides synced to `binds.d/mshell-plugins.conf`. (The compositor separately kept its Rhai plugin dirs) |
| **D4** | **App-theme generator** — push matugen output to GTK / Qt / kitty / alacritty / wezterm / vscode | Defer — kitty already follows the shell scheme via include; the rest is nice-to-have |
| ~~**D5**~~ | ~~**A1 — Privacy indicator (cam/mic/screencast)**~~ | **SHIPPED** (full noctalia port): mic/cam/screencast monitor (pw-dump + /proc), access log, toasts, per-sensor toggles, pill + menu |
| ~~**D6**~~ | ~~**A4 — Keyboard layout pill + cycle**~~ | **SHIPPED** — margo grew runtime xkb-group switching + socket IPC; pill cycles on click |
| ~~**D7**~~ | ~~**A8 — Setup wizard (first-launch onboarding)**~~ | **SHIPPED** as an in-shell layer-shell menu + `mwizard` shim; pairs with margo's first-run config bootstrap |
| **D8** | **B3 — Process list modal (Ctrl+Shift+Esc task manager)** | Big widget — defer until system-monitoring needs surface. Still the only unshipped Tier B item |
| ~~**D9**~~ | ~~**B5 — Audio visualizer / spectrum bar**~~ | **SHIPPED** — `audio_visualizer` pill (PipeWire FFT) |
| **D10** | **B7 — Hooks system** (`~/.config/mshell/hooks/on_*.sh`) | Small but pure extensibility — defer until users start asking for it. The WASM plugin tier covers much of the original motivation |

## Dropped

- ~~**Notepad module**~~ — owner decision: not needed.

---

## Where mshell is ahead — preserve these

| Area | mshell | Noctalia | Dank |
|---|---|---|---|
| Compositor coupling | Unix control socket native (`watch state` push, tag-aware) | Generic protocols only | Generic + per-compositor service |
| Gamma / blue-light | Built into compositor (twilight) | External (sunsetr / gammastep) | External |
| Auth surface | In-process polkit + PAM | n/a | Separate Go backend daemon |
| Firewall pill | ✅ ufw | ❌ | ❌ |
| DNS-mode pill | ✅ dns (+ VPN-menu DNS section) | ❌ | ❌ |
| Container pill | ✅ podman | ❌ | ❌ |
| VPN | ✅ first-party mvpn (Mullvad) + pill + menu | ❌ | Tailscale only |
| AI assistant | ✅ native streaming chat (mshell-ai) | ❌ | ❌ |
| Plugins | ✅ sandboxed WASM tier + SDK | ❌ | ❌ |
| Runtime stack | Single Rust binary | Quickshell / QML | Quickshell + Go daemon |

---

## Recommended sequencing

Owner-curated short list. Items not on it are explicitly
deferred to Tier D — pull them back up when there's interest.

**Active queue: empty — everything on it shipped.**

1. ~~**A5 — Calendar grid in the clock menu.**~~ **SHIPPED.**
2. ~~**B10 — Output management under Display → Layout.**~~
   **SHIPPED.** `LayoutSettingsModel` drives `mlayout` as a Settings
   backend; lives under Display → Layout sub-sidebar.
3. ~~**B6 — System update indicator pill.**~~ **SHIPPED.**
4. ~~**B4 — Overview dashboard.**~~ **SHIPPED** as the dashboard
   menu hub + control center (the `mshelldash` experiment was
   removed 2026-06-01).
5. ~~**B9 — Window-rule editor in Settings.**~~ **SHIPPED**,
   alongside the Keybinds editor and the compositor
   Appearance/Effects/Behaviour pages.

**Still open** (owner: "do them eventually, not now"):
B3 (process list modal) · B7/D10 (hooks system) · D1 (desktop
widgets) · D2 (Wallhaven search) · D4 (app-theme generator) ·
Tier C. Everything else in Tier D shipped — see the table above.

**Shipped beyond this roadmap** (was never on it): control center ·
native AI assistant (pill + chat menu + Settings) · alarm clock ·
MRU window switcher (margo-side) · dock · SSH-sessions widget ·
Valent pill · VPN pill/menu + first-party `mvpn` · `mkeys` OSK ·
`mplay` mpv companion · `mpower` · GNOME-parity Settings
(Network/Bluetooth/Power/Default Apps/Privacy/Backup/Logging) ·
daily wallpaper (Bing/NASA) · first-login config bootstrap +
convention default keybinds.

---

## Out of scope

- File management (use Nautilus / Thunar / lf)
- Display login greeter management (LightDM / greetd setup is system-level)
- Window tiling / overview (margo's job; mshell renders, doesn't decide)
- Removable drive mounting (udisks)
- Screen mirroring / casting (compositor + dedicated tool)

Same boundary noctalia draws in its README — a shell is the visual layer
on top of the compositor, not a desktop environment.
