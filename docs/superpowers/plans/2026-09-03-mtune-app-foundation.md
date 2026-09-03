# mtune App Foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fork the upstream GPL-3.0 GTK music player at `~/.kod/amberol` into a new, fully de-branded margo workspace crate `mtune` that builds with `cargo`, then add the folder-first library subsystem (persistent roots, recursive scan, inotify watch, tag-index cache) so `mtune` auto-plays everything under the configured music folders on launch.

**Architecture:** `mtune` is a top-level workspace member alongside `margo/`, `mkeys/`, `mvpn/`. The upstream `audio/` engine and all UI widget modules are copied verbatim and only *renamed* (no relm4 rewrite). The meson build is replaced by a `build.rs` + `glib-build-tools` gresource compile; Blueprint `.blp` templates are compiled to `.ui` once and committed. The new `src/library/` subsystem (config, scanner, watcher, index) plugs into the existing `Queue` / `Song` API — `Song::from_uri` + `Queue::add_song` — and into `Application` startup.

**Tech Stack:** Rust (edition 2024), gtk4 0.11 + libadwaita 0.9 + gtk-rs (mtune-local, current generation), gstreamer / gstreamer-play / gstreamer-audio 0.25, `lofty` 0.24, `mpris-server` 0.10, `serde` + `toml`, `notify` (inotify), `ignore` or `walkdir` (scan), `glib-build-tools` (build). Cargo workspace, `just`, `install.sh`.

**Spec:** `docs/superpowers/specs/2026-09-03-mtune-music-player-design.md` — read it alongside this plan. This plan implements **Phase 1 (fork + workspace + de-brand + build swap)** and **Phase 2 (directory library)** of that spec's 6-phase roadmap. Phases 3–6 (D-Bus + tray, shell pill/menu, reskin, Settings page) get their own plans once this one lands, because their exact module paths depend on the post-fork tree.

## Global Constraints

- **Crate / binary name:** `mtune`. **Display name:** `Tune`. **App-ID / `app_id` / gschema id:** `org.margo.Tune`. **gresource / gschema path:** `/org/margo/Tune/`. **D-Bus names:** `org.margo.Tune` and `org.mpris.MediaPlayer2.org.margo.Tune`. Copy these verbatim.
- **De-brand rule:** after the fork, `mtune/` must contain **zero** case-insensitive occurrences of `amberol`, `io.bassi`, `bassi`, `ebassi`, or the upstream `AmberolXxx` glib type-name prefix. A CI grep-gate (`scripts/mtune-debrand-gate.sh`) enforces this and runs inside `just check`. The upstream GPL-3.0 licence text and a name-free author-copyright attribution line are kept in `mtune/licenses/`.
- **No relm4 rewrite** of the app UI — keep the fork's raw gtk-rs subclassing + composite templates.
- **Dependency split:** non-GUI deps → `workspace = true` (exact match with margo: `serde`, `serde_json`, `toml`, `zbus`, `tracing`, `tracing-subscriber`, `anyhow`, `regex`, `sha2`, `clap` if used). GUI stack → mtune-local, current generation (`gtk4`/`gdk4` 0.11, `libadwaita` 0.9, `gstreamer*` 0.25, `gdk-pixbuf` 0.22, `lofty` 0.24, `mpris-server` 0.10, `color-thief`, `fuzzy-matcher`, plus `notify`, `ignore`). **`ashpd` is dropped.**
- **`Cargo.lock` must be regenerated and committed** in the same commit as any dependency change (AUR `makepkg --locked` breaks otherwise — see `reference_cargo_lock_locked_packaging`).
- **Config world:** `~/.config/margo/mtune.toml` is mtune's own file at the config-dir root (like `mpower.toml`). It is **not** `margo-config` and **not** `mshell-config` / `config_manager()`. Re-read on change.
- **Panic ratchet:** the repo-wide `.unwrap()/.expect()/panic!(…)` count (`scripts/panic-baseline.txt`, currently `315`) may only grow with a commit-message justification. Harden `audio/` where practical; raise the baseline for the rest with the rationale "mtune is a standalone application binary — a panic kills only the music player, not the compositor or the bar."
- **No live key injection** to test the GUI (see `feedback_no_live_key_injection`). GUI verification is: it builds, it opens, `cargo run` plays a folder; the user drives the keyboard.
- **Network:** the first `cargo build -p mtune` fetches gtk4 0.11 / gstreamer / libadwaita — run build/test steps on a networked machine.
- **`PKGBUILD` / AUR packaging is not touched by this plan.**

---

## File Structure

### New crate `mtune/` (copied from `~/.kod/amberol`, then transformed)

| Path | Responsibility |
|---|---|
| `mtune/Cargo.toml` | crate manifest — workspace member, dep split per Global Constraints |
| `mtune/build.rs` | compile `src/mtune.gresource.xml` via `glib-build-tools` |
| `mtune/src/main.rs` | entry: logging, gettext, `gst::init`, resource register, run `Application` |
| `mtune/src/config.rs` | plain consts (was `config.rs.in` meson template): `VERSION`, `GETTEXT_PACKAGE`, `LOCALEDIR`, `PKGDATADIR`, `APPLICATION_ID`, `PROFILE` |
| `mtune/src/application.rs` | `TuneApplication` — GActions, settings, keep-alive hold guard (ashpd removed) |
| `mtune/src/window.rs` | `TuneWindow` — main window, `open_files`, drag-drop, `adw::OverlaySplitView` |
| `mtune/src/audio/**` | the engine — copied verbatim, identifiers renamed only |
| `mtune/src/{playback_control,playlist_view,queue_row,volume_control,waveform_view,cover_picture,song_cover,song_details,marquee,search,sort,drag_overlay,i18n,utils,waveform_view}.rs` | UI widgets — copied verbatim, identifiers renamed only |
| `mtune/src/gtk/*.ui` | composite templates — compiled from `.blp`, committed |
| `mtune/src/gtk/style.css` | app stylesheet (reskin is Phase 5 — copied verbatim here) |
| `mtune/src/assets/icons/*.svg` | in-app action icons — copied verbatim (reskin is Phase 5) |
| `mtune/src/mtune.gresource.xml` | gresource manifest (was `amberol.gresource.xml`) |
| `mtune/src/library/mod.rs` | `pub mod config; pub mod scanner; pub mod watcher; pub mod index;` + `LibraryEvent` enum |
| `mtune/src/library/config.rs` | parse/write `~/.config/margo/mtune.toml`; `MtuneConfig` struct + defaults + `~` expansion + a `notify` watch on the file |
| `mtune/src/library/scanner.rs` | off-thread recursive folder scan streaming playable file paths over a channel |
| `mtune/src/library/watcher.rs` | debounced inotify watcher on the roots → `LibraryEvent::{Added,Removed}` |
| `mtune/src/library/index.rs` | on-disk tag cache `~/.cache/margo/mtune/index.json` with per-file mtime invalidation |
| `mtune/data/org.margo.Tune.desktop.in` | desktop entry (no meson `.in.in` double-template — a single `@bindir@`-free file, icons by app-id) |
| `mtune/data/org.margo.Tune.metainfo.xml` | AppStream metainfo — Tune, margo URLs, attribution |
| `mtune/data/org.margo.Tune.service` | D-Bus service file (`Exec=/usr/bin/mtune --gapplication-service`) |
| `mtune/data/org.margo.Tune.gschema.xml` | GSettings schema (window geometry, recolouring, replay-gain, background-play, resume state) |
| `mtune/data/icons/hicolor/scalable/apps/org.margo.Tune.svg` | colour app icon (placeholder until Phase 5) |
| `mtune/data/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg` | symbolic app icon = tray logo (placeholder until Phase 5) |
| `mtune/po/**` | translations — infra kept, project name scrubbed |
| `mtune/licenses/GPL-3.0-or-later.txt` + `mtune/licenses/ATTRIBUTION` | licence + name-free attribution |

### Modified repo files

| Path | Change |
|---|---|
| `Cargo.toml` | add `"mtune"` to `[workspace] members` |
| `Cargo.lock` | regenerated |
| `justfile` | new `mtune:` recipe; add `mtune` to `all:` |
| `install.sh` | binary loop + desktop/icons/gschema/metainfo install for mtune |
| `deny.toml` | (only if a new dep's licence isn't already in `[licenses] allow`) |
| `scripts/panic-baseline.txt` | raised, with commit-message rationale |
| `scripts/mtune-debrand-gate.sh` | **new** — grep-gate |
| `.github/workflows/*.yml` (or wherever `just check` steps live) | call the grep-gate (it is already inside `just check`; confirm CI runs `just check`) |

---

## Phase 1 — Fork into the workspace, de-brand, swap the build system

### Task 1: Vendor the source, strip upstream infra, register the crate

**Files:**
- Create: `mtune/` (copied tree), `mtune/Cargo.toml`, `mtune/licenses/GPL-3.0-or-later.txt`, `mtune/licenses/ATTRIBUTION`
- Modify: `Cargo.toml` (workspace members)
- Delete (inside `mtune/`): `meson.build`, `meson_options.txt`, `build-aux/`, `subprojects/`, `src/meson.build`, `src/gtk/meson.build`, `data/meson.build`, `data/icons/meson.build`, `po/meson.build`, `amberol.doap`, `code-of-conduct.md`, `CONTRIBUTING.md`, `.gitlab-ci.yml`, `io.bassi.Amberol.json`, `RELEASING.md`, `CHANGES.md`, `README.md`, `data/screenshots/`, `.editorconfig`, `.typos.toml`, `.reuse/`, `REUSE.toml`, `LICENSES/` (moved to `licenses/`), `src/config.rs.in` (replaced in Task 3)

**Interfaces:**
- Produces: a `mtune/` directory that `cargo metadata` recognises as a workspace member named `mtune` (it will **not** build yet).

- [ ] **Step 1: Copy the upstream tree**

```bash
cd /repo/archive/.kod/margo
rsync -a --exclude='.git' --exclude='target' ~/.kod/amberol/ mtune/
```

