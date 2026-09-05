# mtune: "Repeat Each N" mode + per-playlist resume position — design spec

**Status:** approved 2026-09-05 (in chat, after two rounds of structured
questions during brainstorming).

## 1. Problem

Two related playback-memory requests:

1. mtune's repeat modes are `off` (consecutive) / `all` / `one`. The user
   wants a fourth: repeat each track a configurable number of times (N,
   default 3), then advance to the next track and repeat *that* one N
   times, continuing (wrapping) through the whole queue.
2. Loading a saved playlist always starts at track 0. The user wants it
   to resume at whichever track was playing the last time that playlist
   was the active queue — continuously updated while listening, not only
   at an explicit "Save Playlist".

## 2. Existing mechanisms this builds on (found during exploration)

- mtune already has an **app-wide** "resume where you left off" feature:
  GSettings keys `resume-uri` / `resume-position`, written by
  `Application::persist_resume()` (called on a timer, on MPRIS/tray
  `Quit`, on shutdown, on window close — `mtune/src/application.rs`),
  read back via `Window`'s `StartIntent::Resume(uri, pos)` when
  `[playback] on_start = "resume"` and the queue was empty
  (`mtune/src/window.rs` ~L1104). This is a **single global slot** — one
  remembered position for the whole app, not per-playlist. Feature 2
  is additive to this, not a replacement.
