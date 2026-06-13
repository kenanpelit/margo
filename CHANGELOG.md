# Changelog

All notable changes to **margo** are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.6] – 2026-06-13

An internals pass: no user-visible behaviour change, but the codebase paid
down its two watched ratchets and gained safe config migration.

### Changed

- **`state.rs` split back under 3k (4045 → 2441 lines).** The compositor's
  central file regrew past its Phase-2 `<3000` target; the window/tag-rule +
  placement cluster, the tiling-arrange cluster (incl. the ~526-line
  `arrange_monitor`), keyboard-focus + pointer-monitor methods, and DPMS +
  monitor enable/disable moved into sibling `impl MargoState` modules
  (`state/{window_rules,arrange,focus_methods,dpms}.rs`), and
  `apply_theme_preset` sits beside its `ThemeBaseline` in `state/theme.rs`.
  Pure lift-and-shift — no behaviour change.
- **Settings + bar boilerplate consolidated.** A `build_pages!` macro collapses
  the 47 hand-written settings-page controller builds into one declarative list
  (the page stack + sidebar were already table-driven); the three bar-slot
  rebuild guards fold into one `BarModel::rebuild_slot` helper.

### Added

- **Profile schema versioning + stepped migration** (`mshell-config`). A new
  `config_version` file-format meta key with a `migrate_yaml` load pre-pass that
  brings an older profile up to the current format and writes it back once
  (idempotent), and a save-side stamp. The framework makes the next config
  rename/reshape a one-step, round-trip-tested change instead of a silent
  "works-on-my-fresh-config" bug. v0→v1 is the versioning baseline (no field
  transform yet). 7 round-trip tests.

### Docs

- Roadmaps de-drifted: `road_map.md` / `1.0-readiness.md` current-status bumped
  to v1.0.6; `code-quality-roadmap.md` marks the `state.rs` split, config
  versioning, and the settings/bar boilerplate items done/partial.

## [1.0.5] – 2026-06-12

The design-language release: one central geometry source, GNOME metrics
everywhere.

### Changed

- **GNOME/libadwaita visual overhaul.** The whole shell moved to Adwaita
  metrics, driven from a new central component-token layer
  (`01-tokens/_components.scss`) — buttons pull shape, size and colour
  roles from one place (`--button-*`, `--row-*`, `--card-*`,
  `--entry-*`). Radius scale 10/16/20/28/32 → **6/9/12/15/18** (Adwaita
  button/card/window); every button is now 9 px / ≥34 px with size
  equality across a row; the pill shape is reserved for switches,
  progress, chips, the clipboard panel search and prominent CTAs (action
  buttons are never pills anymore — power/session/DNS rows demoted).
  New shared **boxed-list** primitive (hairline-separated grouped rows,
  Adwaita ActionRow anatomy) adopted across ~45 Settings pages; compact
  searches (settings sidebar, keybinds, SSH) dropped from pill/xl to the
  Adwaita entry corner; control-center tiles, dashboard tiles and mdock
  take the card/window corners. DESIGN.md §0/§1/§5/§12/§21/§22 rewritten
  to the new language (incl. stale doc↔token drift), with the design-lint
  CI gate unchanged and green. Spec:
  `docs/superpowers/specs/2026-06-12-gnome-visual-overhaul-design.md`.
- **Standalone binaries follow the same metrics.** mvpn's layer-shell
  panel (embedded CSS) and mkeys' key buttons align to the Adwaita scale
  (window 15 / card 12 / button 9 px · 34 px targets), and the
  theme/bar/menu/network-editor Settings sub-pages got the same
  boxed-list rows as the main pages.

## [1.0.4] – 2026-06-12

A notification-center deepening on top of housekeeping: notifications go
from "informing" to "acting" — reply from the toast, hear it, watch it
progress, find it later. Plus ledger sync, Settings polish, and two new
CI gates.

### Changed

- **`mvpn fav` got pick numbers.** `fav list` (and `fav refresh`) now
  number the favorites; `fav connect 2` connects to the 2nd entry (relay
  names still work); bare `fav connect` on a terminal shows the numbered
  list and asks (Enter = fastest, q = cancel) — when piped or bound to a
  key it keeps the old "fastest favorite" behaviour.
- **Settings panel polish.** Sidebar nav rows tightened (38 → 33 px, 2 px
  list spacing, group headers pulled in and aligned to the icon column;
  the search separator gained breathing room). The Widgets sub-sidebar
  now fits its longest names ("Screen Recording", "System Bluetooth")
  via a 216 px column + density overrides instead of ellipsizing. Page
  descriptions (`label-small`, action-row subtitles, hero subtitle) are
  re-toned with a palette-proof surface blend (`color-mix` 62–70 % toward
  the page surface) so section titles carry the contrast even on schemes
  where `--on-surface-variant` equals `--on-surface`; boxed-list cards
  gained a 1 px hairline edge for low-contrast palettes.

### Added

- **Inline reply (KDE-style).** Notifications that carry an
  `"inline-reply"` action (chat apps, Valent SMS, KDE apps) now render a
  reply entry right on the popup toast; Enter or the send button emits
  the `NotificationReplied(id, text)` signal back to the app. The daemon
  advertises the `inline-reply` capability so clients light the feature
  up. Implemented by vendoring `wayle-notification` 0.1.3 with a
  contained, upstreamable extension (`vendor/wayle-notification` +
  `[patch.crates-io]`); the popup layer-shell surface switches to
  on-demand keyboard so the entry can type. Toggle: Settings → Widgets →
  Notifications → "Inline reply".
- **Notification sounds.** A synthesized in-tree chime (gentle two-tone;
  brighter rising tone for critical) plays when a popup appears —
  per-urgency toggles, an "app-provided sounds" switch honouring the
  spec's `sound-file` hint, the `suppress-sound` hint always respected,
  and a **quiet hours** window (wraps past midnight). Off by default;
  everything under Settings → Widgets → Notifications. The daemon now
  advertises the `sound` capability.
- **Progress bars.** Notifications carrying the spec's `value` hint
  (downloads, transfers, backups) render a real progress bar on the
  popup **and** in the history; `replaces_id` re-sends now update the
  live toast in place (text + bar) instead of freezing the first frame.
- **History search.** The notification history menu gained a search box
  filtering app name + summary + body as you type, with a proper
  "No matches" empty state.
- **CI design-lint gate (W1.7).** New `scripts/design-lint.sh` enforces
  `DESIGN.md §15` L1–L7 as hard gates in `ci.yml`: no hardcoded hex, no
  raw ≥4 px spacing/radius/font, `--radius-widget` only in bar/frame
  styles, no literal transition durations, no `gtk::Popover` as a
  bar-widget primary surface, no `add_css_class("")`, no
  `DragSource`/`DropTarget` row reorder. The pre-flip audit's three real
  hits (privacy-menu rows on the bar-pill radius token; two deliberate
  5 px launcher paddings) were fixed in the same commit.
- **Plugin-host path-sandbox tests.** The WASM tier's only filesystem
  boundary (`resolve_scoped` behind the `read-file`/`write-file`
  capabilities) moved into an unconditionally-compiled `sandbox` module
  with 10 unit tests (traversal matrix incl. `..`/absolute/CurDir/
  percent-encoding cases, scoped write/read round-trip, no-escape
  guarantees) — they run on every `cargo test --workspace`, even in
  builds that never link wasmtime. No behaviour change.
- **CI panic ratchet.** New `scripts/panic-ratchet.sh` +
  `scripts/panic-baseline.txt` gate in `ci.yml`: the number of
  `.unwrap()` / `.expect()` / `panic!()` calls in non-test code (334 at
  seed) can only go **down**. A rise fails CI as a regression; a drop
  fails too until the baseline is lowered, so every cleanup is locked in.

### Fixed

- **Mic no longer jumps to 100 % after `mshellctl audio switch-mic`.**
  With "Restore volume on startup" enabled, the configured default
  output/input level is now re-applied to the newly promoted device
  after every `audio switch` / `switch-mic`, not just at login —
  WirePlumber carries each device's own last volume, so a switch could
  land on a 100 % mic even though Settings → Sound pins 50 %.
- **Settings → Network: wired "Edit connection" button now works.** It was
  a stub since the connection-editor task landed — it sent an empty UUID
  and just toasted "no connection UUID". It now resolves the ethernet
  profile (active one preferred, falling back to any saved ethernet
  profile) and opens it in the embedded connection editor; the toast only
  remains for the genuine "no wired profile exists" case.

### Documentation

- **Roadmaps + readiness ledgers synced to v1.0.3 reality.** `road_map.md`
  header/TL;DR updated (1.0 shipped 2026-06-09; protocol score 14/17 →
  **15/17** with `output_power`; socket-IPC supersession note on the
  dwl-ipc section). `road_map_mshell.md` finally marks
  A2/A3/A4/A5/A8/A9/B4/B5/B9 and D3/D5/D6/D7/D9 as **shipped**, replaces
  dwl-ipc-v2 references with the Unix control socket, and ledgers the
  beyond-roadmap additions (control center, AI assistant, mvpn, mkeys,
  mplay, dock, …). `docs/1.0-readiness.md` got a "Resolved — 1.0 shipped"
  banner; `docs/protocol-comparison.md` re-audits margo's column at
  v1.0.3 (~60 globals); `docs/code-quality-roadmap.md` metrics refreshed
  (765 test fns; `state.rs` regrew to 4045 — flagged as the next ratchet).

## [1.0.3] – 2026-06-09

A VPN + window-switcher release: a native Mullvad VPN tool (CLI + GTK panel +
bar pill + Settings page) and a niri-style most-recently-used Super/Alt+Tab
window switcher with a live thumbnail overlay.

### Added

- **`mvpn` — native Mullvad VPN control.** A new standalone binary: a full CLI
  (`connect`/`toggle`/`<cc>`/`fastest`/`fav`/`obf`/`slot`/`timer`/`test`/…) and
  a GTK4 layer-shell control panel (`mvpn menu`) themed from the matugen
  palette. Ports the favorites (ping-sorted), fastest-relay, obfuscation
  (`anti-censorship`), device-slot (multi-machine), blocky DNS-guard, timer and
  leak-test logic natively in Rust — file-compatible with the existing
  `~/.mullvad/{favorites,slot.state}`. Honours the `OSC_MULLVAD_*` env overrides.
  Replaces the external `mullvad` WASM plugin.
- **Native "DNS / VPN" bar pill + in-shell menu + Settings → VPN page.** One
  widget-picker-selectable `BarWidget` (status icon, accent-tinted when up,
  right-click → toggle). Left-click opens a **native layer-shell menu** (not a
  separate process): the full Mullvad control set — Connect/Disconnect, Random,
  Fastest, Add-favourite, Lockdown, Auto-connect, Quantum-resistant,
  anti-censorship, favourites list — plus a collapsible **DNS section** carrying
  the Blocky guard, system-default reset, and the DNS presets. The standalone
  DNS pill is folded into this one (old `Dns` bar configs migrate automatically).
  The Settings → VPN page mirrors the controls, reading live state via
  `mvpn toggles`. `mshellctl menu vpn` toggles the combined menu from a
  terminal, and Settings → Widgets lists separate **VPN** (the combined menu)
  and **DNS** (the standalone `mshellctl menu dns` menu) entries for
  position/size tuning.
- **Native AI assistant.** The `assistant-panel` WASM plugin is now a
  first-class core feature: a new GTK-free **`mshell-ai`** engine (Gemini /
  OpenAI / Anthropic / Ollama / Custom, token-by-token streaming, API key in
  the keyring) with **live model discovery** — pick a provider and the model
  dropdown auto-offers its models (fetched from the provider's list-models
  endpoint, with a curated fallback), no hand-typing. A **Settings → AI** page
  (provider → model cascade + Refresh + endpoint / temperature / tokens /
  system prompt), an **AI** bar pill, a native streaming **chat menu**
  (bubbles, Stop / Retry / New / Copy, persisted history), and
  `mshellctl menu ai`.
- **MRU window switcher (niri-style Super/Alt+Tab).** Hold the modifier, tap Tab
  to walk windows in most-recently-used order, release to commit. A live
  thumbnail-row overlay (scope title + per-thumb app-id labels), separate from
  the grid overview. The row is a **carousel** — the selected window's
  thumbnail is centred on the output and the strip scrolls as you cycle.
  Dispatch `mru_next` / `mru_prev` (`arg.v` = scope, `arg.v2` = filter); knobs
  `mru_thumb_height` / `mru_scope` / `mru_filter` / `mru_show_labels`, also
  editable under Settings → Overview.

### Fixed

- **VPN menu redesigned** (DESIGN.md): a segmented **Mullvad / Blocky /
  Default** mode selector on top (active mode accent-filled), with Favourites /
  Countries / DNS-presets as collapsible sections.
- **`mvpn fastest` now genuinely finds the fastest relay.** It used to ping a
  random subset of 8 relays, so a ~10-relay country could miss the actual
  fastest (connecting to a 350 ms relay over a 60 ms one). It now pings every
  relay in the requested country (matching osc-mullvad), tries them
  fastest-first, and prints each result; `fastest` no longer touches favorites
  while `fastest-fav` saves the winner.
- **`mvpn` desktop notifications.** connect / disconnect / toggle / random /
  fastest now raise a `notify-send` toast with the resulting relay + location
  (silenceable via `MVPN_NO_NOTIFY`).
- The Mullvad pill opens the shell's **own native layer-shell menu** instead of
  spawning the standalone `mvpn menu` popup, so it matches the other menus'
  chrome and DESIGN.md exactly.
- A standalone **DNS** bar pill is available again in the widget picker
  (alongside the combined VPN pill) — it opens the `mshellctl menu dns` menu.
- The VPN pill shows the connected **country** beside the shield; the menu's
  favourites list marks the currently-connected relay active ("Connected" +
  accent); and a small **account-expiry** line sits under the DNS section.
- The VPN menu has a collapsible **Countries** picker (every Mullvad country +
  relay count, from `mvpn countries`; per-row Connect runs `mvpn <cc>`), and
  the favourites' Connect buttons now connect to *that* relay
  (`mvpn fav connect <relay>`) instead of always the fastest favourite.
- Changing a widget's position/config no longer flashes the clock menu over
  whatever menu is open: the per-region stacks' visible child is now preserved
  across the menu restack.
- **`mvpn` Protocol button** repurposed to a working WireGuard
  quantum-resistance toggle (modern Mullvad removed `relay set tunnel-protocol`).
- **`mvpn menu` is a real layer-shell panel**, not a floating "popup", and Esc
  closes it. Driven by a raw GLib main loop (no `GtkApplication`, whose window
  management forced an xdg-toplevel) with exclusive keyboard. The surface is
  translucent like the shell's native menus (matugen menu opacity).