- [ ] **Step 2: Strip upstream infra files**

```bash
cd /repo/archive/.kod/margo/mtune
rm -rf build-aux subprojects data/screenshots .reuse
rm -f meson.build meson_options.txt src/meson.build src/gtk/meson.build \
      data/meson.build data/icons/meson.build po/meson.build \
      amberol.doap code-of-conduct.md CONTRIBUTING.md .gitlab-ci.yml \
      io.bassi.Amberol.json RELEASING.md CHANGES.md README.md \
      .editorconfig .typos.toml REUSE.toml
mkdir -p licenses
git -C ~/.kod/amberol show HEAD:LICENSES/GPL-3.0-or-later.txt > licenses/GPL-3.0-or-later.txt || cp LICENSES/GPL-3.0-or-later.txt licenses/
rm -rf LICENSES
```

- [ ] **Step 3: Write the attribution file**

Create `mtune/licenses/ATTRIBUTION`:

```
mtune is a fork of a GPL-3.0-or-later GTK music player.
Original work Copyright © 2022–2025 the upstream authors.
Fork modifications Copyright © 2026 Kenan Pelit.
Licensed under the GNU General Public License v3.0 or later;
see GPL-3.0-or-later.txt.
```

- [ ] **Step 4: Write `mtune/Cargo.toml`**

```toml
[package]
name = "mtune"
version.workspace = true
edition.workspace = true
license = "GPL-3.0-or-later"
repository.workspace = true
rust-version.workspace = true
description = "Folder-first music player for the margo desktop"

[[bin]]
name = "mtune"
path = "src/main.rs"

[dependencies]
# GUI stack — mtune-local, current gtk-rs generation (NOT workspace-pinned;
# mtune shares no glib-typed API with the rest of the workspace).
gtk = { version = "0.11", package = "gtk4", features = ["v4_16"] }
gdk-pixbuf = { version = "0.22", features = ["v2_42"] }
adw = { package = "libadwaita", version = "0.9", features = ["v1_5"] }
gst = { package = "gstreamer", version = "0.25" }
gst-audio = { package = "gstreamer-audio", version = "0.25" }
gst-play = { package = "gstreamer-play", version = "0.25" }
color-thief = "0.2.1"
lofty = "0.24"
mpris-server = "0.10"
fuzzy-matcher = "0.3.7"
rand = { version = "0.10", features = ["thread_rng"] }
itertools = "0.14"
async-channel = "2.2"
futures = "0.3"
gettext-rs = { version = "0.7", features = ["gettext-system"] }
once_cell = "1"
# Library subsystem (Phase 2)
notify = "8"
ignore = "0.4"
# Shared with margo — pinned to the workspace
serde = { workspace = true }
serde_json = { workspace = true }
toml = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
anyhow = { workspace = true }
regex = { workspace = true }
sha2 = { workspace = true }

[build-dependencies]
glib-build-tools = "0.20"

[lints]
workspace = true
```

> If a `workspace = true` dep above is not actually declared in the root `[workspace.dependencies]`, either add it there or pin it inline — check `Cargo.toml` first. `once_cell` / `itertools` / `async-channel` / `futures` may already be workspace deps; prefer `workspace = true` when they are.

- [ ] **Step 5: Register the crate**

