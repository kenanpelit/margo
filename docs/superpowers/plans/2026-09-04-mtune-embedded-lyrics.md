# mtune Embedded Lyrics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the shell's Lyrics pill/menu shows a track's embedded lyrics (from
the audio file's own tags) when mtune is the playing source and has them,
instead of always querying lrclib.net.

**Architecture:** mtune reads embedded lyrics via `lofty` at song-load time
and mirrors the text through its existing `Snapshot`/`org.margo.Tune`
pipeline (same path `title`/`artist` already use). The shell's
`mshell-services` layer mirrors that one new property. The shell's
source-agnostic `lyrics.rs` engine gets a tiny extension — an optional
pre-fetched hint string — so it skips its disk-cache/lrclib chain entirely
when the hint is present; the two call sites (bar pill, menu) detect "is the
playing source mtune" via the MPRIS bus name and pass the hint.

**Tech Stack:** Rust, `lofty` 0.24 (tag reading), `zbus` (mtune's
`org.margo.Tune` interface), `wayle_core::Property<T>` (shell-side mirror),
`wayle-media` 0.1.2 (generic MPRIS aggregator — read-only, not modified),
`relm4` (the two Lyrics widgets).

**Spec:** `docs/superpowers/specs/2026-09-04-mtune-embedded-lyrics-design.md`

## Global Constraints

- **rustc pinned 1.95.0.** `cargo +1.95.0 fmt --all -- --check` for the fmt
  gate.
- **Per repo workflow, the human runs the full compile/test/`just check`
  cycle and the push.** Each task below lists the exact targeted
  `cargo test -p <crate>` command; run it if you have build access,
  otherwise hand the batch to the human. Never a full release build as
  verification.
- **panic-ratchet:** non-test `.unwrap()`/`.expect(`/`panic!(`/
  `unreachable!(`/`todo!(`/`unimplemented!(` may not grow past
  `scripts/panic-baseline.txt` (currently 450). New code in this plan uses
  `Option`/`filter`/`unwrap_or_default` throughout — no new panics.
- **v1 scope, per the approved spec: plain-text embedded lyrics only.** No
  synced (SYLT) embedded reading, no settings toggle — embedded, when
  present, always wins over lrclib. Do not add either.
- **`Lyrics::Plain(Vec<String>)` is reused for embedded text** — never
  construct it without also carrying `LyricsSource` alongside (see Task 4).
- Commits: English, end with
  `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>` — no session
  link, no other trailer.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `mtune/src/audio/song.rs` | `resolve_lyrics()` pure fn; `SongData.lyrics`; `SongData::lyrics()`; `Song::lyrics()` | 1 |
| `mtune/src/audio/state.rs` | `PlayerState::lyrics()` | 1 |
| `mtune/src/bridge.rs` | `Snapshot.lyrics: String` (+ manual `Default`) | 2 |
| `mtune/src/application.rs` | `refresh_bridge()` populates `lyrics` | 2 |
| `mtune/src/dbus.rs` | `EmbeddedLyrics` read-only property on `org.margo.Tune` | 2 |
| `mshell-crates/mshell-services/src/mtune.rs` | `MtunePlayer.lyrics_embedded: Property<String>`; `refresh()` reads it; `BUS_NAME` made non-private | 3 |
| `mshell-crates/mshell-frame/src/lyrics.rs` | `LyricsSource` enum; `fetch()` takes an embedded hint, returns `(Lyrics, LyricsSource)` | 4 |
| `mshell-crates/mshell-frame/src/bars/bar_widgets/lyrics.rs` | build + pass the hint; track `source`; status text | 5 |
| `mshell-crates/mshell-frame/src/menus/menu_widgets/lyrics/lyrics_menu_widget.rs` | same + `badge_text()` | 5 |
| `docs/widgets.md` | Lyrics row mentions embedded-first | 6 |

---

## Task 1: mtune reads embedded lyrics into `SongData` / `Song` / `PlayerState`