- `mtune/src/utils.rs`'s `store_playlist`/`load_cached_songs`
  (`current.pls` cache) is a *different* mechanism (snapshotting the
  live queue's song list for the resume-on-launch flow) — unrelated to
  named saved playlists, not touched by this work.
- `RepeatMode` is a `glib::Enum` (`mtune/src/audio/player.rs`) — no
  associated data per variant (GObject enums can't carry a payload), so
  the configurable repeat count must be a **separate** `Queue` property,
  not baked into the enum.
- `next_index()` (`mtune/src/audio/queue.rs`) is a pure function unit
  tested directly; Feature 1 keeps it pure by threading the count state
  through as parameters/return values instead of reading `Cell`s inside it.

## 3. Feature 1: `RepeatMode::RepeatEach`

### Data model
- `RepeatMode` gains a fourth variant `RepeatEach` (after `RepeatOne`).
  `Display` impl: `"repeat-each"`.
- `Queue` gains two `Cell`s (mirroring the existing `repeat_mode` field):
  - `repeat_count: Cell<u32>` — the configurable N, default `3`. A real
    GObject property (`ParamSpecUInt`, like `n-songs`) so it round-trips
    the same way `repeat_mode` does.
  - `repeat_plays: Cell<u32>` — how many times the *current* track has
    played consecutively under `RepeatEach`. Internal only, no GObject
    property (it's derived playback state, not something a caller reads
    or the UI displays).

### Behavior
- **Automatic advance** (track ends on its own, `manual = false`):
  under `RepeatEach`, if `repeat_plays + 1 < repeat_count`, replay the
  same index and increment `repeat_plays`; otherwise reset
  `repeat_plays` to `0` and advance to `current + 1` (wrapping to `0` at
  the end of the queue — same "keeps going" semantics as `RepeatAll`,
  per the user's "sonrakine geçecek ... devam edecek").
- **Manual skip** (`manual = true`, Next button / MPRIS `Next` / shell
  IPC): behaves like `RepeatAll` — advance immediately, `repeat_plays`
  resets to `0` for the new track. Same convention `RepeatOne` already
  uses ("an explicit skip always moves to a different track").
- `repeat_plays` resets to `0` at every place `current_pos` is set to a
  value *other* than the natural in-place repeat inside `next_song()`
  (jump/`PlayIndex`, previous, `set_current_song`, clear, start-at-0) —
  all of these already live inside `mtune/src/audio/queue.rs`, so the
  reset is local to that file.
- MPRIS has no fourth `LoopStatus` value (`None`/`Track`/`Playlist`
  only) — `RepeatEach` reports as `LoopStatus::Playlist` outward (an
  external MPRIS client sees "some form of loop is on"); an external
  client setting `LoopStatus::Playlist` still maps back to
  `RepeatMode::RepeatAll` (it has no way to *ask for* `RepeatEach` — a
  real, accepted MPRIS limitation, not a bug).

### Surfaces
- `toggle_repeat_mode()` (`mtune/src/audio/player.rs`) cycle becomes
  4-way: `Consecutive → RepeatAll → RepeatOne → RepeatEach → Consecutive`.
- `playback_control.rs`'s repeat button: no dedicated "repeat song N
  times" icon exists in standard symbolic icon sets, so `RepeatEach`
  reuses `media-playlist-repeat-song-symbolic` (the `RepeatOne` icon)
  with a distinguishing tooltip, `"Repeat Each Song {N} Times"` — user
  confirmed this is acceptable.
- `org.margo.Tune` (`mtune/src/dbus.rs`): `RepeatMode` string gains
  `"repeat-each"` (already string-passthrough on the shell side, no
  shell change needed for the mode name itself); new read-write
  `RepeatCount: u32` property + `SetRepeatCount(u32)` method.
- `mshell-services` (`mshell-crates/mshell-services/src/mtune.rs`):
  mirror `repeat_count: Property<u32>` + `set_repeat_count()` proxy —
  the existing `repeat_mode: Property<String>` needs no change (it
  already mirrors arbitrary strings).
- `mshell-frame` bar pill (`bars/bar_widgets/mtune.rs`) and Tune menu
  (`menus/menu_widgets/mtune/mtune.rs`): repeat icon `match` gains a
  `"repeat-each"` arm (same icon reuse as mtune's own window); the
  menu's `CycleRepeat` input cycles the same 4 states as
  `toggle_repeat_mode()`.
- `mshellctl` (`mshellctl/src/subcommands/mtune.rs`): `Repeat { mode }`
  accepts `"each"` (alongside `off`/`all`/`one`), `cycle` extends to 4
  states; new `RepeatCount { value: Option<u32> }` subcommand (omit to
  print — same convention as `Volume`/`Rate`).
- Persistence: `repeat_count` lives in `[playback]` of `mtune.toml`
  (`mtune/src/library/config.rs`'s `PlaybackSection`), loaded at
  startup into `Queue`. `repeat_mode` itself is **not** persisted
  today (always starts at `Consecutive`) and this doesn't change that.
  **Gotcha caught during design:** `PlaybackSection` currently
  `#[derive(Default)]`s — adding `repeat_count: u32` there would default
  it to `0` (bad: "repeat 0 times" breaks the feature). `PlaybackSection`
  needs a **manual** `impl Default` instead (`BehaviourSection` right
  below it in the same file already does exactly this, for the same
  reason — non-zero defaults can't come from `#[derive(Default)]`).

## 4. Feature 2: per-playlist resume position

### Storage
- Each saved playlist's `.m3u` file (written by `playlist::write_m3u`,
  `mtune/src/playlist.rs`) gains one optional comment line right after
  `#EXTM3U`: `#TUNE-RESUME:<index>` (0-based). Omitted when the index is
  `0` (the common/default case), so a freshly-saved or never-resumed
  playlist's file looks exactly as it does today. `#`-prefixed lines are
  already skipped by `parse_m3u`'s reader — this is fully backward- and
  forward-compatible with any other m3u-reading tool.
- Updating the resume index does **not** rewrite the song list — a
  small helper reads the file, replaces/inserts just that one comment
  line, and writes it back. The playlist's actual contents are only
  ever changed by an explicit "Save Playlist".

### Tracking "which playlist is active"
- New state (`Window`, alongside the existing `pending_start`):
  `active_playlist: RefCell<Option<PathBuf>>` — the saved playlist's
  file path, when the current queue was loaded via `LoadPlaylist(name)`
  (i.e. `crate::playlist::saved_path(name)`, which is always a `.m3u`
  Tune owns and can write to). Set in `open_playlist_file` when the
  path lives under `playlist::library_dir()`; cleared on folder/library
  load, `RescanLibrary`, `clear_queue`, or loading a *different*
  playlist. Arbitrary `OpenPlaylist(path)` (the "Open playlist file…"
  dialog, any path) is **out of scope** — only the named saved-playlist
  library participates, matching "bir playlisti geri yüklediğimde"
  (reloading a saved playlist), and avoiding writes to files mtune
  doesn't necessarily have permission to modify.
- At the same checkpoints `persist_resume()` already runs (timer, quit,
  shutdown, window close), if `active_playlist` is `Some(path)`, also
  write the current queue index to that path via the new resume-index
  helper.

### Restoring on load
- `StartIntent` gains a fifth... no — a new variant `AtIndex(u32)`
  (alongside `Top`/`Resume`/`Nothing`). `open_playlist_file` reads
  `playlist::resume_index(path)`; if present (and the playlist is
  non-empty), sets `pending_start = Some(StartIntent::AtIndex(ix))`
  instead of always `Top`. The existing consumption `match` in
  `queue_songs` (`window.rs` ~L1103) gains an arm:
  `Some(StartIntent::AtIndex(ix)) => player.skip_to(ix.min(queue.n_songs().saturating_sub(1)))`
  — clamped in case the playlist file was hand-edited shorter since the
  index was last recorded.

## 5. Non-goals

- No per-playlist resume for arbitrary (non-library) playlist files
  opened via "Open playlist file…" — only the saved-playlist library.
- No UI for setting `repeat_count` inside mtune's own window (no
  existing in-app settings dialog to hang it off of, per the folder-
  first minimalist design) — config file + `mshellctl` only, matching
  how `start_hidden` shipped earlier this cycle.
- No change to the app-wide `resume-uri`/`resume-position`/`current.pls`
  mechanism — Feature 2 is additive, playlist-scoped, and separate.
- No MPRIS extension for `RepeatEach` — it reports/accepts as
  `LoopStatus::Playlist`, an accepted limitation of the MPRIS spec.
