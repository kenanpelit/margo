# mtune Repeat-Each + Playlist Resume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a configurable "repeat each track N times, then advance"
mode across mtune + the shell's Tune menu + `mshellctl`, and make
reloading a saved playlist resume at the track it was last on.

**Architecture:** Feature 1 adds a fourth `RepeatMode` variant plus a
separate `Queue.repeat_count` property (GObject enums can't carry a
payload); the existing `next_index()` pure function is left untouched
and a sibling `next_index_repeat_each()` handles the new mode, called
from `next_song()`. Feature 2 embeds a small `#TUNE-RESUME:<index>`
comment line in each saved playlist's `.m3u` file, updated at the same
checkpoints mtune's existing app-wide `persist_resume()` already runs,
and read back through a new `StartIntent::AtIndex` variant.

**Tech Stack:** Rust, GTK4 + glib (mtune, `glib::Enum`/GObject
properties), zbus (`org.margo.Tune`), relm4 (shell Tune menu), clap
(`mshellctl`).

**Spec:** `docs/superpowers/specs/2026-09-05-mtune-repeat-each-and-playlist-resume-design.md`

## Global Constraints

- Commits: English, end with `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>` — no session link, no other trailer.
- Every task ends with the relevant `cargo build`/`clippy --all-targets`/
  `fmt -- --check`/`test`, plus `./scripts/panic-ratchet.sh` for mtune
  tasks — all clean before commit.
- `repeat_mode` itself is never persisted (matches existing behaviour —
  always starts at `Consecutive`); only `repeat_count` is config-persisted.
- Feature 2 only covers the named saved-playlist library
  (`playlist::saved_path`) — arbitrary `OpenPlaylist(path)` files are
  out of scope (see spec §5).

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `mtune/src/audio/player.rs` | `RepeatMode::RepeatEach` + `Display`; `toggle_repeat_mode()` 4-way | 1, 2 |
| `mtune/src/audio/queue.rs` | `repeat_count`/`repeat_plays` state; `next_index_repeat_each()`; reset sites | 1 |
| `mtune/src/audio/mpris_controller.rs` | `loop_status()` maps `RepeatEach` → `LoopStatus::Playlist` | 2 |
| `mtune/src/playback_control.rs` | repeat button icon/tooltip for the new mode | 2 |
| `mtune/src/library/config.rs` | `PlaybackSection.repeat_count` (manual `Default`) | 3 |
| `mtune/src/application.rs` | apply `repeat_count` at startup; `AppCommand::SetRepeatCount` handler + persist; `Snapshot.repeat_count` | 3, 4 |
| `mtune/src/bridge.rs` | `AppCommand::SetRepeatCount`; `Snapshot.repeat_count` | 4 |
| `mtune/src/dbus.rs` | `"repeat-each"` string mapping; `RepeatCount`/`SetRepeatCount` on `org.margo.Tune` | 4 |
| `mshell-crates/mshell-services/src/mtune.rs` | mirror `repeat_count`; `set_repeat_count()` proxy | 5 |
| `mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs` | 4-way `CycleRepeat`; icon/tooltip with count | 6 |
| `mshellctl/src/subcommands/mtune.rs` | `repeat each`; 4-way `cycle`; `RepeatCount` subcommand | 7 |
| `mtune/src/playlist.rs` | `#TUNE-RESUME:` read/write/update helpers | 8 |
| `mtune/src/window.rs` | `StartIntent::AtIndex`; `active_playlist` tracking; resume-on-load | 9 |
| `docs/companion-tools.md` / `docs/widgets.md` | mention both features | 10 |

---

## Task 1: `RepeatMode::RepeatEach` + queue-level state + pure logic

**Files:**
- Modify: `mtune/src/audio/player.rs` (`RepeatMode` enum ~L44-51, `Display` impl ~L53-61)
- Modify: `mtune/src/audio/queue.rs` (`next_index()` ~L14-27, `imp::Queue` struct ~L36-42, `ObjectImpl` ~L63-90, `next_song()` ~L232-253, 6 `current_pos` reset sites, `#[cfg(test)] mod tests` ~L344-381)

**Interfaces:**
- Produces:
  - `RepeatMode::RepeatEach` variant.
  - `fn next_index_repeat_each(current: u32, n_songs: u32, repeat_count: u32, plays_so_far: u32, manual: bool) -> (Option<u32>, u32)` — pure, unit-tested. Returns `(next_index, new_plays_so_far)`.
  - `Queue::repeat_count(&self) -> u32` / `Queue::set_repeat_count(&self, n: u32)` (GObject `"repeat-count"` property, read-only at the GObject level like `repeat-mode` — set only via the dedicated method, which `notify()`s).
- Consumes: nothing new.

- [ ] **Step 1: Write the failing tests**

Add to `mtune/src/audio/queue.rs`'s existing `mod tests`:

```rust
    #[test]
    fn repeat_each_replays_then_advances() {
        // 3-song queue, count=3, currently on song 0, auto (track ended).
        assert_eq!(next_index_repeat_each(0, 3, 3, 0, false), (Some(0), 1)); // 1st replay
        assert_eq!(next_index_repeat_each(0, 3, 3, 1, false), (Some(0), 2)); // 2nd replay
        assert_eq!(next_index_repeat_each(0, 3, 3, 2, false), (Some(1), 0)); // 3rd play done, advance
    }

    #[test]
    fn repeat_each_wraps_at_the_end() {
        assert_eq!(next_index_repeat_each(2, 3, 3, 2, false), (Some(0), 0));
    }

    #[test]
    fn repeat_each_manual_skip_advances_immediately() {
        // Manual skip always moves on and resets the counter, regardless
        // of how many replays have happened so far -- same convention as
        // RepeatOne's manual-skip-breaks-the-loop rule.
        assert_eq!(next_index_repeat_each(0, 3, 3, 1, true), (Some(1), 0));
        assert_eq!(next_index_repeat_each(2, 3, 3, 0, true), (Some(0), 0));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p mtune next_index_repeat_each`
