# mtune Embedded Lyrics — Design

**Goal:** the shell's Lyrics pill/menu shows a track's **embedded** lyrics
(from the audio file's own tags) when mtune is the playing source and the
file has them, instead of always going to lrclib.net.

**Non-goals:** no synced (timestamp-per-line) embedded lyrics — v1 is plain
text only (see §1). No new settings toggle — embedded, when present, always
wins (user decision). No change to how non-mtune players (Spotify, MPD, …)
resolve lyrics — they keep using lrclib.net exactly as today.

## Background — what already exists

- **The Lyrics feature is shipped and source-agnostic.** `mshell-crates/mshell-frame/src/lyrics.rs`
  is a pure lrclib.net client: `pub(crate) fn fetch(key: &TrackKey) -> Lyrics`
  does disk-cache (`~/.cache/mshell/lyrics/<hash>.json`, keyed by a
  `DefaultHasher` of lowercased artist+title+album+duration, no TTL) →
  lrclib `/api/get` exact match → `/api/search` fuzzy fallback. A transient
  network error returns `Lyrics::None` **without** caching so a retry
  happens next play; a definitive "no lyrics"/instrumental answer IS cached.
  `Lyrics` (lyrics.rs ~L27-35): `Synced(Vec<LyricLine>) | Plain(Vec<String>) |
  Instrumental | None`. `fetch` is blocking (`ureq`); both call sites run it
  via `tokio::task::spawn_blocking`.
- **Bar pill** (`bars/bar_widgets/lyrics.rs`) and **menu panel**
  (`menus/menu_widgets/lyrics/lyrics_menu_widget.rs`) both pick the display
  player the same way: `display_player()` — first `Playing` in
  `media_service().player_list`, else wayle's `active_player`, else the
  first in the list (`wayle-media`, the generic MPRIS aggregator). Neither
  file has any per-source special-casing today.
- **`mshellctl menu lyrics` already exists** (`mshellctl/src/subcommands/menu.rs`) —
  nothing to add there.
- **mtune already shows up in the generic aggregator** — confirmed live,
  `mshellctl media list` lists an `identity: "Tune"` entry alongside Spotify/
  MPD. mtune's MPRIS bus name is `org.mpris.MediaPlayer2.org.margo.Tune`
  (`mtune/src/audio/mpris_controller.rs`). `wayle_media::core::player::Player`
  carries `pub id: PlayerId`, and `PlayerId::bus_name(&self) -> &str` returns
  that exact string (`~/.cargo/.../wayle-media-0.1.2/src/types.rs`) — the
  robust way to detect "the playing source is mtune" from either lyrics call
  site, no string-matching on the human-readable `identity`.
- **mtune's own tag reading** (`mtune/src/audio/song.rs::SongData::from_uri`)
  uses `lofty` 0.24 (`Probe` → `guess_file_type` → `.read()` →
  `primary_tag()`) and currently reads only `TrackArtist` / `title()` /
  `album()` / cover art. No lyrics field exists on `SongData` today.
- **lofty 0.24's `ItemKey`** has `Lyrics` ("(possibly synchronized) lyrics
  text") and `UnsyncLyrics` ("unsynchronized lyrics text") — confirmed via
  docs.rs. Critically, **`ItemKey::Lyrics` is *not* supported in ID3v2**; an
  MP3's USLT frame only maps through `ItemKey::UnsyncLyrics`. Vorbis Comments
  (FLAC/OGG) and MP4 (`©lyr`) map through `ItemKey::Lyrics`. So a correct
  read tries both keys. ID3v2's SYLT (the one format with a *specified*
  synced-lyrics frame) has no generic-`Tag` accessor in lofty — reading it
  needs the format-specific `id3v2` API, which is why synced embedded is out
  of scope for v1.
- **`org.margo.Tune`** (`mtune/src/dbus.rs`) already exposes
  `Playing/HasSong/Title/Artist/Album/CoverArt/Position/Duration/Rate/
  Playlists/…` as zbus properties, mirrored into the shell by
  `mshell-services/mtune.rs`'s `MtunePlayer` (`wayle_core::Property<T>`
  fields, refreshed from mtune's `Changed` signal + `NameOwnerChanged`,
  `CacheProperties::No` — see the existing `refresh()` fn).