In `/repo/archive/.kod/margo/Cargo.toml`, add `"mtune",` to `[workspace] members` (keep the list's ordering style — put it after `"mtm",` alphabetically, or wherever the m-crates sit).

- [ ] **Step 6: Verify the workspace sees it**

Run: `cargo metadata --no-deps --format-version 1 | python3 -c "import sys,json; print([p['name'] for p in json.load(sys.stdin)['packages']])"`
Expected: the list contains `mtune`. (A full `cargo build` will still fail — `config.rs.in`, `.blp`, no `build.rs` — that is Task 2 and Task 3.)

- [ ] **Step 7: Commit**

```bash
cd /repo/archive/.kod/margo
git add mtune Cargo.toml
git commit -m "feat(mtune): vendor the upstream player source as a workspace crate

Copied from an upstream GPL-3.0-or-later GTK music player; meson/flatpak/
GNOME-infra files stripped, licence + name-free attribution kept in
mtune/licenses/. Does not build yet (build-system swap + de-brand follow).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 2: De-brand pass — identifiers, bus names, app-id, data files, translations

**Files:**
- Modify: every file under `mtune/src/`, `mtune/data/`, `mtune/po/`
- Create: `mtune/src/config.rs`, `scripts/mtune-debrand-gate.sh`
- Rename: `mtune/src/amberol.gresource.xml` → `mtune/src/mtune.gresource.xml`; `mtune/data/io.bassi.Amberol.*` → `mtune/data/org.margo.Tune.*`; `mtune/data/icons/**/io.bassi.Amberol*.svg` → `org.margo.Tune*.svg`
- Delete: `mtune/src/config.rs.in`

**Interfaces:**
- Produces: `APPLICATION_ID = "org.margo.Tune"` and friends in `mtune/src/config.rs`; a de-branded tree with zero `amberol`/`io.bassi` strings; `scripts/mtune-debrand-gate.sh` exiting 0.

- [ ] **Step 1: Write the grep-gate first (it should FAIL now)**

Create `scripts/mtune-debrand-gate.sh`:

```bash
#!/usr/bin/env bash
# Fails if any upstream branding survives in mtune/.
set -euo pipefail
cd "$(dirname "$0")/.."
needle='amberol|io\.bassi|ebassi|AmberolApplication|AmberolWindow|AmberolVolumeControl|AmberolPlaybackControl|AmberolQueueRow|AmberolSongCover|AmberolSongDetails|AmberolPlaylistView|AmberolWaveformView|AmberolCoverPicture|AmberolMarquee|AmberolDragOverlay|AmberolRepeatMode|AmberolReplayGainMode'
if rg -n -i --hidden -g '!*.po' -g '!licenses/*' "$needle" mtune/ ; then
  echo "ERROR: upstream branding found in mtune/ (see matches above)" >&2
  exit 1
fi
# .po files: only the header project-id line matters
if rg -n -i 'Project-Id-Version:\s*amberol' mtune/po/ ; then
  echo "ERROR: upstream project-id in a .po header" >&2
  exit 1
fi
echo "mtune de-brand gate: clean"
```

```bash
chmod +x scripts/mtune-debrand-gate.sh
./scripts/mtune-debrand-gate.sh   # expected: FAIL with matches
```

- [ ] **Step 2: Replace `config.rs.in` with `config.rs`**

Delete `mtune/src/config.rs.in`. Create `mtune/src/config.rs`:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later

pub static VERSION: &str = env!("CARGO_PKG_VERSION");
pub static GETTEXT_PACKAGE: &str = "mtune";
pub static APPLICATION_ID: &str = "org.margo.Tune";
pub static PROFILE: &str = "";

/// Localedir: the system dir in a packaged build, the source `po/` tree in dev.
pub fn localedir() -> String {
    if cfg!(debug_assertions) {
        concat!(env!("CARGO_MANIFEST_DIR"), "/po").to_string()
    } else {
        "/usr/share/locale".to_string()
    }
}

/// Pkgdatadir: where the gresource lives in a packaged build.
pub fn pkgdatadir() -> String {
    "/usr/share/mtune".to_string()
}
```

> `main.rs` currently reads `LOCALEDIR` / `PKGDATADIR` as `&str` consts — Step 4 updates the call sites to `localedir()` / `pkgdatadir()`. The `env!("CARGO_MANIFEST_DIR")` use is guarded by `cfg!(debug_assertions)` so no build path is baked into a release binary (see `reference_env_manifest_dir_srcdir_leak`).

- [ ] **Step 3: Global identifier rename across `mtune/src/`**

Run these in order (GNU `sed` in-place):

```bash
cd /repo/archive/.kod/margo/mtune

# glib type-name prefixes and the crate/domain word
grep -rlZ 'Amberol' src/ | xargs -0 sed -i 's/Amberol/Tune/g'
grep -rlZ 'amberol' src/ | xargs -0 sed -i 's/amberol/mtune/g'

# app-id / dbus / gresource path
grep -rlZ 'io\.bassi\.Amberol' src/ | xargs -0 sed -i 's/io\.bassi\.Amberol/org.margo.Tune/g'
grep -rlZ 'io/bassi/Amberol'   src/ | xargs -0 sed -i 's#io/bassi/Amberol#org/margo/Tune#g'
grep -rlZ '/io/bassi/'         src/ | xargs -0 sed -i 's#/io/bassi/#/org/margo/#g'

# rename the gresource manifest
git mv src/amberol.gresource.xml src/mtune.gresource.xml 2>/dev/null || mv src/amberol.gresource.xml src/mtune.gresource.xml
```

Then **manually review** these files for leftovers the blanket rename can mangle or miss:
- `src/main.rs` — `glib::set_application_name("Tune")`, `glib::set_program_name(Some("mtune"))`, `PULSE_PROP_application.name` → `"Tune"`, `PULSE_PROP_media.role` stays `"music"`, `PULSE_PROP_application.icon_name` → app-id. The log-domain filter `builder.filter(Some("mtune"), …)` must match the crate name. Resource load: `PKGDATADIR.to_owned() + "/mtune.gresource"`.
- `src/audio/player.rs` — `#[enum_type(name = "TuneRepeatMode")]`, `#[enum_type(name = "TuneReplayGainMode")]` (the blanket sed already did this; confirm).
- `src/audio/mpris_controller.rs` — `Player::builder(APPLICATION_ID).identity("Tune").desktop_entry(APPLICATION_ID)`.
- `src/config.rs` — untouched by the sed (it's the new file); confirm `APPLICATION_ID = "org.margo.Tune"`.

- [ ] **Step 4: Fix the `main.rs` config call sites**

In `mtune/src/main.rs`, the `use config::{APPLICATION_ID, GETTEXT_PACKAGE, LOCALEDIR, PKGDATADIR, PROFILE};` line and its uses: replace `LOCALEDIR` with `config::localedir()` and `PKGDATADIR` with `config::pkgdatadir()` at each call site (`bindtextdomain`, `gio::Resource::load`). Keep `APPLICATION_ID`, `GETTEXT_PACKAGE`, `PROFILE` as-is.

- [ ] **Step 5: Rename + scrub the `data/` files**

```bash
cd /repo/archive/.kod/margo/mtune/data
for f in io.bassi.Amberol.*; do git mv "$f" "${f/io.bassi.Amberol/org.margo.Tune}" 2>/dev/null || mv "$f" "${f/io.bassi.Amberol/org.margo.Tune}"; done
cd icons
find . -name 'io.bassi.Amberol*' | while read -r p; do mv "$p" "${p/io.bassi.Amberol/org.margo.Tune}"; done
```

Then rewrite each `data/org.margo.Tune.*` file:

`data/org.margo.Tune.gschema.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<schemalist gettext-domain="mtune">
  <enum id="org.margo.Tune.ReplayGainMode">
    <value nick="album" value="0"/>
    <value nick="track" value="1"/>
    <value nick="off" value="2"/>
  </enum>
  <schema id="org.margo.Tune" path="/org/margo/Tune/">
    <key name="window-width" type="i"><default>600</default></key>
    <key name="window-height" type="i"><default>300</default></key>
    <key name="enable-recoloring" type="b"><default>true</default></key>
    <key name="replay-gain" enum="org.margo.Tune.ReplayGainMode"><default>'off'</default></key>
    <key name="background-play" type="b"><default>true</default></key>
    <key name="resume-uri" type="s"><default>''</default></key>
    <key name="resume-position" type="t"><default>0</default></key>
  </schema>
</schemalist>
```

`data/org.margo.Tune.desktop.in` (drop the meson `.in.in` double-template; keep one `.in` for gettext `Name`/`Comment` only, or make it plain — plain is simpler):

```ini
[Desktop Entry]
Name=Tune
GenericName=Music Player
Comment=Play everything in a folder
TryExec=mtune
Exec=mtune %U
Icon=org.margo.Tune
Terminal=false
Type=Application
Categories=GTK;Music;Audio;AudioVideo;
Keywords=music;player;media;audio;playlist;folder;
StartupNotify=true
X-SingleMainWindow=true
DBusActivatable=true
MimeType=audio/mpeg;audio/wav;audio/x-aac;audio/x-aiff;audio/x-ape;audio/x-flac;audio/x-m4a;audio/x-m4b;audio/x-mp1;audio/x-mp2;audio/x-mp3;audio/x-mpg;audio/x-mpeg;audio/x-mpegurl;audio/x-opus+ogg;audio/x-pn-aiff;audio/x-pn-au;audio/x-pn-wav;audio/x-speex;audio/x-vorbis;audio/x-vorbis+ogg;audio/x-wavpack;inode/directory;
```

`data/org.margo.Tune.service`:

```ini
[D-BUS Service]
Name=org.margo.Tune
Exec=/usr/bin/mtune --gapplication-service
```

`data/org.margo.Tune.metainfo.xml` — hand-write a minimal AppStream file: `<id>org.margo.Tune</id>`, `<name>Tune</name>`, `<summary>Folder-first music player for the margo desktop</summary>`, `<project_license>GPL-3.0-or-later</project_license>`, `<metadata_license>CC0-1.0</metadata_license>`, `<developer id="org.margo"><name>margo</name></developer>`, a `<description>` paragraph, `<url type="homepage">https://github.com/kenanpelit/margo</url>`, `<launchable type="desktop-id">org.margo.Tune.desktop</launchable>`. No screenshots yet.

- [ ] **Step 6: Fix the gresource manifest**

Edit `mtune/src/mtune.gresource.xml`: the `prefix` attributes are now `/org/margo/Tune/icons/scalable/actions/` and `/org/margo/Tune` (the sed did this — confirm). Leave the `.ui` `<file>` entries as-is (Task 3 produces those `.ui` files).

- [ ] **Step 7: Scrub `po/`**

```bash
cd /repo/archive/.kod/margo/mtune/po
# POTFILES.in: repoint at the renamed data files and the .ui (not .blp — Task 3 converts them)
cat > POTFILES.in <<'EOF'
data/org.margo.Tune.desktop.in
data/org.margo.Tune.gschema.xml
data/org.margo.Tune.metainfo.xml
src/gtk/playback-control.ui
src/gtk/playlist-view.ui
src/gtk/queue-row.ui
src/gtk/shortcuts-dialog.ui
src/gtk/window.ui
src/application.rs
src/audio/inhibit_controller.rs
src/audio/song.rs
src/cover_picture.rs
src/playback_control.rs
src/window.rs
EOF
# .po headers: replace the project-id
grep -rlZ 'Project-Id-Version: amberol' . | xargs -0 -r sed -i 's/Project-Id-Version: amberol/Project-Id-Version: mtune/'
```

- [ ] **Step 8: Run the grep-gate — it must now PASS**

Run: `./scripts/mtune-debrand-gate.sh`
Expected: `mtune de-brand gate: clean`
(If it fails, fix the reported files. Common leftovers: comments in `src/gtk/*.blp`, `i18n.rs` domain string, `utils.rs` `settings_manager` comment.)

- [ ] **Step 9: Wire the gate into `just check`**

In `justfile`, the `check:` recipe — add a line `./scripts/mtune-debrand-gate.sh` after the fmt/clippy steps (before or after `panic-ratchet.sh`, either is fine). Confirm CI invokes `just check` (grep `.github/workflows` for `just check` or the equivalent step list; if CI runs the steps individually, add the gate there too).

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat(mtune): de-brand — org.margo.Tune identity, no upstream strings

Global identifier rename (AmberolXxx -> TuneXxx, app-id, gresource/gschema
path, dbus names), data files renamed + rewritten, config.rs.in -> plain
config.rs, po/ project-id scrubbed. New scripts/mtune-debrand-gate.sh runs
in 'just check' and CI. Still does not build (Blueprint -> .ui is next).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 3: Swap the build system — Blueprint → `.ui`, `build.rs`, gresource compile

**Files:**
- Create: `mtune/build.rs`, `mtune/src/gtk/*.ui` (8 files, compiled)
- Delete: `mtune/src/gtk/*.blp` (8 files)
- Modify: `mtune/src/main.rs` (resource load path already fixed in Task 2 — confirm)

**Interfaces:**
- Consumes: `src/mtune.gresource.xml` from Task 2 (already repointed at `.ui`).
- Produces: `cargo build -p mtune` compiles; `cargo run -p mtune` opens the window.

- [ ] **Step 1: Compile every `.blp` to `.ui`**

```bash
cd /repo/archive/.kod/margo/mtune/src/gtk
for f in *.blp; do
  blueprint-compiler compile "$f" --output "${f%.blp}.ui"
done
ls *.ui   # expect: playback-control.ui playlist-view.ui queue-row.ui shortcuts-dialog.ui song-cover.ui song-details.ui volume-control.ui window.ui
```

- [ ] **Step 2: Verify the `.ui` templates carry the renamed class names**

Run: `grep -l 'class="Amberol' *.ui ; grep -h 'template class=' *.ui`
Expected: **no** `Amberol` matches (Task 2 renamed the `.blp` `$AmberolXxx` → `$TuneXxx` before this compile); each `<template class="TuneXxx" ...>` present.
If any `Amberol` slipped through, `sed -i 's/Amberol/Tune/g' *.ui` and re-run the debrand gate.

- [ ] **Step 3: Delete the `.blp` sources**

```bash
cd /repo/archive/.kod/margo/mtune/src/gtk
rm -f *.blp
```

> Decision (per spec's open question): the `.ui` files are now the maintained source. A future template edit is done in `.ui` directly. `blueprint-compiler` stays available if anyone wants to regenerate, but there is no build-time `.blp` step.

- [ ] **Step 4: Write `mtune/build.rs`**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
fn main() {
    glib_build_tools::compile_resources(
        &["src"],
        "src/mtune.gresource.xml",
        "mtune.gresource",
    );
    println!("cargo:rerun-if-changed=src/mtune.gresource.xml");
    println!("cargo:rerun-if-changed=src/gtk");
    println!("cargo:rerun-if-changed=src/assets");
}
```

- [ ] **Step 5: Point `main.rs` at the built gresource**

In `mtune/src/main.rs`, the dev/`MESON_DEVENV` branch logic loads `mtune.gresource` from `PKGDATADIR` or next to the exe. Replace the whole `resources` block with the standard cargo pattern:

```rust
let resources = gio::Resource::load(
    concat!(env!("OUT_DIR"), "/mtune.gresource")
)
.or_else(|_| {
    // packaged path
    gio::Resource::load(format!("{}/mtune.gresource", config::pkgdatadir()))
})
.expect("Unable to load mtune.gresource");
gio::resources_register(&resources);
```

> `OUT_DIR` is not available at runtime — use `include_bytes!` instead for the dev path:
> ```rust
> let resources = gio::Resource::from_data(
>     &glib::Bytes::from_static(include_bytes!(concat!(env!("OUT_DIR"), "/mtune.gresource")))
> ).expect("compiled-in gresource is valid");
> gio::resources_register(&resources);
> ```
> This bakes the gresource into the binary (like mkeys' `rust-embed` approach) — no packaged `pkgdatadir` file needed. Remove the `pkgdatadir()` fallback and the `MESON_DEVENV` env check entirely. Drop the now-unused `use std::env` items if the linter complains.

- [ ] **Step 6: First build**

Run: `cargo build -p mtune 2>&1 | tail -40`
Expected: compiles. Likely fixups along the way:
- gtk-rs 0.10→0.11 API: a handful of renamed methods / changed signatures. Fix each as the compiler points at it; they are mechanical.
- `gst_player` → `gst_play` (already in upstream's Cargo — confirm the `use` paths in `audio/gst_backend.rs` say `gst_play`).
- `pretty_env_logger` / `log` — upstream `main.rs` uses `pretty_env_logger`; either keep it as a dep (add `pretty_env_logger = "0.5"`, `log = "0.4"` to `Cargo.toml`) **or** switch to `tracing_subscriber::fmt().with_env_filter(...)`. Prefer keeping `log`+`pretty_env_logger` for a minimal diff in Phase 1; a `tracing` switch can be its own later cleanup.

- [ ] **Step 7: Run it**

Run: `cargo run -p mtune` (on a graphical session)
Expected: the window opens with the "add folder" empty state. Open a folder → tracks load and play. Close it.

- [ ] **Step 8: clippy**

Run: `cargo clippy -p mtune --all-targets -- -D warnings 2>&1 | tail -40`
Expected: clean. Fix warnings (mostly `needless_return`, `redundant_clone`, edition-2024 idioms from the 2018→2024 bump).

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(mtune): cargo build system — Blueprint compiled to committed .ui

build.rs + glib-build-tools compiles src/mtune.gresource.xml, baked into
the binary with include_bytes!. .blp deleted; .ui is now the maintained
template source. gtk-rs 0.11 API fixups. Builds and runs.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 4: Drop `ashpd`, keep-alive via `gio::Application::hold()`

**Files:**
- Modify: `mtune/src/application.rs`, `mtune/Cargo.toml` (no `ashpd` — already absent from the Task 1 manifest; confirm)

**Interfaces:**
- Consumes: `imp.background_hold: RefCell<Option<gio::ApplicationHoldGuard>>` (already a field in upstream `application.rs`).
- Produces: `Application::set_background_hold(active: bool)` — public method the player calls when playback starts/stops (and, in Phase 3, when the tray registers/unregisters).

- [ ] **Step 1: Remove the ashpd Background block**

In `mtune/src/application.rs`:
- Delete `use ashpd::{desktop::background::Background, WindowIdentifier};`.
- Delete the `request_background()` method (the `Background::request().identifier(...).reason(...)` future and its `.response()` handling) and any call to it (`setup_gactions` / `background-play` action handler / `activate`).
- Keep the `background_hold` field.

- [ ] **Step 2: Add the hold-guard toggle**

Add to `impl Application` (the `glib::wrapper!` type, not `imp`):

```rust
impl Application {
    /// Hold the GApplication alive with no window (background playback,
    /// and — Phase 3 — while the tray item is registered), or release it.
    pub fn set_background_hold(&self, active: bool) {
        let imp = self.imp();
        let want = active && imp.settings.boolean("background-play");
        let held = imp.background_hold.borrow().is_some();
        if want && !held {
            imp.background_hold.replace(Some(self.hold()));
        } else if !want && held {
            imp.background_hold.replace(None);
        }
    }
}
```

- [ ] **Step 3: Call it from the player state transitions**

In `mtune/src/audio/player.rs`, `set_playback_state` (or wherever `PlaybackState` is pushed to controllers/state) — after updating state, notify the application. The player already has `app_sender: Sender<ApplicationAction>`; add a variant:

```rust
// in application.rs
pub enum ApplicationAction {
    Present,
    BackgroundHold(bool),
}
```

and in the app's channel receiver loop, `ApplicationAction::BackgroundHold(b) => app.set_background_hold(b)`. In `player.rs` `set_playback_state`:

```rust
let _ = self.app_sender.send_blocking(ApplicationAction::BackgroundHold(
    !matches!(state, PlaybackState::Stopped)
));
```

- [ ] **Step 4: Build + manual check**

Run: `cargo build -p mtune && cargo run -p mtune`
Manual: open a folder, start playback, close the window → the process stays alive and audio continues (check `pgrep -a mtune` and that sound keeps playing). Stop playback with no window (via `playerctl --player=org.margo.Tune stop` or by reopening and pausing) → process exits. Toggle the `background-play` GSetting off → closing the window while playing exits immediately.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(mtune): drop ashpd; keep-alive via gio hold guard

The upstream Background portal call is removed (margo does not service it).
Windowless playback is now a gio::Application::hold() guard, held while
playback != stopped and background-play is enabled, released otherwise.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 5: `justfile`, `install.sh`, `Cargo.lock`, panic baseline, `deny.toml`, `just check`

**Files:**
- Modify: `justfile`, `install.sh`, `Cargo.lock`, `scripts/panic-baseline.txt`, `deny.toml` (conditionally)

**Interfaces:**
- Produces: `just mtune` builds + installs; `just check` passes with `mtune` in scope.

- [ ] **Step 1: `justfile` recipe**

Add after the `dots:` recipe:

```make
# Build + install mtune (the folder-first music player).
mtune:
    cargo build --release -p mtune
    sudo install -m755 target/release/mtune {{bindir}}/mtune
    sudo install -Dm644 mtune/data/org.margo.Tune.desktop.in /usr/share/applications/org.margo.Tune.desktop
    sudo install -Dm644 mtune/data/org.margo.Tune.gschema.xml /usr/share/glib-2.0/schemas/org.margo.Tune.gschema.xml
    sudo glib-compile-schemas /usr/share/glib-2.0/schemas
    sudo install -Dm644 mtune/data/icons/hicolor/scalable/apps/org.margo.Tune.svg /usr/share/icons/hicolor/scalable/apps/org.margo.Tune.svg
    sudo install -Dm644 mtune/data/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg /usr/share/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg
    @echo "mtune installed"
```

Change `all: margo shell cli dots` → `all: margo shell cli dots mtune`.

- [ ] **Step 2: `install.sh`**

In the binary loop (`for bin in margo start-margo mctl … mcal; do`), append `mtune`. After the man-pages block, add an mtune assets block mirroring the margo `.desktop` / icon install already there:

```bash
  # ── mtune (music player) assets ──
  install_file 644 "${REPO_ROOT}/mtune/data/org.margo.Tune.desktop.in" \
    "/usr/share/applications/org.margo.Tune.desktop"
  install_file 644 "${REPO_ROOT}/mtune/data/org.margo.Tune.metainfo.xml" \
    "/usr/share/metainfo/org.margo.Tune.metainfo.xml"
  install_file 644 "${REPO_ROOT}/mtune/data/org.margo.Tune.service" \
    "/usr/share/dbus-1/services/org.margo.Tune.service"
  install_file 644 "${REPO_ROOT}/mtune/data/org.margo.Tune.gschema.xml" \
    "/usr/share/glib-2.0/schemas/org.margo.Tune.gschema.xml"
  install_file 644 "${REPO_ROOT}/mtune/data/icons/hicolor/scalable/apps/org.margo.Tune.svg" \
    "/usr/share/icons/hicolor/scalable/apps/org.margo.Tune.svg"
  install_file 644 "${REPO_ROOT}/mtune/data/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg" \
    "/usr/share/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg"
  $SUDO glib-compile-schemas /usr/share/glib-2.0/schemas || true
```

Add the same paths to the uninstall list near the end (grep for `/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.margo.service` and add the mtune paths alongside).

- [ ] **Step 3: Regenerate `Cargo.lock`**

Run: `cargo build -p mtune` (this updates `Cargo.lock`) then `git diff --stat Cargo.lock` (expect a large addition — the whole gtk-rs 0.11 + gstreamer 0.25 tree).

- [ ] **Step 4: Measure and raise the panic baseline**

Run: `./scripts/panic-ratchet.sh`
Expected: FAIL — `panic-prone calls (non-test): <N>  (baseline: 315)` with `N` ≈ 420–460.

Before raising: in `mtune/src/audio/`, convert the highest-value `.unwrap()/.expect()` sites to graceful handling — target the `gst_backend.rs`, `player.rs`, `queue.rs`, `song.rs` paths where a panic aborts playback. For each: replace `x.unwrap()` with `match`/`if let` + `tracing::warn!` + early-return / skip-track. Aim to cut 25–40 sites. Re-run the ratchet.

Then set `scripts/panic-baseline.txt` to the new count.

- [ ] **Step 5: `deny.toml` licence check**

Run: `cargo deny check licenses 2>&1 | tail -30` (if `cargo-deny` is installed; else skip — CI will catch it).
Expected: pass. gstreamer/gtk/adw/lofty/color-thief/mpris-server/notify/ignore are MIT/Apache/LGPL-permissive. If one flags (e.g. an LGPL transitive), add its licence id to `deny.toml` `[licenses] allow` with a one-line comment.
The `multiple-versions = "warn"` for the dual gtk-rs generation is expected and does **not** fail — leave it.

- [ ] **Step 6: Full `just check`**

Run: `just check`
Expected: PASS — fmt, `clippy --all-targets -D warnings` (workspace-wide, mtune included), panic-ratchet (at the new baseline), design-lint, example-config parse, tests, and `mtune-debrand-gate.sh`.
Fix anything red.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "build(mtune): justfile recipe, install.sh assets, Cargo.lock, panic baseline

'just mtune' builds+installs the binary, desktop file, gschema, icons,
metainfo, dbus service. 'just all' includes mtune. Panic baseline raised
315 -> <N> after hardening audio/ (~<M> unwraps converted to graceful
handling); the remaining sites are in a standalone application binary
whose panic kills only the player, not the compositor or bar.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
git push
```

---

## Phase 2 — Directory-library subsystem

### Task 6: `library/config.rs` — the `mtune.toml` model

**Files:**
- Create: `mtune/src/library/mod.rs`, `mtune/src/library/config.rs`
- Modify: `mtune/src/main.rs` (`mod library;`)
- Test: inline `#[cfg(test)]` in `config.rs`

**Interfaces:**
- Produces:
  - `struct MtuneConfig { library: LibrarySection, playback: PlaybackSection, behaviour: BehaviourSection }` (all `#[derive(Debug, Clone, Deserialize, Serialize)]`, all sections `#[serde(default)]`)
  - `struct LibrarySection { roots: Vec<PathBuf>, scan_on_start: bool, watch: bool, recursive: bool, extensions: Vec<String> }`
  - `struct PlaybackSection { on_start: OnStart }` where `enum OnStart { Resume, Library, Nothing }` (`#[serde(rename_all = "lowercase")]`, default `Resume`)
  - `struct BehaviourSection { close_to_tray: bool, single_instance: bool }`
  - `fn MtuneConfig::path() -> PathBuf` → `~/.config/margo/mtune.toml` (respects `XDG_CONFIG_HOME`)
  - `fn MtuneConfig::load() -> MtuneConfig` (missing/broken file → `Default`, logs a warning on parse error)
  - `fn MtuneConfig::save(&self) -> anyhow::Result<()>` (creates parent dir, atomic write via temp + rename)
  - `fn LibrarySection::resolved_roots(&self) -> Vec<PathBuf>` (each root `~`- and `$HOME`-expanded, non-existent dropped with a warning)
  - `fn LibrarySection::is_playable(&self, path: &Path) -> bool` (extension in `extensions`, case-insensitive)
  - `const DEFAULT_EXTENSIONS: &[&str]` = `["mp3","flac","ogg","oga","opus","m4a","m4b","aac","wav","wma","aiff","ape","wv"]`

- [ ] **Step 1: Write the failing tests**

Create `mtune/src/library/config.rs` with only the test module and empty type stubs:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

// ... types added in Step 3 ...

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = MtuneConfig::default();
        assert!(c.library.scan_on_start);
        assert!(c.library.watch);
        assert!(c.library.recursive);
        assert_eq!(c.playback.on_start, OnStart::Resume);
        assert!(!c.library.extensions.is_empty());
    }

    #[test]
    fn partial_toml_fills_from_defaults() {
        let toml = r#"
            [library]
            roots = ["/music"]
            watch = false
        "#;
        let c: MtuneConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.library.roots, vec![PathBuf::from("/music")]);
        assert!(!c.library.watch);
        assert!(c.library.scan_on_start); // from default
        assert_eq!(c.playback.on_start, OnStart::Resume); // whole section defaulted
    }

    #[test]
    fn roundtrip_preserves_values() {
        let mut c = MtuneConfig::default();
        c.library.roots = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        c.playback.on_start = OnStart::Library;
        let s = toml::to_string(&c).unwrap();
        let back: MtuneConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.library.roots, c.library.roots);
        assert_eq!(back.playback.on_start, OnStart::Library);
    }

    #[test]
    fn tilde_expansion() {
        let home = std::env::var("HOME").unwrap();
        let lib = LibrarySection { roots: vec![PathBuf::from("~/Music")], ..Default::default() };
        // ~/Music won't exist in CI, so test the expansion helper directly:
        assert_eq!(expand_tilde(Path::new("~/Music")), PathBuf::from(format!("{home}/Music")));
        assert_eq!(expand_tilde(Path::new("/abs/path")), PathBuf::from("/abs/path"));
        let _ = lib;
    }

    #[test]
    fn is_playable_matches_extension_case_insensitively() {
        let lib = LibrarySection::default();
        assert!(lib.is_playable(Path::new("/x/Song.MP3")));
        assert!(lib.is_playable(Path::new("/x/song.flac")));
        assert!(!lib.is_playable(Path::new("/x/cover.jpg")));
        assert!(!lib.is_playable(Path::new("/x/noext")));
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.toml");
        assert_eq!(MtuneConfig::load_from(&p).library.watch, MtuneConfig::default().library.watch);
    }

    #[test]
    fn save_then_load_from_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mtune.toml");
        let mut c = MtuneConfig::default();
        c.library.roots = vec![PathBuf::from("/music/lib")];
        c.save_to(&p).unwrap();
        let back = MtuneConfig::load_from(&p);
        assert_eq!(back.library.roots, vec![PathBuf::from("/music/lib")]);
    }
}
```

Add `tempfile = "3"` to `mtune/Cargo.toml` `[dev-dependencies]`.

- [ ] **Step 2: Run the tests — verify they fail to compile**

Run: `cargo test -p mtune library::config 2>&1 | tail -20`
Expected: compile errors — `MtuneConfig` / `OnStart` / `LibrarySection` / `expand_tilde` not found.

- [ ] **Step 3: Implement the types + functions**

```rust
pub const DEFAULT_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "m4b", "aac", "wav", "wma", "aiff", "ape", "wv",
];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OnStart { Resume, Library, Nothing }
impl Default for OnStart { fn default() -> Self { OnStart::Resume } }

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LibrarySection {
    pub roots: Vec<PathBuf>,
    pub scan_on_start: bool,
    pub watch: bool,
    pub recursive: bool,
    pub extensions: Vec<String>,
}
impl Default for LibrarySection {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            scan_on_start: true,
            watch: true,
            recursive: true,
            extensions: DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        }
    }
}
impl LibrarySection {
    pub fn resolved_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|p| expand_tilde(p)).filter(|p| {
            let ok = p.is_dir();
            if !ok { tracing::warn!("library root does not exist: {}", p.display()); }
            ok
        }).collect()
    }
    pub fn is_playable(&self, path: &Path) -> bool {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => {
                let ext = ext.to_ascii_lowercase();
                self.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext))
            }
            None => false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PlaybackSection { pub on_start: OnStart }

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct BehaviourSection { pub close_to_tray: bool, pub single_instance: bool }
impl Default for BehaviourSection {
    fn default() -> Self { Self { close_to_tray: true, single_instance: true } }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MtuneConfig {
    pub library: LibrarySection,
    pub playback: PlaybackSection,
    pub behaviour: BehaviourSection,
}

pub fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    p.to_path_buf()
}