Expected: FAIL — function not defined.

- [ ] **Step 3: Add the enum variant + Display**

In `mtune/src/audio/player.rs`:

```rust
#[derive(Clone, Copy, Debug, glib::Enum, PartialEq, Default)]
#[enum_type(name = "TuneRepeatMode")]
pub enum RepeatMode {
    #[default]
    Consecutive,
    RepeatAll,
    RepeatOne,
    RepeatEach,
}

impl Display for RepeatMode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            RepeatMode::Consecutive => write!(f, "consecutive"),
            RepeatMode::RepeatAll => write!(f, "repeat-all"),
            RepeatMode::RepeatOne => write!(f, "repeat-one"),
            RepeatMode::RepeatEach => write!(f, "repeat-each"),
        }
    }
}
```

- [ ] **Step 4: Implement `next_index_repeat_each` and the queue state**

In `mtune/src/audio/queue.rs`, next to `next_index`:

```rust
/// `next_index` counterpart for `RepeatMode::RepeatEach`: repeats the
/// current track `repeat_count` times before advancing (wrapping at the
/// end of the queue), tracking replays via `plays_so_far`. Manual skip
/// always advances immediately and resets the counter -- same
/// "an explicit skip always moves to a different track" convention
/// `next_index` already applies to `RepeatOne`. Pure so it's directly
/// unit-testable; the caller stores the returned counter back into
/// `Queue`'s `repeat_plays` cell.
fn next_index_repeat_each(
    current: u32,
    n_songs: u32,
    repeat_count: u32,
    plays_so_far: u32,
    manual: bool,
) -> (Option<u32>, u32) {
    if manual || plays_so_far + 1 >= repeat_count {
        let next = if current + 1 < n_songs { current + 1 } else { 0 };
        (Some(next), 0)
    } else {
        (Some(current), plays_so_far + 1)
    }
}
```

Add to `imp::Queue`:

```rust
    pub struct Queue {
        pub model: ShuffleListModel,
        pub store: gio::ListStore,
        pub repeat_mode: Cell<RepeatMode>,
        /// Configurable N for `RepeatMode::RepeatEach` (Settings/CLI via
        /// `mshellctl mtune repeat-count`; persisted in `mtune.toml`).
        pub repeat_count: Cell<u32>,
        /// How many times the *current* track has played consecutively
        /// under `RepeatEach`. Internal only -- not a GObject property.
        pub repeat_plays: Cell<u32>,
        pub current_pos: Cell<Option<u32>>,
        pub shuffled: Cell<bool>,
    }
```

In `ObjectSubclass::new()`, add `repeat_count: Cell::new(3), repeat_plays: Cell::new(0),` alongside the existing field inits.

In `ObjectImpl::properties()`, add to the `vec![...]`:

```rust
                    ParamSpecUInt::builder("repeat-count").read_only().build(),
```

And in `property()`:

```rust
                "repeat-count" => self.repeat_count.get().to_value(),
```

Add to `impl Queue` (near `repeat_mode()`/`set_repeat_mode()`):

```rust
    pub fn repeat_count(&self) -> u32 {
        self.imp().repeat_count.get()
    }

    pub fn set_repeat_count(&self, count: u32) {
        let old = self.imp().repeat_count.replace(count);
        if old != count {
            self.notify("repeat-count");
        }
    }
```

- [ ] **Step 5: Wire `next_song()` to use it**

Replace the body of `next_song()`:

```rust
    pub fn next_song(&self, manual: bool) -> Option<Song> {
        let n_songs = self.imp().model.n_items();
        if n_songs == 0 {
            return None;
        }

        let Some(current) = self.current_song_index() else {
            self.imp().current_pos.replace(Some(0));
            self.notify("current");
            return self.song_at(0);
        };

        let repeat_mode = self.imp().repeat_mode.get();
        let next = if repeat_mode == RepeatMode::RepeatEach {
            let (next, plays) = next_index_repeat_each(
                current,
                n_songs,
                self.imp().repeat_count.get(),
                self.imp().repeat_plays.get(),
                manual,
            );
            self.imp().repeat_plays.set(plays);
            next
        } else {
            next_index(current, n_songs, repeat_mode, manual)
        };
        self.imp().current_pos.replace(next);
        self.notify("current");
        next.and_then(|n| self.song_at(n))
    }
```

- [ ] **Step 6: Reset `repeat_plays` at every other `current_pos` mutation site**

Add `self.imp().repeat_plays.set(0);` immediately alongside each of these
6 existing `current_pos.replace(...)` calls (every one *except* the one
inside `next_song()` just rewritten above):
- `set_current_song()`'s two branches (`Some(i)` match and the `None` else)
- `remove_song()`'s `if self.is_empty() { self.imp().current_pos.replace(None); }`
- `clear()`
- `skip_song()`
- `previous_song()`

Example for `skip_song`:

```rust
    pub fn skip_song(&self, pos: u32) -> Option<Song> {
        self.imp().current_pos.replace(Some(pos));
        self.imp().repeat_plays.set(0);
        self.notify("current");
        self.song_at(pos)
    }
```