## Design

### 1. mtune reads embedded lyrics (plain text only)

`SongData` gains `lyrics: Option<String>`. In `from_uri`, after the existing
`primary_tag()` block:

```rust
let lyrics = tag
    .get_string(&ItemKey::Lyrics)
    .or_else(|| tag.get_string(&ItemKey::UnsyncLyrics))
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_string);
```

`Lyrics` first (covers Vorbis/MP4, and — per lofty's own doc note — some
Vorbis-tagged files overload it with LRC-format text anyway), falling back
to `UnsyncLyrics` (ID3v2 USLT). Whatever comes back is shown **as raw text,
line-split on `\n`** — if a Vorbis `LYRICS` blob happens to contain
`[mm:ss.xx]` timestamps, v1 displays those literally as part of the line
text rather than parsing them; a future pass could detect and promote that
case to `Lyrics::Synced`, out of scope here per the "always show embedded
as-is" decision.

No synced (SYLT) reading in v1 (see Background).

### 2. Expose it on `org.margo.Tune`

New read-only property `EmbeddedLyrics: String` — the current song's
embedded text, or `""` when absent. Same shape as the existing
`Title`/`Artist`/… properties: read from `player.current_song()` on the
GTK/glib side (`mpris_controller.rs` pattern), no new signal — it changes
exactly when `Title`/`Artist` change (song change), which already fires
`org.margo.Tune`'s `Changed` signal via `refresh_bridge`.

### 3. Mirror it in the shell

`mshell-services/mtune.rs`'s `MtunePlayer` gains
`pub lyrics_embedded: Property<String>` (default `String::new()`), read in
the existing `refresh()` alongside `Title`/`Artist` (one more `get!(...)`
line — no new watcher, no new poll).

### 4. `lyrics.rs`: accept an embedded hint, track provenance

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LyricsSource { Embedded, Lrclib }

pub(crate) fn fetch(key: &TrackKey, embedded: Option<&str>) -> (Lyrics, LyricsSource) {
    if let Some(text) = embedded.filter(|s| !s.trim().is_empty()) {
        let lines = text.lines().map(str::to_string).collect();
        return (Lyrics::Plain(lines), LyricsSource::Embedded);
    }
    (fetch_remote(key), LyricsSource::Lrclib) // today's body, renamed
}
```

`Lyrics`'s shape doesn't change — embedded text reuses `Plain(Vec<String>)`
exactly like an unsynced lrclib result. When `embedded` is `Some`, the disk
cache and the network are **not touched at all** (no read, no write) — it's
strictly cheaper than today, and re-reading the tag on every song change is
free (mtune already did it once at load time; the shell just mirrors the
string).

### 5. Call sites pass the hint, badge learns the source

Both `bars/bar_widgets/lyrics.rs` and
`menus/menu_widgets/lyrics/lyrics_menu_widget.rs`, right before calling
`fetch`:

```rust
let embedded = (player.id.bus_name() == mshell_services::mtune::BUS_NAME)
    .then(|| mtune_service().player.lyrics_embedded.get())
    .filter(|s| !s.is_empty());