**Files:**
- Modify: `mtune/src/audio/song.rs` (`SongData` struct ~L29-38, tag-read
  block in `from_uri` ~L124-134 where `artist`/`title`/`album` are pulled
  from `tag`, `SongData` accessors ~L40-63, `Song` accessors ~L338-355)
- Modify: `mtune/src/audio/state.rs` (`PlayerState` accessors ~L105-123)
- Test: inline `#[cfg(test)] mod tests` in `song.rs`

**Interfaces:**
- Produces:
  - `fn resolve_lyrics(tag: &lofty::tag::Tag) -> Option<String>` — free fn
    in `song.rs`, pure, no I/O.
  - `SongData.lyrics: Option<String>` field.
  - `SongData::lyrics(&self) -> Option<&str>`.
  - `Song::lyrics(&self) -> String` (empty string when absent — same shape
    as `Song::artist()`/`Song::title()`).
  - `PlayerState::lyrics(&self) -> Option<String>` (same shape as
    `PlayerState::title()`).

- [ ] **Step 1: Write the failing unit tests**

Add to the bottom of `mtune/src/audio/song.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use lofty::tag::{Tag, TagType};

    #[test]
    fn prefers_the_generic_lyrics_key() {
        let mut tag = Tag::new(TagType::VorbisComments);
        tag.insert_text(ItemKey::Lyrics, "la la la".to_string());
        tag.insert_text(ItemKey::UnsyncLyrics, "should not win".to_string());
        assert_eq!(resolve_lyrics(&tag).as_deref(), Some("la la la"));
    }

    #[test]
    fn falls_back_to_unsync_lyrics_for_id3v2() {
        // ID3v2 doesn't map ItemKey::Lyrics at all — only UnsyncLyrics
        // (the USLT frame). A tag that only has UnsyncLyrics must still
        // resolve.
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::UnsyncLyrics, "hello darkness".to_string());
        assert_eq!(resolve_lyrics(&tag).as_deref(), Some("hello darkness"));
    }

    #[test]
    fn trims_and_rejects_whitespace_only() {
        let mut tag = Tag::new(TagType::VorbisComments);
        tag.insert_text(ItemKey::Lyrics, "  padded  ".to_string());
        assert_eq!(resolve_lyrics(&tag).as_deref(), Some("padded"));

        let mut blank = Tag::new(TagType::VorbisComments);
        blank.insert_text(ItemKey::Lyrics, "   ".to_string());
        assert_eq!(resolve_lyrics(&blank), None);
    }

    #[test]
    fn no_lyrics_tag_at_all() {
        let tag = Tag::new(TagType::VorbisComments);
        assert_eq!(resolve_lyrics(&tag), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p mtune audio::song::tests`
Expected: FAIL — `resolve_lyrics` not defined, `SongData.lyrics` doesn't
exist.

- [ ] **Step 3: Add `resolve_lyrics()` and wire it into `SongData::from_uri`**

Add the free function near the top of the tag-reading section of
`song.rs` (above `impl SongData` or right before `from_uri`, matching
where other small helpers in the file live):

```rust
/// Embedded lyrics text, if the file's tag carries any. `ItemKey::Lyrics`
/// ("possibly synchronized") covers Vorbis Comments and MP4 `©lyr`;
/// **ID3v2 does not map that key at all** — an MP3's USLT frame only
/// comes through `ItemKey::UnsyncLyrics`, so that's the fallback. v1 shows
/// whatever comes back as plain text, even if it happens to contain
/// `[mm:ss.xx]`-style timestamps (no LRC parsing here).
fn resolve_lyrics(tag: &lofty::tag::Tag) -> Option<String> {
    tag.get_string(ItemKey::Lyrics)
        .or_else(|| tag.get_string(ItemKey::UnsyncLyrics))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
```

In `SongData` struct definition, add the field next to the other optional
metadata:

```rust
pub struct SongData {
    artist: Option<String>,
    title: Option<String>,
    album: Option<String>,
    lyrics: Option<String>,
    cover_art: Option<CoverArt>,
    cover_uuid: Option<String>,
    uuid: Option<String>,
    duration: u64,
    file: gio::File,
}
```

In `from_uri`, in the `if let Some(tag) = tagged_file.primary_tag()` block
(where `artist`/`title`/`album`/cover are currently read), add:

```rust
let lyrics = resolve_lyrics(tag);
```

and thread `lyrics` into every `SongData { .. }` construction in that
function (the success path *and* every early-return `SongData::default()`
gets `lyrics: None`, matching how the other `Option` fields already
default). Add the accessor next to the others:

```rust
impl SongData {
    pub fn lyrics(&self) -> Option<&str> {
        self.lyrics.as_deref()
    }
    // existing artist()/title()/album()/duration() unchanged
}
```

- [ ] **Step 4: Add `Song::lyrics()` and `PlayerState::lyrics()`**

In `mtune/src/audio/song.rs`, next to `Song::album()`:

```rust
pub fn lyrics(&self) -> String {
    match self.imp().data.borrow().lyrics() {
        Some(l) => l.to_string(),
        None => String::new(),
    }
}
```

In `mtune/src/audio/state.rs`, next to `PlayerState::title()`:

```rust
pub fn lyrics(&self) -> Option<String> {
    if let Some(song) = &*self.imp().current_song.borrow() {
        return Some(song.lyrics());
    }
    None
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p mtune audio::song::tests`
Expected: PASS — all 4 tests.

- [ ] **Step 6: Build + gates**

Run: `cargo build -p mtune`, `cargo clippy -p mtune --all-targets`,
`cargo +1.95.0 fmt -p mtune -- --check`, `./scripts/panic-ratchet.sh`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add mtune/src/audio/song.rs mtune/src/audio/state.rs
git commit -m "$(cat <<'EOF'
feat(mtune): read embedded lyrics from the audio file's own tags