impl MtuneConfig {
    pub fn path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
            });
        base.join("margo").join("mtune.toml")
    }
    pub fn load() -> Self { Self::load_from(&Self::path()) }
    pub fn load_from(p: &Path) -> Self {
        match std::fs::read_to_string(p) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("mtune.toml parse error ({e}); using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
    pub fn save(&self) -> anyhow::Result<()> { self.save_to(&Self::path()) }
    pub fn save_to(&self, p: &Path) -> anyhow::Result<()> {
        if let Some(dir) = p.parent() { std::fs::create_dir_all(dir)?; }
        let body = toml::to_string_pretty(self)?;
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }
}
```

Create `mtune/src/library/mod.rs`:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
pub mod config;
pub mod index;
pub mod scanner;
pub mod watcher;

use std::path::PathBuf;

/// A live change to the library while mtune is running.
#[derive(Debug, Clone)]
pub enum LibraryEvent {
    Added(PathBuf),
    Removed(PathBuf),
}
```

> `index`/`scanner`/`watcher` don't exist yet — comment those `pub mod` lines out until their tasks, or create empty files with a `//!` doc line so the crate compiles. Prefer empty stub files.

Add `mod library;` to `mtune/src/main.rs` (after the other `mod` lines).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p mtune library::config 2>&1 | tail -20`
Expected: all 8 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add mtune/src/library mtune/src/main.rs mtune/Cargo.toml
git commit -m "feat(mtune): library config model (mtune.toml)

MtuneConfig (library/playback/behaviour sections, serde-defaulted),
~ expansion, atomic save, extension matching. 8 unit tests.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 7: `library/scanner.rs` — off-thread recursive scan

**Files:**
- Create/replace: `mtune/src/library/scanner.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `LibrarySection` (`resolved_roots`, `is_playable`, `recursive`) from Task 6.
- Produces:
  - `fn scan(roots: Vec<PathBuf>, lib: LibrarySection) -> async_channel::Receiver<ScanMsg>` — spawns a `std::thread`, walks each root (recursive per `lib.recursive`), sends `ScanMsg::Found(PathBuf)` per playable file (sorted within a directory: dir order then filename), then one `ScanMsg::Done { total: usize }`. The thread ends when the receiver is dropped.
  - `enum ScanMsg { Found(PathBuf), Done { total: usize } }`
  - `fn scan_blocking(roots: &[PathBuf], lib: &LibrarySection) -> Vec<PathBuf>` — the synchronous core, used by tests and reused by `scan`.

- [ ] **Step 1: Write the failing tests**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;
use crate::library::config::LibrarySection;

// ... impl in Step 3 ...

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("Album A")).unwrap();
        fs::create_dir_all(r.join("Album B/disc2")).unwrap();
        fs::write(r.join("Album A/01.mp3"), b"x").unwrap();
        fs::write(r.join("Album A/02.flac"), b"x").unwrap();
        fs::write(r.join("Album A/cover.jpg"), b"x").unwrap();
        fs::write(r.join("Album B/1.ogg"), b"x").unwrap();
        fs::write(r.join("Album B/disc2/2.ogg"), b"x").unwrap();
        fs::write(r.join("top.wav"), b"x").unwrap();
        d
    }

    #[test]
    fn recursive_finds_all_playable_skips_others() {
        let d = tree();
        let lib = LibrarySection::default(); // recursive = true
        let found = scan_blocking(&[d.path().to_path_buf()], &lib);
        assert_eq!(found.len(), 5); // 4 in albums + top.wav; cover.jpg excluded
        assert!(found.iter().any(|p| p.ends_with("Album B/disc2/2.ogg")));
    }

    #[test]
    fn non_recursive_stays_top_level() {
        let d = tree();
        let lib = LibrarySection { recursive: false, ..Default::default() };
        let found = scan_blocking(&[d.path().to_path_buf()], &lib);
        assert_eq!(found, vec![d.path().join("top.wav")]);
    }

    #[test]
    fn results_are_sorted_stably() {
        let d = tree();
        let lib = LibrarySection::default();
        let a = scan_blocking(&[d.path().to_path_buf()], &lib);
        let b = scan_blocking(&[d.path().to_path_buf()], &lib);
        assert_eq!(a, b);
        // within "Album A", 01.mp3 before 02.flac
        let ia = a.iter().position(|p| p.ends_with("Album A/01.mp3")).unwrap();
        let ib = a.iter().position(|p| p.ends_with("Album A/02.flac")).unwrap();
        assert!(ia < ib);
    }

    #[tokio::test]
    async fn async_scan_streams_then_done() {
        let d = tree();
        let rx = scan(vec![d.path().to_path_buf()], LibrarySection::default());
        let mut found = 0usize;
        loop {
            match rx.recv().await.unwrap() {
                ScanMsg::Found(_) => found += 1,
                ScanMsg::Done { total } => { assert_eq!(total, found); assert_eq!(total, 5); break; }
            }
        }
    }
}
```

> The `#[tokio::test]` needs `tokio` with `macros,rt` in `[dev-dependencies]`. If the crate has no tokio at all, use `async_channel`'s `recv_blocking` in a plain `#[test]` on a spawned thread instead — simpler, no tokio. Prefer that:
> ```rust
> #[test]
> fn async_scan_streams_then_done() {
>     let d = tree();
>     let rx = scan(vec![d.path().to_path_buf()], LibrarySection::default());
>     let mut found = 0;
>     loop {
>         match rx.recv_blocking().unwrap() {
>             ScanMsg::Found(_) => found += 1,
>             ScanMsg::Done { total } => { assert_eq!(total, found); assert_eq!(total, 5); break; }
>         }
>     }
> }
> ```

