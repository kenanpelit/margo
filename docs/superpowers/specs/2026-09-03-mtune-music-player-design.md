# mtune — folder-first music player — design

**Date:** 2026-09-03
**Status:** Approved (design); implementation plan to follow.
**Crate:** `mtune` (new top-level workspace member)

## Goal

A native music player for the margo desktop that **plays everything under
one or more folders you point it at** — a persistent library root, scanned
recursively on launch and watched for changes while running — with:

- a branded **bar pill** and its own **bar menu** (`MenuType::Mtune`) in
  the shell,
- a **system-tray icon** (SNI) with transport controls and a dbusmenu,
- standard **MPRIS2** so `playerctl` / KDE Connect / other bars see it,
- matugen-aware theming plus the forked player's album-art recolour.

The player is a **fork of an upstream GPL-3.0 GTK music player** (an
`adw::OverlaySplitView` playlist-sidebar + player-content app; reference
screenshot: `~/Pictures/Screenshots/Amberol.png`). After the fork the
codebase is **independent** — no upstream rebase is planned — and the
upstream project's name, branding, bus names, type-name prefixes, icons,
and infra files must not survive anywhere in `mtune/`.

> Throughout this doc the upstream project is called **"upstream"**. The
> fork keeps its GPL-3.0 licence and an author-copyright attribution line
> in `mtune/licenses/`, with no project name.

## Non-goals

- No relm4 rewrite of the app UI — the fork keeps its raw gtk-rs
  subclassing + composite-template architecture. Only the shell pill/menu
  (which live in `mshell-frame`) are relm4, as every mshell widget is.
- No music-library *management* (tag editing, playlist files, ratings).
  mtune plays folders; it does not curate them.
- No separate `mtunectl` CLI in phase 1 (the pill/menu/tray talk D-Bus
  directly; `mshellctl` can proxy later if wanted).
- No change to `PKGBUILD` / the AUR package (per standing instruction —
  AUR is hand-updated only).

## Current state (what exists)

- **Upstream fork source** lives at `~/.kod/amberol` (working tree only —
  not a submodule, not vendored yet). ~7.7k LOC. Stack: gtk4 0.11
  (`v4_16`), libadwaita 0.9 (`v1_5`), gstreamer / gstreamer-play /
  gstreamer-audio 0.25, `lofty` 0.24, `mpris-server` 0.8, `color-thief`,
  `ashpd` 0.13 (background portal only). Builds with **meson** (which
  drives cargo); UI templates are **Blueprint** (`.blp`).
  - `audio/` (~2.5k LOC) — message-passing engine: `AudioPlayer` (`Rc`)
    ↔ controllers (`MprisController`, `InhibitController`) over
    `async-channel`; `GstBackend` wraps `gst_play::Play`; `Queue`
    (GListModel), `PlayerState` (GObject the UI binds to), `Song`
    (lofty), `CoverCache` (sha2-keyed), `ShuffleListModel`,
    `WaveformGenerator`.
  - UI — `window.rs` (~1.6k LOC, `open_files` + drag-drop + the
    `adw::OverlaySplitView`), `playback_control`, `playlist_view`,
    `queue_row`, `volume_control`, `waveform_view`, `cover_picture`
    (+ color-thief recolour via `--background-color-*` custom props),
    `marquee`, `search`, `sort`, `drag_overlay`.
  - `ashpd` is used in exactly one place — `application.rs`
    `Background::request()`, a courtesy "don't kill me when windowless"
    call. The actual keep-alive is `gio::ApplicationHoldGuard`
    (`self.hold()`).
- **margo workspace** — one Cargo workspace, one `Cargo.lock`. GUI stack
  pinned to the **gtk-rs 0.10 generation** (`gtk4 = "0.10"` `v4_20`,
  `relm4 = "0.10"`, `gtk4-layer-shell = "0.7"`, glib 0.20). **No
  libadwaita, no gstreamer anywhere.** `Cargo.toml` carries an explicit
  comment against pulling a second gtk-rs generation into the tree.