Apply the same one-line addition (right after the `current_pos.replace`
call, before any early return) at the other 5 sites.

- [ ] **Step 7: Run to verify pass**

Run: `cargo test -p mtune`
Expected: all pass, including the 3 new tests and the existing 4
`next_index` tests (untouched).

- [ ] **Step 8: Build + gates**

```bash
cargo build -p mtune
```

Expected: **errors** — `mpris_controller.rs`'s `loop_status()` and
`playback_control.rs`'s `set_repeat_mode()` are exhaustive matches with
no wildcard arm; adding a 4th `RepeatMode` variant breaks both. That's
expected; Task 2 fixes them. Confirm the *only* errors are in those two
files (nothing else references `RepeatMode` exhaustively).

- [ ] **Step 9: Commit**

```bash
git add mtune/src/audio/player.rs mtune/src/audio/queue.rs
git commit -m "$(cat <<'EOF'
feat(mtune): RepeatMode::RepeatEach -- repeat each track N times

New Queue.repeat_count (default 3, GObject property like repeat-mode)
and internal repeat_plays counter. next_index_repeat_each() is a new
pure sibling to the existing next_index() (kept untouched, still
covers Consecutive/RepeatAll/RepeatOne) so the existing tests and
call sites for those three modes are undisturbed. Manual skip always
advances immediately and resets the counter -- same "an explicit
skip breaks the loop" rule next_index already applies to RepeatOne.

This intentionally breaks the two other exhaustive-match call sites
(mpris_controller, playback_control) -- fixed in the next commit.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Cycle button, MPRIS mapping, icon/tooltip

**Files:**
- Modify: `mtune/src/audio/player.rs` (`toggle_repeat_mode()` ~L564-576)
- Modify: `mtune/src/audio/mpris_controller.rs` (`loop_status()` ~L45-50)
- Modify: `mtune/src/playback_control.rs` (`set_repeat_mode()` ~L110-126)

**Interfaces:** none new — closes out Task 1's intentional breakage.

- [ ] **Step 1: 4-way cycle**

```rust
    pub fn toggle_repeat_mode(&self) {
        let cur_mode = self.queue.repeat_mode();
        let new_mode = match cur_mode {
            RepeatMode::Consecutive => RepeatMode::RepeatAll,
            RepeatMode::RepeatAll => RepeatMode::RepeatOne,
            RepeatMode::RepeatOne => RepeatMode::RepeatEach,
            RepeatMode::RepeatEach => RepeatMode::Consecutive,
        };
        self.queue.set_repeat_mode(new_mode);

        for c in &self.controllers {
            c.set_repeat_mode(new_mode);
        }
    }
```

- [ ] **Step 2: MPRIS `LoopStatus` mapping**

`RepeatEach` has no MPRIS equivalent (`None`/`Track`/`Playlist` only) —
report it as `Playlist` (an external client sees "some loop is on"):

```rust
fn loop_status(repeat: RepeatMode) -> LoopStatus {
    match repeat {
        RepeatMode::Consecutive => LoopStatus::None,
        RepeatMode::RepeatOne => LoopStatus::Track,
        RepeatMode::RepeatAll | RepeatMode::RepeatEach => LoopStatus::Playlist,
    }
}
```

(The reverse mapping, `set_loop_status`, is untouched — an external
client setting `LoopStatus::Playlist` still lands on `RepeatAll`; it has
no way to *request* `RepeatEach`, an accepted MPRIS limitation.)

- [ ] **Step 3: Repeat button icon/tooltip**

```rust
    pub fn set_repeat_mode(&self, repeat_mode: RepeatMode) {
        let repeat_button = self.imp().repeat_button.get();
        match repeat_mode {
            RepeatMode::Consecutive => {
                repeat_button.set_icon_name("media-playlist-consecutive-symbolic");
                repeat_button.set_tooltip_text(Some(&i18n("Do Not Repeat")));
            }
            RepeatMode::RepeatAll => {
                repeat_button.set_icon_name("media-playlist-repeat-symbolic");
                repeat_button.set_tooltip_text(Some(&i18n("Repeat All Songs")));
            }
            RepeatMode::RepeatOne => {
                repeat_button.set_icon_name("media-playlist-repeat-song-symbolic");
                repeat_button.set_tooltip_text(Some(&i18n("Repeat the Current Song")));
            }
            RepeatMode::RepeatEach => {
                // No dedicated "repeat song N times" icon exists in the
                // standard symbolic set -- reuse RepeatOne's icon, the
                // tooltip is what actually distinguishes the mode.
                repeat_button.set_icon_name("media-playlist-repeat-song-symbolic");
                repeat_button.set_tooltip_text(Some(&i18n_f(
                    "Repeat Each Song {} Times",
                    &[&self.imp().queue.get().map(|q| q.repeat_count()).unwrap_or(3).to_string()],
                )));
            }
        }
    }
```

Check `playback_control.rs`'s existing imports/fields for how it reaches
the live `Queue` (it may already hold a reference, or may need `i18n_f`
imported alongside the existing `i18n`) — adapt the exact accessor to
whatever this struct already has; the point is the tooltip must read the
*current* `repeat_count`, not a hardcoded `3`.

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mtune
cargo clippy -p mtune --all-targets
cargo +1.95.0 fmt -p mtune -- --check
cargo test -p mtune
./scripts/panic-ratchet.sh
```