let (lyrics, source) = lyrics::fetch(&key, embedded.as_deref());
```

(`mshell-services/mtune.rs`'s `BUS_NAME` const — currently private —
becomes `pub(crate)`/`pub` for this cross-module read; no other change to
it.) Both widget models gain a `source: LyricsSource` field alongside their
existing `lyrics: Lyrics` field, set together. `badge_text()` (menu) reads
it:

```rust
match (&self.lyrics, self.source) {
    (Lyrics::Synced(_), _) => "Synced · lrclib.net",              // embedded is never Synced (v1)
    (Lyrics::Plain(_), LyricsSource::Embedded) => "Embedded",
    (Lyrics::Plain(_), LyricsSource::Lrclib) => "Unsynced · lrclib.net",
    (Lyrics::Instrumental, _) => "Instrumental",
    (Lyrics::None, _) => "No lyrics found",
}
```

The bar pill's simpler status text (icon/short label, not a full badge) gets
the same `Embedded` case added wherever it currently distinguishes
synced/unsynced.

## Edge cases

| case | behaviour |
|---|---|
| mtune playing, file has no embedded lyrics tag | `EmbeddedLyrics = ""` → hint filtered to `None` → falls through to lrclib exactly as today |
| mtune playing, embedded present, lrclib also has (better) synced lyrics | embedded wins unconditionally (user decision) — no network call is even made |
| mtune not the display player (e.g. Spotify is playing louder/newer) | `player.id.bus_name()` check is false → `embedded = None` → today's lrclib path, unchanged |
| mtune playing but window/tray only, no track yet (`HasSong=false`) | `Title`/`EmbeddedLyrics` are empty strings → same as "no player"/"no track" today, no special case needed |
| song changes mid-fetch (fast skip) | unchanged existing behaviour — `TrackKey` mismatch on completion is already handled by the current code (re-render only if the key still matches) |
| a Vorbis `LYRICS` tag contains `[00:12.34]`-style text | shown verbatim as part of the line (documented v1 limitation, not a bug) |
| lofty fails to parse the file (corrupt tag) | `SongData::from_uri` already falls back to `SongData::default()` on any probe/read error (existing behavior) → `lyrics: None`, same as no tag |

## Testing

- `mtune`: unit tests in `song.rs` for the `ItemKey::Lyrics` /
  `ItemKey::UnsyncLyrics` fallback order and the empty/whitespace-only
  filter, using synthetic `Tag` values (no real audio file I/O needed — the
  existing `SongData` tests, if any, are the pattern to follow; if none
  exist, a small `#[cfg(test)]` module using `lofty::tag::Tag::new` +
  `insert_text` is enough).
- `mshell-frame`: unit tests for `lyrics::fetch` — embedded-hint path
  returns `(Lyrics::Plain(_), LyricsSource::Embedded)` and never touches the
  cache/network path (pure function once `embedded` is `Some`, no I/O to
  mock); empty/whitespace hint falls through to the existing remote-path
  tests unchanged.
- On-device (user): a file with an embedded USLT/Vorbis-LYRICS tag playing
  in mtune shows "Embedded" in the Lyrics menu instantly (no "Searching
  lyrics…" flash); a file without one still round-trips to lrclib as before;
  switching from mtune to Spotify mid-session still uses lrclib for Spotify.

## Files touched

| file | change |
|---|---|
| `mtune/src/audio/song.rs` | `SongData.lyrics: Option<String>` + read in `from_uri` |
| `mtune/src/dbus.rs` | `EmbeddedLyrics` read-only property on `org.margo.Tune` |
| `mshell-crates/mshell-services/src/mtune.rs` | `MtunePlayer.lyrics_embedded: Property<String>` + `refresh()` read; `BUS_NAME` → non-private |
| `mshell-crates/mshell-frame/src/lyrics.rs` | `LyricsSource` enum; `fetch()` takes `embedded: Option<&str>`, returns `(Lyrics, LyricsSource)` |
| `mshell-crates/mshell-frame/src/bars/bar_widgets/lyrics.rs` | build + pass the embedded hint; track `source`; status text gains the Embedded case |
| `mshell-crates/mshell-frame/src/menus/menu_widgets/lyrics/lyrics_menu_widget.rs` | same + `badge_text()` gains the Embedded case |

## Open risks

- **lofty's cross-format guarantee is documented, not battle-tested by us.**
  The `Lyrics`-then-`UnsyncLyrics` fallback is correct per docs.rs, but the
  three real-world formats (ID3v2 USLT, Vorbis `LYRICS`, MP4 `©lyr`) should
  each get one on-device smoke file during implementation, since lofty's tag
  abstraction has had rough edges before (this codebase already special-cased
  `Texture::for_pixbuf` deprecation and other lofty-adjacent lessons).
- **`Lyrics::Plain` reuse means embedded and unsynced-remote are
  structurally identical** — only `LyricsSource` tells them apart. Any
  future code path that constructs a `Lyrics::Plain` without threading
  `LyricsSource` through will silently mislabel the badge. Keep the two
  always paired (a tuple return from `fetch`, never a bare `Lyrics`) rather
  than inferring source after the fact.