- **Shell already has generic media UI:** `BarWidget::MediaPlayer` +
  `menus/menu_widgets/media_player/` (786-LOC MPRIS/MPD menu) +
  `MenuType::MediaPlayer`, all driven by `wayle_media`. `system_tray`
  bar widget is an SNI **host** via `wayle-systray`. `Lyrics` pill too.
- **Config conventions** (`docs/config-conventions.md`): a standalone
  tool's config file sits at the config-dir **root** (like `mpower.toml`,
  `mlock.conf`), hand-edited and optionally machine-written; it is **not**
  `margo-config` and **not** `mshell-config` — a third world.
- **CI gate** (`just check`): `cargo fmt --check`, `clippy --all-targets
  -D warnings`, `scripts/panic-ratchet.sh` (repo-wide `.unwrap()/.expect(
  )/panic!(...)` count vs `scripts/panic-baseline.txt`, currently **315**;
  may only go down, or up with a justification in the commit),
  `scripts/design-lint.sh`, example-config parse, tests. Separate
  `cargo-deny` job; `deny.toml` has `multiple-versions = "warn"` (a dual
  gtk-rs generation is a **warning, not a failure**).

## Design

### 1. Identity

| Field | Value |
|---|---|
| Crate / binary | `mtune` |
| Display name | **Tune** (window title, MPRIS `identity`, About, tray tooltip) |
| App-ID / `app_id` (WM_CLASS) | `org.margo.Tune` |
| D-Bus names | `org.margo.Tune` (supplementary) + `org.mpris.MediaPlayer2.org.margo.Tune` |
| gschema | `org.margo.Tune` at path `/org/margo/Tune/` |
| gresource | `/org/margo/Tune/…` |
| Config file | `~/.config/margo/mtune.toml` (config-dir root, hand- + GUI-written) |
| Tag-index cache | `~/.cache/margo/mtune/index.json` |
| Icons | new `org.margo.Tune.svg` (colour) + `org.margo.Tune-symbolic.svg`; the symbolic is the tray logo |

**De-brand audit** — the fork must contain zero occurrences (case-
insensitive) of the upstream project name, `io.bassi` / `bassi` /
`ebassi`, upstream type-name prefixes (`<Upstream>Application`,
`"<Upstream>RepeatMode"`, …), the upstream `.gresource` name, upstream
Matrix / Discourse / GitLab URLs, upstream screenshots, `.doap`. A
CI grep-gate enforces it:

```
rg -i 'amberol|io\.bassi|ebassi' mtune/ && exit 1 || true
```

(the exact needle list is finalised during the fork; the gate script
lives at `scripts/` and runs inside `just check`).

### 2. Placement & dependency strategy

`mtune` is a **full workspace member** (sibling of `margo/`, `mkeys/`,
`mvpn/`): added to `Cargo.toml` `members`, `version.workspace = true`,
`[lints] workspace = true`, in `install.sh`'s binary loop, with a
`justfile` recipe and a slot in `all:`.

**Dependencies:**

- **Everything non-GUI → `workspace = true`**, exact match with margo:
  `serde`, `serde_json`, `toml`, `tokio` (if used), `zbus` (5), `clap`
  (if a CLI surface lands), `tracing` / `tracing-subscriber`, `anyhow`,
  `directories` / `dirs`, `regex`, `sha2`.
- **GUI stack → mtune-local, current generation** (not `workspace =
  true`): `gtk4` / `gdk4` 0.11, `libadwaita` 0.9, `gstreamer` /
  `gstreamer-play` / `gstreamer-audio` 0.25, `gdk-pixbuf` 0.22, `lofty`
  0.24, `mpris-server` 0.10, `color-thief`, `fuzzy-matcher`, `ksni`
  (tray), `notify` (folder watch), `ignore` or `walkdir` (scan).
- **`ashpd` is dropped entirely.** Keep-alive is a `gio::Application::
  hold()` guard, acquired while (tray active OR playback not stopped),
  released otherwise. No portal call. The native `gtk::FileDialog`
  covers folder picking with no portal.
