# mshell Road Map — surpass noctalia / DankMaterialShell

**Last updated:** 2026-05-15
**Source audit:** `~/.kod/noctalia-shell/` and `~/.kod/DankMaterialShell/` walked
end-to-end (Modules/, Services/, Widgets/) on 2026-05-15.

> mshell already has a strong baseline (24 bar pills, full menu/notification/
> wallpaper rotation/lockscreen stack, mctl-driven margo IPC, in-process
> polkit + PAM). This document is the **catch-and-surpass** plan against
> the two QML shells most often compared with us.

---

## Current state — what mshell already has

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
- Tight margo integration via dwl-ipc-v2 (tag-native, not workspace-emulated).
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
| **A1** | **Privacy indicator** — bar pill that lights when cam / mic / screencast is active | Dank `PrivacyService` | low | `xdg-desktop-portal-wayland` global-shortcut state OR PipeWire node inspection. New service in `mshell-services` + bar pill in `mshell-frame/src/bars/bar_widgets/privacy.rs` |
| **A2** | **CPU / RAM / GPU / temp monitor widgets** — four pills, optional combined "system" pill | Dank `CpuMonitor`, `RamMonitor`, `CpuTemperature`, `GpuTemperature` | low | `/proc/stat` (CPU), `/proc/meminfo` (RAM), hwmon (temp), nvidia-smi/`/sys/class/drm` for GPU. Each pill ~80 LOC + shared `SysStatService` |
| **A3** | **Lock-key indicator** — Caps / Num / Scroll lock status pill | Noctalia `LockKeys` | low | Read xkb state from Wayland virtual-keyboard or input-method. ~50 LOC |
| **A4** | **Keyboard layout pill + cycle** — current layout label, click cycles | Noctalia `KeyboardLayout`, Dank `KeyboardLayoutName` | low | mctl-side IPC for `setxkblayout`; or `dwl-ipc` keyboard-layout message. Pill ~80 LOC. Also tie into OSD on switch |
| **A5** | **Calendar grid in clock menu** — month view with day cells, today highlighted, prev/next month nav | Dank `CalendarService` + overview card | mid | Replace the current clock menu's simple body. Pure Rust date math via `chrono`. UI ~200 LOC. Future: event-source plugins |
| **A6** ✅ | **Dark-mode toggle pill** — flips `matugen.mode` light↔dark | Noctalia `DarkMode` | low | **Shipped** `2e9a33b`. Reactive — picks up external config writes too. |
| **A7** ✅ | **KeepAwake (idle inhibit) pill** — bar toggle for idle-inhibit | Noctalia `KeepAwake`, Dank `IdleInhibitor` | low | **Shipped** `2e9a33b`. Subscribed to `IdleInhibitor::global().watch()` so `mctl idle inhibit` toggles update the pill. |
| **A8** | **Setup wizard** — first-launch onboarding modal | Noctalia `SetupWizard` | mid | Wallpaper pick, font choice, locale, lat/lng (for twilight), accent color preview. Triggered when `~/.config/margo/mshell/.welcomed` is absent. ~400 LOC across one new menu widget |
| **A9** | **Screen-corners overlay** — rounded display corners drawn by mshell | Noctalia `ScreenCorners` | low | Layer-shell anchored Cairo draw, per-output. Config knob for radius. ~120 LOC |
| **A10** | **OSD coverage for brightness / keyboard layout / network state** — currently OSD only fires for volume | Noctalia OSD pattern | low | Wire existing `mshell-osd` to brightness change events, keyboard-layout-switch, wifi connect/disconnect. ~80 LOC per source |

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

## TIER B — meaningful, mid-to-large effort

Each item below is a single-session goal. Don't batch.

| # | Item | Source | Notes |
|---|---|---|---|
| **B4** | **Overview dashboard** — super-key full-screen overview with cards (clock / weather / media / calendar / system / user) | Dank `DankDash/Overview` | New `overview_menu_widget`. Re-uses existing components. Bound to Super or a margo dispatch action |
| **B6** | **System update indicator** | Dank `SystemUpdateService` | Pacman / dnf / apt count polled every 30 min. Bar pill shows count, click → terminal helper (configurable) |
| **B9** | **Window-rule editor in Settings** — GUI builder over margo's `windowrule` config, lives under a new Widgets sub-page or Bar→Window-rules tab | Dank `WindowRuleModal` | List existing rules + add/edit form (regex / class / title / actions). Writes through to `config.conf` via the same in-place line-edit pipeline already used for twilight |
| **B10** | **Output management** — display arrangement panel: position, resolution, scale, rotation under Display → Layout | `mlayout` (in-tree) | Wire the existing `mlayout` CLI as the Settings backend. `mlayout list / preview / set / new / next / prev / pick` are already implemented; the missing pieces are (a) seed-on-first-run so a virgin install actually has layout files (today `mlayout list` returns "no layouts" until the user runs `mlayout init` by hand), and (b) a GUI panel under Display → Layout that drives those commands. Big UX win — multi-monitor users get a graphical arrangement editor without any new compositor protocol work |