- [ ] **Step 2: Run — verify failure**

Run: `cargo test -p mtune library::scanner 2>&1 | tail -20`
Expected: `scan` / `scan_blocking` / `ScanMsg` not found.

- [ ] **Step 3: Implement**

```rust
use crate::library::config::LibrarySection;
use ignore::WalkBuilder;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum ScanMsg {
    Found(PathBuf),
    Done { total: usize },
}

pub fn scan_blocking(roots: &[PathBuf], lib: &LibrarySection) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        let mut builder = WalkBuilder::new(root);
        builder
            .standard_filters(false)          // no .gitignore semantics
            .hidden(true)                      // skip dotfiles
            .follow_links(false)
            .max_depth(if lib.recursive { None } else { Some(1) });
        let mut batch: Vec<PathBuf> = builder
            .build()
            .filter_map(|r| r.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.into_path())
            .filter(|p| lib.is_playable(p))
            .collect();
        batch.sort();
        out.extend(batch);
    }
    out
}

pub fn scan(roots: Vec<PathBuf>, lib: LibrarySection) -> async_channel::Receiver<ScanMsg> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("mtune-scan".into())
        .spawn(move || {
            let files = scan_blocking(&roots, &lib);
            let total = files.len();
            for f in files {
                if tx.send_blocking(ScanMsg::Found(f)).is_err() {
                    return; // receiver dropped
                }
            }
            let _ = tx.send_blocking(ScanMsg::Done { total });
        })
        .expect("spawn mtune-scan thread");
    rx
}
```