- **Why the dual generation is safe:** `mtune` exchanges **no
  glib-typed values** with any other workspace crate. Theming reads the
  matugen palette as a **file**, not through a crate. Logging is
  `tracing`. So `Cargo.lock` carrying both gtk-rs 0.10 and 0.11 is a
  build-time cost (one extra gtk-rs compile, larger `target/`, a
  `cargo-deny` `multiple-versions` warning) with **no correctness
  risk**. `deny.toml` stays `warn`; new-dep licences (gstreamer
  MIT/Apache, libadwaita MIT, lofty MIT/Apache, color-thief MIT,
  mpris-server MIT, ksni, notify) are verified against `[licenses]
  allow` during the fork.
- If the whole workspace is later moved to current gtk-rs, `mtune`'s
  GUI deps collapse to `workspace = true`; that is a separate cleanup,
  out of scope here.

**Build-system swap (meson → cargo):**

| Upstream | Fork |
|---|---|
| `meson.build`, `meson_options.txt`, `build-aux/`, `subprojects/` | deleted |
| `config.rs.in` (meson-templated) | plain `config.rs` — consts + `env!("CARGO_PKG_VERSION")`; dev resource path behind `cfg(debug_assertions)` (avoids the `$srcdir` bake — see `reference_env_manifest_dir_srcdir_leak`) |
| Blueprint `.blp` (8 files) | compiled to `.ui` once (`blueprint-compiler compile`), the `.ui` committed, `.blp` deleted, `blueprint-compiler` build-dep removed |
| `.gresource` | `build.rs` + `glib-build-tools::compile_resources` (`.ui` + icons + `style.css`) |
| gschema | renamed; `install.sh` installs to `/usr/share/glib-2.0/schemas/` + runs `glib-compile-schemas` |
| `po/` (60+ translations) | infra kept; `LINGUAS` kept; `POTFILES.in` paths updated; project name scrubbed from `.po` headers |
| `.doap`, `code-of-conduct.md`, `.gitlab-ci.yml`, flatpak json, `RELEASING.md`, `data/screenshots/` | deleted |

**Panic ratchet:** the fork adds ~140 `.unwrap()/.expect()` sites. Plan:
harden `audio/` (a panic there kills playback) — ~30-40 sites converted
to `Result` + `tracing` + graceful degrade; the rest raise
`panic-baseline.txt` in one commit whose message states the rationale
(**mtune is a standalone application binary — a panic kills only the
music player, not the compositor or the bar**, which is the ratchet's
actual concern). Remaining cleanup tracked as follow-up.

### 3. App architecture — kept vs reskinned

**Kept as-is (identifiers renamed only):** the entire `audio/` engine
and every UI widget module listed in *Current state*. The
`adw::OverlaySplitView` two-pane layout (playlist sidebar + player
content) from the reference screenshot is preserved unchanged.

**Reskinned:**

- glib type-name prefixes, gresource path, PulseAudio props,
  `glib::set_application_name`, log-domain filter (`"mtune"`).
- `src/gtk/style.css` (~160 LOC) — re-themed to margo's design language:
  calm motion, matugen tokens where sensible, a monospace-leaning font
  stack matching the reference screenshot (user-overridable).
- `src/assets/icons/` — replaced with margo one-family symbolics.
- New app icon (colour + symbolic) — also the tray logo.
- About dialog, `.metainfo.xml`, `.desktop` — Tune + margo project URLs.

**Removed:** `ashpd` background block in `application.rs` (see §2).

### 4. Directory-library subsystem (new)

`~/.config/margo/mtune.toml` — mtune's own file, re-read on change (like
`mpower.toml`; **not** `config_manager()`):

```toml
[library]
roots = ["~/Music"]        # one or more persistent roots
scan_on_start = true       # rescan roots at launch vs trust the cached index
watch = true               # inotify the roots while running
recursive = true
# extensions = ["mp3","flac","ogg","opus","m4a","wav","aac","wma"]  # default: gstreamer+lofty support

[playback]
on_start = "resume"        # resume | library | nothing
gapless = true
replaygain = "off"         # off | track | album

[behaviour]
background_play = true      # keep playing when the window is closed
close_to_tray = true
single_instance = true
```

New modules under `src/library/`:

| Module | Responsibility |
|---|---|
| `config.rs` | serde parse/write `mtune.toml`; `~` expansion; defaults; a `notify` watch on the file itself for live re-read |
| `scanner.rs` | off-main-thread recursive walk (`ignore`/`walkdir`), extension filter, streams `Song`s into the `Queue` via channel as they are found (a 10k-file library must not block startup) |
| `watcher.rs` | debounced `notify` (inotify) on the roots; add/remove `Song`s live; gated on `library.watch` |
| `index.rs` | on-disk cache `~/.cache/margo/mtune/index.json` (path → mtime → parsed tags); per-file invalidation by mtime; lets a large library skip a full tag re-read each launch |

**Launch flow:** load `mtune.toml` → if `scan_on_start`, start `scanner`
on the roots in the background and stream results into the queue, showing
a subtle "scanning… N tracks" until done; else load `index.json`
immediately then background-reconcile → apply `playback.on_start` (resume
song+position from gschema, or start at library top, or idle) → if
`watch`, start `watcher`.

The upstream fork's ad-hoc "add folder" action is kept — persistent roots
sit **on top of** it. Library load order: folder, then album, then track
number (`sort.rs` already track-aware). Shuffle uses the existing
`ShuffleListModel`.

### 5. D-Bus surface

One zbus connection, two interfaces:

1. **Standard MPRIS2** — `org.mpris.MediaPlayer2` + `.Player`, already
   implemented by the fork's `MprisController` (identity rebranded).
   **Add `org.mpris.MediaPlayer2.TrackList`** so the queue is
   introspectable/controllable via a standard interface. This alone
   makes mtune visible to `playerctl`, KDE Connect, other bars, and
   margo's *generic* `MediaPlayer` pill.

2. **Supplementary `org.margo.Tune`** — what MPRIS can't express;
   consumed by the dedicated shell menu + tray:
   - **Properties:** `LibraryRoots: as`, `Scanning: b`,
     `ScanProgress: (uu)` (done, total), `QueueLength: u`,
     `CurrentIndex: i`, `RepeatMode: s`, `Shuffle: b`, `Volume: d`
   - **Methods:** `SetLibraryRoots(as)`, `AddFolder(s)`, `PlayFolder(s)`
     (replace queue with a folder), `RescanLibrary()`, `PlayIndex(u)`,
     `RemoveIndex(u)`, `SetRepeatMode(s)`, `SetShuffle(b)`, `Raise()`,
     `Quit()`
   - **Signal:** `Changed` — coalesced "something the menu shows moved".

   Same pattern as `mshell` ↔ `mshellctl`. No `mtunectl` binary.

### 6. System tray (SNI)

mtune registers its own `org.kde.StatusNotifierItem` +
`com.canonical.dbusmenu` via the **`ksni`** crate, on its own thread,
talking to the player through the same `async-channel`
`PlaybackAction` sender the MPRIS controller uses plus a state receiver.
`SniController` becomes another `Controller`-trait impl next to
`MprisController` / `InhibitController`.

| Interaction | Behaviour |
|---|---|
| Icon | mtune symbolic + a small play/pause state badge |
| Tooltip | `Title — Artist` / `Tune — idle` |
| Left click / Activate | toggle window (show+raise if hidden, hide-to-tray if visible) |
| Scroll | volume |
| Context menu | Play/Pause · Next · Previous · — · Shuffle (toggle) · Repeat ▸ (off / all / one) · — · Show Tune · Quit |

`close_to_tray` / `background_play`: the `gio::Application::hold()` guard
is held while the tray item is registered or playback ≠ stopped.

### 7. Shell integration

**Talk path.** Transport + metadata + cover + position → the existing
`wayle_media` plumbing, **filtered to the mtune bus name**
(`org.mpris.MediaPlayer2.org.margo.Tune`). Library / queue extras → a new
`mshell-services` singleton `mtune_service()` wrapping a zbus proxy to
`org.margo.Tune`. If mtune is not running, the pill shows a launch
affordance (click → spawn `mtune`).

**Bar pill — `bars/bar_widgets/mtune.rs`, `BarWidget::Mtune`.** Leads with
the **mtune icon** (not the generic media glyph) + `track — artist`
ellipsized; collapses to the icon when idle; `paused` dims via CSS. Left
click → toggle `MenuType::Mtune`; right click → play/pause in place. The
generic `MediaPlayer` pill stays separate (Spotify / browser / mpd).