Expected: all clean now (Task 1's intentional breakage resolved).

- [ ] **Step 5: Commit**

```bash
git add mtune/src/audio/player.rs mtune/src/audio/mpris_controller.rs mtune/src/playback_control.rs
git commit -m "$(cat <<'EOF'
feat(mtune): wire RepeatEach into the cycle button, MPRIS, and UI

toggle_repeat_mode() cycles all 4 modes now. MPRIS has no fourth
LoopStatus value, so RepeatEach reports as Playlist -- an external
client sees "some loop is on" but can't specifically request
RepeatEach (an accepted MPRIS-spec limitation). The repeat button
reuses RepeatOne's icon (no "repeat song N times" glyph exists in
the standard symbolic set) with a tooltip naming the live count.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Persist `repeat_count` in config

**Files:**
- Modify: `mtune/src/library/config.rs` (`PlaybackSection` ~L77-81)
- Modify: `mtune/src/application.rs` (`load_library()` ~L272-282)

**Interfaces:**
- Produces: `PlaybackSection.repeat_count: u32` (default `3`, manual
  `impl Default` -- see the gotcha below).

- [ ] **Step 1: Add the field with a manual `Default`**

`PlaybackSection` currently `#[derive(..., Default, ...)]`s. Adding
`repeat_count: u32` there would default it to `0` via the derive --
"repeat 0 times" degrades to "always advance", silently defeating the
feature for anyone who never touches the setting. `BehaviourSection`
right below it in this same file hits the identical problem for its
own non-zero-ish defaults and solves it with a manual `impl Default` --
follow that exact precedent:

```rust
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct PlaybackSection {
    pub on_start: OnStart,
    /// Times to repeat each track under `RepeatMode::RepeatEach` before
    /// advancing. Settings/CLI via `mshellctl mtune repeat-count`.
    pub repeat_count: u32,
}

impl Default for PlaybackSection {
    fn default() -> Self {
        Self {
            on_start: OnStart::default(),
            repeat_count: 3,
        }
    }
}
```

(Remove `Default` from the `#[derive(...)]` list on `PlaybackSection`
since it's now a manual `impl`.)

- [ ] **Step 2: Apply it at startup**

In `load_library()`, right after `let cfg = self.imp().config.borrow().clone();`
and *before* the `if roots.is_empty()` early return (repeat_count must
apply even when no library is configured yet):

```rust
        let cfg = self.imp().config.borrow().clone();
        self.imp()
            .player
            .queue()
            .set_repeat_count(cfg.playback.repeat_count);
        let roots = cfg.library.resolved_roots();
```

- [ ] **Step 3: Test the config round-trip**

Add near `library/config.rs`'s existing `on_start` round-trip tests
(same file, `#[cfg(test)] mod tests`):

```rust
    #[test]
    fn repeat_count_defaults_to_three() {
        let c = MtuneConfig::default();
        assert_eq!(c.playback.repeat_count, 3);
    }

    #[test]
    fn repeat_count_roundtrips() {
        let mut c = MtuneConfig::default();
        c.playback.repeat_count = 5;
        let toml = toml::to_string_pretty(&c).unwrap();
        let back: MtuneConfig = toml::from_str(&toml).unwrap();
        assert_eq!(back.playback.repeat_count, 5);
    }
```

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mtune
cargo clippy -p mtune --all-targets
cargo +1.95.0 fmt -p mtune -- --check
cargo test -p mtune
./scripts/panic-ratchet.sh
```

- [ ] **Step 5: Commit**

```bash
git add mtune/src/library/config.rs mtune/src/application.rs
git commit -m "$(cat <<'EOF'
feat(mtune): persist repeat_count in mtune.toml

PlaybackSection switches to a manual impl Default (matching
BehaviourSection right below it in this file, same reasoning): a
derived Default would zero repeat_count, silently turning
RepeatEach into a no-op "always advance" for anyone who never
touches the setting. Applied to the live Queue at startup in
load_library(), before the no-roots-configured early return so it
takes effect even with an empty library.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `org.margo.Tune` D-Bus surface for `RepeatCount`

**Files:**
- Modify: `mtune/src/bridge.rs` (`Snapshot` ~L16-41 + `Default`, `AppCommand` ~L84-106)
- Modify: `mtune/src/dbus.rs` (`set_repeat_mode` ~L278-284, new property + method next to `rate`/`set_rate`)
- Modify: `mtune/src/application.rs` (`refresh_bridge()`'s `Snapshot { .. }` literal, `AppCommand` dispatch `match`)

**Interfaces:**
- Consumes: `Queue::repeat_count()`/`set_repeat_count()` (Task 1).
- Produces: `Snapshot.repeat_count: u32`; `AppCommand::SetRepeatCount(u32)`;
  `org.margo.Tune`'s `RepeatCount: u32` (read-only property) +
  `SetRepeatCount(u32)` method. Task 5 mirrors this exact property name.

- [ ] **Step 1: `Snapshot` + `AppCommand`**

In `mtune/src/bridge.rs`, add to `Snapshot` (after `pub repeat: RepeatMode,`):

```rust
    /// Configured N for `RepeatMode::RepeatEach`.
    pub repeat_count: u32,
```

And to its `Default` impl (after `repeat: RepeatMode::default(),`):

```rust
            repeat_count: 3,
```

Add to `AppCommand` (after `SetRepeat(RepeatMode),`):

```rust
    /// Configured N for `RepeatMode::RepeatEach`.
    SetRepeatCount(u32),
```

- [ ] **Step 2: `refresh_bridge()` + dispatch**

In `mtune/src/application.rs`'s `refresh_bridge()`, add
`repeat_count: queue.repeat_count(),` to the `Snapshot { .. }` literal
(next to `repeat: queue.repeat_mode(),`).

Add a dispatch arm (next to `AppCommand::SetRepeat(m) => player.update_repeat_mode(m),`):

```rust
            AppCommand::SetRepeatCount(n) => {
                {
                    let mut cfg = imp.config.borrow_mut();
                    cfg.playback.repeat_count = n;
                    if let Err(e) = cfg.save() {
                        debug!("mtune: could not save mtune.toml: {e}");
                    }
                }
                player.queue().set_repeat_count(n);
            }
```

- [ ] **Step 3: D-Bus property + method**

In `mtune/src/dbus.rs`, add the `"repeat-each"` mapping to
`set_repeat_mode`:

```rust
    async fn set_repeat_mode(&self, mode: String) {
        let mode = match mode.as_str() {
            "repeat-all" => RepeatMode::RepeatAll,
            "repeat-one" => RepeatMode::RepeatOne,
            "repeat-each" => RepeatMode::RepeatEach,
            _ => RepeatMode::Consecutive,
        };
        self.send(AppCommand::SetRepeat(mode));
    }
```

Add, next to `rate`/`set_rate`:

```rust
    /// Configured N for `RepeatMode::RepeatEach`.
    #[zbus(property)]
    async fn repeat_count(&self) -> u32 {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .repeat_count
    }

    async fn set_repeat_count(&self, count: u32) {
        self.send(AppCommand::SetRepeatCount(count));
    }
```

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mtune
cargo clippy -p mtune --all-targets
cargo +1.95.0 fmt -p mtune -- --check
cargo test -p mtune
./scripts/panic-ratchet.sh
```

- [ ] **Step 5: Commit**

```bash
git add mtune/src/bridge.rs mtune/src/dbus.rs mtune/src/application.rs
git commit -m "$(cat <<'EOF'
feat(mtune): expose RepeatCount + repeat-each on org.margo.Tune

Read-only RepeatCount property (mirrors Snapshot, same shape as
every other org.margo.Tune field) + SetRepeatCount method (mirrors
SetRate's write-via-method pattern -- the property itself isn't
zbus-settable). set_repeat_mode's string parser gains "repeat-each".
SetRepeatCount persists to mtune.toml immediately, same pattern
SetLibraryRoots already uses.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `mshell-services` mirrors `RepeatCount`

**Files:**
- Modify: `mshell-crates/mshell-services/src/mtune.rs` (`MtunePlayer` struct, constructor, `refresh()`, method list)

**Interfaces:**
- Consumes: `org.margo.Tune::RepeatCount`/`SetRepeatCount` (Task 4).
- Produces: `MtunePlayer.repeat_count: Property<u32>`;
  `MtunePlayer::set_repeat_count(&self, n: u32)`. Task 6 consumes both.

- [ ] **Step 1: Add the field**

```rust
    /// Configured N for repeat-each mode.
    pub repeat_count: Property<u32>,
```

In `MtunePlayer::new()`: `repeat_count: Property::new(3),`.

- [ ] **Step 2: Read it in `refresh()`**

```rust
    if let Some(v) = get!("RepeatCount", u32) {
        p.repeat_count.set(v);
    }
```

- [ ] **Step 3: Add the proxy method**

Next to `set_rate`:

```rust
    pub async fn set_repeat_count(&self, count: u32) {
        self.call("SetRepeatCount", &(count,)).await;
    }
```

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mshell-services
cargo clippy -p mshell-services --all-targets
cargo +1.95.0 fmt -p mshell-services -- --check
cargo test -p mshell-services
```

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-services/src/mtune.rs
git commit -m "$(cat <<'EOF'
feat(mshell-services): mirror mtune's RepeatCount

Same Property<T> mirror shape as every other org.margo.Tune field;
repeat_mode's existing String mirror needs no change (already a raw
passthrough, so "repeat-each" flows through it automatically).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Tune menu's repeat cycle gains the 4th mode

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs` (repeat_btn view ~L305-321, `CycleRepeat` handler ~L588-595)

**Interfaces:** none new.

- [ ] **Step 1: Icon/tooltip**

```rust
                    #[name = "repeat_btn"]
                    gtk::Button {
                        set_css_classes: &["mtune-toggle"],
                        #[watch]
                        set_icon_name: match model.repeat.as_str() {
                            "repeat-one" | "repeat-each" => "media-playlist-repeat-song-symbolic",
                            "repeat-all" => "media-playlist-repeat-symbolic",
                            _ => "media-playlist-consecutive-symbolic",
                        },
                        #[watch]
                        set_tooltip_text: Some(&match model.repeat.as_str() {
                            "repeat-one" => "Repeat: one".to_string(),
                            "repeat-all" => "Repeat: all".to_string(),
                            "repeat-each" => format!("Repeat: each ({}\u{00d7})", model.repeat_count),
                            _ => "Repeat: off".to_string(),
                        }),
                        connect_clicked => MtuneMenuInput::CycleRepeat,
                    },
```

- [ ] **Step 2: 4-way cycle**

```rust
            MtuneMenuInput::CycleRepeat => {
                let next = match self.repeat.as_str() {
                    "consecutive" => "repeat-all",
                    "repeat-all" => "repeat-one",
                    "repeat-one" => "repeat-each",
                    _ => "consecutive",
                };
                tokio_rt_spawn(async move { mtune_service().player.set_repeat_mode(next).await });
            }
```

- [ ] **Step 3: Add `repeat_count` to the model**

Add `repeat_count: u32,` to `MtuneMenuWidgetModel` (next to `rate: f64,`),
initialise `repeat_count: 3,` in `init()`'s model literal, add
`m.repeat_count = p.repeat_count.get();` to `read()` (next to
`m.rate = p.rate.get();`), and add
`Box::pin(p.repeat_count.watch().map(|_| ())),` to `init()`'s watched
stream `Vec` (next to `p.rate.watch()`).

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mshell-frame
cargo clippy -p mshell-frame --all-targets
cargo +1.95.0 fmt -p mshell-frame -- --check
cargo test -p mshell-frame
./scripts/panic-ratchet.sh
./scripts/design-lint.sh
```

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs
git commit -m "$(cat <<'EOF'
feat(mshell-frame): Tune menu's repeat cycle gains repeat-each

Reuses the repeat-one icon (no dedicated glyph exists) with a
tooltip naming the live repeat_count, mirroring mtune's own window.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `mshellctl` CLI surface

**Files:**
- Modify: `mshellctl/src/subcommands/mtune.rs` (`Repeat` doc comment + match ~L39, L132-149; new `RepeatCount` variant + handler)

**Interfaces:** none new (pure CLI, talks to Task 4's D-Bus surface directly).

- [ ] **Step 1: Extend `Repeat`**

```rust
    /// Repeat mode: `off` / `all` / `one` / `each` / `cycle`; omit to print it.
    Repeat { mode: Option<String> },
    /// Times to repeat each track under `repeat each` mode; omit to print it.
    RepeatCount { value: Option<u32> },
```

```rust
        MtuneCommands::Repeat { mode } => match mode {
            None => println!("{}", get::<String>(&conn, "RepeatMode").await?),
            Some(m) => {
                let cur = get::<String>(&conn, "RepeatMode").await.unwrap_or_default();
                let target = match m.as_str() {
                    "off" | "none" | "consecutive" => "consecutive",
                    "all" | "playlist" | "repeat-all" => "repeat-all",
                    "one" | "track" | "repeat-one" => "repeat-one",
                    "each" | "repeat-each" => "repeat-each",
                    "cycle" | "next" => match cur.as_str() {
                        "consecutive" => "repeat-all",
                        "repeat-all" => "repeat-one",
                        "repeat-one" => "repeat-each",
                        _ => "consecutive",
                    },
                    other => bail!("unknown repeat mode '{other}' (off / all / one / each / cycle)"),
                };
                call(&conn, "SetRepeatMode", &(target,)).await?;
            }
        },
        MtuneCommands::RepeatCount { value } => match value {
            None => println!("{}", get::<u32>(&conn, "RepeatCount").await?),
            Some(n) => call(&conn, "SetRepeatCount", &(n,)).await?,
        },
```

- [ ] **Step 2: Build + gates**

```bash
cargo build -p mshellctl
cargo clippy -p mshellctl --all-targets
cargo +1.95.0 fmt -p mshellctl -- --check
```

- [ ] **Step 3: Commit**

```bash
git add mshellctl/src/subcommands/mtune.rs
git commit -m "$(cat <<'EOF'
feat(mshellctl): mtune repeat each + repeat-count subcommand

`mshellctl mtune repeat each` and `repeat cycle` (now 4-way) join
off/all/one; `mshellctl mtune repeat-count [N]` reads/sets the
configurable repeat-each count, same "omit to print" convention as
volume/rate.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Playlist resume-index storage

**Files:**
- Modify: `mtune/src/playlist.rs`

**Interfaces:**
- Produces:
  - `pub fn resume_index(path: &Path) -> Option<u32>` — parses
    `#TUNE-RESUME:<n>` from an existing playlist file, if present.
  - `pub fn update_resume_index(path: &Path, index: u32) -> std::io::Result<()>` —
    rewrites *only* that comment line (or inserts/removes it), leaving
    every other line untouched.

- [ ] **Step 1: Write the failing tests**

Add a `#[cfg(test)] mod tests` at the bottom of `mtune/src/playlist.rs`
(none exists yet in this file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture(dir: &std::path::Path, body: &str) -> PathBuf {
        let path = dir.join("fixture.m3u");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn resume_index_absent_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(dir.path(), "#EXTM3U\n/song1.mp3\n/song2.mp3\n");
        assert_eq!(resume_index(&path), None);
    }

    #[test]
    fn resume_index_reads_the_comment() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "#EXTM3U\n#TUNE-RESUME:2\n/song1.mp3\n/song2.mp3\n/song3.mp3\n",
        );
        assert_eq!(resume_index(&path), Some(2));
    }

    #[test]
    fn update_resume_index_inserts_then_updates_without_touching_songs() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(dir.path(), "#EXTM3U\n/song1.mp3\n/song2.mp3\n");

        update_resume_index(&path, 1).unwrap();
        assert_eq!(resume_index(&path), Some(1));
        let songs = parse(&path);
        assert_eq!(songs.len(), 2);

        update_resume_index(&path, 0).unwrap();
        // Index 0 removes the line entirely (keeps a pristine file for
        // the common/never-resumed case).
        assert_eq!(resume_index(&path), None);
        assert_eq!(parse(&path).len(), 2);
    }
}
```

This needs a dev-dependency: check `mtune/Cargo.toml`'s `[dev-dependencies]`
for `tempfile` — add `tempfile = "3"` there if it isn't already present
(other crates in this workspace already depend on `tempfile`, so the
version is already pinned in `Cargo.lock`; run `cargo metadata --offline`
after editing to confirm it resolves without a lockfile change, per
`docs/superpowers/... reference_cargo_lock_locked_packaging` -- if it
does need a `Cargo.lock` update, that's expected and must be committed
alongside).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p mtune resume_index`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement**