> `WalkBuilder::max_depth(Some(1))` includes the root itself at depth 0 and its direct children at depth 1 — correct for "non-recursive = top-level files only". Verify against the test; if `ignore` counts depth differently, use `Some(1)` vs `Some(0)` per the test outcome.
> The one `.expect()` on thread spawn: spawning fails only on OS resource exhaustion at which point the app is already doomed — acceptable, and it is the same shape as existing workspace code. It adds 1 to the panic count; fold into Task 5's baseline or bump by 1 here with a note.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p mtune library::scanner 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add mtune/src/library/scanner.rs mtune/Cargo.toml
git commit -m "feat(mtune): off-thread recursive library scanner

scan() spawns a worker thread walking the roots (ignore::WalkBuilder),
streams ScanMsg::Found per playable file then ScanMsg::Done{total}.
scan_blocking() is the sync core. 4 tests.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 8: `library/index.rs` — the on-disk tag cache

**Files:**
- Create/replace: `mtune/src/library/index.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `struct IndexEntry { path: PathBuf, mtime: u64, title: String, artist: String, album: String, duration_secs: u64 }` (`Serialize, Deserialize, Clone`)
  - `struct LibraryIndex { entries: Vec<IndexEntry> }` (`Default`)
  - `fn LibraryIndex::path() -> PathBuf` → `~/.cache/margo/mtune/index.json` (respects `XDG_CACHE_HOME`)
  - `fn LibraryIndex::load() -> LibraryIndex` / `load_from(&Path)` (missing/corrupt → empty)
  - `fn LibraryIndex::save(&self) -> anyhow::Result<()>` / `save_to(&Path)` (atomic)
  - `fn LibraryIndex::fresh_paths(&self) -> Vec<PathBuf>` — entries whose file still exists with an unchanged mtime
  - `fn LibraryIndex::reconcile(&mut self, found: &[PathBuf]) -> Reconcile` where `struct Reconcile { added: Vec<PathBuf>, removed: Vec<PathBuf>, stale: Vec<PathBuf> }` — `added` = in `found` not in the index (or mtime changed); `removed` = in the index, not in `found`; `stale` = in both but mtime changed
  - `fn mtime_of(path: &Path) -> Option<u64>` — seconds since epoch

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn save_load_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("index.json");
        let mut idx = LibraryIndex::default();
        idx.entries.push(IndexEntry {
            path: PathBuf::from("/m/a.mp3"), mtime: 111,
            title: "A".into(), artist: "B".into(), album: "C".into(), duration_secs: 200,
        });
        idx.save_to(&p).unwrap();
        let back = LibraryIndex::load_from(&p);
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].album, "C");
    }

    #[test]
    fn load_corrupt_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("index.json");
        fs::write(&p, b"{ not json").unwrap();
        assert!(LibraryIndex::load_from(&p).entries.is_empty());
    }

    #[test]
    fn reconcile_classifies_added_removed_stale() {
        let d = tempfile::tempdir().unwrap();
        let keep = d.path().join("keep.mp3");
        let stale = d.path().join("stale.mp3");
        fs::write(&keep, b"x").unwrap();
        fs::write(&stale, b"x").unwrap();
        let mut idx = LibraryIndex::default();
        idx.entries.push(IndexEntry { path: keep.clone(), mtime: mtime_of(&keep).unwrap(),
            title: String::new(), artist: String::new(), album: String::new(), duration_secs: 0 });
        idx.entries.push(IndexEntry { path: stale.clone(), mtime: 1, // wrong on purpose
            title: String::new(), artist: String::new(), album: String::new(), duration_secs: 0 });
        idx.entries.push(IndexEntry { path: d.path().join("gone.mp3"), mtime: 1,
            title: String::new(), artist: String::new(), album: String::new(), duration_secs: 0 });

        let new_file = d.path().join("new.mp3");
        fs::write(&new_file, b"x").unwrap();
        let found = vec![keep.clone(), stale.clone(), new_file.clone()];
        let r = idx.reconcile(&found);
        assert_eq!(r.added, vec![new_file]);
        assert_eq!(r.removed, vec![d.path().join("gone.mp3")]);
        assert_eq!(r.stale, vec![stale]);
    }

    #[test]
    fn fresh_paths_excludes_changed_and_missing() {
        let d = tempfile::tempdir().unwrap();
        let ok = d.path().join("ok.mp3");
        fs::write(&ok, b"x").unwrap();
        let mut idx = LibraryIndex::default();
        idx.entries.push(IndexEntry { path: ok.clone(), mtime: mtime_of(&ok).unwrap(),
            title: String::new(), artist: String::new(), album: String::new(), duration_secs: 0 });
        idx.entries.push(IndexEntry { path: d.path().join("missing.mp3"), mtime: 1,
            title: String::new(), artist: String::new(), album: String::new(), duration_secs: 0 });
        assert_eq!(idx.fresh_paths(), vec![ok]);
    }
}
```

- [ ] **Step 2: Run — verify failure**

Run: `cargo test -p mtune library::index 2>&1 | tail -20`

- [ ] **Step 3: Implement**

```rust
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub mtime: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LibraryIndex {
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reconcile {
    pub added: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub stale: Vec<PathBuf>,
}

pub fn mtime_of(path: &Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

impl LibraryIndex {
    pub fn path() -> PathBuf {
        let base = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache")
            });
        base.join("margo").join("mtune").join("index.json")
    }
    pub fn load() -> Self { Self::load_from(&Self::path()) }
    pub fn load_from(p: &Path) -> Self {
        std::fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> anyhow::Result<()> { self.save_to(&Self::path()) }
    pub fn save_to(&self, p: &Path) -> anyhow::Result<()> {
        if let Some(dir) = p.parent() { std::fs::create_dir_all(dir)?; }
        let body = serde_json::to_vec_pretty(self)?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }
    pub fn fresh_paths(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|e| mtime_of(&e.path).map(|m| m == e.mtime).unwrap_or(false))
            .map(|e| e.path.clone())
            .collect()
    }
    pub fn reconcile(&self, found: &[PathBuf]) -> Reconcile {
        use std::collections::HashMap;
        let indexed: HashMap<&PathBuf, u64> =
            self.entries.iter().map(|e| (&e.path, e.mtime)).collect();
        let found_set: std::collections::HashSet<&PathBuf> = found.iter().collect();
        let mut r = Reconcile::default();
        for f in found {
            match indexed.get(f) {
                None => r.added.push(f.clone()),
                Some(&m) if mtime_of(f) != Some(m) => r.stale.push(f.clone()),
                Some(_) => {}
            }
        }
        for e in &self.entries {
            if !found_set.contains(&e.path) {
                r.removed.push(e.path.clone());
            }
        }
        r
    }
}
```

> Test calls `idx.reconcile(&found)` on `&mut idx` in one place — signature is `&self`; adjust the test binding to `let idx = ...` (drop `mut`) or keep `mut` (still compiles). Match the interface: `reconcile(&self, ...)`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p mtune library::index 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add mtune/src/library/index.rs
git commit -m "feat(mtune): on-disk tag index with mtime invalidation

LibraryIndex <-> ~/.cache/margo/mtune/index.json (atomic write, corrupt
-> empty). reconcile() classifies found paths into added/removed/stale
by mtime. 4 tests.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 9: `library/watcher.rs` — debounced inotify

**Files:**
- Create/replace: `mtune/src/library/watcher.rs`
- Test: inline `#[cfg(test)]` (a real-filesystem integration test with a timeout)