_B3 (process list) · B5 (audio visualizer) · B7 (hooks) moved to Tier D — see below._

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
| **D2** | **Wallpaper search (Wallhaven)** — search + download from inside the wallpaper menu | Defer — current rotation + manual-pick flow covers the daily case |
| **D3** | **Plugin system** — Lua / Rhai-loaded plugins for bar widgets, menu providers, launcher entries | Save for after the in-tree feature set settles — no point freezing a plugin API while we're still adding bar pills weekly |
| **D4** | **App-theme generator** — push matugen output to GTK / Qt / kitty / alacritty / wezterm / vscode | Defer — kitty already follows the shell scheme via include; the rest is nice-to-have |
| **D5** | **A1 — Privacy indicator (cam/mic/screencast)** | Bigger lift (PipeWire node inspection). Pull forward when a real-world need shows up |
| **D6** | **A4 — Keyboard layout pill + cycle** | Blocked by margo-side: runtime xkb_layout switching doesn't exist yet (only startup config). Pair with a dedicated margo session |
| **D7** | **A8 — Setup wizard (first-launch onboarding)** | Multi-step modal — sizeable. The shell is already usable without one; revisit when there's an onboarding pain point |
| **D8** | **B3 — Process list modal (Ctrl+Shift+Esc task manager)** | Big widget — defer until system-monitoring needs surface |
| **D9** | **B5 — Audio visualizer / spectrum bar** | Eye-candy; ships after the functional slate clears |
| **D10** | **B7 — Hooks system** (`~/.config/mshell/hooks/on_*.sh`) | Small but pure extensibility — defer until users start asking for it |

## Dropped

- ~~**Notepad module**~~ — owner decision: not needed.

---

## Where mshell is ahead — preserve these

| Area | mshell | Noctalia | Dank |
|---|---|---|---|
| Compositor coupling | dwl-ipc-v2 native, tag-aware | Generic protocols only | Generic + per-compositor service |
| Gamma / blue-light | Built into compositor (twilight) | External (sunsetr / gammastep) | External |
| Auth surface | In-process polkit + PAM | n/a | Separate Go backend daemon |
| Firewall pill | ✅ ufw | ❌ | ❌ |
| DNS-mode pill | ✅ ndns | ❌ | ❌ |
| Container pill | ✅ npodman | ❌ | ❌ |
| Runtime stack | Single Rust binary | Quickshell / QML | Quickshell + Go daemon |

---

## Recommended sequencing

Owner-curated short list. Items not on it are explicitly
deferred to Tier D — pull them back up when there's interest.

**Active queue:**

1. **A5 — Calendar grid in the clock menu.** Daily-visible
   upgrade; the existing clock-menu body is sparse and a real
   month grid is what users expect when they click a clock.
2. **B10 — Output management under Display → Layout.** The
   `mlayout` CLI already implements list / preview / set / new
   / next / prev / pick; this work is wiring it to a Settings
   panel + seeding the layout dir on first run. Settings →
   Display is built for it (sub-sidebar already in place).
3. **B6 — System update indicator pill.** Polled count of
   pending updates (pacman / dnf / apt). Tiny widget, daily
   utility.
4. **B4 — Overview dashboard.** Super-key full-screen card
   view (clock / weather / media / calendar / system / user).
   Look-and-feel piece; lands after B6.
5. **B9 — Window-rule editor in Settings.** GUI rule builder
   over margo's `windowrule` config; writes through the same
   in-place config pipeline twilight already uses.

**Deferred to Tier D** (owner: "do them eventually, not now"):
A1 (privacy indicator) · A4 (keyboard layout) · A8 (setup
wizard) · B3 (process list modal) · B5 (audio visualizer) ·
B7 (hooks system). Plus the original D1–D4.

**Tier C** stays where it is — niche, pick up if a use case
surfaces.

---

## Out of scope

- File management (use Nautilus / Thunar / lf)
- Display login greeter management (LightDM / greetd setup is system-level)
- Window tiling / overview (margo's job; mshell renders, doesn't decide)
- Removable drive mounting (udisks)
- Screen mirroring / casting (compositor + dedicated tool)

Same boundary noctalia draws in its README — a shell is the visual layer
on top of the compositor, not a desktop environment.