```rust
const RESUME_PREFIX: &str = "#TUNE-RESUME:";

/// The 0-based track index a playlist last left off at, if the file
/// carries a `#TUNE-RESUME:` comment (written by `update_resume_index`).
pub fn resume_index(path: &Path) -> Option<u32> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(RESUME_PREFIX))
        .and_then(|n| n.trim().parse().ok())
}

/// Rewrite just the `#TUNE-RESUME:` line in `path` -- every other line
/// (the `#EXTM3U` header, `#EXTINF` metadata, song paths) is copied
/// through unchanged. `index == 0` removes the line entirely, keeping a
/// never-resumed or freshly-saved playlist's file pristine. A missing or
/// unreadable file is a silent no-op (nothing to update).
pub fn update_resume_index(path: &Path, index: u32) -> std::io::Result<()> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(());
    };
    let mut out = String::with_capacity(text.len() + 16);
    let mut inserted = false;
    for line in text.lines() {
        if line.starts_with(RESUME_PREFIX) {
            if index > 0 && !inserted {
                out.push_str(&format!("{RESUME_PREFIX}{index}\n"));
                inserted = true;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if !inserted && line.starts_with("#EXTM3U") && index > 0 {
            out.push_str(&format!("{RESUME_PREFIX}{index}\n"));
            inserted = true;
        }
    }
    if index > 0 && !inserted {
        // No #EXTM3U header (a bare/legacy playlist) -- prepend it.
        out = format!("{RESUME_PREFIX}{index}\n{out}");
    }
    fs::write(path, out)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p mtune resume_index update_resume_index`
Expected: PASS.

- [ ] **Step 5: Build + gates**

```bash
cargo build -p mtune
cargo clippy -p mtune --all-targets
cargo +1.95.0 fmt -p mtune -- --check
cargo test -p mtune
./scripts/panic-ratchet.sh
```

- [ ] **Step 6: Commit**

```bash
git add mtune/src/playlist.rs mtune/Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
feat(mtune): #TUNE-RESUME comment line in saved playlists

resume_index()/update_resume_index() read/write a single optional
comment line right after #EXTM3U -- every m3u reader already skips
'#'-prefixed lines, so this is fully backward/forward compatible.
Only that one line is ever touched; the playlist's actual song list
is only ever changed by an explicit Save. index == 0 removes the
line, keeping a never-resumed playlist's file exactly as before.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Wire resume into load/playback

**Files:**
- Modify: `mtune/src/window.rs` (`StartIntent` enum ~L77-85, `open_playlist_file` ~L986-1000, `queue_songs`'s `pending_start` match ~L1103-1117, `persist_resume`-adjacent trigger points ~L1549-1558)

**Interfaces:**
- Consumes: `playlist::resume_index()`/`update_resume_index()` (Task 8).
- Produces: `StartIntent::AtIndex(u32)`.

- [ ] **Step 1: New `StartIntent` variant**

```rust
pub enum StartIntent {
    /// Select the top of the queue, paused.
    Top,
    /// Skip to the track with this URI and seek to this position (secs),
    /// paused; falls back to `Top` if the URI is no longer in the library.
    Resume(String, u64),
    /// Skip to this queue index (clamped to the queue's bounds), paused.
    /// Used to resume a saved playlist at its remembered position.
    AtIndex(u32),
    /// Leave the queue untouched — no current song.
    Nothing,
}
```

- [ ] **Step 2: Track the active playlist path**

Add a field to `Window`'s `imp` struct (alongside `pending_start`):

```rust
    /// The saved playlist backing the current queue, if any -- set by
    /// `open_playlist_file` when the path is one of the named
    /// saved playlists (`playlist::library_dir()`), cleared on
    /// folder/library load or loading a different playlist. Drives
    /// live resume-index persistence (see `persist_resume` call sites).
    pub active_playlist: RefCell<Option<PathBuf>>,
```

- [ ] **Step 3: `open_playlist_file` sets intent + tracks the path**

```rust
    pub fn open_playlist_file(&self, path: &std::path::Path) {
        let files: Vec<gio::File> = crate::playlist::parse(path)
            .into_iter()
            .map(gio::File::for_path)
            .collect();
        if files.is_empty() {
            self.add_toast(i18n("Playlist is empty or unreadable"));
            return;
        }
        if let Some(p) = self.player() {
            p.clear_queue();
        }
        let is_saved = path.starts_with(crate::playlist::library_dir());
        self.imp()
            .active_playlist
            .replace(is_saved.then(|| path.to_path_buf()));
        let intent = is_saved
            .then(|| crate::playlist::resume_index(path))
            .flatten()
            .map(StartIntent::AtIndex)
            .unwrap_or(StartIntent::Top);
        self.imp().pending_start.replace(Some(intent));
        self.queue_songs(files);
    }
```

- [ ] **Step 4: Clear `active_playlist` on folder/library loads**

Find every other place `pending_start` is set to `StartIntent::Top` for
a *non*-playlist load (folder open, library load, "Open File") in
`window.rs` and `application.rs` (`AppCommand::PlayFolder`'s
`win.load_library_files(vec![...], StartIntent::Top)` call at
`application.rs:683` is one; grep for
`StartIntent::Top` to find the rest) and add
`self.imp().active_playlist.replace(None);` (or the equivalent on
whichever object owns that call) right alongside each, so a playlist's
resume-index only keeps updating while that playlist is *actually* the
active queue's source.

- [ ] **Step 5: Consume `AtIndex` in `queue_songs`**

```rust
                            if was_empty {
                                match win.imp().pending_start.borrow_mut().take() {
                                    Some(StartIntent::Resume(uri, pos)) => {
                                        match queue.position_of_uri(&uri) {
                                            Some(ix) => {
                                                player.skip_to(ix);
                                                player.queue_resume_seek(pos);
                                            }
                                            None => player.skip_to(0),
                                        }
                                    }
                                    Some(StartIntent::AtIndex(ix)) => {
                                        let clamped = ix.min(queue.n_songs().saturating_sub(1));
                                        player.skip_to(clamped);
                                    }
                                    Some(StartIntent::Nothing) => {}
                                    _ => player.skip_to(0),
                                }
                            }
```

- [ ] **Step 6: Persist the resume index at the existing checkpoints**

At `mtune/src/window.rs:1549-1558` (the "remember where we were" block
next to `persist_resume`) and wherever `Application::persist_resume()`
runs (`mtune/src/application.rs` ~L909-918), add: if
`self.imp().active_playlist.borrow().as_deref()` is `Some(path)` (on
`Window`) — or the equivalent reachable from `Application` at its own
checkpoint — call
`let _ = crate::playlist::update_resume_index(path, queue.current_song_index().unwrap_or(0));`
right alongside the existing GSettings writes. Both call sites already
run on the same timer/quit/shutdown/close triggers, so no new
scheduling is needed — just add the extra write next to the existing one.

- [ ] **Step 7: Build + gates**

```bash
cargo build -p mtune
cargo clippy -p mtune --all-targets
cargo +1.95.0 fmt -p mtune -- --check
cargo test -p mtune
./scripts/panic-ratchet.sh
```

- [ ] **Step 8: Commit**

```bash
git add mtune/src/window.rs
git commit -m "$(cat <<'EOF'
feat(mtune): saved playlists resume where they were last left off

New StartIntent::AtIndex + Window.active_playlist tracking. Loading
a saved playlist (LoadPlaylist / the library sidebar) now reads its
#TUNE-RESUME comment and skips there instead of always starting at
track 0; the index keeps updating at the same timer/quit/shutdown/
close checkpoints the existing app-wide resume-uri mechanism already
uses. Arbitrary (non-library) playlist files opened via "Open
playlist file..." are unaffected -- only the named saved-playlist
library participates.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Docs + push

**Files:**
- Modify: `docs/companion-tools.md` (mtune section — repeat modes / playlist behaviour, if listed)
- Modify: `docs/widgets.md` (`Mtune` row, if it enumerates repeat modes)

**Interfaces:** none.

- [ ] **Step 1: Update whichever doc(s) enumerate mtune's repeat modes**

Grep `docs/companion-tools.md` and `docs/widgets.md` for "repeat" — if
either lists the 3 existing modes by name, add "repeat each" and a
one-line mention that saved playlists resume at their last position.
If neither doc goes into this level of detail today, skip this step
(don't invent a section that didn't exist).

- [ ] **Step 2: Commit (only if Step 1 changed something)**

```bash
git add docs/companion-tools.md docs/widgets.md
git commit -m "$(cat <<'EOF'
docs: mtune repeat-each mode + playlist resume

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
- `mshellctl mtune repeat each` then play a track to the end 3 times
  (default count) — it should advance to the next track on the 3rd
  natural end, not sooner; pressing Next manually at any point during
  the 3 should jump immediately instead of just consuming one repeat.
- `mshellctl mtune repeat-count 5` then repeat the above — 5 plays now.
- Tune menu's repeat button cycles through all 4 icons/tooltips;
  bar-pill/menu tooltip shows the live count for "each".
- Save a playlist, play into track 3, quit mtune (or wait for the
  periodic save), relaunch, load that playlist — it resumes at track 3,
  not track 1. Playing a *different* playlist and reloading the first
  one again still resumes each at its own last position.
- `mshellctl mtune repeat off` / `all` / `one` / `cycle` still all work
  exactly as before.

---

## Self-Review

**1. Spec coverage:** §3 (`RepeatMode::RepeatEach`, all surfaces) →
Tasks 1-2, 4-7. §3's config gotcha → Task 3. §4 (playlist resume
storage, tracking, restore) → Tasks 8-9. Non-goals (§5: no in-app
settings UI, no MPRIS extension, arbitrary-file playlists out of
scope, app-wide resume untouched) — confirmed no task introduces any
of these.

**2. Placeholder scan:** no TBD/TODO; every step has literal code
(Task 9 Step 4's "grep for the rest" is a directed search over a
known, small pattern — not an unresolved placeholder — since the
exact first instance and file are already named).

**3. Type consistency:** `RepeatMode::RepeatEach` (Task 1) is consumed
identically by `toggle_repeat_mode`/`loop_status`/`playback_control`
(Task 2), `set_repeat_mode`'s string parser (Task 4), the shell menu's
`&str` match (Task 6), and `mshellctl`'s CLI tokens (Task 7) — all via
the same `"repeat-each"` wire string, never a second spelling.
`repeat_count: u32` is the one shape end-to-end: `Queue.repeat_count`
(Task 1) → `PlaybackSection.repeat_count` (Task 3) →
`Snapshot.repeat_count` / `org.margo.Tune::RepeatCount` (Task 4) →
`MtunePlayer.repeat_count` (Task 5) →
`MtuneMenuWidgetModel.repeat_count` (Task 6) → `mshellctl`'s
`RepeatCount { value: Option<u32> }` (Task 7).