**Bar menu — `MenuType::Mtune`, `menus/menu_widgets/mtune/`.** DESIGN.md
§5 card chrome, compact layer-shell panel, top to bottom:

| Region | Content |
|---|---|
| Cover | medium-large album art (album-art accent optional) |
| Meta | title / artist / album — marquee on overflow (existing pattern) |
| Seek | bar + `elapsed / remaining` |
| Transport | ⏮ ⏯ ⏭ circular, ≥40px (DESIGN §0.9) |
| Volume | slider + speaker icons |
| Toggles | shuffle · repeat (off/all/one cycle) |
| Library | "Choose folder…" (`gtk::FileDialog` → `PlayFolder` / `SetLibraryRoots`) + scan status ("↻ 1240/5000" while `Scanning`) |
| Queue peek | next 3–5 tracks (TrackList), click to jump |
| Footer | "Open Tune" (Raise) |

**Wiring — the 11 touch-points (DESIGN §6, copy `bluetooth` /
`audio_dashboard`):**

1. `bars/bar_widgets/mtune.rs` — pill emits `MtuneOutput::Clicked`
2. `bars/bar.rs` — `BarOutput::MtuneClicked` + `.forward(...)` dispatch
3. `menus/menu.rs` — `MenuType::Mtune` + match arm (css class + widgets/
   min-width/max-height from `config.menus().mtune_menu()`)
4. `menus/menu_widgets/mtune/` — the menu component (+ `mod.rs`, +
   `menu_widgets/mod.rs` entry)
5. `menus/builder.rs` — `MenuWidget::Mtune` → build it
6. `mshell-config/.../menu_widgets.rs` — `MenuWidget::Mtune` +
   `display_name()` + `all_defaults()`
7. `mshell-config/.../config.rs` — `mtune_menu: Menu` on `Menus` +
   `#[serde(default = "default_mtune_menu")]` + the default fn + entry
   in `Default for Menus`
8. `frame.rs` — `MTUNE_MENU` const, `Controller<MenuModel>` field,
   `ToggleMtuneMenu` FrameInput, `build_menu(...)`, struct-init entry,
   toggle handler, `add_to_stack(...)` with config position, and the
   `BarOutput::MtuneClicked => FrameInput::ToggleMtuneMenu` map
9. `mshell-core/.../relm_app.rs` — `ToggleMtuneMenu(Option<String>)`
   ShellInput + handler
10. `mshell-core/.../ipc.rs` — `IPCCommand::Mtune` + dispatch + `async
    fn mtune` interface method
11. `mshellctl/.../subcommands/menu.rs` — `MenuCommands::Mtune` +
    `bus_command("Mtune")`

**Settings registration:**

- **DESIGN §8a (required):** `MenuKind::Mtune` variant in
  `widget_menu_settings.rs` (`display_name`, `all()`, all 12 dispatch
  arms) + a `WidgetEntry::Menu { … }` row in `settings.rs` — so the
  pill/menu is movable & resizable.
- **DESIGN §8b (phase 6, optional):** a "Tune / Music" settings page
  (`mtune_settings.rs`, copy `idle_settings.rs`) — library roots as
  reorder rows, `on_start`, `close_to_tray`, `replaygain`, `gapless`.
  Writes `~/.config/margo/mtune.toml` directly; mtune re-reads on change.
  **Not** `config_manager()` (separate world). Phase 1–5 rely on the bar
  menu's folder picker for the essential need.

### 8. Theming

mtune loads CSS in order:

1. baked `style.css` (the reskinned fork CSS).
2. the matugen palette — read as a **file** (mshell-matugen's generated
   CSS; the `last_theme.css` path used by `mlock`, see
   `reference_mlock_render`), mapping matugen tokens → mtune's
   `--background-color-*` / accent vars. No crate dependency;
   generation-independent.