- MRU switcher: genuine most-recently-used ordering (cycling no longer
  rewrites the order) and the stuck-modifier bug (the Alt/Super release now
  reaches the focused window, so it isn't left "held"). Thumbnails are
  off-screen snapshots taken before render, so every window (even on other
  tags) shows a real preview from the first frame; the backing band is
  preview-sized and scrolls with the carousel.

## [1.0.2] – 2026-06-09

A compositor-effects + bar-consistency release: opt-in dual-Kawase blur,
Hyprland-style tabbed window groups, a per-output frame clock, and a single
bar-pill standard that finally makes every widget (and plugin) render with
the same surface, hover, height, and shape.

### Added

- **Dual-Kawase background blur (opt-in, default off).** `blur` / `blur_layer`
  blur behind translucent windows / layer-shell surfaces; strength + radius
  are tunable from Settings → Effects (`blur_params_num_passes`,
  `blur_params_radius`) and per-window/-layer `noblur:1` rules.
- **Tabbed window groups (Hyprland-style, opt-in).** `togglegroup`,
  `changegroupactive`, `movegroupwindow`, `movewindowtogroup`, `lockgroups`
  merge windows into one tile with a tab strip. The strip shows each member's
  app-name label (rendered with `fontdue`) on rounded, matugen-tinted chips;
  height/colours via `group_bar_height` / `group_active_color` /
  `group_inactive_color`.
- **Per-output frame clock (opt-in).** `per_output_frame_clock` paces each
  monitor by its own refresh so a 60 Hz panel can't hold back a 144 Hz one.
- **`mctl plugin` subcommand.** `list` / `enable` / `disable` for compositor
  Rhai plugins.
- **Manual frame colour override.** Settings → Bar → Frame can pin the bar
  frame fill + border to fixed colours instead of the matugen palette.
- **Setup-wizard polish.** Prominent nav + in-page action buttons, a
  default-shortcuts cheat-sheet on the Review step, and centred footer nav.
- **Configurable bar separator colour.** Settings → Bar can override the
  Separator widget's colour (`separator_color`) instead of matugen `--outline`.
- **Panel power (DPMS) off/on.** A `dpms on|off|toggle [output]` dispatch
  action truly powers monitors down (via smithay's `DrmCompositor::clear()`)
  and back up — real power saving, not just a dim. Recovery is guaranteed two
  ways: any input wakes a darkened panel, and a VT-switch round-trip always
  restores every output (both force the all-outputs render path so a stalled
  per-output clock can't keep a panel dark). The input that wakes a darkened
  panel is swallowed, so the keystroke that turns the screen back on no longer
  lands on the focused window (no stray newline in the terminal you ran
  `mctl dispatch dpms off` from).
- **External DPMS control (`zwlr_output_power_management_v1`).** Idle daemons
  (`swayidle`) and `wlr-randr --off/--on` can now power outputs down/up; their
  `set_mode` maps onto the same recoverable `request_dpms` path, and `mode`
  events track every power change (whoever triggered it).
- **Example compositor plugin.** Ships `app-workspaces` under
  `margo/examples/plugins/` (app-id → home tag on open) and documents the
  full `plugin.toml` + `init.rhai` format + `mctl plugin` workflow.

### Changed

- **One central bar-pill standard (`.bar-pill-std`).** `build_widget` tags
  every pill — native widgets AND plugin pills — with a single class +
  centre alignment, so they all share the same surface, matugen hover wash,
  height, and rounded shape (no more per-widget drift or full-width
  ballooning). Adjustable from one place.
- Wizard's full starter profile renamed **Nova → margo** (lowercase,
  brand-consistent); the minimal one stays **default**.

### Fixed

- **Blur shimmer.** Forcing a full redraw per frame while blur is on stops the
  stale-backdrop flicker from age-based damage tracking; the composite quad is
  now positioned via a deterministic viewport→NDC mapping.
- **Tabbed-group glitches.** The tab strip reserves its height inside the tile
  (title no longer hides under the bar / eats the gap), and hidden members are
  pre-sized so `changegroupactive` doesn't flash the wallpaper.
- Flatpak / sandboxed (security-context) clients no longer crash the
  compositor on launch.
- External-monitor hotplug now allocates a scanout-capable buffer so a
  re-plugged display is detected.
- Bar separator stays visible in every mode again (the new central pill
  standard had painted the Separator/Spacer helpers transparent); the frame
  Fill/Border colour pickers no longer drift apart in Settings → Bar.
- `mctl plugin list` no longer errors when there's no
  `~/.config/margo/plugins/` directory — it explains the difference between
  packaged compositor plugins, auto-loaded `init.rhai`, and shell plugins.

## [0.9.8] – 2026-06-04

A shell-polish release: a walker-class app launcher, a much richer
clipboard (CLI + smart typing), notification timeout bars, and Control
Center additions — plus a second-generation config parser and an
expanded setup wizard.

### Added

- **App launcher preview pane.** `mshellctl menu app-launcher` gains a
  detail pane beside the results (text / monospace / colour swatch),
  per-provider rich rows + type classes, and comfortable density.
  Settings → Launcher → Appearance toggles cover the preview pane,
  compact rows, and large app icons.
- **`mshellctl clipboard` CLI.** Headless access to the clipboard
  history — `list [--json]`, `copy`, `pin`/`unpin`, `delete`, `clear`,
  `wipe` — driving the same store as `menu clipboard`.
- **Smart clipboard content typing.** Entries are classified as URL /
  colour / code / email (plus text/image), with per-row type icons and
  a `clipboard.image_max_kb` cap; every knob is exposed under
  Settings → Widgets → Clipboard.
- **Notification timeout bar.** Popup toasts show a shrinking bar
  counting down their on-screen time, paused while hovered. Toggle +
  duration in Settings → Notifications (`show_timeout_bar`,
  `popup_duration_ms`).
- **Control Center: power-profile control + battery chip.** A
  Saver / Balanced / Performance segmented control above the sliders
  and a battery pill in the header, both toggleable from
  Settings → Widgets → Control Center → Sections.
- **Session widget menu controls.** Settings → Widgets → Session gains
  the standard menu size & position section.
- **Second-generation config parser diagnostics.** "Did you mean"
  suggestions for unknown keys, incomplete-`bind` (E004), modifier /
  enum validation (E005/W002), and scalar type-checking (W003).
- **Expanded setup wizard.** Hardware-aware steps + Display / Power /
  Night-light pages, review-edit, keyboard navigation, and the margo
  logo on the Welcome step.

### Changed

- Control Center SCSS moved fully onto tokens; volume / mic /
  brightness sliders gain a sensible scroll step.
- `~/.config/margo` reorganised toward `~/.cachy/modules/margo`
  (dotfiles-managed) with `binds.conf` split out of `config.conf`.

### Fixed

- App launcher Ctrl+N/K navigation no longer triggers a display-wide
  restyle per keystroke; dropped the unsupported GTK4 `gap` / `cursor`
  CSS that produced theme-parser errors.
- Honour `$MARGO_SOCKET` in the compositor IPC socket path.
- Dropped the removed `mshelldash` menu verb from the shell completions
  and docs.

## [0.9.6] – 2026-06-03

A Bluetooth + overview release: a native auto-connect engine that
retires the external scripts, a configurable scroller-overview backdrop,
and a fix so the scroller overview renders windows on never-visited tags
correctly.

### Added

- **Native Bluetooth auto-connect + audio routing.** Replaces the external
  `bt-autoconnect.service` + `bt-autoconnect-once` + `bluetooth_toggle`
  scripts (and the F10 binding) with an in-shell engine. Settings →
  Bluetooth gains an **Auto-connect** section: a master switch, a
  post-login delay, an ordered MAC device list (drag to reorder; first
  that connects wins), "use as audio output" / "use as microphone"
  routing toggles, and connect/disconnect notifications. At login the
  shell waits the delay then connects with a few retries and routes audio
  to the device; `mshellctl bluetooth toggle | connect | disconnect`
  (smart toggle: power-on + connect, or disconnect if already connected)
  backs a keybind and the Settings "Toggle now" button. The mic toggle is
  off by default — forcing a headset mic drops the codec to HSP/HFP and
  degrades playback.
- **Bluetooth menu redesign.** The opened menu groups devices into
  Paired / Available, sorts connected devices first, shows a spinner
  while a connect/disconnect is in flight, and adds a ★ pin to add or
  remove a device from the login auto-connect list. Device name + battery
  text bumped to a larger, legible size.
- **Configurable scroller-overview backdrop.** Settings → Overview gains a
  **Backdrop** section: pick a solid colour (with alpha) or an image
  (cover-fit) painted behind the tag cells. Both apply live; a missing
  image path falls back to the solid colour.
- **Per-script autostart trigger.** Settings → Launcher startup scripts can
  run on **every shell start** or **login only** (once per session), set
  per script.
- **Per-bar enable + explicit auto-hide.** The top and bottom bars each get
  their own enable switch and a configurable auto-hide delay.
- **Project-ethos note on the About page.**

### Fixed

- **Scroller overview shows un-visited tags correctly.** Windows on tags
  you hadn't switched to yet rendered crammed at their stale map-time
  position and size until the tag was visited once. The overview now
  pre-arranges every off-screen tag it will show (layout geometry + a
  resize configure) and feeds those off-`space` windows frame callbacks
  while it's open, so frame-throttled clients (GTK, Electron) repaint to
  their slot size from the first open.
- **SCSS edits always rebuild the baked stylesheet.** `mshell-style`'s
  build script now emits a per-file `rerun-if-changed` for every nested
  SCSS partial, so editing a component partial no longer bakes a stale
  stylesheet into the binary until an unrelated file change forces a
  rebuild.
- **Startup-scripts list height** grows with content (capped, then
  scrolls) instead of cramming into a tiny box.

## [0.9.5] – 2026-06-02

A shell polish release: drag-to-reorder across Settings, a frameless
"floating panels" bar mode, a batch of post-resume / log-noise fixes,
and DESIGN.md turned into an enforceable quality gate.

### Added

- **Drag-to-reorder in Settings.** Every reorderable list — bar-widget
  sections, menu-widget lists (incl. nested containers), quick actions,
  and Control-Center tiles — now has a ≡ grip handle you can grab and
  drag to reorder, alongside the existing ↑/↓ buttons (kept for
  keyboard/accessibility). A live `.drop-target` indicator highlights the
  landing row as you drag. Implemented with a shared `GestureDrag` helper
  (`reorder_dnd`), not GTK DnD — `GtkListBox` swallows drag-and-drop
  motion/drop before rows see it.
- **Frameless bar mode.** Turning off **Settings → Bar → "Enable frame
  drawing"** no longer leaves the bars/menus transparent: each paints its
  own opaque, themed, rounded surface, so the shell reads as discrete
  floating panels (matugen-tracked).

### Changed

- **Islands bar toggle applies live** — flipping Settings → Bar → Islands
  no longer needs a shell restart.
- **UFW bar poll no longer shells out to `sudo`.** The pill/tile state
  comes from a privilege-free `systemctl is-active ufw.service`; the full
  rule list is fetched (sudo/pkexec) only when the menu is opened. Poll
  interval relaxed 120 s → 300 s. Eliminates the per-poll sudo/PAM
  journal spam (and the `/usr/bin/ufw` stdout leak from the `which`
  probe).
- **DESIGN.md is now an enforceable quality gate** (§15 lint rules with
  grep recipes, §16 component state matrix, §17 async states, §18 reuse
  registry, §19 surface decision tree, reorderable-row + positioning
  standards). Doc only.

### Fixed

- **Shell froze for 1–2 min after suspend/resume.** Synchronous
  `state.json` reads on the GTK main thread blocked while the just-resumed
  compositor drained its input backlog. Bounded with a 250 ms timeout so
  a busy compositor can never freeze the shell.
- **Window border lagged content on `switch_proportion_preset` resize** —
  the resize snapshot is now held until the live buffer reaches the new
  slot size (or a grace ceiling), keeping border and content locked
  together on slow grows.
- Stop a burst of harmless-but-noisy GTK assertions: control-center grid
  rebuild (`gtk_grid_remove`), menu scroller min/max content-size, and an
  empty-string CSS class on borderless menu-widget lists.
- Setup wizard menu opens at a sensible fixed 640×720.

## [0.9.4] – 2026-06-01

Brings the user's external helper scripts in-house: a new first-party `mplay`
binary, a synthetic-key `sendkey` action, and manual power-profile control.

### Added

- **`mplay` — native mpv companion** (new binary). Replaces `margo-mpv.sh`:
  - **Window control:** `start` / `toggle` / `play [URL]` / `download` / `snap`
    (corner cycle) / `pin` (all-tags) / `focus` / `stop`, over mpv's JSON IPC
    socket + `mctl`.
  - **Native video wallpaper:** `mplay wallpaper start <SRC>` / `stop` — an
    in-tree mpvpaper port (wlr-layer-shell background surface + EGL +
    hand-written libmpv render-gl FFI), no external `mpvpaper`/uinput.
  - **Smart media control:** `mplay media <toggle|play|pause|stop|next|prev|
    status> [player]` — auto-detects the best active player across MPRIS
    (`playerctl`), MPD (`mpc`), and mpv; scoring + last-player memory + Spotify
    autostart + album-art notifications (osc-media.sh port).
  - Embedded yt-dlp shim (anti-bot fallback, cookies, browser UA) — no external
    `yt-dlp-mpv` script. optdepends: `mpv`, `yt-dlp`, `playerctl`, `mpc`.
- **`sendkey` dispatch action** — inject a synthetic key combo into the focused
  window: `sendkey,<combo>[,<appid-regex>][,<fallback>]`. Forwards the keys via
  the seat keyboard (no ydotool/uinput/virtual-keyboard). Powers 3-finger
  touchpad browser tab-switching (`ctrl+Tab` / `ctrl+shift+Tab`, app-id gated,
  with a `focusdir` fallback) — replaces fusuma + fusuma-plugin-sendkey.
  Layout-independent keys only (Tab, Page_Up/Down, arrows, F-keys, …).
- **`mpower cycle` / `mpower set <profile>`** — manual power-profile switching
  (e.g. on a keybind); the auto-profile daemon honours the manual change until
  the next AC transition.

### Fixed

- **`mctl dispatch spawn` multi-word args** — `spawn 'kitty -e htop'` no longer
  drops everything past the first token over the socket.
- **IPC outbound back-pressure** — a slow `watch` subscriber is buffered
  (bounded, with a WRITE source) instead of being dropped on the first partial
  write; the event loop never blocks or spins.
- **CI** — drop debuginfo + incremental artifacts so the full-workspace build
  fits the runner disk.

### Internal

- Large unit-test expansion (mplay controller/media/engine helpers, sendkey
  combo/regex parsing, IPC framing) — workspace suite well past 600 tests,
  clippy `-D warnings` clean.
- Docs (site + README) refreshed for the socket IPC + the new tools; all
  `config.conf` comments translated to English.

## [0.9.3] – 2026-06-01

A from-scratch IPC rewrite: the legacy `dwl-ipc-unstable-v2` Wayland protocol
and the polled `state.json` snapshot file are gone, replaced by a single
Unix-domain control socket.

### Changed

- **New socket IPC (replaces dwl-ipc-v2 + `state.json`).** margo now serves a
  single newline-delimited control socket at
  `$XDG_RUNTIME_DIR/margo/margo-ipc.sock` (exported as `MARGO_SOCKET`) speaking
  `get <topic>`, `watch <topic>` (push-on-change), and
  `dispatch <action> [args…]`. mshell subscribes with `watch state` instead of
  inotify-watching a file, and `mctl` talks the same protocol — no Wayland
  client needed to script the compositor. Protocol documented in `docs/ipc.md`.
  - **Breaking:** the `dwl-ipc-unstable-v2` protocol and the `state.json` file
    are removed entirely; external bars that read them need a socket adapter.
    The standard `ext-workspace` and `foreign-toplevel-list` protocols are
    unchanged.

### Added

- **`mctl get <topic>` / `mctl watch <topic>`** — raw socket queries and
  change streams (topics: `state`, `clients`, `client <id>`, `monitors`,
  `monitor <name>`, `tags <monitor>`, `focused`, `layouts`, `keyboard-layout`,
  `twilight`, `config-errors`).
- **Dispatch actions** `settagset`, `setclienttags`, `setlayoutindex` (plus the
  existing `cyclekblayout`), all reachable over the socket.
- **`tag_carousel`** — wrapping a relative tag move past the first/last tag
  slides in the travel direction instead of reversing the long way.
- **`edge_scroller_focus_allow_speed`** — in a scroller layout, a slow pointer
  resting at the leading/trailing edge shifts focus to the adjacent column
  (debounced; `0` disables).

### Fixed

- **Multi-word `spawn` over the socket** — `mctl dispatch spawn 'kitty -e htop'`
  no longer drops everything past the first token.
- **Outbound back-pressure** — a slow `watch` subscriber is now buffered
  (bounded to 4 MiB, with a level-triggered WRITE source) instead of being
  dropped on the first partial write; a subscriber past the cap is cut loose to
  reconnect. The event loop never blocks or spins on a stalled reader.
- **`mctl get`** is now a real subcommand (was only used internally).
- **GTK4 CSS parser warnings** — scrubbed the remaining unsupported properties
  (`overflow`, `cursor`, `column-gap`, `@charset`, `-var()` negation) from the
  shell stylesheet.

### Internal

- Large unit-test expansion around the new IPC surface: request framing,
  outbound drain/back-pressure, topic projection (incl. error frames), dispatch
  arg mapping, tag-carousel + edge-scroller decisions, and the `mctl` socket
  client. Workspace suite at 565 tests, clippy `-D warnings` clean.

## [0.9.2] – 2026-06-01

### Added

- **Man pages** for the core tools — `margo(1)`, `mctl(1)`, `mshellctl(1)`
  (hand-written roff under `man/`, installed to `/usr/share/man/man1` by both
  the PKGBUILD and `install.sh`).

### Changed

- **CLI help polish** — `mshellctl` now has a proper description + examples
  (was a bare "MShell CLI"), and `margo --help` lists all companion binaries
  with aligned columns plus a FILES section and `man` pointers.

## [0.9.1] – 2026-06-01

A wave of widgets ported from noctalia-shell v5, plus the removal of the
mshelldash surface.

### Added

- **Keyboard Layout** bar widget — shows the active xkb layout (e.g. `US`,
  `TR`); click cycles to the next configured layout. The compositor now tracks
  the active layout group (`KeyboardHandle::with_xkb_state`), publishes its name
  in `state.json` (`keyboard_layout`), and exposes a `cyclekblayout` dispatch
  action (`mctl dispatch cyclekblayout`). Configure multiple layouts with
  `xkb_rules_layout = tr,us` for cycling to do anything.
- **Audio Visualizer** bar widget — a live spectrum strip driven by the `cava`
  CLI (raw/ascii mode); pulses with playback and shows a flat resting strip on
  silence. Degrades gracefully when `cava` isn't installed.
- **Keyword-aware Settings search** — the Settings search now matches section
  keywords, not just page labels, so `brightness` finds Display, `vpn` finds
  Network, `suspend` finds Power, etc. (both Enter-to-jump and the live sidebar
  filter).

### Removed

- **mshelldash** — the standalone tabbed dashboard surface and its
  `mshellctl menu mshelldash` verb were removed. The classic `dashboard` menu is
  unaffected. The Screen Time tracker that lived only inside it was removed with
  it.

## [0.9.0] – 2026-06-01

### Added

- **mpower** — a native automatic power-profile manager (new binary +
  `systemd --user` service). Picks the power-profiles-daemon profile from live
  CPU load and AC/battery state (performance under load on AC, balanced /
  power-saver on battery), honours a manual override until the next AC change,
  and is fully configurable from Settings → Power → Automatic Power Profile and
  `~/.config/margo/mpower.toml`. Replaces the external `ppp-auto-profile`;
  gated to margo sessions so it never fights another compositor's tool.
- **Hidden Bar** bar widget — a collapsible drawer (native port of the DMS
  hidden-bar plugin) that hides a configurable group of pills behind a trigger:
  hover or click to reveal, right-click to pin, auto-collapse on leave.
  Settings → Widgets → Hidden Bar picks the contents + behaviour;
  `mshellctl hidden-bar {toggle,expand,collapse,pin,unpin}` drives it from
  keybindings.
- **Catwalk** bar widget — a CPU-reactive animated cat (port of the noctalia
  plugin): idles below a CPU threshold, walks faster as load climbs; click
  opens the CPU dashboard. Configurable in Settings → Widgets → Catwalk.
- **Kenp** colour scheme (dark + light) added to the static theme catalogue.
- **Power menu battery details** — time remaining, power draw (W), health,
  capacity (Wh) and charge cycles, plus a **charge-limit control**
  (preset 60/80/100 + custom) via the kernel `charge_control_end_threshold`
  (ThinkPad/generic; set through pkexec). (Inspired by dms-framework-battery.)
- **Dock**: middle-click launches a new instance, scrolling over an icon
  cycles that app's windows, and per-app **icon overrides**
  (`dock.icon_overrides`, class → icon name/path) fix apps started with a
  synthetic `--class`.
- **Media player**: ± relative-seek buttons and an optional large album cover
  (the mplayerplus plugin folded into the native MediaPlayer widget);
  Settings → Widgets → Media Player.
- **Per-tag default tiling layout** (`taglayout` / `taglayout_force` config +
  Settings → Tiling Layout page with a per-tag override editor); the
  compositor seeds them and re-applies on reload.
- **Plugins**: plugin-declared keybinds with conflict resolution +
  Settings → Plugins → Keybinds, a panel-archetype design language with a
  family of `plugin-*` style classes, `mshellctl plugin list`, MPRIS player
  name exposure, and shell completions.
- **Docs**: a complete configuration guide (all layouts + the full dispatch
  action catalogue), a README for every binary, and a GitHub wiki.

### Changed

- Plugin install copies only runtime files (manifest / wasm / assets), never
  the plugin source tree.

### Fixed

- **Dock — focus the exact window** from the right-click menu (the long-parked
  bug): the bare `focuswindow address:…` string was silently dropped; it now
  goes through the recognised `dispatch focuswindow <idx>` path.
- **Menus** no longer flash a stale previous menu when opening a different menu
  at a shared screen position (instant stack-child switch; the revealer still
  animates).
- Catwalk + Hidden-Bar triggers use the canonical bar-pill hover
  (`.ok-bar-widget`) instead of a non-existent class.
- Tiling-layout precedence (`taglayout` > `tagrule` > `default_layout`) and
  live re-apply on `mctl reload`.

## [0.8.5] – 2026-05-27

### Added

- **Auto light/dark from wallpaper** (Settings → Theme → Wallpaper Matugen) —
  an opt-in toggle that derives the Material You light/dark polarity from the
  wallpaper's average luminance on each wallpaper change (bright → Light,
  dark → Dark), overriding the manual Mode. Only affects the wallpaper-driven
  theme. (Inspired by VibePanel's wallpaper-adaptive theming.)

## [0.8.4] – 2026-05-27

### Added

- **Control Center** — a noctalia/GNOME-style quick-settings menu (bar pill
  with a new `margo-symbolic` icon). A header (avatar + username + uptime +
  lock / power / settings / edit actions), volume / mic / brightness sliders,
  and a configurable tile grid. Tiles: Wi-Fi, Bluetooth, Audio Out, Mic, VPN,
  Valent, Battery, Keep Awake, DND, Airplane Mode, Dark Mode, Twilight,
  Color Picker, Disk, Firewall (UFW), Podman. Expandable tiles use
  GNOME-style inline-expand (a sliding `gtk::Stack` with a back arrow) and
  reuse the real detail components (Network, Bluetooth, Audio, Power, DNS,
  Valent, Twilight, Keep Awake, UFW, Podman menus). **Left-click opens the
  detail page, right-click quick-toggles** (Bluetooth / Twilight / Keep Awake
  / UFW power via pkexec / stop Podman machine). The active power profile
  shows on the Battery tile, the Twilight profile + temperature on Twilight,
  and the running machine name on Podman. **Edit mode** + Settings → Widgets →
  Control Center configure per-tile visibility, order, and wide (2-column)
  tiles.
- **Settings → Network + Bluetooth** — GNOME-parity pages. Network: Wi-Fi
  scan / connect, wired status, VPN list + import, a per-connection editor
  (General / IPv4 / IPv6 / Security via `nmcli`), and a manual proxy section
  (config schema + `environment.d` applier). Bluetooth: adapter power, device
  list, pair / connect / trust.
- **Settings → Power + Default Apps + Privacy** — GNOME-parity pages. Power:
  battery, power profiles, suspend, low-battery warning, and lid / power-button
  behaviour via a logind drop-in (pkexec, applies next login). Default Apps:
  per-category default handler via `gio::AppInfo`. Privacy: location toggle,
  camera / mic indicator, lock summary, file-history remember + clear
  (`GtkRecentManager`), and flatpak portal-permission list + revoke.
- **Audio** — optional toggle to hide HDMI / DisplayPort outputs from the
  output list + switcher (Settings + `mshellctl audio`).

### Changed

- **Dashboard design system** — a stabilization pass: a `--space-*` spacing
  scale, semantic `--warning` / `--success` colours, a 3-tier surface
  elevation model, and a shared card contract. The media widget was relaid out
  (cover / title / artist / progress + centred controls), the calendar gained
  a today / selected / hovered / inactive state model, and weather / clock /
  audio / system / CPU widgets were brought to token + elevation conformance,
  with consistent motion tokens for hover / focus / selection / reveal.
- **Bluetooth, Clipboard, and Notification menus** — redesigned to the flat
  audio-dashboard design language (engine preserved). Bluetooth shows flat
  device rows with battery + a connected accent.
- **Bundled Nova profile** — refreshed to mirror the current showcase
  (new bar widgets + menus, Control Center, alarm / proxy / power / privacy /
  audio sections), with machine-specific bits neutralized.

### Performance

- **Notification history** — virtualized the list (`GtkListView` + factory +
  lightweight row model, mirroring the clipboard menu). A persisted history no
  longer rebuilds N heavy widgets per open / per incoming toast, fixing the
  lag that grew as notifications accumulated. Per-app grouping is preserved.
- **Control Center pollers** — the shell-out probes (Twilight / UFW / Podman)
  and the Keep-Awake countdown only run while the panel is revealed.

### Fixed

- **Wallpaper menu** — no longer freezes the GTK main loop on first open
  (blocking receive → async oneshot).
- **Bluetooth menu** — device battery now shows (watch each device's
  battery / state property, not just the device list).
- **Dashboard** — rounded the CalendarGrid card's bottom corners and made the
  media player fill its column height.

## [0.8.3] – 2026-05-26

### Added

- **Alarm Clock** — alarm + stopwatch widget. A bar pill (alarm-bell glyph;
  shows the running stopwatch time inline and pulses while ringing,
  right-click to silence) opens a tabbed menu: an **Alarms** tab (reactive
  list with per-alarm enable / time / repeat-day chips / delete, plus an add
  row) and a **Stopwatch** tab (start / pause / reset). Alarms persist in the
  shell profile and fire from a main-thread scheduler that plays a looping
  tone and pops a Stop / Snooze notification; one-shot alarms auto-disable
  after firing.
- **Settings → Users** — GNOME-style account management replacing the old
  read-only list. Per-account expandable cards: change picture, edit full
  name, toggle Administrator, change password, and add / remove users. Every
  privileged action runs through `pkexec` so the polkit agent prompts, with
  guards against demoting or deleting the last administrator.
- **Daily Wallpaper** (Settings → Wallpaper) — opt-in Bing or NASA
  image-of-the-day, fetched on login and refreshed daily.
- **Bundled default wallpaper** — a margo-branded `margo-hero.png` ships as
  the default desktop wallpaper (shown when no wallpaper directory is set)
  and is always the first tile in the Wallpaper menu.
- **`mshellctl media`** — MPRIS player control: `toggle` / `next` / `prev` /
  `status` / `list`, with an optional player-name fragment (`spotify`,
  `browser`, …) and a now-playing toast on each action.
- **`mshellctl audio`** — full CLI audio control: `list` / `status` / `set` /
  `switch` for output + input devices.

### Fixed

- **Media targeting** — a player-name fragment (e.g. `browser`) now matches on
  the D-Bus bus name + desktop entry, not just the MPRIS identity (so Chromium
  forks like Helium match), and picks the *playing* player when a fragment
  matches several.
- **Audio device list** — stable ordering so `switch` cycles predictably, dead
  / monitor sinks filtered out, and friendly device names in the Sound page.
- **Cursor shapes** — margo now honours named cursor shapes
  (pointer / text / grab / …).
- **mlogind** — square (Plain) borders so corners join cleanly on a real TTY.
- **Settings → Menus** — "Add widget" uses the same scrollable popover as the
  Bar editor.

## [0.8.2] – 2026-05-25

### Added

- **Settings → Keybinds** — a full editor for margo's keyboard shortcuts:
  read every existing bind, add new ones, edit and delete. The editor owns a
  dedicated `binds.conf`; on the first edit it migrates every inline `bind*`
  line out of `config.conf` into `binds.conf` (grouped by category), leaves a
  single `source = binds.conf` behind, and backs the original up to
  `config.conf.bak`. From then on each change is a clean full rewrite + live
  reload. The UI is a searchable list (modifier/key chips + humanised action)
  with an inline editor: modifier chips, a **press-to-capture** key field, a
  searchable action picker over the dispatch verbs with a contextual argument
  hint, and an optional description.
- **Settings → Animations** — five curated motion presets (Smooth, Snappy,
  Bouncy, Cinematic, Glide), each a full coherent set of per-domain
  durations / bezier curves / clocks / open-close types. Pick a card, hit
  Apply, and the whole set is written to `config.conf` and reloaded live.
  Master on/off + layer-animation switches apply on the spot. **Smooth** is
  the recommended daily-driver (spring glide for moves, gentle zoom-in,
  snappy fade-out).
- **Settings → About / Date & Time / Region & Language** — three new
  system pages closing the gap with a conventional desktop settings app.
  *About* shows read-only system info (OS, kernel, host, CPU / GPU / memory,
  desktop session, margo version, uptime). *Date & Time* wraps `timedatectl`
  (automatic time / NTP, searchable timezone, 24-hour clock). *Region &
  Language* wraps `localectl` (system `LANG`, searchable). The `timedatectl`
  / `localectl` writes authenticate through margo's polkit agent.
- **Settings → Users** — a read-only roster of the system's human accounts
  (username, full name, administrator status from `wheel`/`sudo`, and
  `~/.face` / AccountsService avatars), parsed from `/etc/passwd` +
  `/etc/group`. The current user is listed first.
- **Settings → Sound** — output + input device selection, volume, and mute,
  backed by the same reactive `wayle_audio` service as the bar's Audio
  Dashboard. The page stays live (default-device, device-list, and
  per-device volume/mute watchers) without feeding programmatic refreshes
  back into a write loop.
- **Settings → Input** — a full keyboard / touchpad / mouse page (replacing
  the narrower Gestures page). Keyboard: xkb layout / variant / options
  (e.g. `ctrl:nocaps`), repeat rate + delay, Num Lock on start. Touchpad:
  tap-to-click / tap-and-drag / drag-lock, natural scroll, disable-while-
  typing, left-handed, middle-button emulation, click + scroll method,
  scroll button, send-events mode. Mouse: natural scroll, acceleration
  profile + speed. Plus swipe sensitivity, and a **gesture-binding editor**
  — list / add / remove `gesturebind` swipe→action mappings (direction,
  fingers, action, argument, modifiers) right from the UI. Everything writes
  the compositor `config.conf` and applies live.
- **Bundled shell profiles + a starting-profile picker in the setup wizard.**
  margo now ships two example mshell profiles — **Default** (clean, minimal)
  and **Nova** (the full-featured showcase) — installed to
  `/usr/share/margo/mshell/profiles/` and baked into the binary. The setup
  wizard's Welcome step now offers them with descriptions; picking one seeds
  it into `~/.config/margo/mshell/profiles/` (only if absent — it never
  overwrites a profile you've customised) and activates it live. Both
  profiles are documented inline in English.
- **mshell now starts automatically on a margo session (packaged).** The
  package ships a `mshell.service` systemd **user** unit and auto-enables it
  via a `graphical-session.target.wants` drop-in, so a fresh install brings
  up the bar / menus on login with no manual `systemctl --user enable` —
  and `systemctl --user status mshell` works out of the box. The unit is
  guarded (`ConditionEnvironment=XDG_SESSION_DESKTOP=margo` + `WAYLAND_DISPLAY`)
  so it never starts under another desktop, and a user's own
  `~/.config/systemd/user/mshell.service` still overrides it.

### Changed

- **Settings → General decluttered.** With dedicated pages now in place,
  General keeps only the account, config profile, and shell behaviour
  (Network OSD). The duplicate Clock toggle was dropped (Date & Time owns
  it), the Settings-panel font scale moved to **Fonts**, and rounded screen
  corners + radius moved to a new **Screen** sub-tab under Display.
- **Twilight menu — source-mode selector now uses power-profile-style
  tiles.** The Auto / Manual / Static / Schedule buttons gained icons and
  the same vertical icon-over-label tile look as the Power menu's profile
  switcher (active mode filled primary), so the two quick-control panels
  feel like one family instead of plain text segments.
- **mlogind greeter redesigned to match the mlock lock screen.** The login
  TUI now mirrors mlock's centred stack: a time-aware greeting, a big block
  clock (live, ticking every second), the full date, a single rounded
  accent-bordered card holding the session / username / password rows, a
  centred status line, and a centred row of power-control chips. Colours
  come from the same matugen palette as before (accent for the card, muted
  for secondary text), so the greeter is now visually of a piece with the
  locker. The layout is **fully responsive** to the terminal size — the bare
  VT can be much shorter *or* narrower than a terminal-emulator `--preview`:
  the power chips are pinned to the bottom and the clock + card always
  render, with the greeting / date / status dropping out (in that order) on
  a short console, and the chips collapse from `[F1] Shutdown` to a compact
  `[F1] [F2] [F3]` when the row is too narrow — so the F-keys can never be
  clipped off-screen. The selected **session is drawn inline** next to its
  label (`Session  Margo (UWSM) ›`, with `‹ ›` arrows and graceful
  truncation) instead of a fixed-width carousel that vanished at some
  widths. The credential card is wider (room for full session names + a
  comfortable password field) and the power keys read as bracketed accent
  chips.

### Fixed

- **Settings pages used the wrong live-reload command.** The Input,
  Animations and Keybinds pages spawned `mctl config reload`, which mctl
  rejects ("unrecognized subcommand 'config'") — so the file was rewritten
  correctly but the compositor never reloaded and edits looked like no-ops.
  The reload action is `mctl reload`; fixed all three.
- **General avatar pinned to a fixed 72×72.** A `~/.face` photo shown via a
  `GtkPicture` leaked the image's intrinsic size up through the box and
  ballooned the avatar; it's now centre-cropped to a square and drawn through
  a `GtkImage` at a fixed pixel size, so it stays 72×72 at any source
  resolution.
- **Setup wizard — theme / colour mode / font size now apply live.** The
  appearance picks on the wizard's Theme step only took effect at the final
  "Apply & finish", so selecting a theme mid-wizard looked like it did
  nothing. They now apply on selection (via `config_manager`, the same path
  as Settings → Theme).
- **Lock-screen background control moved to the Lock page.** It had been
  misfiled under Session; it now lives with the rest of the lock settings.

## [0.8.1] – 2026-05-24

### Added

- **mlogind — margo's TTY login manager** (forked from lemurs). A
  matugen-themed ratatui greeter that launches the margo session
  supervised through `start-margo`, syncs its colours from the active
  wallpaper palette (11 theme variables, active session visually
  distinct), and offers F3 Suspend alongside the usual power controls.
  Optional fingerprint login via PAM (`pam_fprintd`). Packaged like a
  display manager — binary plus config / PAM / systemd unit installed to
  `/etc`. The bare-VT greeter now matches `--preview` by reprogramming the
  console palette (no truecolor on a raw Linux VT). Documented in the
  README and project site, installable from the AUR (`margo-git`).
- **`pass` launcher provider** — a password-store browser in the
  launcher, with a configurable store path (Settings → Launcher). Copy and
  type both honour `PASSWORD_STORE_DIR`.
- **Configurable lock-screen background** — mlock can now use the desktop
  wallpaper (default), a flat solid colour, or a fixed custom image, set
  from Settings → Session and read from `~/.config/margo/mlock.conf`.
- **Settings → Gestures** — a dedicated section for touchpad and swipe
  gesture settings.
- **Dock settings page** (under Widgets) — configurable icon size, plus
  the groundwork for click-to-focus.
- **Notification history-menu size controls** — width and height are now
  adjustable in Settings.

### Changed

- **mlock UI refreshed** — modern vector icons (replacing emoji) and
  power-action chips.
- The Settings sidebar is now sorted alphabetically, with General kept
  first.

### Fixed

- **Clipboard performance** — the history menu is virtualized
  (`GtkListView`, the copyq pattern), rebuilds and the active-search are
  gated on reveal + debounced, and image thumbnails are `Arc`-shared so
  preview clones are O(1). Menu width and height once again track the
  configured values (both had regressed to a fixed size).
- **`pass` copy/type** honoured the wrong store, so nothing was copied
  despite the "Copied" toast — they now set `PASSWORD_STORE_DIR`
  explicitly (mshell runs as a systemd user service and doesn't inherit
  shell-rc env).
- Notification history rebuilds and the SSH-sessions active poll are now
  gated on reveal (no idle CPU when the menus are closed).
- The cellular icon recognizes 5G network-type variants (valent).
- The session display name is capitalized (margo → **Margo**).

## [0.8.0] – 2026-05-24

### Added

- **In-shell setup wizard** — the guided first-run flow is now a real
  layer-shell menu (never a floating window), opening contiguous with the
  bar like every other menu. Eight steps: Welcome, Theme (mode / preset /
  font scale / clock), Keyboard (xkb layout / variant / `xkb_rules_options`
  such as `ctrl:nocaps`), Touchpad (tap-to-click, natural scroll,
  disable-while-typing), Wi-Fi (scan + connect via `nmcli`), Wallpaper
  (with a sensible directory fallback so rotation always has a source),
  Bar (top / bottom), and a Review summary. Apply writes everything live
  and runs `mctl config reload`, so the keyboard layout/options take effect
  immediately — no logout — with an optional reboot offer. The base profile
  can be fresh defaults or a snapshot of the running config ("active").
  Reachable from Settings → Setup, the bar's **Setup** pill,
  `mshellctl wizard` (and the `mwizard` shim), and automatically on first
  launch when no profile is saved.
- **Microphone-mute key with an OSD** — `XF86AudioMicMute` /
  `mshellctl audio mic-mute` toggles the default audio source and pops the
  same bottom-centre pill as the volume keys (the input-side twin of the
  volume OSD, with the muted-mic glyph).
- **Settings overhaul** — an account section (`~/.face` avatar + user
  identity + picker), a sidebar search box that jumps to any section or
  widget page, widget gears that deep-link straight to their own page, an
  embedded **Setup** page, keyboard focus from first open, and a window
  size proportional to the screen resolution.
- **Settings → Fonts** — a monospace family slot, a global UI font scale
  and a separate bar-pill font scale, each with a live preview.
- **Settings → Display** — a GNOME-style drag-to-arrange monitor editor
  with visual mini-maps, backed by a new `mlayout outputs --json` live
  state (and hex colours in `mlayout list --json`).
- **Dashboard** — the Overview tile shows the pending-update count (its own
  throttled, always-visible probe) in place of the duplicated CPU
  temperature.
- **OSD** — the volume / brightness OSDs show the numeric level and are
  slimmer.

### Changed

- **Window glow is now a soft ambient halo** that sits *below* the content
  in the visual hierarchy — desaturated, low-opacity, roughly 5–10 % of the
  surface — instead of the previous neon-outline look.

### Fixed

- **The wizard's Wi-Fi dropdown spun the GTK main loop at ~100 % CPU.** A
  `#[watch] set_model` rebuilt the `StringList` every view pass, whose
  `selected-notify` fed back into the update cycle; it self-triggered at
  startup and starved every other reactive update (the margo-tags pill
  stopped tracking the active tag and window occupancy). The dropdown model
  is now held and mutated in place, so CPU returns to idle.
- The wizard menu now builds **lazily on first reveal** (its `nmcli` scan
  included), so it no longer adds startup cost on sessions that never open
  it.
- Wizard buttons use the canonical `ok-button-*` component classes per
  DESIGN.md — one accent region (`ok-button-primary`) per step, neutral
  actions on `ok-button-surface`.

## [0.7.9] – 2026-05-23

### Added

- **DESIGN.md §13–§14 — the interaction-philosophy layer** — eight binding
  subsections (cognitive load, the one-accent attention hierarchy, spatial
  logic, the responsiveness motion budget, surface ownership, density, state
  continuity, accessibility) plus a "visual restraint & identity" section.
  Codifies *why* the shell feels calm, not just which tokens to use.

### Changed

- **The whole shell now draws from the design tokens** — the last surfaces
  carrying hardcoded values were swept onto the token scales: the standalone
  **mlock** lock screen is themed from the full matugen palette (background /
  text / accent / danger) instead of a fixed scheme; the media-player bar
  cover, notification, clipboard, power and session widgets had their stray
  radius / motion / colour literals replaced with `--radius-*`, `--motion-*`
  and matugen colour vars. No off-scale radii, raw-millisecond transitions,
  or gruvbox hex left in the SCSS.

### Fixed

- **Browser screen-sharing works again (Meet window/screen pick)** — a
  regression from the lazy-menu refactor destroyed the screenshare picker's
  pending portal reply on the menu's first reveal, so the chooser never
  appeared and the browser saw an instant "cancel". The screenshare menu is
  now marked built when its widget is installed, so the lazy first-reveal
  rebuild can't wipe it.

## [0.7.8] – 2026-05-23

### Added

- **uwsm session, shipped by the package** — `margo-uwsm.desktop` plus the
  `margo-uwsm-session` / `margo-session` wrappers now install to
  `/usr/share/wayland-sessions` and `/usr/bin`, and `/etc/xdg/uwsm/env-margo`
  restores the standard XDG user-bin dirs (`~/.local/bin`, `~/bin`) onto the
  session `PATH` (uwsm rebuilds it from a POSIX login shell and would
  otherwise drop them). uwsm moves from optional to the default way to run
  margo; `uwsm` is now a runtime dependency.
- **Tracy profiling support** — building with `--features profile-with-tracy`
  now starts a Tracy client and marks frames, so the existing `span!`
  instrumentation actually records and a Tracy GUI can attach.

### Changed

- **Menus build lazily** — the menu pollers (network / IP / DNS / UFW /
  podman) and the entire menu content widget tree are now constructed on a
  menu's first reveal instead of eagerly at shell startup. A menu the user
  never opens does zero work: no GTK trees, no background polling, no
  `sudo` / subprocess probes.
- **`state.json` writes coalesced** — a burst of compositor state changes
  (one layout switch touches focus + windows + tags) now serializes the
  snapshot once per event-loop iteration instead of on every change.
- **One gtk-rs generation** — cairo/pango pinned to gtk4 0.10's 0.21 stack,
  dropping a duplicate 0.22 gtk-rs build (glib / gio / cairo / pango /
  pangocairo + proc-macros) for faster compiles.
- **Workspace is clippy-clean** — zero warnings, with a documented lint
  policy; the substantive lints were fixed rather than silenced.

### Fixed

- **Integrated polkit agent now works** — mshell-polkit registers for the
  logind *Display* session instead of the unset `$XDG_SESSION_ID` it sees
  under the systemd user manager, so it actually receives authentication
  requests; and the password dialog wraps to fit its window.
- **Twilight toggle reflects state** — `state.json` is refreshed on every
  `mctl twilight` change, so the night-light button no longer looks stuck
  "on" after toggling.
- **Network / IP / DNS / UFW / podman menus populate on open** — those
  menus now receive the reveal signal that drives their first fetch.
- **Client misbehavior can't crash the session** — the layer-shell and
  screencopy handlers degrade to a logged no-op instead of panicking the
  whole compositor on a map failure or a destroy/copy race.

## [0.7.7] – 2026-05-23

### Added

- **DESIGN.md §12 — the "panel archetype"** — a binding spec for
  spacious, app-like menu surfaces: panel surface metrics, a reusable
  header (leading glyph + SemiBold title + circular actions), a
  segmented control, a pill query field, and lightweight content rows.
- **Reusable `MenuWidget::PanelHeader`** — `[icon] title` + a live date
  + a settings gear. The dashboard leads with it in place of the old
  Clock hero, and it's the shared implementation behind every menu's
  header.
- **Clipboard panel (Phase 2)** — segmented type tabs
  (All · Text · Images · Files · ★) with live counts, relative
  timestamps, a pill search field, copy/pin toasts, and a panel-density
  setting (comfortable / compact).
- **System Updates — its own Settings page** — menu size/position plus a
  "check every N hours" interval and per-source toggles
  (repo / AUR / Flatpak).
- **New MargoMaterial symbolic icons** — refresh, copy, open-in-new,
  and a cube glyph.

### Changed

- **§12 panel header rolled out everywhere** — clipboard, dashboard, and
  every menu (UFW, DNS, Podman, Notes, Power, Valent, Network,
  Bluetooth, Audio, CPU, Public IP, Keep Awake, Notifications, Margo
  Layout, Twilight, Media Player, Weather, Screenshot, Wallpaper, Theme
  Picker, SSH Sessions, Keybinds, Session, Screen Record, Screen Share,
  System Updates) plus the Settings window sidebar, all sharing the
  `.panel-header` / `.panel-title` / `.panel-action-btn` chrome. Header
  titles settled at a calm `--font-md` (16) SemiBold.
- **System Updates no longer re-probes on every restart** — the last
  result is cached to disk and the check runs once per configured
  interval (deduplicated across monitors); the panel re-probes only when
  opened. This stops the AUR helper — and its `sudo` — firing on each
  shell start.

### Fixed

- **Dashboard columns** — left and right columns end at the same bottom
  edge again (the Weather anchor stretches to fill its column).
- **Clipboard** — Tab cycles past an empty type tab instead of sticking;
  timestamps and the per-row trash sit at the dim hint tier; the search
  reads as a proper pill query surface.
- **Bluetooth** — the §12 header sits on the panel surface, not the
  tile-card colour.

## [0.7.6] – 2026-05-22

### Added

- **mshelldash** — a standalone tabbed dashboard
  (Overview · Media · Weather · Wallpaper · System), rebuilt on margo's
  DESIGN.md language and coexisting with the classic dashboard. The
  Overview tab is a live mosaic (clock hero + a `/proc`-sampled system
  glance); the other tabs reuse the existing menu-widget components so
  they stay in sync. Open it with `mshellctl menu mshelldash [tab]` to
  land straight on a named view.
- **CPU dashboard enrichment** — current frequency, CPU model / core /
  thread identity, a history sparkline, a user vs. system load split,
  and memory detail; the bar pill gained per-metric glyphs (a new
  processor-chip and RAM-module symbolic icon ship in MargoMaterial,
  plus the thermometer), recoloured by the calm/warn/danger ladder.
- **Weather** — a standalone bar pill + menu; the standalone menu is the
  full all-in-one Current / Hourly / Daily surface, today's high / low
  rides on a right-click toggle in the pill, a dedicated Weather page in
  Settings, and clearer city / district querying.
- **Twilight** — the menu now surfaces each preset's actual values
  (colour temperature + time), not just its name.
- **Notifications** — configurable popup-toast width, a read / unread
  bell dot in the bar (unread → error dot, seen history → muted dot),
  and toggleable history grouping.
- **Bar** — a unified, configurable pill hover strength; the keep-awake
  pill gained a 24 h preset; the bluetooth pill shows the connected
  device name.
- **Dashboard** — wallpaper + screenshot quick-action buttons.
- **mlock** — consolidated as margo's single lock screen: always-visible
  matugen accent (card border + avatar ring), media keys and a
  keyboard-layout indicator on the lock surface, and a bare
  `mshellctl lock`.

### Changed

- **Sweeping DESIGN.md conformance pass** across the shell: the Settings
  window was rebuilt on shared chrome; dashboard / clipboard / launcher /
  theme-picker / wallpaper widgets were aligned to *surfaces over
  borders*, the `--font-*` size scale, matugen tokens, and one canonical
  hover; a single radius rule (scale for widgets, config for window
  chrome); and scattered `px` / `em` font sizes were tokenized.
- **DESIGN.md** itself was extended to codify the scrollable-list /
  footer ("dark band"), label-toggling-button, and read/unread-marker
  rules so they don't regress.

### Fixed

- Settings panel corners are now clipped via `set_overflow` (GTK4
  ignores CSS `overflow` on a `GtkBox`).
- Dropped the dark background band below the UFW / Podman rule lists.
- DNS preset Apply / Active buttons keep one width across both states.
- Audio-dashboard / Bluetooth revealer rows are centred and slimmer.
- Reaped keybind-spawned child processes (no more zombie pile-up) and
  stopped zombie `mlock` procs from wedging `lock_session`.
- Weather menu is registered in Settings → Menus; Valent pill icon
  states; assorted clipboard spacing fixes.

## [0.7.5] – 2026-05-21

### Changed

- **License set trimmed + reorganised to match the actual code lineage.**
  Removed the wlroots / tinywl / sway license files — margo is a
  pure-Smithay Rust compositor and carries no code derived from any of
  them (audit confirmed: 0 derived files; the only mentions are
  interop/behaviour comments, and the one shipped wlr protocol XML
  self-attributes its own copyright). Added attributions that were
  actually missing: **niri** (GPL-3.0-or-later) for the three protocol
  files ported in 0.7.4, and **noctalia** (MIT) for the mshell widgets
  that reimplement noctalia patterns. The project's own `LICENSE` stays
  at the repo root; all upstream attributions (mango, dwl, dwm, OkShell,
  niri, noctalia) now live in a **`licenses/`** directory. `PKGBUILD`,
  `install.sh` and the post-install smoke test ship/verify the whole
  set; README / CONTRIBUTING updated to match.

## [0.7.4] – 2026-05-21

### Added

- **Three Wayland protocols**, closing the gap with mango / Hyprland on
  the set wlroots compositors get for free:
  - **`zwlr_foreign_toplevel_manager_v1` (write-side).** Taskbars and
    docks can now *act* on toplevels — activate, close, (un)fullscreen —
    not just list them. Runs alongside the existing read-only
    `ext-foreign-toplevel-list-v1`; activate jumps to the window's tag
    and focuses it. The mshell active-window pill becomes clickable.
  - **`ext_workspace_v1`.** The standardized workspace protocol, so
    shells that don't speak dwl-ipc (sfwbar, ironbar, …) can show
    margo's tags. Each output is a workspace group with 9 fixed
    tag-workspaces; "active" mirrors the monitor's tag bitmask. dwl-ipc
    still runs in parallel.
  - **`zwlr_virtual_pointer_manager_v1`.** Synthetic pointer injection —
    companion to the existing virtual-keyboard. `wtype --click`, remote
    desktop and accessibility tools can drive the cursor, buttons and
    scroll through margo's normal input path.

  margo now advertises **~57 Wayland globals — ahead of mango (~53) and
  niri (~41)**, pursuing Hyprland (~67). See `docs/protocol-comparison.md`.

- **Configurable notification buttons.** Toast action buttons and the
  close button are now toggled from **Settings → Notifications** (default:
  action buttons off, close button on), trimming the default toast.

### Fixed

- **Stuck notification toast.** A dismissed toast could linger as a
  half-collapsed remnant (a dangling "View" button) because the
  always-visible layer-shell overlay kept its last committed frame. The
  popup surface now hides when the list empties, so the compositor drops
  it — matching the mshell-osd lifecycle.

### Notes

- **Test coverage** expanded: integration tests for the 14 tiling-layout
  algorithms, window-rule parsing, the mshell-config YAML schema, and
  multi-output placement. CI runs the full suite in an Arch container.
- **Three protocols remain unadvertised** — `zwlr_output_power_management_v1`,
  `wp_tearing_control_v1`, `wp_drm_lease_device_v1` — blocked by smithay
  capability (no tearing / async page-flip), margo's State/backend split
  (drm-lease), or untested-DRM risk (DPMS). Tracked in `road_map.md` §15.10.

## [0.7.3] – 2026-05-21

### Added

- **Cross-distro `install.sh`.** A single self-contained installer at
  the repo root that detects the distribution and builds, installs, and
  uninstalls margo. On **Arch / CachyOS** it builds via the repo
  `PKGBUILD` with `makepkg` and installs with `pacman` (uninstall =
  `pacman -R margo-git`). On **Debian / Ubuntu** it installs the apt
  build deps, bootstraps a current Rust via `rustup` when the system one
  is too old, builds `gtk4-layer-shell` from source when it isn't
  packaged, compiles the workspace, and installs to `/usr` — recording
  every path in a manifest so `uninstall` removes exactly what was
  added. Validated end-to-end on Ubuntu 24.04.3.

### Notes

- **Ubuntu requires GTK ≥ 4.20.** margo's gtk4-rs (0.10) needs GTK 4.19+,
  so Ubuntu 24.04 LTS (GTK 4.14) cannot build the shell, and `apt
  upgrade` won't change that for the LTS lifetime. Use Ubuntu 25.10+ /
  26.04 LTS or any distro with GTK 4.20+. `install.sh` checks the GTK
  version up front and stops early with a clear message. README install
  section rewritten around the installer; design in
  `docs/install-script.md`.

## [0.7.2] – 2026-05-21

### Added

- **Native Screenshot portal.** `margo-portal` now serves
  `org.freedesktop.impl.portal.Screenshot` in addition to
  `ScreenCast`, driving the compositor's `org.gnome.Shell.Screenshot`
  shim. The shim's screenshot path was previously a stub that always
  failed; it now captures the desktop via `grim` to a temp PNG
  (asynchronously, off the compositor event loop) and returns a
  `file://` URI to the requesting app.

### Changed

- **gnome-portal-free sessions.** `margo-portals.conf` no longer
  routes any interface to `xdg-desktop-portal-gnome`:
  `ScreenCast=margo`, `Screenshot=margo`, `RemoteDesktop=none`,
  `default=gtk`. `Secret` stays on `gnome-keyring` (the standalone
  freedesktop secret daemon — not the gnome portal). `margo.portal`
  advertises `Screenshot` with `UseIn=margo`, and the wayland-session
  entries declare `DesktopNames=margo` (dropping `mango;wlroots`) to
  keep portal config resolution unambiguous.
- **Packaging.** `xdg-desktop-portal` and `xdg-desktop-portal-gtk`
  are now hard dependencies; the `xdg-desktop-portal-gnome` and
  `polkit-gnome` optdepends were dropped.

## [0.7.1] – 2026-05-21

### Added

- **Twilight (blue-light filter) UI.** Bar pill + quick-control
  panel — master toggle, live temperature / phase / mode readout,
  source-mode selector (Auto / Manual / Static / Schedule), a
  temperature slider that previews live, and schedule-preset
  chips. A live schedule-preset editor lands in Settings →
  Display. In Schedule mode the preset whose time slot is current
  is tinted with the accent. `mshellctl menu twilight`.
- **Keyboard-shortcuts cheatsheet.** Bar pill + searchable menu
  built live from `config.conf` `bind =` lines (following
  `source` includes), grouped by action category with
  colour-coded modifier chips. `mshellctl menu keybinds`.
- **Valent Connect.** Bar pill + panel — paired-phone battery /
  connectivity status plus find / ping / browse / share / pair
  actions. Ported from the noctalia plugin.
- **System Updates.** Pill + panel listing pending updates grouped
  by source (repo / AUR / Flatpak) with Refresh + Update and
  per-source toggles in Settings.
- **Keep Awake (idle inhibitor).** Bar pill + duration-picker menu
  (30 m / 1 h / … / ∞) with a live countdown and quick-extend.
- **Audio dashboard.** Combined output+input bar pill — scroll the
  icon to change volume — opening a revealer-row menu with
  sliders, mute toggles, and device pickers for both sides.
- **CPU + Power pills.** Combined CPU dashboard pill + per-core /
  RAM / load menu; combined Power pill (profile + battery) with
  right-click cycle, plus a battery-only mode.
- **Network Console.** Live activity arrows and TX/RX traffic
  graphs on the pill + panel; default ↓/↑ speed readouts coloured
  per direction.
- **Clipboard history.** Persistence, favorites / pin,
  sensitive-skip, All / Favorites tabs, keyboard navigation,
  vim-style `/` search, debounced writes, configurable entry
  limit. Persists the full history by default.
- **Notifications.** Album art / app icon, click-to-open,
  swipe-to-dismiss, action icons, body markup, 2FA-code copy,
  per-app grouping, mute blocklist, and `mshellctl` DND / count.
- **Launcher startup scripts.** User-managed autostart list — add
  by name, per-script enable toggle + startup delay, delete.
- **Dashboard widgets.** CompactAudio, Connectivity, OverviewIntel
  and SystemStatus tiles; equal-width two-column body with a
  last-child-fill so bottom cards match.
- **Bluetooth** standalone layer-shell menu.
- **matugen-driven window borders.** The compositor's border
  colours now follow the shell's palette: mshell generates
  `~/.config/margo/colors.conf` from the active scheme and
  `mctl reload`s on change. `source` it from `config.conf` to
  override the static border colours (`rootcolor` stays static).

### Changed

- **Material-3 design language.** Sweep across bars, menus, the
  dashboard, and the Settings window, codified in
  `mshell-frame/DESIGN.md`.
- **margo tag / layout widgets** redesigned to the M3 spec —
  active-indicator pill, tag occupancy as an accent digit.
- De-`n`-prefixed the ported plugin widgets and removed six
  redundant standalone bar pills.
- Twilight temperature slider now reads whole kelvin.

### Fixed

- **Twilight:** presets now appear in the menu, the mode no longer
  reverts on a poll race, and the temperature slider is visible
  (it was an unstyled, invisible `GtkScale`).
- **Notifications:** resolve icon-name `image-path` hints (and the
  `-symbolic` fallback) so `notify-send -i <name>` icons render
  instead of a broken-image placeholder.
- **Keep Awake** countdown was invisible on the active pill.
- **Frame:** re-place menus whose anchor was changed in Settings.
- Valent file-share crash + clearer no-connectivity state; Settings
  ghost Battery / PowerProfile pills; dashboard CompactAudio
  percentage and Bluetooth-connected detection.

## [0.7.0] – 2026-05-18

### Added

- **`mwizard` — first-launch setup wizard.** New top-level
  binary that opens when no
  `~/.config/margo/mshell/profiles/default.yaml` exists and
  walks the user through 5 pages: Welcome → Theme
  (Dark/Light matugen + 24h/12h clock) → Keyboard (xkb
  layout from 14 common codes, defaults from `$LANG`, with
  free-form layout + variant overrides) → Wallpaper
  (FileDialog picker) → Done. Writes both `default.yaml`
  (full shell profile) and `~/.config/margo/config.conf`
  (surgical `xkb_rules_*` line patch). `mwizard --force`
  re-runs even when a profile exists. No-flag `mwizard` is a
  no-op when a profile is already there, so it's safe to
  hook into a session-start script.

- **`mpicker` — native colour picker binary.** Replaces the
  hyprpicker fallback that used to ship in `mshell-utils`.
  Frozen wlr-screencopy + 10× zoom lens overlay + hex chip;
  CLI supports `--autocopy`, `--notify`, `--no-zoom`,
  `--format hex/rgb/hsl/cmyk`, `--lowercase-hex`,
  `--quiet`. Bundled in the `margo-git` package; mshell's
  launcher ColorPicker button calls `mpicker` directly with
  no hyprpicker fallback.

- **Dashboard menu — compound clock + calendar + weather +
  quick settings.** Two-column 860 px panel: left holds
  Clock hero / CalendarGrid / Weather / MediaPlayer; right
  is a verbatim clone of the standalone Quick Settings
  widget stack (Network / Bluetooth / Audio Out / Audio In /
  Power Profile + the two QuickActions rows). Columns
  equalised at 400 px each. Open via `mshellctl menu
  dashboard` or the new Dashboard bar pill.

- **Dashboard bar widget.** New `BarWidget::Dashboard` —
  Clock-style pill that shows the current time/date using
  the shared `[tempo]` chrono format list. Left-click
  toggles the dashboard menu, right-click double-press
  cycles formats.

- **Settings → Widgets → Dashboard + Settings → Menus →
  Dashboard.** Both entry points are now in the Settings
  UI; Menus page covers position / min-width / max-height /
  widget list.

- **Per-menu `maximum_height` config.** Spinbox in the
  Menus settings page caps the vertical viewport of any
  menu; scroll-past behaviour kicks in when the content
  overflows.

- **Launcher right-click context menu — Pin/Unpin +
  Hide/Unhide.** New `HiddenStore` mirrors `PinStore` at
  `~/.cache/margo/launcher_hidden.json`. Hidden items only
  appear in search results (non-empty query), suppressed
  from empty-query browse mode so the user can curate the
  at-a-glance app pile without losing the ability to type
  the app name. Right-click menu auto-suppresses on rows
  without a `usage_key` (calculator, command palette).

- **`mshellctl menu app-launcher --tab <name>` and
  `--list-tabs`.** Open the launcher pre-selected on a
  category (All / Run / System / Insert / Search /
  Compositor / Connect). `--list-tabs` prints the known
  categories.

- **mscreenshot region selector via mshell IPC.**
  `mscreenshot area` now bridges to mshell's rich in-shell
  selector (preview state, snap, aspect info) when mshell
  is running, drops to the bare `slurp` overlay otherwise.
  New IPC method `SelectRegion` returns the picked geometry
  to the CLI.

- **Screenshot widget UX polish.** Area selector grew
  preview state (Enter to commit, arrows to nudge,
  Shift+arrows for 10 px jumps), aspect-ratio chip,
  snap-to-window helper, Ctrl+S / Ctrl+E shortcuts that
  override the commit target. Inline annotate path now
  prefers `satty` over `swappy` to match the rest of the
  workspace.

### Changed

- **`menu_settings.rs` collapsed 4041 → 394 LOC** via a new
  `MenuConfigPanel` sub-component + extended `MenuKind` with
  Notifications/Wallpaper variants + `read_widgets` /
  `tracked_widgets` / `write_widgets`. Adding a new menu to
  the aggregate Menus page is now ~10 lines instead of
  ~250.

- **HyprPicker → ColorPicker rename across the workspace.**
  Drops the Hyprland brand from a margo-native helper that
  was never tied to Hyprland after the mpicker port. No
  serde aliases — user YAMLs need a one-time
  `s/HyprPicker/ColorPicker` sweep; the in-tree default
  profile is already migrated.

- **App launcher row padding tightened 8 → 1 px** so each
  app visually reads as 2 lines (name + description)
  instead of the previous "blank / name / desc / blank"
  4-line feel.

- **`mshell-config::paths` module is now `pub`** so external
  binaries (mwizard, future tools) can resolve the profile
  path without re-deriving the layout.

- **mshell + launcher caches honour `$XDG_RUNTIME_DIR`**
  instead of `/tmp/` for fallback paths — per-user,
  race-free on shared machines.

### Fixed

- **Compositor zbus/tokio panic at session start.**
  Packaging bug: a single `cargo build -p margo -p ...`
  invocation that included mpicker pulled in
  `mshell-screenshot` → `mshell-services` → `wayle-*` →
  `zbus[tokio]` via Cargo's per-invocation feature
  unification. The compositor linked against the
  tokio-enabled zbus and panicked at `start_object_server`
  ("there is no reactor running, must be called from the
  context of a Tokio 1.x runtime"). Fix: PKGBUILD puts
  mpicker in the shell-side invocation alongside mshell so
  the compositor's zbus stays `async-io`-only.

- **Dashboard right column wasn't rendering card chrome.**
  `MenuModel`'s `css_class` field stored
  `"quick-settings-menu dashboard-menu"` as a single literal
  class name (`set_css_classes` was being passed the whole
  string as one entry), so `.quick-settings-menu
  .network-menu-widget` descendant selectors never matched.
  Split on whitespace post-`view_output!`.

- **matugen "no progress, hangs forever" bug.** The
  output-log drainer thread used `.flatten()` on a
  `BufReader::lines()` iterator; a single persistent IO
  error spun the thread forever instead of bailing.
  Switched to `.map_while(Result::ok)`.

- **PKGBUILD bundled bin list complete.** mwizard + mpicker
  now appear in both the build invocations and the install
  loop. Ldd verification comment refreshed.

### Engineering

- `cargo clippy --fix --workspace` sweep — 37 auto-fixes
  landed.
- `CODE_REVIEW.md` — full audit report (Critical / High /
  Medium / Low findings) added at repo root, with a status
  table tracking which findings landed in this release.
- Production `unwrap()` audit (`#187`) — case-by-case
  review confirmed the flat-grep count of 265 prod unwraps
  was largely a false alarm; 80 %+ are framework
  guarantees (Mutex::lock in single-threaded GTK, GTK
  downcast_ref, const-string parsing, PipeWire format
  invariants). `capture.rs` documented its in-vec
  invariant via `expect()`.

## [0.6.3] – 2026-05-17

### Added

- **Power-user keyboard bindings for the launcher.** Walker- and
  noctalia-inspired shortcuts that work out of the box, no config
  required:
  - **Ctrl+1..Ctrl+9** — activate the Nth result (no arrow keys).
  - **Ctrl+Shift+P** — toggle pin on the selected item. Pinned
    items rank at the top of every browse pass with a ★ marker,
    saved to `~/.cache/margo/launcher_pins.json`. Ctrl+Shift+P
    rather than plain Ctrl+P so the emacs "previous selection"
    binding stays intact.
  - **Tab / Shift+Tab** — cycle through provider categories
    (Apps → Compositor → System → Run → Insert → Search → Connect
    → All).
  - **Delete** — drop the selected frecency / history entry
    (Apps frecency forget, Command history `forget(expr)`,
    Scripts frecency forget). Provider opts in via `can_delete`.
  - **Ctrl+E** — toggle fuzzy ↔ exact-substring matching. Visible
    `~/=` chip indicator next to the search entry.
  - **Ctrl+R** — repopulate the search entry with the last query
    the launcher saw before closing.
  - **Ctrl+Enter** — run the provider's alt action: Apps launch
    in `$TERMINAL` with a "press enter to close" shell wrapper;
    Websearch copies the resolved URL to the clipboard.
  - **PageDown / PageUp** — jump 10 rows at a time.

- **Category tab strip** — small pill row above the result list,
  one pill per provider category with an icon + label pair.
  Selected pill picks up the primary accent; tooltip shows the
  full category name on hover. Pills also accept direct mouse
  clicks. Mappings: All → view-grid, Apps → app-grid,
  Compositor → display, System → preferences-system, Run →
  terminal, Insert → input-keyboard, Search → search, Connect →
  server.

- **Walker-style keybind hint footer.** Small chip strip at the
  bottom of the launcher that lists the currently-relevant
  shortcuts. Always-on chips (↵ Activate / Ctrl 1-9 Quick / Tab
  Categories / Ctrl E Exact / Ctrl R Last / Esc Close) anchor
  the strip; contextual chips (Ctrl ↵ Alt action / Ctrl ⇧ P
  Pin·Unpin / Del Remove) only render when the selected row
  actually supports the action.

- **`Provider::browse(filter)` trait method.** Lets prefix-only
  providers (Symbols, Emoji, Clipboard, Scripts, Tags, Bluetooth,
  Wireplumber, Playerctl, ProviderList, Ssh, Command) fill their
  category tab with real content — typing inside the tab also
  narrows by filter. Default impl falls through to `search(filter)`
  so providers like Apps / Calculator / Websearch work unchanged.

- **`Provider::can_delete(item)`, `delete_item(item)`, `alt_action(item)`,
  `category()`** — four new optional trait methods that drive the
  Delete / Ctrl+Enter / Tab strip features. All have sensible
  defaults so existing providers compile without edits.

- **`DisplayItem` wrapper** — runtime-stamped decorations
  (pinned flag, quick-key digit) handed to the UI. Providers still
  emit raw `LauncherItem`; the runtime wraps each one after
  scoring so the prefix-only providers don't need to know about
  pins or quick keys.

### Changed

- **Launcher redesign — clock-menu visual language.** The launcher
  now reads like the rest of the mshell card stack rather than the
  previous flat list:
  - Search header: bigger entry font, accent ring on focus (mirrors
    the calendar-hero day number tone), small chip-style fuzzy/exact
    badge.
  - Result list: deep margo-tone card (`--surface-container-lowest`
    + 1 px `--outline-variant` border) — clear "well inside the
    panel" effect instead of the previous mid-grey wash that
    blended into the menu surface on the tight Margo palette.
  - Result rows: transparent default, hover picks up
    `--surface-container-high`, selected row flips to `--primary`
    with icon + label + quick-key + ★ all reflowing to
    `--on-primary` so contrast survives the tint swap.

- **Default browse pipeline** — runtime tracks an `active_category`.
  Selecting a specific tab bypasses `handles_search` and calls
  `browse(filter)` on every provider in the category, so the
  prefix-only providers actually contribute to their tab. The
  All tab keeps the standard search pipeline. Runtime also adds
  a name+description substring post-filter for category-tab mode
  so providers that don't filter themselves still respond to typing.

### Fixed

- **Insert tab icon** — switched from `format-text-symbolic`
  (missing in MargoMaterial → rendered as missing-icon glyph) to
  `input-keyboard-symbolic` which exists in MargoMaterial / kora
  / breeze / Adwaita.

- **Bind hint chips repaint contextually** — selecting a calculator
  result drops Pin / Remove / Alt-action; selecting a pinned app
  flips the chip label from "Pin" to "Unpin" automatically.

## [0.6.2] – 2026-05-17

### Added

- **mshell-launcher — provider-based app launcher with 19 providers.**
  The legacy single-purpose `AppLauncher` widget is replaced by a
  provider runtime + a uniform 0–200 scoring scale so results from
  every source interleave cleanly. Providers ship in two crates:

  Compositor-independent (in `mshell-launcher`):
  - **Apps** — fuzzy-search desktop entries, frecency-boosted.
  - **Calculator** — inline math via `evalexpr`, `2+2` → `4`, `sqrt(2)` etc.
  - **Session** — Lock / Logout / Suspend / Reboot / Shutdown.
  - **Settings** — jumps directly to a Settings sidebar section.
  - **Command** (`>cmd echo hi`) — run a shell command, history-aware.
  - **Scripts** (`>start brave`) — fuzzy-launch `start-*` scripts from
    `$PATH`, frecency-boosted.
  - **Clipboard** (`>clip`) / **Clear** (`>clear`) — history browse / wipe.
  - **Symbols** (`.arrow`) — Unicode special chars (→ ± π …).
  - **Emoji** (`:smile`) — keyword emoji picker.
  - **Websearch** (`g`/`y`/`ddg`/`gh`/`aur`/`arch`/`wiki`) — open the
    query in the default browser.
  - **ProviderList** (`;`) — discoverable cheatsheet of every prefix.
  - **Playerctl** (`player`) — MPRIS play / pause / next.
  - **ArchPkgs** (`p`) — Arch / AUR package search.
  - **Wireplumber** (`audio`) — sink / source switcher.
  - **Bluetooth** (`bt`) — bluez 5.65+ paired-devices picker.
  - **Ssh** (`ssh <host>`) — opens `$TERMINAL -e ssh <name>` against
    hosts in `~/.ssh/assh.yml` (assh format).

  Compositor-aware (in `mshell-frame`, pull `mshell-margo-client`):
  - **Windows** (`win [query]`) — alt-tab-style open-window switcher.
  - **Mctl** — margo compositor quick-actions (wallpaper next, twilight,
    screenshot region, …).
  - **Tags** (`tag [N]`) — switch focused output to tag N (1–9), with
    glyph indicators ● active / ◐ occupied / ○ empty.

  Cross-cutting:
  - Frecency cache at `~/.cache/margo/launcher_usage.json` (boost
    `5·log2(1+count)` applied in both browse and command-mode dispatch).
  - Command history cache at `~/.cache/margo/launcher_command_history.json`.
  - Toast notification on activation (visual feedback).
  - `>` palette enumerates every provider's `commands()`.
  - **Launcher** settings page with cache-clear buttons + scripts list.

- **mshell-auth: real PAM authentication.** Shared `libpam` FFI extracted
  from mlock so mshell-lockscreen can actually unlock. The previous PAM
  stub always failed; mlock's libpam wiring now lives in a shared
  `mshell_auth::pam` module (avoids the bindgen/clang-sys problem the
  `pam-sys` crate has on Arch).

- **Margo layout switcher — in-frame menu (rewrite).** Replaces the
  legacy in-bar `gtk::PopoverMenu` (xdg_popup, detached feel) with a
  regular menu surface that slides out from the bar like every other
  menu in mshell.

- **mshell-settings — Menus promoted to top-level sidebar entry.**
  Previously buried under a sub-section. Tab / Up / Down now walk the
  left sidebar.

### Changed

- **mshell-margo-client: `Reactive::watch()` snapshot-on-subscribe.**
  New subscribers receive the current value before any subsequent
  `set()` broadcasts (BehaviorSubject semantics). The old change-only
  stream missed the single startup `set()` for the workspaces vec
  (margo's `tag_count = 9` is fixed, so the membership never changes
  in steady state) — symptom was bar widgets that sat empty until the
  user opened a window. The watcher now fills in on the first scheduler
  tick.

- **mshell-margo-client: inotify-based `state.json` watch.** Replaces
  the 250 ms steady-state poll with kernel-driven wakeups (notify v9,
  parent-dir watch so atomic-rename writes survive). A 2 s polling
  loop stays as a safety net (init failure, parent dir not yet
  created). Idle CPU drops since mshell is no longer waking up 4×
  per second forever.

- **mshell-style: compile-time baseline = Margo brand palette.** SCSS
  `_colors.scss` ships the Margo (Dracula-style) palette instead of
  Everforest so the first paint on first login matches the steady
  state.

- **mshell-style: matugen output cached to disk.** Last successful
  matugen CSS is atomically written to `~/.cache/mshell/last_theme.css`
  and loaded synchronously at startup. On every login after the first
  there's no theme flash — the cached palette paints from frame one
  and the async matugen run that follows is visually a no-op.

- **margo: state.json `active_output` is now pointer-first.** Previously
  the field tracked the focused-client's monitor, which left it stuck on
  the old output when the user moved the cursor (or ran `focusmon`) to
  an empty monitor. Mshell's IPC handler then routed `Super+Space` to
  the wrong frame. The field now follows the pointer monitor and
  refreshes on cursor crossings + `focusmon` dispatches.

- **mshell: settings deep-navigation race + +1 pt fonts + scale slider.**
  Launcher → Settings → specific section no longer slams Settings shut.
  All Settings fonts wrapped in
  `calc(Npx * var(--font-scale-settings, 1.0))` so the Settings →
  General "Settings font scale" slider rescales the panel dynamically.

- **mshell: symlink-preserving config writes.** Writing through `dcli` /
  `stow` / `chezmoi` symlinks no longer replaces the link with a
  regular file.

- **margo_tags widget cleanup (~150 lines).** Now that the underlying
  Reactive race is fixed (BehaviorSubject + inotify + `focused_idx`
  null-parse), the five-layer belt-and-suspender stack (cold-start
  poll, brute-force timer, bootstrap_rows fallback, …) is gone. One
  clean subscriber loop.

### Fixed

- **The big one — mshell-margo-client parses `null` `focused_idx`.**
  Root cause of "tag pills on the bar stay empty until the first
  window opens, *every login*". At session start margo writes
  `focused_idx: null` (no client focused yet); the old schema required
  `i64` and rejected the whole document with `invalid type: null,
  expected i64`. `apply_snapshot()` never ran, `service.workspaces`
  stayed empty, and the snapshot-on-subscribe stream yielded an empty
  vec. Opening any window flipped `focused_idx` to a real integer →
  parse OK → 9 workspaces published → pills appeared. Now declared
  `Option<i64>` with an explicit deserializer documenting the wire
  shape. **Verified end-to-end after a fresh reboot:** 9 pills paint
  before any window opens.

- **mshell-launcher: dispatch fires for every prefix, not just `>`.**
  Symbols (`.`), Emoji (`:`), `;`, `audio`, `bt`, `player`, `p`, `ssh`,
  `tag`, `win` previously hit no provider and silently returned nothing.
  Every provider's `handles_command()` now participates in dispatch.

- **mshell-launcher: frecency boost applies in command-mode too.**
  `>start brave` no longer always sorts alphabetically.

- **mshell-launcher: `bt` prefix works with bluez 5.65+.** Upstream
  removed the `bluetoothctl paired-devices` subcommand; try
  `devices Paired` first, fall back to the old form.

- **mshell-launcher: `lock` actually locks.** Session provider routes
  through `mshellctl menu session lock` (mshell's in-process session
  dispatcher); the old `loginctl lock-session` is a no-op under margo
  (no logind session-locking integration).

- **mshell-launcher: `;` cheatsheet click writes to the search entry.**
  Previously updated the internal filter without touching the visible
  `GtkEntry`.

- **mshell-launcher: Settings deep navigation race.** Activating
  `settings:display` no longer races `CloseMenus` against
  `OpenSettingsAtSection`.

- **mshell-ndns: probe accumulates DNS from Global + per-link.** Old
  parser only read Global; DNS servers configured on a single link
  were missed. Presets now apply via `nmcli con mod` + `up`.

- **mshell: DNS preset active highlight + 8 layout icons.**

- **mshell: session menu Tab navigation.** Tab cycles entries; key
  controller attached to the menu's root so focus traversal works even
  before any child has been clicked.

### Removed

- **mshell-launcher: stale `set_on_activated` from AppsProvider.** Dead
  code from an earlier provider trait shape.

## [0.6.1] – 2026-05-16

### Added

- **mshell bar pills — A1, A2, A3, A6, A7, A9 + B6 shipped.**
  - **A1 Privacy** — mic + camera in-use indicator with PipeWire
    backend; pill lights up while any app holds the device.
  - **A2 SysStat** — CPU / RAM / Temp / GPU pills with configurable
    poll cadence; matches the bar pill density.
  - **A3 LockKeys** — Caps / Num / Scroll lock state via libinput;
    discrete on/off rendering, no flicker.
  - **A6 DarkMode** — light/dark toggle pill that flips the GTK
    color-scheme preference + persists across sessions.
  - **A7 KeepAwake** — idle-inhibit toggle pill backed by
    `ext_idle_notify_v1`; the bar shows a coffee glyph while
    active.
  - **A9 Screen Corners** — per-monitor rounded overlay, off by
    default, exposed in Settings → General. Matches GNOME / macOS
    rounded display edges.
  - **B6 System Update** — package-manager update count badge with
    right-click refresh, configurable polling cadence, fixes a
    pre-shipped exit-1 false-error.
- **mshell A5 Calendar** — noctalia-style calendar grid inside the
  clock menu, locale-aware day-of-week header, week numbers.
- **mshell Dashboard menu** — clock + weather + quick-settings
  composed into a single panel (hero + 2-col grid + power
  footer); replaces the old triple-menu pattern.
- **mshell quick-settings** — card stack matching the clock-menu
  visual language; rows surface real per-toggle state at-a-glance
  instead of opaque labels.
- **mshell S1 — Settings embedded in frame menu stack.** Settings
  no longer pops a separate window; lives in the same panel as
  other menus, sharing the frame's animation pipeline.
- **mshell-osd — network change OSD + Settings toggle**, lowered
  to a noctalia-style 320 px wide pill.
- **mshell Settings — alphabetic sidebar + Widgets group.** Sub-
  sidebar surfaces every pill + menu individually; Bar moves to
  top level; Display gains a Layout sub-page that drives mlayout;
  Fonts gets its own entry.
- **Twilight Schedule mode** — multi-step time-of-day preset
  schedule with sunsetr-compatible TOML preset files under
  `~/.config/margo/twilight/`. Reads `schedule.conf` (HH:MM →
  preset name) and interpolates in mired space between
  consecutive presets. First-run seeds a starter set of six
  presets the user can edit.
- **`mctl twilight preset`** subcommand family — `list`, `show`,
  `set <name> <K> [%]`, `remove`, `schedule set <HH:MM> <name>`,
  `schedule remove <HH:MM>`. Writes the TOML / schedule.conf
  files directly, then best-effort dispatches `reload_config` so
  the change is live immediately.
- **`mctl twilight set mode=geo|manual|static|schedule`** — the
  `mode` field is now live-tweakable from the CLI (previously
  only the six numeric fields).
- **mshell Settings → Twilight — Open presets folder** button
  (xdg-open shortcut into `~/.config/margo/twilight/`), plus
  stronger hint text pointing at the new `mctl twilight preset`
  family.
- **mscreenshot — three new options:**
  - `--delay N` / `-d N` global flag: pop a notification, wait N
    seconds, then capture. Catches menus / tooltips that close
    when focus moves to a selector.
  - `--output NAME` / `-o NAME` global flag: pin screen-capture
    modes (`screen` / `sc` / `sf` / `si` / `sec`) to a specific
    output regardless of focus.
  - Notification action buttons after save (Open / Show in
    folder / Delete) — spawns a detached `mscreenshot
    notify-handle` helper that drives `notify-send --wait
    --action`, executes the click via `xdg-open` or
    `fs::remove_file`. Main process exits immediately.
- **mango 0.13+ backports — three runtime feature ports:**
  - Split mouse / trackpad acceleration (`mouse_accel_profile`,
    `mouse_accel_speed`, `trackpad_accel_profile`,
    `trackpad_accel_speed`, `trackpad_scroll_factor`). Legacy
    `accel_*` keys populate both fields so old configs keep
    working.
  - `width:50%` / `height:50%` fraction syntax in windowrules,
    capped at 100 %. Prefers the fraction when both absolute and
    fraction are set.
  - `drag_tile_to_tile` + `drag_tile_small` runtime — dragging a
    tiled window with the flag on shrinks it to a 300×300
    thumbnail centred on the cursor; releasing over another
    tiled client swaps the two via `data.clients.swap`. Restores
    pre-grab float_geom on release so the thumbnail never
    lingers.
- **MargoMaterial icon theme** — renamed from OkMaterial, +17 new
  glyphs covering the new pill set.

### Fixed

- **`isfloating:1` rule with no size hint → invisible 0×0 window.**
  `apply_window_rules` only synthesised `float_geom` when the
  rule carried a width/height/offset hint, so `isfloating:1` on
  its own flagged the client floating but left `float_geom` at
  (0,0,0,0). Arrange's `if float_geom.width > 0` then skipped
  the apply and the toplevel got configured at 0×0 — listed in
  `mctl clients`, rendered in overview, but invisible on the
  output. Post-loop fallback now synthesises a default geometry
  (60 % of work_area centred) when `is_floating` ended true and
  `float_geom` is still empty.
- **windowrule typo class — silent drops.** `monitor_name:` (the
  tagrule key) on a windowrule now aliases to `rule.monitor`;
  `is_overlay` and `overlay` now alias to `isoverlay`. Both used
  to parse but be silently ignored, leading to "the rule
  doesn't work" reports for typos that look like docs spellings.
- **mshell menus opened on the wrong monitor** after first
  reboot. Frame routing now reads the focused-client's monitor
  instead of `active_output`, which stayed pinned to the
  pre-restart selection.
- **mshell dashboard duplicate hero** — the panel rendered two
  clocks at the top after the hero + grid restructure; cleaned.
- **mshell-settings — `Add widget` menu didn't scroll** for users
  with > ~12 widgets; wrapped in a `ScrolledWindow`.
- **mshell-settings — `Widgets → Layout` renamed to `Widgets →
  Menus`** for accuracy.
- **mshell-settings — sidebar icons** for Fonts and Display were
  missing or swapped after the alphabetic restructure.
- **mshell — bar minimum_height crash + debounce spin.** A spin
  button driving live re-layout could push the bar's minimum
  below the actual content and crash gtk's measure pass.
- **mshell — screen corners default off** (the previous default
  enabled them globally, surprising users) + Twilight schedule
  panel becomes visible when Mode = Schedule.
- **mlayout — follow symlinks in gather_layouts** so users who
  symlink their layouts directory get listed correctly.
- **mshell session menu Tab / Shift+Tab / Ctrl+N / Ctrl+P / Ctrl+J
  / Ctrl+K** focus-walk attempts shipped (four iterations:
  `EventControllerKey` default + Capture phase,
  `ShortcutController` Local-Bubble + Local-Capture). Number
  keys 1–5 work; Tab + Ctrl-letter cluster still doesn't. See
  road_map B9 for the open follow-up.
- **PKGBUILD — also remaps C build-script paths** to silence the
  `$srcdir` debug-info warning.

### Changed

- **mshell-matugen owns its own CLUT** — Margo's theme is
  independent of Dracula references; previous wrapper around
  matugen-pure for the Margo palette is now a first-class palette
  inside mshell.
- **Twilight owns `~/.config/margo/twilight/`** instead of
  sharing sunsetr's directory. Migration is automatic — the
  first run of Schedule mode bootstraps the new directory.
- **mshell-settings restructure** — Bar moves to top level;
  Widgets becomes a group containing per-pill + per-menu pages;
  sidebar is alphabetised.
- **Bar font scaled to noctalia size** (~13.3 px) for visual
  parity with the noctalia reference.

## [0.6.0] – 2026-05-15

### Added

- **Wayland protocol surface — 16 new globals advertised in one sweep.**
  Cross-checked margo's smithay `delegate_*!` macros + hand-rolled
  globals against niri and Hyprland on 2026-05-15. Margo's surface
  grew from ~38 to ~54 advertised globals, passing niri (~41) and
  pursuing Hyprland (~62) on standard protocols. Three protocols
  (`zwp_xwayland_keyboard_grab_v1`, `xdg_toplevel_icon_v1`,
  `xdg_toplevel_tag_v1`) are advertised by margo alone among the
  three. Full side-by-side audit lives in
  `docs/protocol-comparison.md`; work plan in `road_map.md` §15.10.
  Shipped in commits `dc44818` + `74a0edb` + `c146aac`:
  - `zwp_keyboard_shortcuts_inhibit_v1` — VNC / RDP / VM clients can
    grab host shortcuts. `input_handler.rs` short-circuits the
    keybinding match when the focused surface has an active
    inhibitor; auto-activate policy matches niri.
  - `zwp_pointer_gestures_v1` — touchpad pinch / swipe / hold
    forwarded to clients (Firefox pinch-zoom, GNOME, Inkscape).
  - `xdg_foreign_v2` — cross-process surface embedding for Firefox /
    Chromium Picture-in-Picture and xdg-desktop-portal screencast.
  - `wp_single_pixel_buffer_v1` — solid-color buffer fast-path.
  - `zwp_tablet_manager_v2` — Wacom / Huion drawing tablets. Folds
    the orphan `TabletSeatHandler` impl that was sitting unwired
    in state.rs into a proper handler module.
  - `wp_security_context_v1` — Flatpak / sandboxed clients. Handler
    inserts the listener source into margo's calloop; restricted-
    client enforcement is a follow-up.
  - `org_kde_kwin_server_decoration` — legacy KDE deco for older
    Qt5 / KDE apps. Default mode Server (matches SSD-first policy).
  - `wp_content_type_v1` — game / video / photo surface hints.
  - `wp_fifo_v1` + `wp_commit_timing_v1` — newer presentation
    pacing protocols.
  - `wp_alpha_modifier_v1` — per-surface alpha hint.
  - `xdg_wm_dialog_v1` — modal-dialog hint.
  - `zwp_xwayland_keyboard_grab_v1` — XWayland-side keyboard grab.
    Direct complement to `keyboard_shortcuts_inhibit_v1` — same
    VNC / VM story via the X11 mechanism. Handler maps the
    XWayland-managed wl_surface to its `MargoClient.window` so the
    grab attaches to the correct toplevel `FocusTarget`.
  - `xdg_toplevel_icon_v1` — toplevels ship inline PNG / SVG icons;
    smithay caches them on the surface as `ToplevelIconCachedState`.
    mshell taskbar / active-window pill consumer is the natural
    next step.
  - `xdg_system_bell_v1` — logged-only for now; routing to a sound
    daemon / notification toast is a future enhancement.
  - `wp_pointer_warp_v1` — programmatic cursor warp; default no-op
    (opt-in policy).
  - `xdg_toplevel_tag_v1` — semantic tags + description strings;
    default no-op, could feed window-rule matching down the road.
- **mshell session power menu** — Lock / Logout / Suspend / Reboot /
  Shutdown with 1-5 number-key actions, 3-second countdown
  confirmation, configurable command overrides per-action via
  `[session]` config, Settings UI entry, `super+delete` keybind,
  and `mshellctl menu session [action]` IPC. Tab / Ctrl+N
  navigation works (focus delivered through a 160 ms post-reveal
  glib timeout to clear smithay's `sync_keyboard_mode` debounce).
- **`mshellctl menu notifications {clears,read}`** — `clears` is
  destructive (history wipe); `read` marks currently-visible
  popups as read without dismissing history.
- **`zwp_virtual_keyboard_v1`** — wayvnc / wtype / ydotool / IMEs
  can inject synthetic key events into the focused surface.
  Opens the protocol to all clients (the wayland socket is already
  per-user).

### Fixed

- **XWayland keyboard input not delivered to X11 clients** — the
  `KeyboardTarget for FocusTarget` impl forwarded only via
  `inner_wl_surface()`, which returns `None` for X11-backed
  windows. As a result, smithay's `KeyboardTarget for X11Surface`
  (the path that calls `XSetInputFocus` + sends `WM_TAKE_FOCUS`)
  never ran when an X11 window had keyboard focus. Pointer events
  arrived through the Wayland-pointer-surface path, so touchpad
  worked but keys never reached the X11 client. Now
  `FocusTarget::Window` variants with an X11 underlying surface
  forward keyboard events to the `X11Surface` directly. Fixes
  vncviewer / xfreerdp / X11 apps under margo not receiving
  keyboard input while niri / Hyprland worked.
- **mshell config directory rationalized.** Moved from
  `~/.config/mshell/` to `~/.config/margo/mshell/` so all margo-
  related config lives under one tree. Affects `profiles/`,
  `styles/`, and `icons/` lookup paths.
- **Session menu keyboard nav was unreachable.** The session menu's
  EventControllerKey was wired but the focus path was broken —
  `RevealChanged` was not forwarded into the session widget, and
  the bar's hardcoded broadcast list omitted it. Both fixed, plus
  a 160 ms post-reveal `glib::timeout_add_local_once` so
  `first.grab_focus()` lands after smithay's layer-shell focus
  debounce.
- **Settings → Session entries typed right-to-left.** The
  `gtk::Entry` widgets had `#[watch] set_text` bindings that
  fed back into themselves on every keystroke, resetting the
  cursor to position 0. Entries now seed text once at init and
  write via `connect_changed` only — no reactive read-back loop.

### Changed

- **`KeyboardTarget for FocusTarget` dispatches X11 vs Wayland**
  through a new `inner_x11_surface()` accessor. Wayland-native
  variants (Window/Wayland, LayerSurface, SessionLock, Popup)
  keep the existing `WlSurface` forwarding.

## [0.5.0] – 2026-05-14

### Fixed

- **Menu content widgets recreated ~once per second.** The menu's
  `SetWidget` handler tore down and rebuilt every menu's content
  controllers unconditionally; the coarse config store re-notifies
  every effect bound to it, so any unrelated config touch recreated
  all the menus. The ndns / nufw / npodman menu widgets shell out
  to `ufw` / `nmcli` / `podman` / `resolvectl` / `mullvad` on init,
  so this meant a steady subprocess storm — their 30/60/120 s
  refresh intervals never even applied because the widgets never
  lived that long. The bar already guarded against this; the menu
  now does too. Idle CPU ~25% → ~2%.
- **Startup RSS spike into the gigabytes.** The wallpaper menu's
  `GridView` factory spawned one bare OS thread per thumbnail
  decode; a directory of a few hundred wallpapers, times one bar
  per monitor, meant hundreds of threads each loading an image at
  once (~557 threads / 2.2 GB RSS at peak, cgroup peak 6.6 GB).
  Decodes now run through a fixed six-worker pool — extra binds
  just queue. Startup peak drops to ~1.5 GB; the mshell process
  settles at ~400 MB.

## [0.4.9] – 2026-05-14

### Added

- **In-tree `mshell` desktop shell.** margo now ships its own
  bar / shell / menu system (GTK4 + relm4 + gtk4-layer-shell),
  built from the same Cargo workspace — three binaries
  (`mshell`, `mshellctl`, `mshellshare`) plus helper crates
  under `mshell-crates/`. Replaces the need for a separate
  third-party panel.
- **Margo-native widgets.** `MargoTags` (single-row capsule
  workspace pills with occupancy dots), `MargoLayoutSwitcher`
  (driven by `mctl layout`), media-player pill + rich menu
  (cover art, seek, controls, follows the playing player),
  battery pill (charge % + AC/battery state), and an
  ActiveWindow pill showing the focused window title.
- **Ported noctalia plugins** as first-class mshell modules,
  each with a bar pill + layer-shell menu: `npower` (power
  profiles + battery + Cycle / Lock Auto / Idle Toggle),
  `nnetwork` (Network Console — Wi-Fi list / connect / rescan),
  `ndns` (DNS mode switcher), `nufw` (firewall), `npodman`
  (containers / images / pods), `nip` (public-IP panel),
  `nnotes` (scratchpad / notes / todos).
- **Wallpaper rotation** — change every N minutes, configurable
  in Settings → Wallpaper, plus `mshellctl menu wallpaper
  next/prev/random` to cycle from the CLI.
- **Idle manager** — staged dim → lock → suspend on inactivity,
  timeouts configurable in Settings (built on `ext-idle-notify-v1`).
- Bundled the MargoMaterial icon theme (margo-branded fork of
  OkMaterial) + new plugin glyphs so the shell renders
  consistently without relying on the host icon theme.

### Changed

- **`npower` and `nnetwork` are now reactive over D-Bus.** Both
  widget pairs previously ran per-monitor poll loops that
  shelled out to `powerprofilesctl` (a Python script) and
  `nmcli` — a sustained ~25% idle CPU and a multi-GB RSS climb
  on multi-monitor setups. They now read state from the wayle
  services (`power_profile_service()`, `battery_service()`,
  `network_service()`); idle CPU drops to ~2-3% with no
  steady-state subprocess spawning.
- The super+d night-light button drives `mctl twilight`
  instead of `mshell-gamma`.
- Bar layout: dropped the vertical Left / Right bar surfaces;
  all widgets migrated to the Top bar. Clock font shrunk one
  step to match the other pills.
- **PKGBUILD** builds the compositor-side binaries and the
  mshell trio in two separate `cargo` invocations — a single
  `--workspace` build unified `zbus`'s `tokio` feature into the
  compositor, which then panicked at startup.

### Fixed

- **margo bar flicker** — ported niri's render + frame-callback
  pacing, paced `frame_done` to VBlank, dropped the
  `wp_linux_drm_syncobj_v1` global, disabled DRM overlay-plane
  scanout (Intel MTL quirk), and fixed margo-client `Arc`
  identity churn.
- **margo startup panic** — `zbus` was pulled in with its
  `tokio` feature via workspace feature unification; the
  compositor drives `zbus` over `async-io` and has no Tokio
  runtime, so it panicked before the session came up.
- Settings crash from unsanitised `GAction` names derived from
  widget labels.
- Cleared all mshell build warnings.

## [0.4.8] – 2026-05-13

### Added

- **Compositor-side wallpaper renderer.** margo now paints the
  wallpaper itself, behind every window and layer surface, instead
  of waiting for an external daemon (`swaybg` / `swww` / a noctalia
  background widget) to cover the root color. New top-level config
  fields:
  * `wallpaper = PATH` — explicit image path. Resolution chain when
    unset:
    1. `~/.local/share/margo/wallpapers/default.jpg` (user override)
    2. `/usr/share/margo/wallpapers/default.jpg`     (package default)
  * `wallpaper_fit = cover|contain|fill|center` — only `cover`
    is wired through the renderer right now; the other variants
    parse cleanly so configs picking them don't fail validation.
  Cover mode crops a centred sub-rectangle of the source whose
  aspect ratio matches the output, then scales it to the output
  rectangle (no letterboxing, no stretch). External shells can
  still draw on top via layer-shell — layer surfaces sit above
  the background, so a noctalia / swww overlay wins the z-fight
  regardless.

### Changed

- **mlock + compositor share the same wallpaper resolution chain.**
  Both now check the same three locations in the same order, so a
  clean install never lands on flat dark for either the desktop or
  the lock screen.

## [0.4.7] – 2026-05-13

### Added

- **Default lock-screen wallpaper.** A 4K JPG ships at
  `assets/wallpapers/default.jpg` and lands at
  `/usr/share/margo/wallpapers/default.jpg` after install, so a fresh
  margo session never falls through to a flat dark lock backdrop just
  because the user's external shell hasn't populated `state.json` yet.

### Changed

- **`mlock` wallpaper resolution is now tiered.** Previous behaviour
  was state.json or nothing; new chain:
  1. `state.json` active output's `wallpaper` field (margo tagrule
     passthrough — unchanged primary path).
  2. `~/.local/share/margo/wallpapers/default.jpg` — user override.
  3. `/usr/share/margo/wallpapers/default.jpg` — package default
     (shipped by `margo-git`).
  Every layer is `metadata().is_file()`-checked, so a stale path in
  state.json no longer wins against a real fallback. The candidate
  that lands is logged via `tracing::info!` so the source of the
  current lock wallpaper is one log line away from diagnosis.

## [0.4.6] – 2026-05-13

### Added

- **`start-margo` watchdog supervisor.** New Rust binary in the
  workspace — wraps margo with a rolling crash budget
  (`--max-restarts 3 --restart-window-secs 60` by default), emits
  `sd_notify` `READY=1` after spawn and `STOPPING=1` on graceful
  shutdown, preserves the incoming signal when forwarding
  SIGTERM / SIGINT / SIGHUP to the compositor, and sets
  `PR_SET_PDEATHSIG(SIGKILL)` so a `kill -9 start-margo` can never
  leave an orphaned margo. Single source file (`start-margo/src/main.rs`,
  ~230 lines), depends only on `anyhow` / `clap` / `tracing` /
  `tracing-subscriber` / `libc`. Three concrete improvements over
  Hyprland's `start-hyprland`: crash budget (vs. unbounded respawn),
  systemd-notify integration (vs. pipe-handshake), and original-signal
  forwarding (vs. always SIGTERM).
- **`contrib/sessions/` integration examples.** Ready-to-copy
  Wayland-session glue:
  * `margo-uwsm.desktop` — display-manager session entry.
  * `margo-uwsm-session` — UWSM wrapper that resolves the best
    compositor command (`margo-session` > `start-margo` > `margo`).
  * `margo-session` — minimal launcher that prefers `start-margo`,
    falls back to bare `margo`.
  * `systemd/user/wayland-wm@margo-session.service.d/10-session-lifecycle.conf`
    — drop-in that sets `MARGO_LOG`, fires the session target,
    bumps Nice / CPUWeight.
  See `contrib/sessions/README.md` for the install recipe and the
  full session chain diagram.

### Fixed

- **PKGBUILD now keeps debug symbols.** `options=(!lto)` was missing
  `!strip`, so makepkg's outer strip pass was wiping the symbol
  table on every install — exactly the failure mode `CLAUDE.md`
  warns against ("mesa abort inside the render path on overview
  trigger" coredumps were resolving to `?? ??:0` for every margo
  frame). `options=(!lto !strip)` now matches the `strip = "none"`
  setting that's been in the Cargo release profile all along. The
  next time margo trips an ABRT, `coredumpctl info` / `addr2line`
  will name the exact Rust source line instead of a hex offset.

### Changed

- **README binary table + install loop.** `start-margo` is now in
  the table (between `margo` and `mctl`), the source-install
  one-liner installs seven binaries, and a new "Supervisor
  (`start-margo`)" section + `contrib/sessions/` pointer explain
  the recommended session topology.

## [0.4.5] – 2026-05-13

### Fixed

- **Example `config.conf` now passes `mctl check-config` cleanly.**
  The shipped reference produced 32 errors and 2 warnings against
  the real parser. Three causes: line-continuation `\` is not
  honoured (32 multi-line windowrule / layerrule entries collapsed
  to single lines); `super+shift,h/l` was bound twice (the
  `setmfact` pair moved to `super+alt,h/l`, hjkl muscle memory
  preserved); `focuslast` action used in the example doesn't
  exist in the dispatch table (orphan bind removed). The mirrored
  README windowrule snippet lost its trailing `\` too. Result:
  121 binds, 30 windowrules, 5 layerrules, 9 tagrules, ✓ no
  problems.

### Changed

- **`exec-once` block modernised.** Bar / notifications / launcher
  recommendations updated to reflect the external-shell-first
  architecture: `qs -c noctalia-shell --no-duplicate` or `waybar`
  side by side, with `fnott` / `mako` notification-daemon
  alternatives broken out.
- **`unreachable!()` panic messages.** Eight bare `unreachable!()`
  sites across `protocols/screencopy.rs`, `protocols/gamma_control.rs`,
  `mctl/bin/mctl.rs`, `mlayout/main.rs`, `mscreenshot/main.rs`, and
  `layout/snapshot_tests.rs` now carry a one-line *why* string so
  post-mortems read context instead of the generic
  "entered unreachable code" line. The `ok_or_else(|| unreachable!())`
  pattern in mctl's output-index resolver rewrote to plain
  `unwrap_or(0)` — the original `.or(Some(0))` already guaranteed
  `Some`.
- **mlock `wl_globals` binding tightened.** `if self.X.is_none()`
  guards inside `match g.interface.as_str()` collapsed into
  match-arm guards, and three `min().max()` clamp chains rewritten
  with `.clamp(lo, hi)`.

### Removed

- **Stale `#[allow(dead_code)]` attributes.**
  * `margo/src/screencasting/pw_utils.rs` lost its crate-level
    `#![allow(dead_code)]` — the niri-port scaffolding was fully
    wired up over Phases C / D / F.
  * `mlock/src/state.rs` field `conn`: the allow was a holdover;
    `Connection` is read every iteration via `state.conn.flush()`
    and `state.conn.backend().poll_fd()` in `main.rs`.
  * `margo/src/state.rs`: orphaned attribute above
    `DmabufImportHook` (blank line in between) moved onto the
    type alias so `empty_line_after_outer_attribute` stops firing.
- **Unused dependencies pruned.** Manual audit confirmed zero
  source-level use sites:
  * `margo`: `keyframe`, `nix`, `log` (the codebase standardised
    on `tracing`).
  * `margo-config`: `regex` (window-rule regexes are compiled in
    the compositor crate, not the parser crate).

  Cargo.lock dropped 32 lines of now-unreferenced transitive deps.

### Quality

- `cargo clippy --workspace --all-targets`: **0** warnings
  (previously 9 — 8 in `mlock/src/state.rs`, 1 in `margo/src/state.rs`).
- `cargo test --workspace`: 244 tests, 0 failures.

## [0.4.4] – 2026-05-13

### Removed

- **`mshell` crate.** The iced-then-GTK4 bar / notifications / OSD /
  settings / system-tray stack is gone. margo no longer paints any
  shell chrome of its own; the bar, launcher, notification daemon,
  OSD, and settings panels are delegated to any `dwl-ipc-v2` client
  (noctalia, waybar-dwl, fnott, …). The compositor side of
  `dwl-ipc-v2` is unchanged.
- **`midle` crate.** Idle management moves out of the workspace.
  Any `ext-idle-notify-v1` client (swayidle, hypridle, stasis, …)
  works as a drop-in.
- **Matugen integration.** `mshell matugen`, the
  `~/.cache/margo/margo-colors.conf` `source =` hook, and the
  associated PKGBUILD wiring are removed. The Catppuccin Mocha
  default palette stands on its own; bring your own colour generator
  if you want Material You.
- **mlock `mshell.toml` wallpaper fallback.** Wallpaper resolution
  inside `mlock` now reads exactly one source — `state.json`'s active
  output `wallpaper` field — and falls through to the solid dark
  backdrop on miss. `toml` is dropped from `mlock`'s `Cargo.toml`.

### Changed

- **README rewrite.** Intro, binary table, install paths, file-layout
  block, `At a glance` recipe list, scripting example, and
  acknowledgements are now consistent with the six-binary scope. The
  `dwl-ipc-v2` bullet was promoted to call out external-shell
  integration explicitly.
- **PKGBUILD overhaul.** `depends=` lost the panel-only runtime
  pulls (`libpulse`, `pipewire`) and gained the previously-implicit
  `pam` / `cairo` / `pango` (mlock's actual link-time set).
  `optdepends=` dropped eleven panel-only recommendations
  (`networkmanager`, `iwd`, `bluez`, `bluez-utils`, `pipewire-pulse`,
  `wireplumber`, `pavucontrol`, `nm-connection-editor`, `blueman`,
  `ttf-jetbrains-mono-nerd`, `checkupdates`) and gained
  `noctalia-shell-git` + `fnott` as the suggested external shells.
  `package()` walks a six-binary install loop, ships a hicolor
  scalable icon, and installs the Rhai init template.

## [0.4.3] – 2026-05-12

### Fixed

- **mshell bar no longer shakes on CPU / network refreshes.**
  `system_info` (CPU%, Memory%, Temperature) and `network_speed`
  (Download/Upload) refreshed every 1-3 seconds. Each refresh
  changed the value's text-width by a digit-advance (5% → 23% →
  100%) and the bar's `animated_size` wrapper was tweening that
  width swing over 150 ms — visible as a 1-2 s shake burst every
  time a background process spiked CPU. Fixed in three layers:
  * `Font::MONOSPACE` on every numeric bar value — equal advance
    per digit, so two-digit values are pixel-stable.
  * `Length::Shrink` (text widget hugs its content) instead of
    `Length::Fixed` — no leading/trailing slack between an
    indicator and its neighbour. The earlier "fixed-width"
    iteration over-padded short values ("9KB/s    62KB/s") so
    the design read as broken on idle systems.
  * `build_module_item` now skips the `animated_size` wrap for
    SystemInfo and NetworkSpeed specifically. Cross-decade
    width changes still happen ("9%" → "100%") but reflow is
    instant rather than animated. Other modules (Workspaces tag
    switch, Notifications badge churn) keep their animation.
  Measured: 5× fewer state.json content-burst clusters during
  passive idle, and zero perceptible bar shake.

## [0.4.2] – 2026-05-12

### Fixed

- **Bar layout no longer shakes when an mshell menu opens.** Menus
  now grab keyboard focus via `KeyboardInteractivity::Exclusive`
  (see 0.4.1 ESC fix), which makes margo report the menu's layer
  surface as focused — and the C client list only tracks toplevels,
  so `CompositorState.active_window` collapsed to `None`.
  `WindowTitle::recalculate_value` was overwriting its cached
  string with that `None`, blanking the bar item, and the resulting
  `Length::Shrink` content collapse rippled across every neighbour
  capsule. `recalculate_value` now early-returns whenever
  `active_window` is `None` or the title is empty, holding the
  last-known toplevel title until a real toplevel regains focus.
- **IPC menu bindings are globally-scoped toggles again.**
  `mshell msg notifications` (and every other menu IPC: media,
  settings, tempo, dns, ufw, power, podman, updates, system,
  network) was routing every keypress through the currently active
  output → `ToggleMenu` on that monitor's bar surface. With two
  monitors, if `active_output` shifted between presses the handler
  picked the *other* monitor as the target, opened a fresh menu
  there, and `toggle_menu`'s "close menus on other outputs" pass
  closed the prior surface as a side-effect — visible to the user
  as "the binding moved the menu instead of closing it". The IPC
  handler now scans every output for an already-open instance of
  the requested type first; if any exist, it closes them all and
  bails before reaching the open path.

## [0.4.1] – 2026-05-12

### Highlights

Polish + correctness pass over the 0.4.0 release. Two visible
themes:

* **mshell bar gets a noctalia-grade information density layer** —
  active workspace accent stripes, audio/brightness progress fill
  rails, battery threshold borders, tray collapse, tempo two-line
  composite, notification dot indicator — without abandoning the
  minimal/sakin character of the original design.
* **midle becomes browser-aware** — D-Bus screensaver / session-manager /
  portal Inhibit eavesdropping ported from stasis. Helium / Firefox /
  Chrome no longer block idle just by being open; they only inhibit
  while they're actually claiming the system's idle inhibitor (e.g.
  playing a YouTube video).

### Added

- **mshell `restart` subcommand.** Scans `/proc` for sibling
  `mshell` processes, SIGTERMs them, polls for exit with a 3s
  graceful budget (SIGKILL fallback), gives the compositor 200ms
  to tear down the bar's layer surfaces, then spawns a detached
  fresh instance via `setsid()`. Replaces the
  `pkill mshell && setsid -f mshell …` shell incantation.
- **midle D-Bus inhibit monitor.** Eavesdrops the session bus for
  `org.freedesktop.ScreenSaver`, `org.gnome.SessionManager` and
  `org.freedesktop.portal.Inhibit` traffic, correlates method-call
  serials with their cookie / handle returns per sender, and
  drops sender rows on `NameOwnerChanged` disconnects.
  `Settings::enable_dbus_inhibit` (default `true`) gates it.
  `midle info` now reports an `inhibitors` breakdown
  (`manual / app / media / dbus`) so "why isn't midle firing?"
  becomes a one-liner instead of a log dive.
- **Workspace pill polish.** Active workspace gets a 2.5px accent
  bar across ~55% of the pill width (Stack overlay, no height
  shift); inactive workspaces with open windows get a row of up to
  4 small accent dots. Switch animation curve goes from symmetric
  EASE to EASE_OUT.
- **Status cluster density polish.** `format_indicator` wraps
  Warning / Danger states in a 1px tinted border + 10% accent
  background; `BatteryData::get_indicator_state` gains a Warning
  threshold at <30% in addition to the existing <15% Danger.
  `format_indicator.progress(0..=1)` stacks a 2px accent fill
  along the bottom edge — audio (sink) and brightness now expose
  their live level on the bar. Muted sink hides the bar.
- **Tray chevron collapse.** Once more than 3 icons are registered,
  the tray compacts to 2 icons + a chevron toggle; click to
  expand. Keeps the right-cluster from sprawling.
- **Module active-state indicator.** Every bar capsule now signals
  "my menu is on screen" with a 2px accent stripe along its bottom
  edge (~60% width, centred). Stack overlay so toggling never
  changes bar height. `Outputs::open_menu_type_for_bar` resolves
  the open menu once per render and threads it through to
  `ModuleItem::is_active`.
- **Tempo rich composite.** Opt-in `[tempo] secondary_format = "%a %d %b"`
  renders a 2-line Column: primary clock in semibold at
  `bar_font`, secondary string beneath at `font_size.xs` and 65%
  foreground alpha. Tracks tz cycles and the live update tick
  alongside the primary.
- **Notification dot indicator.** When there are pending
  notifications, the bell icon gets a 5px accent dot in its
  top-right corner, hairlined with the bar background. Critical
  urgency swaps the dot to the danger palette. Independent of the
  existing count badge — heavy users can keep both.

### Fixed

- **ESC inside an open menu now closes it.** Previously menus
  opened with `KeyboardInteractivity::OnDemand`, which margo doesn't
  auto-focus — the keypress went to the background app instead.
  Menus now open `Exclusive`; the compositor moves focus onto the
  menu surface as soon as it appears, ESC reaches mshell's
  `listen_with` Escape handler, and the menu closes.
- **No more blank flash on menu open.** A new
  `MENU_OPEN_PREROLL_MS = 30` constant backdates `open_at` so the
  first paint lands at ~42% opacity (after ease-out-cubic) instead
  of α=0; the animation finishes ~150ms later with no perceptual
  flash. When `theme.animations_enabled = false`, `open_at` /
  `closing_at` are backdated past the animation window so menus
  render and tear down instantly.
- **Updates module bar item sizing.** `view()` was missing
  `.size(bar_font)` on both the StaticIcon and the count text, so
  it rendered visibly bigger than every other capsule. Same fix
  here: size to `theme.bar_font_size`, drop the pointless wrapping
  container, tint the row with `palette.primary` when there are
  pending updates.
- **midle daemon no longer panics margo at startup.** The `tokio`
  feature on midle's `zbus` dependency was getting unified across
  the workspace, forcing margo's zbus (pulled via mctl) into a
  tokio runtime that doesn't exist in the calloop loop. Dropped
  to `default-features = false, features = ["async-io"]` — margo
  stays on async-io, midle's own tokio runtime can still `.await`
  zbus futures regardless of the reactor.

## [0.3.0] – 2026-05-11

### Highlights

Phase 2 closing release. Three technical success criteria from
§15.8 of the roadmap landed on this branch:

* **Snapshot test count ≥ 200** — at **244** workspace-wide (margo
  230, margo-config 14). T1 (window-rule matcher), T2 (animation
  curves), T6 (screenshot region), T8 (theme preset), T9 (session
  round-trip) drove the expansion.
* **state.rs < 3k LOC** — at **2944** after eleven sibling-module
  extractions (see `Changed` below).
* **Cold-path structured-logging migration complete (Q5)** — every
  `tracing` call in `state.rs`, `dispatch/mod.rs`, `scripting.rs`,
  `plugin.rs` now emits structured fields.

### Added

- **Screenshot region selector geometry tests (roadmap T6).** 14
  new tests lock `ActiveRegionSelector::selection_rect`
  normalisation across all four drag directions (TL→BR, BR→TL,
  TR→BL, BL→TR), degeneracy handling (zero area, sub-pixel,
  vertical/horizontal line), `grim -g` geom-string format,
  drag-lifecycle (`begin_drag` snaps anchor, `update_drag`
  no-ops without `begin`, `end_drag` preserves rect), and
  half-pixel rounding edge cases.

- **Theme preset tests (roadmap T8).** 13 new tests cover
  `apply_theme_preset` for `default` / `minimal` / `gaudy`:
  * Lazy baseline capture on first call.
  * Field-deltas locked per preset.
  * Preset chains (minimal→gaudy→default, gaudy→minimal→default)
    restore the captured baseline.
  * `default` is idempotent under repeated calls.
  * Baseline survives intermediate manual config tweaks
    (doesn't refresh from post-tweak state).
  * Unknown preset returns `Err` with a clear "try `default`,
    `minimal`, `gaudy`" hint.

- **Window-rule matcher edge-case tests (roadmap T1).** 16 new
  focused unit tests lock the algebra cell-by-cell, complementing
  the existing two snapshot tests that lock the integration shape:
  * **id pattern semantics** — anchored vs unanchored, case
    sensitivity, regex alternation, character classes.
  * **empty / absent pattern semantics** — `None`, `Some("")`,
    empty value against non-empty pattern (the "newly-mapped
    Electron toplevel before app_id settles" corner case).
  * **multi-field AND semantics** — id + title both required;
    id-only ignores title; title-only ignores id; no patterns
    matches everything.
  * **exclude_* precedence** — `exclude_id` and `exclude_title`
    veto otherwise-matching rules; unmatched exclude does NOT
    block.
  * **invalid-regex fallback** — `[invalid` (unclosed character
    class) falls back to substring, including the
    anchor-stripping path (`^[invalid$` → `[invalid` substring).
  Workspace test count: 164 → 180.

- **Animation curve snapshot tests (roadmap T2).** Nine new
  tests lock the 4-point Bezier evaluator + spring-baked curve
  shapes against accidental coefficient drift:
  * `near_linear_bezier_endpoints_exact` — sanity check.
  * `ease_out_expo_shape_locked` — sample(0.25/0.50/0.75)
    bands locked. A real coefficient swap (`p0` ↔ `p2`) pulls
    each sample out of its band.
  * `ease_in_quad_shape_locked` — mirror of the above.
  * `bezier_bake_is_non_decreasing_in_y` — 4 curves × 256
    points: non-monotone tables produce mid-flight stutter,
    so the property is locked in stone.
  * `sample_endpoints_round_to_zero_and_one` — binary-search
    ceiling behaviour documented + tested.
  * `spring_bake_overshoot_clamped_to_1_05` — under-damped
    spring overshoots get clamped at 1.05 to bound the
    consumer's slot stretch.
  * `critically_damped_spring_is_monotone` — `damping = 1.0`
    spring reaches target without bouncing.
  * `animation_curves_dispatches_every_variant` — full
    AnimationType ↔ curve dispatch exercised.
  * `sample_clamps_out_of_range_t` — defensive boundary check.
  Workspace test count: 155 → 164.

- **Session save/load round-trip test suite (roadmap T9).** Nine
  new tests cover the JSON contract:
  * `save_to_then_load_from_round_trips_every_field` — every
    nested field on both monitors + scratchpads spot-checked
    after a real disk round-trip (write `.tmp` → rename → read
    back).
  * `save_to_is_atomic_via_rename` — the tmp file gets cleaned up
    on success.
  * `load_from_rejects_malformed_json` — no panic, just an Err.
  * `load_from_missing_file_is_io_error` — error message chain
    starts with "read", not parse failure.
  * `pertag_lengths_clamp_on_either_side` — snapshot shorter or
    longer than `MAX_TAGS` both deserialise cleanly.
  * `unknown_layout_name_in_snapshot_does_not_break_serde` —
    snapshots survive a future layout-name renaming (the loader's
    `LayoutId::from_name()?` silently skips unknowns).
  * `scratchpad_entry_defaults_round_trip` — defends against a
    future serde flag tweak.
  * `save_to_produces_pretty_indented_json` — locks the
    pretty-printed shape so `session.json` stays human-diff-able.
  * `captured_at_round_trips_through_serde` — belt-and-braces on
    the hand-rolled `chrono_like_now` string.
  Workspace test count: 146 → 155.

### Changed

- **state.rs split to <3k LOC (roadmap Q1).** Reduced from 6858 →
  **2944** LOC (−57 %) by lifting eleven self-contained pieces into
  siblings under `margo/src/state/`:

  | File | LOC | Content |
  |---|---:|---|
  | `dispatch.rs` | 1274 | every keybind / IPC action: kill, focus_stack, view_tag, set_layout, toggle_floating, fullscreen, gaps, zoom, focus_mon, tag_mon, etc. |
  | `scratchpad.rs` | 496 | named + anonymous scratchpads, `summon`, `unscratchpad_focused` |
  | `data.rs` | 450 | `MargoClient`, `MargoMonitor`, `ResizeSnapshot`, `ClosingClient`, `LayerSurfaceAnim`, `FullscreenMode`, `HotCorner`, rule-match helpers |
  | `overview.rs` | 445 | open / close / toggle, alt-Tab cycle, `overview_visible_clients_for_monitor` |
  | `focus_target.rs` | 295 | `FocusTarget` enum + every smithay trait impl (`IsAlive`, `WaylandFocus`, `Keyboard/Pointer/Touch/DndTarget`) |
  | `state_file.rs` | 247 | `write_state_file` + `build_state_snapshot` (the JSON mctl reads) |
  | `animation_tick.rs` | 245 | per-frame `tick_animations` body — opacity, open, layer slide, close, move/resize (bezier + spring) |
  | `screencast.rs` | 217 | `on_pw_msg` + `stop_cast` + `start_cast`, all `#[cfg(feature = "xdp-gnome-screencast")]` |
  | `twilight_methods.rs` | 132 | `force_tick_twilight` + `tick_twilight` + `apply/clear_twilight_ramp` |
  | `theme.rs` | 102 | `ThemeBaseline` snapshot + tests |
  | `debug_dump.rs` | 78 | `MargoState::debug_dump` (SIGUSR1 / mctl debug-dump) |

  Pure lift-and-shift: every method is still an inherent impl on
  `MargoState` and every call site is unchanged. Workspace test
  count holds at 244. Touching the overview cycle no longer
  recompiles the screencast path, twilight ramp, or state.json
  serializer — Phase 2 success criterion §15.8 ticked.

- **Cold-path structured-logging migration complete (roadmap
  Q5).** Every `tracing::info!/warn!/error!/debug!` call in
  `state.rs` (21 sites), `dispatch/mod.rs` (10 sites),
  `scripting.rs` (12 sites), `plugin.rs` (3 sites) now uses
  structured fields (`field = ?value, "msg"`) rather than
  format-string interpolation. Net wins:
  * `journalctl -u margo --output=json | jq` slices cleanly:
    e.g. `... | jq 'select(.fields.error)'` for every error
    record, or `select(.fields.cmd | test("nautilus"))` for
    every spawn of a specific command.
  * `FocusTarget::enter` / `FocusTarget::leave` demoted from
    INFO to DEBUG. They fire on every sloppy-focus crossing
    and every overview hover sweep — under normal use the
    journal was 90 %+ enter/leave noise. The `target` field
    keeps full pretty-debug detail for users who actively
    want to trace focus routing.
  * Hot-path callers (`backend/udev/{frame,hotplug}`,
    `input_handler` keybind + gesture) were already on the
    structured pattern from earlier sprints; this commit
    closes the gap.
  Phase 2 success criterion §15.8 ticked.

## [0.2.1] – 2026-05-11

Rust 2024 edition migration + clippy zero-warnings sweep. No
behavioural change — every patched site uses the modern 2024
idiom the compiler now stabilises (let_chains, struct-init
spread, end-of-file test modules).

### Changed

- **Workspace migrated to Rust 2024 edition.** `cargo fix
  --edition --workspace` handled the mechanical temp-lifetime
  rewrites across 9 files; the rest of this commit is the
  modern-idiom follow-up:
  * 7 collapsible nested `if let` blocks rewritten as
    `if let A && let B` (let_chains is stable in 2024).
    Sites: `margo-config::parser`, `margo-ipc::migrate`,
    `margo-ipc::bin::mctl` (×3), `mlayout::main`,
    `mscreenshot::main`.
  * 2 `let foo = …; foo` blocks collapsed to direct return
    (`margo::input_handler`, `margo::state`).
  * `theme_baseline_tests` rewrote `Config::default() + 9
    reassignments` into the
    `Config { borderpx: 3, ..Config::default() }` struct-init
    spread idiom.
  * `gesture_tests` mod moved from mid-file to end-of-file
    (`clippy::items_after_test_module`).
  * `gamma_lut::extreme_inputs_clamp_safely` test's
    tautological `assert!(v <= u16::MAX)` (always true for
    `u16`) replaced with `std::hint::black_box(v)` so the
    optimiser can't elide the iteration without losing the
    "no panic / no NaN cast" intent.

After: `cargo clippy --all-targets` is zero warnings,
`cargo test --workspace` still 146 passing. Zero `#[allow(...)]`
escape hatches added — every warning got a real-code fix.

The 2024 idioms are now in place to enable future `let_chains` /
`gen` / async-closure work without per-site nags.

## [0.2.0] – 2026-05-11

First minor bump beyond the 0.1.x sweep. Two headline features —
**Twilight** (built-in blue-light filter, full replacement for
sunsetr / gammastep / redshift) and **niri-style config
validation** (structured diagnostics + on-screen overlay +
compositor fail-soft) — plus the overview cinematic finishing
touches and a fistful of bug fixes from live use.

### Highlights

| Feature | Tagline |
|---|---|
| **Twilight** | Built-in colour-temperature scheduler inside the compositor's own event loop. Zero new deps, planar `wlr_gamma_control_v1` wire format, mired-space interpolation, adaptive tick (60 s ↔ 250 ms), `mctl twilight {status,preview,test,set,reset}` live control. |
| **Config validation** | niri-style diagnostics on `mctl check-config`, fail-soft reload (compositor keeps the previous good config), `mctl config-errors` query, 10 s on-screen red-bordered banner overlay, warning-aware notify. |
| **Overview muscle memory** | Modifier-release auto-commit, cinematic dim + thicker focuscolor border on the pick, visual grid order = cycle order, pointer hover no longer reshuffles the grid. |
| **Quick wins** | 50 ms hotplug rescan coalescer, scratchpad persistence in session-save, `on_output_change` Rhai hook, dwl-ipc arg-slot mapping finally documented. |

### Compared to 0.1.9

* +21 twilight tests, +6 validator tests → workspace 123 → 146.
* +14 config keys (twilight) + 2 cinematic + `overview_cycle_order`.
* `Cargo.toml` `[profile.release]` now keeps line tables in the
  installed binary so future coredumps symbolize cleanly.
* mctl subcommand list reformatted — one neat row per command, no
  more mid-row wraps.

### Added

- **Twilight — built-in blue-light filter / colour-temperature
  scheduler.** Replaces external tools (sunsetr / gammastep /
  redshift) with a tick that lives inside the compositor's event
  loop. One less moving part, smoother ramps, live config swap.
  * Three modes: `geo` (sun-elevation from lat/lon — inline NOAA
    math, no `sunrise` or `chrono` deps), `manual` (HH:MM
    sunrise/sunset), `static` (one fixed temp/gamma 24/7).
  * Temperature interp in *mired space*; gamma linear. Tanner
    Helland blackbody fit → 16-bit per-channel RGB LUT, sRGB
    encode curve baked in, monotonic per channel.
  * Adaptive tick: 60 s at steady Day / Night, ~250 ms during a
    transition, ~50 ms during a forced `mctl twilight test`
    sweep.
  * Reuses the existing `wlr_gamma_control_v1` plumbing —
    `pending_gamma` is fed from the tick, the udev frame handler
    pushes ramps to `GAMMA_LUT` on the next render. Zero new
    surface.
  * 14 new config keys (`twilight`, `twilight_mode`,
    `twilight_day_temp`, `twilight_night_temp`,
    `twilight_day_gamma`, `twilight_night_gamma`,
    `twilight_transition_s`, `twilight_update_interval`,
    `twilight_latitude`, `twilight_longitude`,
    `twilight_sunrise`, `twilight_sunset`,
    `twilight_static_temp`, `twilight_static_gamma`)
    + new `TwilightMode` enum. All clamped at parse time;
    `parser::OPTION_KEYS` extended so the validator picks them
    up automatically.
  * Live control via `mctl twilight {status, preview, test, set,
    reset}`. `status` reads `state.json` (no IPC roundtrip);
    the rest dispatch through the compositor.
  * Disabled by default — flip `twilight = 1` to opt in.
  * 21 new unit tests across gamma LUT, schedule, interpolation,
    override stack. Workspace test count 123 → 144.

- **Config validation with niri-style diagnostics.** Three pieces:
  * **`margo-config::validator`** — new module that re-walks the
    config file and emits structured `ConfigDiagnostic`s with file,
    line, column, severity, code, and the offending line snippet.
    Catches trailing/leading/doubled commas in CSV-shaped values
    (`bind`, `gesturebind`, `windowrule`, …), missing `=`
    separators, unresolved `source`/`include` paths, and unknown
    top-level keys. The allowlist is sourced from
    `parser::OPTION_KEYS` — adding a new option to the parser
    automatically expands what the validator accepts.
  * **`mctl check-config` rewrite** — now drives the new validator
    plus the existing regex / duplicate-bind checks and renders
    every diagnostic in niri format (caret arrow, gutter, ANSI
    colour when the terminal supports it). Exit code 1 on errors,
    0 with warnings only.
  * **`mctl reload --force`** — pre-flight validation by default;
    refuses to reload when the file has errors and prints them in
    the same niri format. `--force` keeps the old "fire and see
    what happens" behaviour.
  * **Compositor fail-soft on reload** — `reload_config` runs the
    validator before parsing; if there are errors it keeps the
    previous config, sets `last_reload_diagnostics`, and triggers
    a 10 s on-screen overlay flag (renderer wiring lands in a
    follow-up commit). The compositor never applies a broken
    config.
  * **`mctl config-errors`** — queries the live compositor for
    `last_reload_diagnostics` via state.json (Hyprland's
    `hyprctl configerrors` analogue). Empty when the last reload
    was clean.
  * **On-screen banner overlay** — niri-style red-bordered dark
    rectangle pinned to the top-right of every output for 10 s
    after a rejected reload. Drawn through the existing
    `SolidColorRenderElement` path (no new shader, no font
    rasterizer), sits above windows + layer surfaces but below the
    cursor. Lives in `render::config_error_overlay`. The banner is
    a visual cue only; the actual error list comes from
    `notify-send`, `mctl check-config`, and `mctl config-errors`.
    `tick_animations`' event-loop sibling watches the deadline and
    clears the overlay one repaint after it expires.

### Fixed

- **Alt-release auto-commit now actually fires on the Alt-release
  event.** Previous attempt read `modifiers` from the release-event
  filter callback and checked whether the snapshot still overlapped.
  Problem: xkbcommon updates its modifier state *after* the filter
  runs, so on the `Alt_L` release event the callback still sees
  `modifiers.alt = true`. The intersection check never went empty and
  overview stayed open until a second alt+Tab press happened. New
  approach reads the *released keysym* (`handle.raw_syms()`) and maps
  it to its `margo_config::Modifiers` bit directly, subtracts that
  bit from the pending-cycle snapshot, and commits when the snapshot
  empties. Works regardless of which order the user releases
  modifiers — Alt+Shift+Tab still needs both keys released, but in
  either order.

- **Alt+Tab opening overview now auto-commits on Alt release.** When
  the user pressed Alt+Tab with overview closed, `overview_focus_step`
  called `open_overview()` first — and `open_overview` reset
  `overview_cycle_pending` + `overview_cycle_modifier_mask` to default
  "fresh open" values. That clobbered the snapshot the input handler
  had just set milliseconds earlier in the keybind-match path. So
  the Alt-release branch read `cycle_pending = false`, did nothing,
  and overview stayed open after the user let go of Alt. Fix: drop
  the defensive reset from `open_overview`. `close_overview` and
  `overview_activate` already handle the flag's lifetime on the way
  out; opens reached through `overview_focus_step` carry the
  freshly-set snapshot through to the release branch.

- **Alt+Tab first press now jumps to the *previously*-used window,
  not back to the focused one.** The cycle anchor was
  `is_overview_hovered.position()` only — which is `None` on the
  very first press while overview is freshly open. The `None`
  fallback landed at index 0, and in MRU mode index 0 is the
  currently-focused window (most-recent entry in `focus_history`).
  So the first Tab tap looked like a no-op: highlight didn't
  move, then the *second* press moved one step. Standard alt+Tab
  on every other DE (i3 / sway / Hypr / niri / GNOME) is "one tap
  = jump to the other window."
  Fix: when there's no in-progress hover, anchor on the focused
  client's *position in the list*. `dir = +1` then moves to
  index 1, which in MRU is the previously-used window. Same fix
  benefits `tag` / `mixed` modes: the user's first cycle step
  moves away from where they already are, not onto it.

### Added

- **`overview_cycle_order` config — let the user pick the alt+Tab
  walk order.** New three-valued config key on top of the existing
  MRU-only behaviour, all wired through one match in
  `overview_visible_clients`:
  * `mru` (default, preserves 0.1.9 behaviour) — `focus_history`
    first (most-recent first), then any remaining visible clients
    in clients-vec order. The Win/GNOME/Hypr muscle memory.
  * `tag` — strict tag-1-to-9 order, clients-vec inside each tag.
    Spatial-memory model: tag 1's windows always come first.
  * `mixed` — current tag's clients in MRU order, remaining tags
    in strict tag order. The "MRU where you live, tag elsewhere"
    hybrid.

  Implementation reuses two helpers (`push_mru` with optional tag
  filter, `push_tag_order` with optional skip mask) — adding any
  future mode is now one `match` arm. Unknown / typo'd values fall
  back to `mru` with a `tracing::warn!`.

## [0.1.9] – 2026-05-10

Overview reborn. The whole release is one focused theme: nail the
zoom-out-grid UX so it beats Hyprland, niri, and the upstream
mango-ext on the metric the user actually feels — keyboard latency,
spatial continuity, modifier muscle memory. Three iterations to get
there (Phase 3 spatial reverted, fixed 3×3 thumbnails reverted,
mango-ext `overview(m){grid(m);}` shipped); then cinematic dim +
thicker selection border + MRU cycle + alt-release auto-commit on
top of the same single-arrange path. End state is one of the
shortest overview implementations in any Wayland compositor and the
most responsive.



### Added

- **Alt+Tab muscle-memory commit — release modifier to confirm.**
  Holding Alt and tapping Tab to walk thumbnails was already
  smooth, but the user still had to press Enter (`alt+Return →
  overview_activate`) to commit the pick. Now, releasing Alt (or
  whichever modifier the binding uses) is enough — overview
  closes onto the highlighted thumbnail and focus moves there.
  Matches the Win/GNOME/Hypr "hold modifier, tap to cycle, let
  go to confirm" muscle memory the user expects from alt+Tab
  outside this compositor.
  * Implemented as a modifier snapshot taken when an
    `overview_focus_next/prev` keybind fires, plus a release-
    branch in the keyboard handler that watches for the snapshot
    set going to zero (every snapshotted modifier released).
  * Works for any modifier — `super,Tab,overview_focus_next`
    binding would commit on Super release.
  * `alt+shift+Tab` walks backwards: releasing Shift alone won't
    commit (Alt is still held); releasing both Alt and Shift
    will.
  * Two new `MargoState` fields: `overview_cycle_pending` and
    `overview_cycle_modifier_mask`. Cleared by `open_overview`,
    `close_overview`, and `overview_activate`. `alt+Return` still
    works as the explicit commit path.

- **Overview cinematic selection — dim + thicker border on the
  pick.** Two new config keys, both clamped, both default-on:
  * `overview_selected_border_multiplier` (default `1.6`, range
    `[1.0, 4.0]`) — multiplies the normal border width on the
    keyboard / hover-selected thumbnail. Border already paints
    `focuscolor` on selection; the multiplier makes the pick read
    even at small thumbnail sizes without a separate render path.
  * `overview_dim_alpha` (default `0.6`, range `[0.1, 1.0]`) —
    alpha multiplier applied to **non-selected** thumbnails while
    overview is open. The selected thumbnail stays at full
    opacity. Result: a spotlight on the focuscolor-bordered
    selection, the cinematic feel niri/Hypr ship by default. The
    multiplier folds into the existing alpha parameter on
    `render_elements_from_surface_tree` (Wayland live surface) and
    the X11 `AsRenderElements` path, so no new render element
    type is needed — one f32 per window per frame.
  Set either to `1.0` to opt out individually.

- **Overview alt+Tab now MRU-ordered.** `overview_visible_clients`
  walks the per-monitor `focus_history` first (most-recent first),
  then appends any remaining visible clients in clients-vec order
  for completeness. Result: `alt+Tab` steps through windows in the
  order the user last touched them — matches every other alt+Tab
  in existence (i3, sway, Hypr, niri, GNOME). Previous behaviour
  cycled in map-then-rearrange order, which felt random when the
  user switched between long-running windows.

### Fixed

- **Overview alt+Tab border lit up instantly.** The cycle path
  (`overview_focus_step`) was running a snap-no-slide
  `arrange_monitor` after every Tab press to push the new
  selection through the layout pipeline. Even at 1 ms duration,
  the arrange-time `border::refresh` ran against per-client move
  state in flux and the focuscolor border landed one frame after
  the user expected. Removed the arrange call entirely — Mango-ext
  overview is a Grid layout, every cell stays put across a cycle,
  and only the *selected* state changes. The cycle now flips
  `is_overview_hovered`, calls `border::refresh`, requests a
  repaint — single render to focuscolor, no animation gate, no
  recompute. ("border anında diğer pencerede değil" → fixed.)

### Changed

- **Overview switched from fixed 3×3 per-tag thumbnails to mango-ext
  `overview(m) { grid(m); }` semantics.** The per-tag thumbnail grid
  always carved the work area into 9 cells regardless of window
  count, so a tag with 1-2 windows ended up at ~⅓ × ⅓ of the screen
  — "küçük gözüküyor, natif değil." Mango-ext's overview is just a
  Grid layout over all visible clients (`tagset = !0` + Grid +
  floating-included filter), so cell count = window count. Net
  effect: 1 window ≈ 90% × 90% of the screen, 2 → side-by-side
  halves, 4 → 2 × 2 quarters, 9 → 3 × 3 evenly. Cells shrink as
  window count grows, matching the native MangoWM feel.
  * Removed `MargoState::arrange_overview_per_tag_grid` helper
    (~95 LOC including doc) and its `is_overview` branch in
    `arrange_monitor`.
  * The `is_overview` setup at the top of `arrange_monitor`
    (`layout = Grid` + `tagset = !0` + `is_tiled` filter relaxed)
    is now sufficient — a single `layout::arrange(layout, &ctx)`
    call produces the dynamic grid.
  * hot-corner / alt+Tab cycle / alt+Return commit / 4-finger
    swipe / snap-no-slide cycle animation all unchanged.

- **Overview reverted from "Infinite Spatial" back to Mango-style
  per-tag thumbnail grid.** Five commits of camera-pan canvas
  (foundation + state + nav + auto-fit + window-centred cycle) were
  reverted in one pass after live UX feedback: the live camera
  felt fiddly compared to a fixed-grid that the user's spatial
  memory could rely on. Final shape:
  * Fixed 3×3 grid (tag 1 top-left → tag 9 bottom-right). Same cell
    index every time, spatial memory carries.
  * Each thumbnail runs that tag's configured layout (Tile /
    Scroller / Grid / Canvas / …). Scroller tag stays
    scroller-shaped, grid tag stays grid-shaped.
  * alt+Tab MRU cycle keeps the snap-no-slide arrange from the
    spatial attempt — each Tab press lights `focuscolor` border
    on the new selection instantly, no animation kaos.
  * `spatial_overview` module + design doc + 7 dispatch actions +
    `OverviewMode`/`overview_mode` config + `MargoState::spatial`/
    `spatial_panning` fields + `SpatialCamera` + frame-tick
    momentum + scroll-zoom intercept + LMB-drag pan handler all
    removed. ~600 LOC out, simpler render path, no spatial state
    to debug.

### Added (replaces previous Phase 3 entries)

- **Phase 3 — Spatial Overview live navigation (3 / 3, final).**
  Mouse + scroll + keyboard navigation all wired through the
  spatial camera; momentum decays every frame on the animation
  tick. Phase 3 is now fully usable.
  * **Mouse left-drag on empty overview space** pans the camera —
    every motion event streams its delta through
    `pan_by_screen_delta` so velocity feeds momentum on release.
  * **Scroll wheel** zooms around the cursor (world point under
    the cursor stays fixed, niri/paperwm/Aerospace default).
  * **Keyboard:** seven new dispatch actions —
    `overview_pan_left/right/up/down` step ¼ of the panel each,
    `overview_zoom_in/out` × 1.2 / × 1/1.2, `overview_zoom_reset`
    snaps to active tag at config zoom. Bind any of them inside
    overview for accessibility / no-mouse flows.
  * **Frame tick:** the same `tick_animations` hop that drives
    window animations now also ticks `MargoState::spatial`. While
    momentum is non-zero or the camera is interpolating toward a
    target, `arrange_all` runs and the next frame schedules — so
    the camera keeps coasting until friction settles it
    (`FRICTION = 0.92` per frame, `VELOCITY_FLOOR = 0.5 px/frame`
    snap-to-rest).
  * **mctl actions** catalogue grew seven entries documenting the
    pan/zoom/reset surface.

  Phase 3 is now feature-complete. Bind freely:

  ```ini
  overview_mode        = spatial      # default
  overview_zoom        = 0.5
  overview_transition_ms = 180
  hot_corner_top_left  = toggle_overview
  bind = alt,Tab,overview_focus_next
  bind = alt+shift,Tab,overview_focus_prev
  bind = alt,Return,overview_activate
  bind = super,Left,overview_pan_left
  bind = super,Right,overview_pan_right
  bind = super,Up,overview_pan_up
  bind = super,Down,overview_pan_down
  bind = super,equal,overview_zoom_in
  bind = super,minus,overview_zoom_out
  bind = super,0,overview_zoom_reset
  ```

- **Phase 3 — Spatial Overview wired into arrange + state (2 / 3).**
  Spatial mode is now the default at config level (`overview_mode =
  spatial` — opt out with `overview_mode = grid`). On open,
  `arrange_monitor` branches into the new
  `arrange_spatial_overview_geometries` helper:
  * Every tag's clients arrange in **that tag's** configured layout
    (Tile / Scroller / Grid / Canvas / …) inside a monitor-sized
    world slot — no override to a single Grid.
  * Each client's world rect (tag anchor + local layout output) is
    transformed through `SpatialCamera::world_to_screen` to land
    its `geom` on screen. Render, border, hit-test paths all read
    `client.geom` unchanged — they don't know spatial mode is on.
  * `open_overview` snaps `MargoState::spatial` to the active tag's
    world centre at `overview_zoom` so the open transition reads
    as "stay where I was, zoom out".

  Camera is loaded at default centred-zero state from
  `MargoState::new`; pan/zoom input handlers + frame-tick momentum
  arrive in commit 3 (final slice). Until commit 3 ships, spatial
  overview displays correctly but is static — exactly the visual
  the design doc calls for, just without live navigation.

- **Phase 3 — Infinite Spatial Overview, foundation (1 / 3).** New
  module `margo/src/spatial_overview.rs` (~450 LOC, 12 unit tests)
  carrying the foundation for the spatial canvas overview that
  replaces the legacy single-Grid overview as the default in
  commit 3 of this slice. Design doc at
  `docs/design/spatial-overview.md` covers the whole arc.

  This commit is foundation-only — no behaviour change:
  * `OverviewMode { Grid, Spatial }` enum + `from_config_str`
    parser (Grid alias: `grid` / `legacy` / `flat`; Spatial alias:
    `spatial` / `infinite` / `canvas`)
  * `SpatialCamera` struct — current + target position, momentum
    velocities, zoom clamps (`ZOOM_MIN = 0.1`, `ZOOM_MAX = 1.5`),
    friction (0.92 per frame), velocity floor (0.5 px/frame for
    snap-to-zero)
  * Methods: `snap_to` (hard re-centre), `pan_to` / `zoom_to_target`
    (set targets without snapping), `pan_by_screen_delta` (mouse
    drag), `zoom_around_screen_point` (scroll-zoom keeps the
    cursor's world point fixed), `tick` (per-frame integration:
    momentum → target, friction, smooth-step current → target)
  * Coordinate transforms `world_to_screen` / `screen_to_world` —
    the single transform every consumer goes through, so arrange,
    render, and input can't drift out of step
  * World layout: `tag_anchor` (3×3 grid, tag 1 top-left, tag 9
    bottom-right), `client_world_rect` (tag anchor + local layout
    rect), `world_bounds`
  * `TAG_PADDING` const (64 logical px between tag slots)

  Commit 2 (next) wires `MargoState::spatial`, `arrange_monitor`
  spatial branch, render path passthrough. Commit 3 adds input
  handlers (mouse pan, scroll zoom, keyboard dispatches),
  frame-tick momentum decay, and spatial-aware
  `overview_focus_next/_prev`.

### Fixed

- **Hot corner no longer leaks through to the lock screen.**
  `update_hot_corner` now early-exits when `session_locked` is true,
  when the screenshot region selector is active, or when smithay
  holds a pointer / keyboard grab (xdg_popup grabs, drag-and-drop).
  Symptom was: pointer in the top-left corner while the lock surface
  owned focus → `dispatch_action("toggle_overview")` fired → Tab /
  Return reached greetd's authentication form and the user landed
  in the login screen. Three guards added; armed_at stays None so
  re-entry restarts the timer cleanly after the guard lifts.

- **`overview_focus_next/_prev` border highlight tracks the
  selection.** The previous attempt called `focus_surface` on every
  Tab press, which fired margo's focus-crossfade opacity animation
  for each step. The crossfade re-painted all borders mid-cycle
  (interpolating between focuscolor and bordercolor), so the
  visible cue was "cursor warps but borders all look smudged".
  Now the cycle relies on the `is_overview_hovered` path that
  `border::refresh` already paints with `focuscolor`
  (`border.rs:64`), with no crossfade kick. Border, cursor, and
  hover flag move together on every Tab; commit goes through
  `overview_activate` (Enter), which runs the focus path once.

### Changed

- **Overview rewritten — Mango/Hypr geometric continuity + niri
  alt+Tab MRU cycle.** The Round 2b/3/4 per-tag thumbnail grid is
  reverted in favour of the previous "single Grid arrangement of
  every visible client over the zoomed work area" — windows keep a
  deterministic spot in the thumbnail, overview reads as a
  zoom-out of the desktop, the user's spatial memory survives.
  `arrange_overview_per_tag_grid`, `compute_overview_grid_layout`,
  `overview_cell_rect`, `overview_cell_at_cursor`,
  `overview_client_at_cursor`, and `OverviewDrag` all removed —
  ~600 LOC out, much simpler render path, no drift between three
  grid-math implementations. Round 1 (hot corner + zoom config +
  4-finger swipe wiring) and Round 2a (geometric zoom +
  transition_ms wiring) stay.

  `overview_focus_step` now opens the overview on its first press
  if it's closed, and every step calls
  `focus_surface(FocusTarget::Window(...))` so border + smithay
  keyboard focus track the cycle. Bind to alt+Tab and the gesture
  feels like a real alt+Tab on every other DE: focus moves with
  the selection, overview stays open between presses, commit via
  Enter (`overview_activate`).

  Try it:
  ```
  bind = alt,Tab,overview_focus_next
  bind = alt+shift,Tab,overview_focus_prev
  bind = alt,Return,overview_activate
  hot_corner_top_left = toggle_overview
  gesture = swipe, 4, up, toggle_overview
  overview_zoom = 0.5
  ```

  `mctl actions` catalogue now documents the three nav actions
  (`overview_focus_next`, `_prev`, `_activate`) explicitly with
  the new auto-open / focus-follows behaviour.

  **Phase 3 mandate (separate sprint):** "Infinite Spatial Overview"
  — workspace → space, 2D pan-zoomable canvas, semantic grouping,
  inertial camera, minimap. Design doc + opt-in `overview_mode =
  spatial` config. Not in this sprint; this overview ships now,
  spatial mode lands as an alternative later.

### Added

- **Niri-overview port — Round 4 (dynamic grid).** The overview no
  longer hard-codes a 3×3 grid of all 9 tags; instead, only tags
  with visible clients on the monitor are shown, and the grid
  shape is picked to fit: 1 occupied → 1×1 (full-screen
  thumbnail), 2 → 2×1, 3 → 3×1, 4 → 2×2, 5–6 → 3×2, 7–9 → 3×3.
  Even at `overview_zoom = 1.0` thumbnails were too small on a
  1080p monitor because we were always burning 6 cells of pixel
  budget on empty tags; now a single-tag day uses the whole
  screen. While a drag is past the 5 px threshold every tag is
  shown so empty tags become valid drop targets — drag UX
  unchanged. New `MargoState::compute_overview_grid_layout`
  helper is the single source of truth for the cell list;
  `arrange_overview_per_tag_grid`, `overview_cell_rect`, and
  `overview_cell_at_cursor` all consume it. Three-way drift gone.

- **Niri-overview port — Round 3 (mouse drag-and-drop windows across
  tags).** Inside the overview, left-press on a window thumbnail
  starts a drag; cursor motion past 5 px arms drag mode and
  highlights the target tag's cell with a `focuscolor` border;
  release on a cell rect retags the dragged window to that tag and
  re-arranges (overview stays open so the user can keep moving
  things). Release below the 5 px threshold, or outside any cell,
  falls back to the legacy click-to-activate-and-close behaviour —
  so a quick click on a thumbnail still opens that window like
  before. New `MargoState::overview_drag: Option<OverviewDrag>`
  state, plus `overview_cell_at_cursor` / `overview_cell_rect` /
  `overview_client_at_cursor` hit-test helpers (kept in math
  lock-step with `arrange_overview_per_tag_grid`). Visual feedback
  is a 4 px accent outline around the target cell — drawn after
  cursor so the cursor stays on top, before `upper_layers` so the
  bar still wins z-order.

  niri's "drag a window across workspaces" feature, adapted: niri
  inserts new workspaces between drop columns; margo doesn't
  (tags are abstract, no spatial "between"), so the drop simply
  retags onto the cell-tag.

### Changed

- **`toggle_overview` is the single dispatch name.** The
  `toggleoverview` / `toggle-overview` / `overview` aliases that
  briefly landed in 0.1.8 have been removed in favour of one
  canonical name. Update any keybinds / hot-corner config strings
  that used the underscore-less spelling. The `mctl actions`
  catalogue entry now reads `toggle_overview`.

- **Niri-overview port — Round 2b (per-tag thumbnails).** Overview
  no longer dumps every visible window into one Grid; instead, each
  tag (1-9) gets its own thumbnail cell in a 3×3 layout over the
  zoomed work area, and *each cell runs that tag's configured
  layout* — a scroller tag stays scroller-shaped at thumbnail size,
  a grid tag stays grid-shaped, etc. Per-tag `mfact` / `nmaster` /
  layout from `Pertag::ltidxs` flow through unchanged. Empty tags
  get an empty cell. Tag → cell mapping: tag 1 top-left, tag 9
  bottom-right (matches the 1-9 keypad mental model). New
  `MargoState::arrange_overview_per_tag_grid` helper drives the
  cell-by-cell arrange; `arrange_monitor` branches into it when
  `is_overview` is set. Round 3's drag-and-drop will hit-test
  against these cell rects to drop windows onto target tags.

- **Niri-overview port — Round 2a (geometric zoom + transition wiring).**
  `overview_zoom` (added in 0.1.8) is now consumed by
  `arrange_monitor`: while overview is open, the work area shrinks
  to `zoom × work_area` centered inside the monitor's logical work
  rect, so windows arrange inside a smaller, centered region —
  niri's "zoom 0.5" feel without a true scene-tree transform.
  Layer-shell positioning is unchanged on purpose: top + overlay
  layers (the bar) stay anchored to the panel edges, matching
  niri's "background + bottom would zoom in lockstep, top + overlay
  stay at 1.0" pattern. `overview_transition_ms` config is now
  honoured via a new `overview_transition_ms()` helper (fallback
  180 ms when config value is 0).

  Round 2b (per-tag thumbnails — every tag gets its own mini-layout
  area, not the current "every window in one Grid") and Round 3
  (mouse drag-and-drop windows across tags inside overview) are
  the next two slices.

## [0.1.8] – 2026-05-10

Niri-overview port — Round 1 (trigger mechanics). The next two rounds
(zoom-out / layer-shell handling, and mouse drag-and-drop windows
across tags) ship as follow-up releases.

### Added

- **Hot corner trigger.** Pointer dwelling in a 1×1-logical-pixel
  rectangle at any of the four output corners fires a configured
  dispatch action — niri pattern with a dwell threshold so a quick
  flick past the corner doesn't trigger. Per-corner config; default
  is "off" until the user opts in.

  ```ini
  hot_corner_top_left      = toggle_overview
  hot_corner_top_right     =
  hot_corner_bottom_left   =
  hot_corner_bottom_right  =
  hot_corner_dwell_ms      = 100
  ```

  Cleared on pointer-leave so out-and-back-in restarts the timer
  (matches niri). Action string accepts every known dispatch name
  (`toggleoverview` / `toggle_overview` / `toggle-overview` /
  `overview` all alias to the same handler).
- **Overview config knobs.** Two new fields:
  - `overview_zoom` (default `0.5`, clamped `[0.1, 1.0]`) — wired in
    config + state today; the Round-2 layer-shell + zoom-out render
    pass consumes it.
  - `overview_transition_ms` (default `180`) — replaces the
    previously-hardcoded transition duration.
- **`toggle_overview` dispatch aliases.** The handler used to only
  accept the no-underscore `toggleoverview` string; now also takes
  `toggle_overview`, `toggle-overview`, and bare `overview` so
  config strings written to the new hot-corner fields don't have to
  guess the spelling. The same handler underpins the existing
  keybind path and the (already-supported) 4-finger swipe-up
  gesture binding:

  ```ini
  bind = super,grave,toggle_overview
  gesture = swipe, 4, up, toggle_overview
  ```

### Changed

- `MargoState` gains `hot_corner_dwelling: Option<HotCorner>` +
  `hot_corner_armed_at: Option<Instant>` to drive the dwell timer.
  `update_hot_corner()` runs at the tail of every `pointer_motion`
  handler — cheap (4 corner checks per output, no allocation).

### What's coming in Round 2 / 3

- **Round 2 (next release):** real zoom-out rendering (overview
  thumbnails respect `overview_zoom`), layer-shell handling
  (background + bottom layers zoom along, overlay + top stay at
  1.0 — niri pattern).
- **Round 3:** mouse drag-and-drop windows across tags inside the
  overview, with target-tag visual highlight.

## [0.1.7] – 2026-05-10

First Phase 2 release. Single user-facing feature: a real fix for
fullscreen — the prior `togglefullscreen` looked full-screen but the
bar (noctalia / wlr-bar) kept rendering on top, covering the
window's top portion. Now there are two distinct fullscreen modes,
each on its own keybind.

### Added

- **`togglefullscreen_exclusive` dispatch action.** True fullscreen:
  window resizes to `monitor_area` (entire output) and the render
  path suppresses every layer-shell surface on that monitor — the
  bar literally disappears while exclusive fullscreen is active.
  Right behaviour for mpv / browser fullscreen movie / fullscreen
  games. Aliases: `togglefullscreen-exclusive`,
  `togglefullscreenexclusive`.

  ```ini
  bind = super,f,togglefullscreen
  bind = super+shift,f,togglefullscreen_exclusive
  ```

### Changed

- **`togglefullscreen` now respects `work_area`.** The default
  fullscreen action used to size the window to the full
  `monitor_area` even though the layer-shell bar kept rendering on
  top — the window's top region was permanently covered. Now the
  window resizes to `monitors[].work_area` (after layer-shell
  exclusion zones), so the bar stays visible and the window covers
  every other pixel below it. Standard `F11` feel.
- **`MargoClient` gains a `fullscreen_mode: FullscreenMode { Off,
  WorkArea, Exclusive }` field** alongside the existing
  `is_fullscreen: bool`. The bool is kept in lock-step
  (`is_fullscreen == fullscreen_mode != Off`) for backward-compat
  with 20+ callsites in render / IPC / window-rule paths;
  `set_client_fullscreen_mode(idx, mode)` is the new source of
  truth and `set_client_fullscreen(idx, bool)` shims to
  `WorkArea`. `xdg_toplevel` size hint matches the active mode so
  client first-frame buffer allocations land correctly.

## [0.1.6] – 2026-05-10

A `mvisual` UX hot-fix. `cargo run -p mvisual` flashed a window for a
single frame and exited — the design tool was unusable.

### Fixed

- **`mvisual` window no longer flashes-and-quits.** GApplication
  registers itself on the session bus by default; if a stale
  `dev.margo.visual` name was still claimed (most commonly: a previous
  `cargo run` session whose dbus name hadn't been released), the
  second start registered as *remote*, forwarded the `activate` signal
  to the (now-dead) primary, and exited immediately. Symptom was a
  window appearing on screen for one frame then disappearing,
  with no error output. Fixed by passing
  `gio::ApplicationFlags::NON_UNIQUE` on the Application builder —
  mvisual is a developer / design tool, multiple parallel instances
  are intentional.

## [0.1.5] – 2026-05-10

A 0.1.4 hot-fix. The `theme` / `session-save` / `session-load`
subcommands wired in 0.1.4 ran without error but had no visible
effect — every preset switch silently fell through to `default`.
`mctl run <file>` was carrying the same latent bug. One commit, one
slot-fix; everything user-facing actually works now.

### Fixed

- **`mctl theme <preset>` payload now reaches the dispatch handler.**
  dwl-ipc-v2's `dispatch` request takes 5 string slots; margo maps
  them as `arg1 → arg.i` (numeric parse), `arg2 → arg.i2`,
  `arg3 → arg.f`, `arg4 → arg.v` (string), `arg5 → arg.v2`. The
  0.1.4 `Theme { preset }` clap variant was stuffing the preset
  into slot 1 — the i32 parse silently failed, `arg.v` stayed
  `None`, and `theme gaudy` quietly resolved to the `default`
  preset. Now lands in slot 4 alongside the convention every other
  string-payload dispatch follows. `session-save` / `session-load`
  don't take args so they were already correct; the latent
  `mctl run <file>` bug (path stuffed into slot 1, `run_script`
  handler reads `arg.v`) is fixed in the same pass.

## [0.1.4] – 2026-05-10

A "0.1.3 follow-up" release. The 0.1.3 commit added the `theme` /
`session_save` / `session_load` dispatch actions on the compositor
side but didn't wire them as `mctl` clap subcommands — running
`mctl theme gaudy` died with "unrecognized subcommand". This fixes
that, plus a hot-path structured-logging migration and a road-map
reorganisation that were already pending in `[Unreleased]`.

### Fixed

- **`mctl theme` / `mctl session-save` / `mctl session-load`
  subcommands.** Three new `Command` variants in the `mctl` clap
  parser route through the existing dispatch path. No
  compositor-side change — the dispatch handlers landed in 0.1.3,
  only the CLI surface was missing. `mctl --help` now lists all
  three; `session-save`/`session-load` accept the underscore alias
  too for symmetry with the action name.

### Changed

- **Hot-path logging migrated to `tracing` structured fields.**
  `backend/udev/frame.rs`, `backend/udev/hotplug.rs`, and the gesture +
  keybinding-match log lines in `input_handler.rs` now emit per-event
  fields (`output = %name`, `reason = …`, `queued = …`, `error = ?e`)
  instead of pre-formatted strings. Run with `tracing-subscriber`'s
  JSON formatter and `journalctl -u margo --output=json | jq` slices
  per-output traces cleanly. Cold-path callsites (state.rs focus /
  dispatch chatter, scripting, plugin loader) still use the old
  format-string shape and convert piecemeal as touched. Roadmap §16
  do-over wishlist item.

### Docs

- **Roadmap §15 reorganised into "Outstanding work — external
  triggers"** with three sub-tables: upstream-blocked (smithay PRs),
  test-setup-deferred (live PipeWire), and hardware-driven (W2.2b
  pixman, W2.3 tablet). All margo-internal long-tail items are
  shipped — what's left is gated on something margo can't unblock by
  itself. §16 do-over wishlist marks the WindowRuleReason and
  RenderTarget refactors as shipped/partial; structured logging note
  added.

## [0.1.3] – 2026-05-10

A "post-W-sweep capability + cleanup" pass. Four features and three
refactors land between the 0.1.2 release and now; together they close
out every internal long-tail item the road map flagged.

### Added

- **`mctl theme <preset>` — live visual theme switch.** Three built-in
  presets (`default` / `minimal` / `gaudy`) toggle border thickness,
  shadow depth, blur, and corner radius without touching the config
  file. First switch captures a `theme_baseline` snapshot so
  `default` always reverts to "what the config said"; `mctl reload`
  invalidates the baseline so the next `default` lands the freshly-
  parsed values. (`feat(theme)`)
- **`mctl session save` / `mctl session load`.** JSON snapshot of
  every monitor's tag selection, per-tag layout / mfact / nmaster /
  canvas-pan to `$XDG_STATE_HOME/margo/session.json`. Atomic write
  via temp + rename so a crash mid-write can't shadow a good file.
  Open windows aren't captured (clients are bound to processes — the
  spawn line lives in user-space). Snapshot entries for absent
  monitors are logged + skipped on load. Versioned format with
  rejection on mismatch. (`feat(session)`)
- **Touchscreen multi-finger swipe → `gesture_bindings` dispatch.**
  True touch events (TouchDown/Motion/Up) are now distilled into
  the same `(fingers, motion, mods) → action` lookup the touchpad
  swipe path uses. A binding written as `gesture = swipe, 3,
  right, view_tag` fires regardless of input surface. (`feat(input)`)
- **`presentation-time` real per-output VBlank seq.** The `seq` field
  in `wp_presentation_feedback.presented` was hardcoded to 0; it's
  now a monotonic `OutputDevice::vblank_seq` bumped at the head of
  every `DrmEvent::VBlank` handler. Frame-pacing-sensitive consumers
  (mpv `--vo=gpu-next`, kitty render loop, gnome-shell's
  `getRefreshRate` polling) now see the contract the protocol
  promises. (`feat(presentation-time)`)

### Changed

- **Window-rule reapply unified via `WindowRuleReason` enum.** Three
  trigger sites (`finalize_initial_map`, late `app_id` settle,
  `mctl reload`) previously called `apply_window_rules_to_client`
  with no shared signal of *why* a rule was firing. New
  `WindowRuleReason::{InitialMap, AppIdSettled, Reload}` is passed
  to a single `reapply_rules(idx, reason)` path; the debug log
  records the trigger so a `RUST_LOG=margo::state::windowrule=debug`
  trace tells you which call site landed. Roadmap §16 #4 do-over
  wishlist item. (`refactor(state)`)
- **`RenderTarget` enum replaces `(include_cursor, for_screencast)`
  bool pair.** `build_render_elements_inner` callsites now read
  `RenderTarget::Display` / `DisplayNoCursor` / `Screencast { .. }`
  instead of two anonymous booleans the reader had to remember the
  meaning of. Internal `flags()` helper unpacks back into the same
  two bools the function body still uses, so the hot path is
  unchanged. Partial address of roadmap §16 #1. (`refactor(udev)`)

## [0.1.2] – 2026-05-10

A "catch-and-surpass-niri sweep" tail-end release. Three commits land
the last three queued W-items: a GTK4 design tool, HDR Phase 4 ICC
scaffolding, and the udev backend split into focused sub-modules. No
behaviour changes for existing daily-driver flows — the W-sweep is
about coverage and architecture, and the test suite (181 passing) +
clippy gate stay green at every step.

### Added

- **`mvisual` design tool (W4.5).** New workspace binary
  (`cargo run -p mvisual`) renders all 14 tile-able layouts side-by-side
  as live thumbnails plus a 1‒9 tag rail that mirrors the compositor's
  `Pertag` so users can rehearse per-tag layout pinning before
  committing to a config. GTK4-rs UI; live re-arrange on every
  parameter tweak (window count / mfact / nmaster / inner+outer gaps /
  focus index / scroller proportion). Wider than `niri-visual-tests`
  on two axes: every layout visible at once (no click-cycle), plus
  the per-tag pinning preview niri can't host since it has no tags.
- **`margo-layouts` workspace crate.** Pure layout arithmetic
  (~1040 LOC, no smithay/wlroots deps) extracted from
  `margo/src/layout/{mod,algorithms}.rs` so the compositor binary
  and `mvisual` consume the exact same `arrange()`. The 38-snapshot
  layout regression suite stays in place, just retargeted at the new
  crate.
- **HDR Phase 4 — per-output ICC profiles (scaffolding).**
  `margo/src/render/icc_lut.rs` (~390 LOC, 6 unit tests). `colord`
  D-Bus client (`org.freedesktop.ColorManager` + Device + Profile
  proxies) resolves a DRM connector name → assigned ICC path;
  `lcms2`-backed `bake_lut` runs an identity 33³ grid through
  sRGB → display-profile transform; `to_atlas_rgba32f` re-lays the
  cube as a 1089 × 33 RGB texture so the GLES2 path can sample it
  without a `sampler3D`. CPU-side trilinear sampler doubles as the
  GLSL reference for the `ICC_LUT_FRAG` shader (ships as `const`).
  `MARGO_HDR_ICC=1` env gate. Runtime activation upstream-blocked
  on smithay's `compile_custom_texture_shader` exposing a
  second-sampler hook.

### Changed

- **`backend/udev.rs` (3934 LOC) split into 4 sub-modules (W4.1).**
  `backend/udev/` is now a directory: `mod.rs` (2873, ~27 % shrink,
  the orchestrator), `helpers.rs` (77, transform / CRTC pick /
  refresh-duration / monotonic clock), `mode.rs` (234, mode select +
  apply via `DrmCompositor::use_mode`), `hotplug.rs` (405, rescan +
  setup_connector + migrate-clients-off-output), `frame.rs` (331,
  render dispatch + presentation feedback + scanout flags). Type
  visibility for `OutputDevice` / `BackendData` / `GammaProps` lifted
  to `pub(super)` so submodules reach shared state without trait
  indirection. Behaviour-preserving — all 181 tests green at every
  extract step. The road map's earlier "split into separate crates"
  framing was rejected: niri's "7 backend crates" turn out to be
  smithay's *feature flags*, not crates, and the real wins
  (incremental compile + readability) land at sub-module granularity
  without trait-abstracting `MargoState` (~3000 LOC churn for no
  downstream consumer).

## [0.1.1] – 2026-05-10

A focused popup-handling bug-fix release. Three commits, one
chain of root causes — GTK and Chromium menus (Helium 3-dot,
Nemo right-click, file-picker dropdowns) were unusable because
xdg_popup wasn't being driven through the full xdg-shell
handshake. After this release, popups, right-click context
menus, and double-click navigation work as expected on every
xdg-shell client we've tested.

### Fixed

- **Initial configure for xdg_popups.** Margo's commit handler
  was pumping the initial `xdg_surface.configure` for toplevels
  and layer surfaces but never for popups. Without it, GTK and
  Chromium would create the popup, send a bufferless commit, and
  sit forever waiting for an ack — the popup was tracked
  internally but never mapped, and clients gave up silently.
  Visible symptom: Helium's 3-dot menu, Nemo's right-click
  context menu, and any GTK chevron dropdown did absolutely
  nothing on click; `GDK_BACKEND=x11` worked because XWayland
  takes a different protocol path. The commit handler now mirrors
  smithay anvil's pattern: find the popup via `PopupManager`, and
  if `is_initial_configure_sent()` is false, call `send_configure()`
  on the first commit. Also restores the original double-click
  navigation in Nemo, which was failing as a side effect of the
  same broken popup state.
- **Pointer/keyboard input no longer steals focus during an active
  grab.** Even after wiring up `PopupPointerGrab`/`PopupKeyboardGrab`,
  GTK and Chromium menus would still flicker open and close because
  `handle_pointer_button` and `apply_sloppy_focus` called
  `state.focus_surface(...)` *before* forwarding the click. The
  toplevel-level `focus_under()` lookup can't see popups (popups
  aren't in `state.space.elements()`), so it returned whichever
  toplevel the popup happened to overlap geometrically — and our
  side effects (`selected`, dwl-ipc broadcast, scripting hooks,
  border crossfade, sloppy-focus arrange) ran against the wrong
  window while the popup was still up. The visible symptoms were
  "menu opens for one frame, then closes", right-click producing a
  brief flash, and Nemo double-clicks getting routed as window
  focus swaps. Both call sites now skip our focus logic when
  `pointer.is_grabbed()` or `keyboard.is_grabbed()` — smithay's
  active grab owns focus routing for the duration, and dismissal
  re-establishes focus through the normal motion path.
- **`xdg_popup.grab` now sets up a real popup grab.** Browser
  context menus (Helium / Chromium right-click), Helium's 3-dot
  toolbar menu, Nemo's right-click context menu, GTK file-picker
  dropdowns, and any other popup that requests `xdg_popup.grab`
  could open and instantly dismiss because margo was only
  flipping keyboard focus to the popup wl_surface — pointer
  events kept being delivered to the parent toplevel, so the
  toplevel saw a click "outside" the popup it had just opened
  and tore the popup down. The visible symptom was "menu doesn't
  open" / "right-click doesn't work" / "double-click does
  nothing". Margo now goes through the standard smithay path:
  `PopupManager::grab_popup` validates the serial, ensures the
  popup is the topmost in its chain, and returns a `PopupGrab`;
  margo then installs that grab on both the keyboard
  (`PopupKeyboardGrab`) and pointer (`PopupPointerGrab`) so
  events drill through the popup hierarchy and clicks outside
  dismiss the chain. Implementing this required two trivial
  `From` impls — `From<PopupKind> for FocusTarget` and
  `From<FocusTarget> for WlSurface` — that the previous
  workaround had explicitly side-stepped.

## [0.1.0] – 2026-05-10

First public release. margo crosses from "in-progress Rust port of mango"
into "daily-driver Wayland compositor with full modern-protocol parity,
the dwm/dwl-style 9-tag workflow, 14-layout catalogue, niri-grade
animations, embedded scripting, an in-compositor screencast portal,
and HDR scaffolding." Every line in the workspace is original to this
project except for the deliberately-attributed portions of dwl, dwm,
sway, tinywl, and wlroots — see `LICENSE.*`.

### Compositor

- **Tag-based workflow** — nine multi-select tags per session,
  `view N` / `tag N`, dwm-style press-twice-for-back, per-tag
  home monitor (`tagrule = id:N, monitor_name:X`) with automatic
  warp on view, per-tag layout / mfact / nmaster pinning via
  `Pertag`, per-tag wallpaper hint surfaced through `state.json`
  for wallpaper daemons.
- **Layout catalogue** — `tile`, `right_tile`, `monocle`, `grid`,
  `deck`, `center_tile`, `scroller`, `vertical_tile`,
  `vertical_grid`, `vertical_scroller`, `vertical_deck`,
  `tgmix`, `canvas`, `dwindle`, plus a global overview mode.
  Each layout is a pure function of `ArrangeCtx → Vec<(idx, Rect)>`
  so every algorithm gets snapshot-tested against a committed
  text fixture.
- **Adaptive layout engine** — per-tag `user_picked_layout`
  sticky bit + window-count / aspect-ratio heuristic; user
  `setlayout` pins the choice, heuristic never overrides.
- **Spatial canvas** — PaperWM-style per-tag pan via
  `canvas_pan` / `canvas_reset` actions, threaded into 5 layout
  algorithms.
- **Animations** — niri-style analytical spring physics with
  mid-flight retarget for window movement, carefully-tuned
  bezier curves for open / close / tag / focus / layer
  transitions. All five animation types support both clocks
  via `animation_clock_*` per-domain config. Snapshot-driven
  open / close so there's no first-frame "pop" before the
  transition starts.
- **Drop shadows + rounded corners** — single-pass SDF GLES
  shader, no offscreen buffers; clipped-surface rounded-corner
  mask shared across windowed / fullscreen / animated paths.
- **Modern protocol stack** — `linux-dmabuf-v1` +
  `linux-drm-syncobj-v1` (Firefox / Chromium / GTK / Qt avoid
  SHM fallback), DMA-BUF screencopy (zero-copy GPU→GPU full-
  output capture), region-based screencopy crop, runtime
  `wlr-output-management-v1` (mode + scale + position changes
  apply live, kanshi compatible), `pointer_constraints_v1` +
  `relative_pointer_v1` (FPS games / Blender), `xdg_activation_v1`
  with strict-by-default anti-focus-steal policy,
  VBlank-accurate `presentation-time`, `wp_color_management_v1`
  (HDR Phase 1 protocol surface), `ext_idle_notifier_v1` +
  `idle-inhibit`, `text-input-v3` + `input-method-v2`,
  `ext-session-lock-v1`, `wlr-gamma-control-v1`,
  `ext-foreign-toplevel-list-v1`,
  `wp_single_pixel_buffer_v1`, `ext-image-capture-source-v1` +
  `ext-image-copy-capture-v1`.
- **Built-in xdg-desktop-portal-gnome backend** — five Mutter
  D-Bus interface shims (`org.gnome.Mutter.ScreenCast`,
  `.DisplayConfig`, `.Shell.Introspect`, `.Shell.Screenshot`,
  `.Mutter.ServiceChannel`) + a PipeWire pipeline that lights
  up the Window / Entire Screen tabs in browser meeting clients
  (Helium, Chromium, Edge, Brave) without a running gnome-shell.
  Includes paced rendering, per-cast damage tracking, embedded
  cursor + metadata cursor sidecar, full-decoration casts
  (borders / shadows / popups / animations / block-out come
  through to the share view), HiDPI scale handling, and live
  `windows_changed` updates so xdp-gnome's window picker stays
  fresh mid-share-dialog.
- **Window rules with PCRE2** — regex match by `app_id` /
  `title` / `exclude_*`, size constraints, floating geometry,
  per-rule animation overrides, `block_out_from_screencast`,
  scratchpad / named-scratchpad opt-in, CSD-allow whitelist.
  Late `app_id` / `title` reapply so Qt clients don't flicker.
- **Scratchpad system** — anonymous + named scratchpads,
  cross-monitor support, `single_scratchpad` mode, recovery
  via `unscratchpad_focused` and `super+ctrl+Escape` reset.
- **Embedded scripting** — Rhai 1.24 sandboxed engine with
  `dispatch(action, args)` plus state-introspection bindings
  (`current_tag`, `focused_appid`, `monitor_count`, …) and
  event hooks (`on_focus_change`, `on_tag_switch`,
  `on_window_open`, `on_window_close`) that fire from the
  compositor mainloop with a re-entrancy guard. Plugin
  packaging via `~/.config/margo/plugins/<name>/{plugin.toml,
  init.rhai}` discovers and loads multiple scripts; per-plugin
  errors don't take down the loader. `mctl run <file>` evaluates
  a script against the live engine for hot-edit workflows.
- **Hot reload** — `mctl reload` (and the bundled
  `Super+Ctrl+R` keybind) re-applies window rules, key binds,
  monitor topology, animation curves, and gestures without a
  logout. `mctl check-config` is the offline validator —
  exit 1 on regex compile errors, unknown fields, duplicate
  binds, or include-resolution loops.
- **HDR scaffolding (Phases 1 + 2 + 3)** —
  `wp_color_management_v1` global advertising primaries / TFs
  / parametric creator (Phase 1, shipped); fp16 linear-light
  composite math + GLSL shaders + spec-value verification
  (Phase 2, gated on smithay's swapchain reformat API);
  `HDR_OUTPUT_METADATA` blob writer + `EdidHdrBlock` parser
  (Phase 3, gated on smithay's `set_hdr_output_metadata`).
  Phase 4 (per-output ICC profiles) is queued.
- **dwl-ipc-v2 wire compat** — drop-in for noctalia,
  waybar-dwl, fnott, and any other dwl/mango widget. Rich
  state.json sidecar exposes `scratchpad_visible`,
  `scratchpad_hidden`, MRU `focus_history`, per-tag wallpaper.

### Companion tools

- **`mctl`** — IPC + dispatch CLI. Subcommands:
  `status` / `clients` / `outputs` / `focused` / `watch`
  (live JSON / table inspection), `dispatch` (40+ typed
  actions; mirrors `bind = …` argument shape),
  `actions [--names | --verbose]` (the dispatch catalogue),
  `rules --appid X --title Y --verbose` (offline rule
  introspection), `check-config` (offline validation),
  `reload`, `run <file>` (live Rhai eval), `spawn`,
  `migrate --from {hyprland, sway} <file>` (offline config
  translator). Stable JSON schema with `version: 1`.
- **`mlayout`** — named monitor-topology profiles for
  laptops with frequent dock changes. `mlayout suggest /
  list / set / save / edit`. Wraps `wlr-randr` against
  margo's `wlr-output-management-v1` handler so changes
  apply live without logout.
- **`mscreenshot`** — region / window / output capture.
  Wraps `grim` + `slurp` + `wl-copy` + an optional editor
  (`swappy` / `satty` if installed). Modes: `rec`, `area`,
  `screen`, `window`, `open`, `dir`. The in-compositor
  region selector (Print key default) replaces slurp's
  separate window with a dim-overlay + drag-rect UI on the
  margo render path itself.

### Architecture

- **State management** — `MargoState` lives in
  `margo/src/state.rs` (~6,100 LOC after the W4.2 split,
  down from 7,651). 15 protocol-handler impls extracted into
  `state/handlers/` files for incremental-compile wins
  (`xdg_decoration`, `session_lock`, `xdg_activation`,
  `layer_shell`, `color_management`, `idle`,
  `pointer_constraints`, `input_method`, `selection`,
  `gamma_control`, `screencopy`, `dmabuf`,
  `output_management`, `x11`, `xdg_shell`).
- **Workspace layout** — `margo` (compositor binary),
  `margo-config` (parser + types), `margo-ipc` (mctl + the
  dispatch action catalogue + Hyprland/Sway migrate),
  `mlayout`, `mscreenshot`. Pinned smithay revision
  `ff5fa7df`; Rust 1.85+.
- **Cargo features** — `dbus` (default; gates D-Bus +
  async-io), `xdp-gnome-screencast` (default; requires
  `dbus`; gates pipewire), `a11y` (off by default; gates
  AccessKit), `profile-with-tracy` (off by default; flips
  `tracy-client` to its full backend so a live Tracy GUI
  can connect). Three build configurations verified.
- **AccessKit a11y** — `accesskit_unix` adapter on a
  dedicated thread (zbus-on-mainloop deadlock avoidance),
  publishes the window list as accessible nodes. Orca and
  AT-SPI consumers can navigate margo's window state.
- **xwayland-satellite mode** — `--xwayland-satellite[=BIN]`
  spawns Supreeeme's xwayland-satellite as a separate
  process so X11 crashes can't take margo down.
  `--no-xwayland` disables X11 entirely. Default path stays
  in-tree (smithay `XWayland::spawn`).
- **Tracy profiler hooks** — six hot-path spans
  (`render_output`, `build_render_elements`,
  `arrange_monitor`, `tick_animations`, `handle_input`,
  `focus_surface`) compile to no-ops in normal builds.

### Test infrastructure

- **Layout snapshot suite** — 20 committed `.snap` text
  fixtures locking the geometry of all 14 layouts × multiple
  scenarios. Insta-based; pure text diff at PR review time.
- **Layout property tests** — 14 invariants verified across
  the full catalogue × {1, 2, 3, 5, 8} window counts × focus
  shift × gap-zero edge cases (cardinality, no-degenerate-rects,
  monocle / deck identity, tile-class disjointness, focus
  invariance for non-scroller layouts, scroller monotonic
  width growth, gap-zero work-area coverage, focus-centring
  invariant for every focused index).
- **Integration test fixture** — calloop-driven
  `Server` + `wayland-client` `Client` + `Fixture` harness
  (port of niri's `src/tests/{fixture,server,client}.rs`).
  All 15 W4.2-extracted protocol handlers have at least one
  integration test; **41 integration tests** across
  `xdg_shell`, `layer_shell`, `idle`, `xdg_decoration`,
  `session_lock`, `xdg_activation`, `pointer_constraints`,
  `gamma_control`, `screencopy`, `output_management`,
  `selection`, `globals`, plus negative-invariant pinning
  for `dmabuf` / `color_management` / `x11/xwm` (gated on
  backend prerequisites that the headless harness can't
  drive). Total in-tree workspace test count: **126**
  (compositor: 102 layout + property + integration; config
  parser: 9; mctl + ipc + migrate: 15).
- **Smoke testing** — `scripts/smoke-winit.sh` (build →
  spawn → IPC → reload → focus → kill → empty-status, runs
  in CI under Xvfb), `scripts/post-install-smoke.sh` (binary
  presence, example config parses, dispatch catalogue
  ≥30 entries, completion paths, license install).
- **Clippy gating** — workspace + all targets run under
  `-D warnings`; `clippy.toml` documents the
  smithay-handle interior-mutability allowlist.

### Documentation

- **Published site** at <https://kenanpelit.github.io/margo/>
  (mkdocs-material; deploy automated via
  `.github/workflows/docs.yml`). Pages: Overview, Install
  (Arch / source / Nix flake), Configuration overview,
  **Full configuration reference** (the entire annotated
  `config.example.conf` rendered inline via `pymdownx.snippets`,
  syntax-highlighted), Companion tools, Scripting, Manual
  checklist, three design notes (HDR, Built-in portal,
  Scripting engine), Roadmap, Contributing.
- **Annotated example config** — 1,028 lines at
  `margo/src/config.example.conf`; every option documented
  inline.
- **CONTRIBUTING.md + PR template** — quick-start build,
  code-layout map, lint posture, test workflow, conventional
  commit style, AI-contribution policy.

### Compatibility

- **Display managers** — ships `margo.desktop` (direct
  session) and `margo-uwsm.desktop` (UWSM-driven for
  systemd graphical-session.target plumbing).
- **Existing widgets / bars** — drop-in for noctalia,
  waybar-dwl, fnott via dwl-ipc-v2.
- **Migration** — `mctl migrate --from {hyprland, sway}`
  translates the high-value config subset (keybinds, spawn
  lines, workspace → tag bitmask conversions, modifier names,
  key aliases). Window rules / animations / monitor topology
  stay manual to avoid inventing wrong semantics.

### Packaging

- **Arch / makepkg** — PKGBUILD at the repo root installs
  `margo`, `mctl`, `mlayout`, `mscreenshot`, the wayland-
  session entries, the example layouts, the XDG portal
  preference at `/usr/share/xdg-desktop-portal/`, shell
  completions for bash / zsh / fish, and license headers
  for the dwl/dwm/sway/tinywl/wlroots inheritance chain.
- **Nix flake** — `flake.nix` exposes `packages.default`,
  `devShells.default` with `rust-analyzer` + `clippy`, plus
  `nixosModules.margo` and `hmModules.margo`.
- **GitHub Actions** — three workflows: `ci.yml`
  (build/test/clippy/check-config on every PR), `smoke.yml`
  (end-to-end nested-mode smoke under Xvfb), `docs.yml`
  (Pages deployment).

[Unreleased]: https://github.com/kenanpelit/margo/compare/v0.8.3...HEAD
[0.8.3]: https://github.com/kenanpelit/margo/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/kenanpelit/margo/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/kenanpelit/margo/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/kenanpelit/margo/compare/v0.7.9...v0.8.0
[0.7.9]: https://github.com/kenanpelit/margo/compare/v0.7.8...v0.7.9
[0.7.8]: https://github.com/kenanpelit/margo/compare/v0.7.7...v0.7.8
[0.7.7]: https://github.com/kenanpelit/margo/compare/v0.7.6...v0.7.7
[0.7.6]: https://github.com/kenanpelit/margo/releases/tag/v0.7.6
[0.1.0]: https://github.com/kenanpelit/margo/releases/tag/v0.1.0