**Interfaces:**
- Consumes: `LibrarySection` (`is_playable`, `resolved_roots`), `LibraryEvent` from `library/mod.rs`.
- Produces:
  - `struct LibraryWatcher { _inner: notify::RecommendedWatcher }` — RAII; dropping it stops watching.
  - `fn LibraryWatcher::start(lib: LibrarySection, sink: async_channel::Sender<LibraryEvent>) -> anyhow::Result<LibraryWatcher>` — watches each resolved root recursively (per `lib.recursive`); coalesces raw events over a 500 ms debounce; emits `LibraryEvent::Added(p)` for a created/renamed-in playable file, `LibraryEvent::Removed(p)` for a deleted/renamed-out one.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{config::LibrarySection, LibraryEvent};
    use std::{fs, time::Duration};

    #[test]
    fn detects_added_and_removed_playable_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = LibrarySection { roots: vec![dir.path().to_path_buf()], ..Default::default() };
        let (tx, rx) = async_channel::unbounded();
        let _w = LibraryWatcher::start(lib, tx).unwrap();

        let song = dir.path().join("new.mp3");
        fs::write(&song, b"x").unwrap();

        // wait up to 3s for the Added event (debounce is 500ms)
        let added = recv_timeout(&rx, Duration::from_secs(3));
        assert!(matches!(added, Some(LibraryEvent::Added(p)) if p == song));

        fs::remove_file(&song).unwrap();
        let removed = recv_timeout(&rx, Duration::from_secs(3));
        assert!(matches!(removed, Some(LibraryEvent::Removed(p)) if p == song));
    }

    fn recv_timeout(rx: &async_channel::Receiver<LibraryEvent>, d: Duration) -> Option<LibraryEvent> {
        let deadline = std::time::Instant::now() + d;
        loop {
            if let Ok(ev) = rx.try_recv() { return Some(ev); }
            if std::time::Instant::now() > deadline { return None; }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn ignores_non_playable_files() {
        let dir = tempfile::tempdir().unwrap();
        let lib = LibrarySection { roots: vec![dir.path().to_path_buf()], ..Default::default() };
        let (tx, rx) = async_channel::unbounded();
        let _w = LibraryWatcher::start(lib, tx).unwrap();
        fs::write(dir.path().join("cover.jpg"), b"x").unwrap();
        assert!(recv_timeout(&rx, Duration::from_secs(2)).is_none());
    }
}
```

> These tests touch the real inotify backend and use wall-clock waits. If CI runs without inotify (unlikely on Linux runners) gate them with `#[cfg_attr(not(target_os = "linux"), ignore)]`.

- [ ] **Step 2: Run — verify failure**

Run: `cargo test -p mtune library::watcher 2>&1 | tail -20`

- [ ] **Step 3: Implement**

```rust
use crate::library::{config::LibrarySection, LibraryEvent};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

pub struct LibraryWatcher {
    _inner: notify::RecommendedWatcher,
    _pump: std::thread::JoinHandle<()>,
}

impl LibraryWatcher {
    pub fn start(
        lib: LibrarySection,
        sink: async_channel::Sender<LibraryEvent>,
    ) -> anyhow::Result<Self> {
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        })?;
        let mode = if lib.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        for root in lib.resolved_roots() {
            watcher.watch(&root, mode)?;
        }

        let lib2 = lib.clone();
        let pump = std::thread::Builder::new()
            .name("mtune-watch".into())
            .spawn(move || debounce_loop(raw_rx, lib2, sink))?;

        Ok(Self { _inner: watcher, _pump: pump })
    }
}

fn debounce_loop(
    raw_rx: mpsc::Receiver<notify::Result<Event>>,
    lib: LibrarySection,
    sink: async_channel::Sender<LibraryEvent>,
) {
    use std::collections::HashMap;
    let debounce = Duration::from_millis(500);
    let mut pending: HashMap<PathBuf, bool> = HashMap::new(); // path -> exists_now

    loop {
        // block for the first event, then drain for `debounce`
        let first = match raw_rx.recv() {
            Ok(ev) => ev,
            Err(_) => return, // watcher dropped
        };
        absorb(first, &lib, &mut pending);
        let deadline = std::time::Instant::now() + debounce;
        while let Ok(Some(ev)) = recv_until(&raw_rx, deadline) {
            absorb(ev, &lib, &mut pending);
        }
        for (path, exists) in pending.drain() {
            let ev = if exists {
                LibraryEvent::Added(path)
            } else {
                LibraryEvent::Removed(path)
            };
            if sink.send_blocking(ev).is_err() {
                return;
            }
        }
    }
}

fn recv_until(
    rx: &mpsc::Receiver<notify::Result<Event>>,
    deadline: std::time::Instant,
) -> Result<Option<notify::Result<Event>>, ()> {
    let now = std::time::Instant::now();
    if now >= deadline {
        return Ok(None);
    }
    match rx.recv_timeout(deadline - now) {
        Ok(ev) => Ok(Some(Ok(ev.unwrap_or_else(|e| {
            tracing::debug!("watch error: {e}");
            Event::new(EventKind::Other)
        })))),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
    }
}

fn absorb(
    ev: notify::Result<Event>,
    lib: &LibrarySection,
    pending: &mut std::collections::HashMap<PathBuf, bool>,
) {
    let Ok(ev) = ev else { return };
    let interesting = matches!(
        ev.kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
    );
    if !interesting {
        return;
    }
    for path in ev.paths {
        if !lib.is_playable(&path) {
            continue;
        }
        pending.insert(path.clone(), path.exists());
    }
}
```

> `recv_until` returning a synthetic `Event::new(EventKind::Other)` on a decode error keeps the type simple; `absorb` ignores `Other`. The two `.unwrap_or_else` there don't panic. If `Event::new` isn't a `notify` 8 API, use `ev.map_err(...)` handling instead — match to the crate version.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p mtune library::watcher 2>&1 | tail -20`
Expected: PASS (allow a few seconds; the debounce + polling adds latency).

- [ ] **Step 5: Commit**

```bash
git add mtune/src/library/watcher.rs
git commit -m "feat(mtune): debounced inotify library watcher

LibraryWatcher::start watches the roots (notify crate), coalesces raw
events over 500ms, emits LibraryEvent::Added/Removed for playable files
only. RAII: drop stops watching. 2 integration tests.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
```

---

### Task 10: Wire the library into `Application` startup

**Files:**
- Modify: `mtune/src/application.rs`, `mtune/src/window.rs`, `mtune/src/audio/player.rs` (small — a `resume` helper), `mtune/src/library/mod.rs` (uncomment the `pub mod` lines), `mtune/src/main.rs`
- Test: a manual on-device checklist (this task is integration glue; the units are covered by Tasks 6–9)

**Interfaces:**
- Consumes: `MtuneConfig`, `scanner::scan` + `ScanMsg`, `LibraryIndex`, `LibraryWatcher` + `LibraryEvent`, `Queue::add_song`, `Song::from_uri`, `AudioPlayer`.
- Produces: on launch, mtune loads the library and (per `playback.on_start`) resumes / starts / idles; while running it reflects filesystem changes.

- [ ] **Step 1: Load config + kick the initial fill on `activate`/`startup`**

In `mtune/src/application.rs`, add a field `config: RefCell<MtuneConfig>` to `imp::Application` (loaded in `new()` via `MtuneConfig::load()`), and in `ObjectImpl::constructed` (or `startup`) call a new `self.load_library()`:

```rust
fn load_library(&self) {
    let cfg = self.imp().config.borrow().clone();
    let roots = cfg.library.resolved_roots();
    if roots.is_empty() {
        tracing::info!("mtune: no library roots configured; waiting for a folder");
        return;
    }
    let player = self.imp().player.clone();

    if cfg.library.scan_on_start {
        // fresh scan, stream into the queue
        let rx = crate::library::scanner::scan(roots.clone(), cfg.library.clone());
        let player = player.clone();
        glib::spawn_future_local(async move {
            let mut idx = crate::library::index::LibraryIndex::default();
            while let Ok(msg) = rx.recv().await {
                match msg {
                    crate::library::scanner::ScanMsg::Found(path) => {
                        if let Some(uri) = path_to_uri(&path) {
                            if let Ok(song) = crate::audio::Song::from_uri(&uri) {
                                idx.entries.push(index_entry_from(&path, &song));
                                player.queue().add_song(&song);
                            }
                        }
                    }
                    crate::library::scanner::ScanMsg::Done { total } => {
                        tracing::info!("mtune: library scan done, {total} tracks");
                        let _ = idx.save();
                        break;
                    }
                }
            }
        });
    } else {
        // trust the index; fill immediately, reconcile in the background
        let idx = crate::library::index::LibraryIndex::load();
        for p in idx.fresh_paths() {
            if let Some(uri) = path_to_uri(&p) {
                if let Ok(song) = crate::audio::Song::from_uri(&uri) {
                    player.queue().add_song(&song);
                }
            }
        }
        // background reconcile
        let roots2 = roots.clone();
        let lib2 = cfg.library.clone();
        let player2 = player.clone();
        glib::spawn_future_local(async move {
            let found = tokio_like_spawn_blocking(move || {
                crate::library::scanner::scan_blocking(&roots2, &lib2)
            }).await;
            let mut idx = crate::library::index::LibraryIndex::load();
            let r = idx.reconcile(&found);
            for p in r.added.iter().chain(r.stale.iter()) {
                if let Some(uri) = path_to_uri(p) {
                    if let Ok(song) = crate::audio::Song::from_uri(&uri) {
                        player2.queue().add_song(&song);
                    }
                }
            }
            // (removed handled by the watcher path / a queue rebuild — see Step 3)
        });
    }

    // start the watcher
    if cfg.library.watch {
        self.start_watch(cfg.library.clone());
    }
}
```

Helpers in `application.rs` (or `library/mod.rs`):

```rust
fn path_to_uri(p: &std::path::Path) -> Option<String> {
    Some(gtk::gio::File::for_path(p).uri().to_string())
}
fn index_entry_from(path: &std::path::Path, song: &crate::audio::Song) -> crate::library::index::IndexEntry {
    crate::library::index::IndexEntry {
        path: path.to_path_buf(),
        mtime: crate::library::index::mtime_of(path).unwrap_or(0),
        title: song.title(),
        artist: song.artist(),
        album: song.album(),
        duration_secs: song.duration(),
    }
}
```

> `tokio_like_spawn_blocking` — the crate has no tokio. Use `gtk::gio::spawn_blocking(closure).await` (returns the value) instead, or run `scan_blocking` on a `std::thread` and `await` an `async_channel` oneshot. Prefer `gio::spawn_blocking`.
> `Song::from_uri`, `Song::title/artist/album/duration` — confirm these accessor names against `mtune/src/audio/song.rs`; adjust if the getters are `title()` vs `get_title()`.

- [ ] **Step 2: `playback.on_start`**

After the initial fill completes (in the `ScanMsg::Done` arm and after the index fast-path), apply the mode:

```rust
match cfg.playback.on_start {
    OnStart::Nothing => {}
    OnStart::Library => { player.queue().select_song_at(0); /* do not auto-play */ }
    OnStart::Resume => {
        let s = self.imp().settings.borrow(); // gio::Settings
        let uri = s.string("resume-uri");
        let pos = s.uint64("resume-position");
        if !uri.is_empty() {
            if let Some(ix) = player.queue().position_of_uri(&uri) {
                player.skip_to(ix);
                player.seek_position_abs(pos);
            }
        }
    }
}
```

Add `Queue::position_of_uri(&self, uri: &str) -> Option<u32>` to `mtune/src/audio/queue.rs` (linear scan over `song_at`, compare `song.uri()`).
Persist resume state: in `window.rs` `close_request` / the app `shutdown`, write `settings.set_string("resume-uri", &current_uri)` and `settings.set_uint64("resume-position", player.state().position())`.

- [ ] **Step 3: Watcher → queue**

```rust
fn start_watch(&self, lib: LibrarySection) {
    let (tx, rx) = async_channel::unbounded();
    match crate::library::watcher::LibraryWatcher::start(lib, tx) {
        Ok(w) => { self.imp().watcher.replace(Some(w)); }
        Err(e) => { tracing::warn!("mtune: library watch failed: {e}"); return; }
    }
    let player = self.imp().player.clone();
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                LibraryEvent::Added(p) => {
                    if let Some(uri) = path_to_uri(&p) {
                        if let Ok(song) = crate::audio::Song::from_uri(&uri) {
                            player.queue().add_song(&song);
                        }
                    }
                }
                LibraryEvent::Removed(p) => {
                    if let Some(uri) = path_to_uri(&p) {
                        if let Some(ix) = player.queue().position_of_uri(&uri) {
                            if let Some(song) = player.queue().song_at(ix) {
                                player.remove_song(&song);
                            }
                        }
                    }
                }
            }
        }
    });
}
```

Add `watcher: RefCell<Option<LibraryWatcher>>` to `imp::Application`.

- [ ] **Step 4: Uncomment `library/mod.rs` sub-modules and build**

Run: `cargo build -p mtune 2>&1 | tail -40`
Fix: accessor-name mismatches, `glib::spawn_future_local` needing `Send`-free closures (it runs on the main context — fine), any `Rc` vs `clone` issues. `player.queue()` returns `&Queue` — clone via `player.queue().clone()` if you need an owned handle in the async block, or capture `player` (an `Rc<AudioPlayer>`) and call `player.queue()` inside.

- [ ] **Step 5: `just check`**

Run: `just check`
Expected: PASS. The panic baseline may tick up by a couple from the new `.expect()`-free glue — if the ratchet complains, convert or bump with a note.

- [ ] **Step 6: On-device verification (user runs it)**

Manual checklist:
1. `mkdir -p /tmp/mtune-lib && cp <a few .mp3/.flac> /tmp/mtune-lib/` (include a subfolder).
2. `printf '[library]\nroots = ["/tmp/mtune-lib"]\n' > ~/.config/margo/mtune.toml`
3. `cargo run -p mtune` → window opens, "library scan done, N tracks" in the log, queue is populated, playback resumes/starts per `on_start`.
4. `cp another.flac /tmp/mtune-lib/sub/` → within ~1s it appears in the queue.
5. `rm /tmp/mtune-lib/sub/another.flac` → it disappears from the queue.
6. Close the window mid-playback → audio continues (`background-play` on), `pgrep mtune` still there.
7. Relaunch → `on_start = resume` returns to the same track + position.

- [ ] **Step 7: Commit + push**

```bash
git add -A
git commit -m "feat(mtune): auto-load the folder library on launch + live watch