resolve_lyrics() tries ItemKey::Lyrics (Vorbis Comments, MP4 (c)lyr)
then ItemKey::UnsyncLyrics (the only key ID3v2's USLT frame maps to —
lofty doesn't map Lyrics for ID3v2 at all). Plain text only; no LRC
timestamp parsing in v1. SongData/Song/PlayerState grow a lyrics()
accessor each, matching the existing artist()/title() shape.

Refs docs/superpowers/specs/2026-09-04-mtune-embedded-lyrics-design.md

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Expose `EmbeddedLyrics` on `org.margo.Tune`

**Files:**
- Modify: `mtune/src/bridge.rs` (`Snapshot` struct + its manual `Default`)
- Modify: `mtune/src/application.rs` (`refresh_bridge()`'s `Snapshot { .. }`
  construction, next to `title`/`artist`/`album`)
- Modify: `mtune/src/dbus.rs` (new `#[zbus(property)] async fn`, next to
  `title()`/`artist()`/`album()`)

**Interfaces:**
- Consumes: `PlayerState::lyrics()` (Task 1).
- Produces: `org.margo.Tune` property `EmbeddedLyrics: String` — the
  current track's embedded text, or `""`. Changes exactly when `Title`
  does (same `Changed` signal, no new signal).

- [ ] **Step 1: `Snapshot.lyrics`**

In `mtune/src/bridge.rs`, add the field right after `album`:

```rust
pub struct Snapshot {
    pub has_song: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub lyrics: String,
    /// Absolute path to the current track's cached cover, or empty.
    pub cover_art: String,
    // ...unchanged...
```

and in the manual `impl Default for Snapshot`, add `lyrics: String::new(),`
right after `album: String::new(),`.

- [ ] **Step 2: Populate it in `refresh_bridge()`**

In `mtune/src/application.rs`, in the `Snapshot { .. }` literal inside
`refresh_bridge()` (where `title: state.title().unwrap_or_default(),` and
`artist: state.artist().unwrap_or_default(),` already are), add:

```rust
lyrics: state.lyrics().unwrap_or_default(),
```

- [ ] **Step 3: `org.margo.Tune::EmbeddedLyrics`**

In `mtune/src/dbus.rs`, next to the existing `async fn title(&self)`:

```rust
#[zbus(property)]
async fn embedded_lyrics(&self) -> String {
    self.snap
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .lyrics
        .clone()
}
```

(zbus maps the snake_case method name to the PascalCase D-Bus property
name automatically — `embedded_lyrics` → `EmbeddedLyrics`, matching how
`cover_art` → `CoverArt` already works in this file.)

- [ ] **Step 4: Build + gates**

Run: `cargo build -p mtune`, `cargo clippy -p mtune --all-targets`,
`cargo +1.95.0 fmt -p mtune -- --check`
Expected: clean. (No new tests here — this task is plumbing that mirrors
three already-tested-by-precedent fields; Task 1 covers the actual logic.)

- [ ] **Step 5: Commit**

```bash
git add mtune/src/bridge.rs mtune/src/application.rs mtune/src/dbus.rs
git commit -m "$(cat <<'EOF'
feat(mtune): expose EmbeddedLyrics on org.margo.Tune

Snapshot.lyrics threaded through refresh_bridge() exactly like
title/artist/album; new read-only EmbeddedLyrics D-Bus property.
Changes on the same Changed signal as the rest of the now-playing
metadata -- no new signal needed.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Mirror `EmbeddedLyrics` in `mshell-services`

**Files:**
- Modify: `mshell-crates/mshell-services/src/mtune.rs` (`MtunePlayer`
  struct + its `Default`/constructor, `refresh()`, `BUS_NAME`)

**Interfaces:**
- Consumes: `org.margo.Tune::EmbeddedLyrics` (Task 2).
- Produces: `MtunePlayer.lyrics_embedded: Property<String>`. `BUS_NAME`
  becomes visible outside this module (`pub(crate)` at minimum — Task 5's
  call sites need to compare against it).

- [ ] **Step 1: Add the field**

Next to `pub cover_art: Property<Option<String>>,` in the `MtunePlayer`
struct, add:

```rust
pub lyrics_embedded: Property<String>,
```

and in the constructor (next to `cover_art: Property::new(None),` — or
wherever the struct is built with `Property::new(default)` per field):

```rust
lyrics_embedded: Property::new(String::new()),
```

- [ ] **Step 2: Read it in `refresh()`**

In the `refresh` function's `macro_rules! get { ... }` block, next to
`if let Some(v) = get!("CoverArt", String) { ... }`, add:

```rust
if let Some(v) = get!("EmbeddedLyrics", String) {
    p.lyrics_embedded.set(v);
}
```

- [ ] **Step 3: Make `BUS_NAME` reachable**

Change `const BUS_NAME: &str = "org.mpris.MediaPlayer2.org.margo.Tune";`
to `pub(crate) const BUS_NAME: &str = "org.mpris.MediaPlayer2.org.margo.Tune";`
(no other change — every existing use in this file is unaffected by
widening visibility).

- [ ] **Step 4: Build + gates**

Run: `cargo build -p mshell-services`,
`cargo clippy -p mshell-services --all-targets`,
`cargo +1.95.0 fmt -p mshell-services -- --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-services/src/mtune.rs
git commit -m "$(cat <<'EOF'
feat(mshell-services): mirror mtune's EmbeddedLyrics property

Same shape as every other MtunePlayer field -- one more get!() line
in refresh(). BUS_NAME -> pub(crate) so the Lyrics widgets (next
task) can identify "the playing source is mtune" by its real MPRIS
bus name instead of string-matching the display identity.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `lyrics.rs` — accept an embedded hint, track provenance

**Files:**
- Modify: `mshell-crates/mshell-frame/src/lyrics.rs`
- Test: inline `#[cfg(test)]` (extend the existing test module if one
  exists in this file; add one at the bottom if not)

**Interfaces:**
- Consumes: nothing new at compile time (this task is self-contained; the
  hint is just `Option<&str>` supplied by the caller).
- Produces:
  ```rust
  pub(crate) enum LyricsSource { Embedded, Lrclib }
  pub(crate) fn fetch(key: &TrackKey, embedded: Option<&str>) -> (Lyrics, LyricsSource)
  ```
  Replaces the old `pub(crate) fn fetch(key: &TrackKey) -> Lyrics`. Every
  caller (Task 5) must pass `(Lyrics, LyricsSource)` through together —
  never re-derive `LyricsSource` from a bare `Lyrics` later (a `Plain`
  embedded result and a `Plain` lrclib result are structurally identical).

- [ ] **Step 1: Write the failing test**

Add to `lyrics.rs` (new `#[cfg(test)] mod tests` if the file has none —
check first; if a test module already exists, add these into it):

```rust
#[cfg(test)]
mod embedded_hint_tests {
    use super::*;

    fn key() -> TrackKey {
        TrackKey {
            artist: "Artist".into(),
            title: "Title".into(),
            album: "Album".into(),
            duration_secs: 180,
        }
    }

    #[test]
    fn embedded_hint_short_circuits_to_plain_lines() {
        let (lyrics, source) = fetch(&key(), Some("line one\nline two\n"));
        assert_eq!(source, LyricsSource::Embedded);
        match lyrics {
            Lyrics::Plain(lines) => assert_eq!(lines, vec!["line one", "line two"]),
            other => panic!("expected Plain, got {other:?}"),
        }
    }

    #[test]
    fn blank_hint_is_treated_as_absent() {
        // Falls through to the real (network) path -- just assert it
        // does NOT take the embedded shortcut; don't assert on network
        // outcome here (that's covered by the existing lrclib tests).
        let (_, source) = fetch(&key(), Some("   "));
        assert_eq!(source, LyricsSource::Lrclib);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p mshell-frame lyrics::embedded_hint_tests`
Expected: FAIL — `fetch` doesn't take a second argument yet,
`LyricsSource` undefined.

(Note: `blank_hint_is_treated_as_absent` will attempt a real network call
once the signature exists but before you've verified test-network
availability — if the sandbox has no network, this specific test may hang
or error on the `ureq` call. If that happens, it's an existing property of
`fetch`'s remote path, not something this task introduces; the fix is the
same one the codebase already uses elsewhere for network-dependent tests
(skip/ignore in an offline CI, or accept it needs a live run) -- do not
change `fetch`'s retry/timeout behavior to work around it.)

- [ ] **Step 3: Implement**

Add the enum near the top of the file, by `Lyrics`:

```rust
/// Where a resolved `Lyrics` value came from -- drives the status badge.
/// Always returned alongside a `Lyrics`, never inferred after the fact
/// (`Lyrics::Plain` is structurally identical for both sources).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LyricsSource {
    Embedded,
    Lrclib,
}
```

Rename the current `fetch` body to a private `fetch_remote`, and add the
new public entry point:

```rust
pub(crate) fn fetch(key: &TrackKey, embedded: Option<&str>) -> (Lyrics, LyricsSource) {
    if let Some(text) = embedded.map(str::trim).filter(|s| !s.is_empty()) {
        let lines = text.lines().map(str::to_string).collect();
        return (Lyrics::Plain(lines), LyricsSource::Embedded);
    }
    (fetch_remote(key), LyricsSource::Lrclib)
}

fn fetch_remote(key: &TrackKey) -> Lyrics {
    // ...the existing fetch() body, unchanged...
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p mshell-frame lyrics::embedded_hint_tests`
Expected: PASS (network permitting per the Step 2 note).

- [ ] **Step 5: Build + gates**

Run: `cargo build -p mshell-frame`,
`cargo clippy -p mshell-frame --all-targets`,
`cargo +1.95.0 fmt -p mshell-frame -- --check`
Expected: **errors** at this point — the two call sites (bar pill, menu)
still call the old one-argument `fetch(&key)`. That's expected; Task 5
fixes them. Confirm the *only* errors are at those two call sites (nothing
else references `lyrics::fetch`).

- [ ] **Step 6: Commit**

```bash
git add mshell-crates/mshell-frame/src/lyrics.rs
git commit -m "$(cat <<'EOF'
feat(mshell-frame): lyrics::fetch() takes an embedded-text hint

New LyricsSource::{Embedded,Lrclib} always travels with a Lyrics --
a Lyrics::Plain from an embedded tag and one from lrclib are
structurally identical, so provenance can't be inferred later. When
the hint is non-blank, fetch() short-circuits before touching the
disk cache or the network at all.

This intentionally breaks the two Lyrics widget call sites -- fixed
in the next commit.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire the bar pill + menu to the embedded hint

**Files:**
- Modify: `mshell-crates/mshell-frame/src/bars/bar_widgets/lyrics.rs`
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/lyrics/lyrics_menu_widget.rs`

**Interfaces:**
- Consumes: `lyrics::fetch(key, embedded) -> (Lyrics, LyricsSource)`
  (Task 4); `mshell_services::mtune::{mtune_service, BUS_NAME}` (Task 3).
- Produces: both widgets compile again; the menu's badge distinguishes
  Embedded from lrclib-unsynced.

- [ ] **Step 1: Fix the menu widget**

In `lyrics_menu_widget.rs`:
1. Add a `source: LyricsSource` field to the model struct, initialised to
   `LyricsSource::Lrclib` alongside wherever `lyrics: Lyrics::None` (or
   equivalent) is initialised.
2. At the `fetch` call site (inside the `spawn_blocking` closure that
   currently does `lyrics::fetch(&key)`), build the hint first:

```rust
let embedded = display_player()
    .filter(|p| p.id.bus_name() == mshell_services::mtune::BUS_NAME)
    .map(|_| mshell_services::mtune::mtune_service().player.lyrics_embedded.get())
    .filter(|s| !s.is_empty());
let (lyrics, source) = lyrics::fetch(&key, embedded.as_deref());
```

   and store both `self.lyrics = lyrics;` / `self.source = source;` where
   the old single assignment was.
3. Update `badge_text(&self) -> &'static str`:

```rust
fn badge_text(&self) -> &'static str {
    if !self.has_player {
        return "";
    }
    if self.loading {
        return "Searching lyrics…";
    }
    match (&self.lyrics, self.source) {
        (Lyrics::Synced(_), _) => "Synced · lrclib.net",
        (Lyrics::Plain(_), LyricsSource::Embedded) => "Embedded",
        (Lyrics::Plain(_), LyricsSource::Lrclib) => "Unsynced · lrclib.net",
        (Lyrics::Instrumental, _) => "Instrumental",
        (Lyrics::None, _) => "No lyrics found",
    }
}
```

- [ ] **Step 2: Fix the bar pill**

In `bars/bar_widgets/lyrics.rs`: same three changes (model field,
hint-building before `lyrics::fetch`, store `source` alongside `lyrics`).
If this file's status/tooltip text also names "lrclib.net" anywhere for
the synced/unsynced states, give it the same `Embedded` branch as the menu
widget; if it only shows a generic icon/state (no source name), no text
change is needed there -- check the file to see which applies.

- [ ] **Step 3: Build + gates**

Run: `cargo build -p mshell-frame`,
`cargo clippy -p mshell-frame --all-targets`,
`cargo +1.95.0 fmt -p mshell-frame -- --check`,
`cargo test -p mshell-frame -p mshell-services -p mtune`
Expected: all clean, all green (this closes out the Task 4 "expected
errors" note).

- [ ] **Step 4: Commit**

```bash
git add mshell-crates/mshell-frame/src/bars/bar_widgets/lyrics.rs \
        mshell-crates/mshell-frame/src/menus/menu_widgets/lyrics/lyrics_menu_widget.rs
git commit -m "$(cat <<'EOF'
feat(mshell): Lyrics pill/menu prefer mtune's embedded lyrics

Both call sites detect "the playing source is mtune" by MPRIS bus
name (wayle_media::Player::id.bus_name(), not the human-readable
identity string) and pass mtune_service().player.lyrics_embedded as
fetch()'s hint. Every other player (Spotify, MPD, ...) is completely
unaffected -- the hint is only ever non-None for mtune. Menu badge
gains an "Embedded" state.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Docs

**Files:**
- Modify: `docs/widgets.md` (the `Lyrics` row)

**Interfaces:** none.

- [ ] **Step 1: Update the Lyrics row**

In the `## Media` table, change the `Lyrics` row's description to mention
the embedded-first behaviour:

```
| `Lyrics` | Lyrics | Current synced lyric line of the now-playing track, scrolling in the bar. Prefers the track's own embedded lyrics (if the file has any -- mtune only) over a lrclib.net lookup. Click opens the full scrolling lyrics panel. |
```

- [ ] **Step 2: Commit**

```bash
git add docs/widgets.md
git commit -m "$(cat <<'EOF'
docs(widgets): note embedded-lyrics preference for mtune

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 3: Push everything**

```bash
git push origin main
```

---

## On-device verification (human)

After `just mtune && just shell`:
- Play a file with an embedded USLT (MP3) or Vorbis `LYRICS` (FLAC/OGG) tag
  in mtune -> the Lyrics menu badge reads **"Embedded"** immediately, no
  "Searching lyrics…" flash, and the text matches the tag.
- Play a file with no embedded lyrics in mtune -> behaves exactly as
  before (lrclib lookup, "Synced · lrclib.net" / "Unsynced · lrclib.net" /
  "No lyrics found").
- Switch the active player to Spotify/MPD mid-session -> lrclib path,
  unaffected by any of this.
- `gdbus call --session --dest org.mpris.MediaPlayer2.org.margo.Tune \
  --object-path /org/margo/Tune --method \
  org.freedesktop.DBus.Properties.Get org.margo.Tune EmbeddedLyrics` returns
  the tag text (or `''`).

---

## Self-Review

**1. Spec coverage:** §1 (mtune reads embedded, plain-only, Lyrics-then-
UnsyncLyrics fallback) → Task 1. §2 (D-Bus property) → Task 2. §3 (shell
mirror) → Task 3. §4 (`fetch` hint + `LyricsSource`) → Task 4. §5 (call
sites + badge) → Task 5. Edge-case table: "mtune not display player" and
"no embedded tag" are both exercised by the hint being `None`/filtered,
which Task 4's tests cover directly; "song changes mid-fetch" is existing,
untouched behaviour (no new task needed, noted in the spec as already
handled). Testing section's on-device checklist → the plan's own
"On-device verification". No spec requirement lacks a task.

**2. Placeholder scan:** no TBD/TODO; every step has literal code or an
exact shell command.

**3. Type consistency:** `resolve_lyrics(tag: &lofty::tag::Tag) -> Option<String>`
(Task 1) used only inside `song.rs`, not exposed further. `SongData::lyrics()
-> Option<&str>` → `Song::lyrics() -> String` → `PlayerState::lyrics() ->
Option<String>` → `Snapshot.lyrics: String` → `EmbeddedLyrics: String` (D-Bus)
→ `MtunePlayer.lyrics_embedded: Property<String>` (Task 3) → `Option<&str>`
hint via `.get()` + `.filter(!is_empty)` at the Task 5 call sites → consumed
by `fetch(key: &TrackKey, embedded: Option<&str>) -> (Lyrics, LyricsSource)`
(Task 4). Every hop matches the next hop's input type; the `String` ↔
`Option<&str>`/`Option<String>` boundary crossings are all explicit
`.filter(|s| !s.is_empty())` / `.as_deref()` calls, not silent coercions.
`LyricsSource` defined once (Task 4), consumed identically in both Task 5
call sites.