3. the color-thief album-art recolour still runs and overrides the
   accent per song (the fork's signature) — kept, toggleable from
   `mtune.toml`.

Font: the reskin sets a monospace-leaning stack (matching the reference
screenshot); user-overridable. `app_id = org.margo.Tune` lets margo
window rules target the window (float, tag pin, screencast blackout…).

## Testing

- **Pure units (host-independent):** `library/config.rs` (toml
  round-trip, `~` expansion, defaults), `library/scanner.rs` (fixture
  tree → expected track list, extension filter, recursion depth),
  `library/index.rs` (mtime invalidation), `sort.rs` (exists).
- **D-Bus:** smoke the `org.margo.Tune` interface against a stub service
  (method signatures + property reads).
- **CSS:** a `CssProvider` parse-error test over the gresource-embedded
  `style.css` (existing pattern).
- **No automated GUI test** — matches the fork's model (not relm4).
- **`just check`** covers `mtune` automatically (workspace member): fmt,
  clippy `--all-targets -D warnings`, panic-ratchet, design-lint (the
  new pill/menu SCSS in `mshell-style` is in scope), tests. Plus the
  de-brand grep-gate.
- **On-device (user runs it; no live key injection —
  `feedback_no_live_key_injection`):** launch `mtune` → library scans →
  plays → pill appears → bar menu opens → tray icon + right-click
  controls → close window, playback continues from tray → `playerctl`
  and KDE Connect see it.

## Phasing

| Phase | Scope | Done when |
|---|---|---|
| **1 — Fork + workspace** | §2 build swap + de-brand; `cargo build -p mtune` green; app runs with old behaviour, new name/theme scaffold | `just check` green, grep-gate clean, app opens + plays a folder |
| **2 — Directory library** | `src/library/` (config / scanner / watcher / index); auto-load on launch; `mtune.toml` | empty config → `~/Music` scanned recursively + playing; watcher picks up a new file |
| **3 — D-Bus + tray** | `org.margo.Tune` iface + `TrackList`; `ksni` tray + dbusmenu; `hold()` keep-alive; close-to-tray | tray icon in margo's `system_tray`, controls work, windowless playback |
| **4 — Shell pill + menu** | `BarWidget::Mtune` + `MenuType::Mtune` (11 points) + `mtune_service()` + `MenuKind::Mtune` §8a | pill + `mshellctl menu mtune` + Settings move/resize all work |
| **5 — Reskin + theme** | `style.css` re-theme, icons, matugen file-read, font; color-thief toggle | reference-screenshot layout + matugen accent; design-lint green |
| **6 — Settings page (opt.)** | §8b "Tune / Music" page (root reorder + knobs) | page writes `mtune.toml`, mtune live-re-reads |

Each phase is its own commit/PR. Phases 1–2 are sequential; 3–4–5 are
largely parallel.

## Risks / open questions

- **`libadwaita` 0.9 vs the system libadwaita runtime.** 0.9 targets a
  recent libadwaita 1.x; the host must have it. If it mismatches, the
  fallback is to drop adw (surface is small: `Application` /
  `ApplicationWindow` / `StyleManager` / `AboutDialog` / `StatusPage` /
  `Bin` → plain gtk are easy; `OverlaySplitView` + the `TimedAnimation`
  helpers in marquee/waveform/drag-overlay are the two non-trivial
  spots). Decision deferred to phase 1 once it actually builds on the
  host.
- **`ksni` and the glib main loop.** `ksni` runs its own thread /
  runtime; the design keeps it fully decoupled (channels only), so this
  should be fine, but phase 3 must confirm the tray thread shuts down
  cleanly on `Quit`.
- **gstreamer 0.25 ↔ glib 0.21** in the same `Cargo.lock` as gtk4 0.10 /
  glib 0.20. Expected: a `cargo-deny` `multiple-versions` warning and a
  longer clean build. Confirm no crate actually resolves to a single
  shared version that breaks (it shouldn't — the sets are disjoint).
- **Blueprint → `.ui` drift.** Committing generated `.ui` means an edit
  must regenerate; document the `blueprint-compiler` step, or convert
  the templates to hand-maintained `.ui` permanently (8 small files —
  likely the cleaner call).
- **Large library first scan.** `index.json` mitigates re-launch cost;
  the first-ever scan of a very large tree still reads every file's
  tags. Streaming into the queue keeps the UI responsive; a hard cap /
  progress cancel may be worth adding in phase 2 if it bites.