Application startup loads mtune.toml, scans (or trusts the index and
reconciles), streams Songs into the Queue, applies playback.on_start
(resume/library/nothing), and starts the inotify watcher which adds/
removes queue entries as the folders change. Resume state persisted to
gschema on close.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01QtWkq4eTWDHYM32zyD8vdZ"
git push
```

---

## Self-Review

**1. Spec coverage (Phases 1–2):**

| Spec item | Task |
|---|---|
| §1 Identity, app-id, gschema/gresource path, D-Bus names | Task 2 |
| §1 De-brand audit + CI grep-gate | Task 2 (steps 1, 8, 9) |
| §2 Workspace member, `members`, `Cargo.toml` | Task 1 |
| §2 Dependency split (non-GUI `workspace=true`, GUI local/current) | Task 1 (step 4) |
| §2 `ashpd` dropped, hold-guard keep-alive | Task 4 |
| §2 meson → cargo, `build.rs` + `glib-build-tools` | Task 3 |
| §2 Blueprint `.blp` → committed `.ui` | Task 3 (steps 1–3) |
| §2 `config.rs.in` → plain `config.rs`, `cfg(debug_assertions)` guard | Task 2 (step 2) |
| §2 gschema install + `glib-compile-schemas` | Task 5 |
| §2 `po/` infra kept, name scrubbed | Task 2 (step 7) |
| §2 Panic ratchet: harden `audio/`, raise baseline w/ rationale | Task 5 (step 4) |
| §2 `Cargo.lock` sync, `deny.toml` licence check, `multiple-versions=warn` | Task 5 (steps 3, 5) |
| §2 `justfile` recipe + `all:` + `install.sh` | Task 5 (steps 1, 2) |
| §3 Engine + widgets copied verbatim, renamed only | Task 1–2 |
| §4 `mtune.toml` at config-dir root, own world, re-read | Task 6 |
| §4 `src/library/{config,scanner,watcher,index}` | Tasks 6–9 |
| §4 Launch flow: scan-on-start vs index, `on_start`, watcher | Task 10 |
| §4 Ad-hoc "add folder" kept | inherited from upstream (untouched) — verified in Task 3 step 7 |
| §4 Load order folder→album→track | Task 7 (sort) + upstream `sort.rs` |
| Reskin (§3 style.css, icons, font), matugen (§8) | **deferred to Phase 5 — own plan** |
| MPRIS `TrackList`, `org.margo.Tune` iface, tray (§5, §6) | **deferred to Phase 3 — own plan** |
| Shell pill + menu + Settings (§7) | **deferred to Phase 4 — own plan** |

Deferred items are explicitly out of this plan's scope (stated in the header). No in-scope spec item is unaddressed.

**Deviation from spec:** `[playback] replaygain` and `gapless` stay in gschema (already wired by the upstream fork) rather than moving into `mtune.toml` this phase — `mtune.toml`'s `[playback]` carries only `on_start` for now. Harmless (both are "where a knob lives"); the Phase 6 Settings-page plan can consolidate if wanted.

**2. Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N". Each code step has real code. The `<N>`/`<M>` in Task 5's commit message are runtime-measured values (the count), not unfilled design — acceptable and labelled.

**3. Type consistency:**
- `MtuneConfig`, `LibrarySection`, `PlaybackSection`, `BehaviourSection`, `OnStart`, `expand_tilde` — defined Task 6, used Tasks 7/9/10. ✔
- `LibrarySection::is_playable` / `resolved_roots` / `recursive` — Task 6, used Tasks 7/9. ✔
- `scan(roots: Vec<PathBuf>, lib: LibrarySection) -> Receiver<ScanMsg>`, `scan_blocking(&[PathBuf], &LibrarySection) -> Vec<PathBuf>`, `ScanMsg::{Found,Done{total}}` — Task 7, used Task 10. ✔
- `LibraryIndex`, `IndexEntry`, `Reconcile`, `mtime_of`, `fresh_paths`, `reconcile(&self, &[PathBuf])` — Task 8, used Task 10. ✔
- `LibraryEvent::{Added,Removed}` — `library/mod.rs` Task 6, used Tasks 9/10. ✔
- `LibraryWatcher::start(LibrarySection, Sender<LibraryEvent>) -> Result<LibraryWatcher>` — Task 9, used Task 10. ✔
- `Queue::add_song(&Song) -> bool`, `Queue::song_at(u32) -> Option<Song>`, `Queue::clear()` — from upstream (verified). `Queue::position_of_uri` is **added in Task 10 step 2** (noted). ✔
- `Application::set_background_hold(bool)` + `ApplicationAction::BackgroundHold(bool)` — Task 4, self-contained. ✔
- `Song::from_uri`, `Song::{title,artist,album,duration}` — from upstream; Task 10 step 1 flags "confirm accessor names against `audio/song.rs`". ✔

No inconsistencies to fix.

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-09-03-mtune-app-foundation.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — a fresh subagent per task, two-stage review between tasks, fast iteration.

**2. Inline Execution** — tasks run in this session via executing-plans, batched with review checkpoints.

**Which approach?**

> Note: Task 3 onward needs a networked machine (first `cargo build -p mtune` fetches the gtk4 0.11 / gstreamer 0.25 / libadwaita 0.9 trees), and the GUI verification steps need a graphical session. Tasks 1–2 and the Phase 2 unit tests (Tasks 6–9) do not.
