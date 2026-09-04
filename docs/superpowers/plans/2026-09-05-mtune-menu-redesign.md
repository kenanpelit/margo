# Tune Menu Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote the Tune bar-pill menu from a compact quick-settings
card to a DESIGN.md §12 "panel" surface with a real queue browser
(browse, filter, play, remove), while keeping every existing feature
(now-playing, seek, transport, shuffle/repeat, speed, library root,
playlists, launch/open).

**Architecture:** One new read-only `org.margo.Tune` property
(`QueueEntries`) carries the queue's song list to the shell, mirrored
into `MtunePlayer` the same way every other property already is. The
menu widget itself is restructured (not rewritten from scratch): the
existing `read()` / `apply_dynamic()` / `setup_seek()` /
`now_playing_meta()` / `library_status()` machinery is kept, extended
with queue state + a client-side substring filter, and the view gains
a hand-rolled §12 header + a scrollable queue section using the
Clipboard/Notifications "cap the inner list, not the outer scroller"
pattern.

**Tech Stack:** Rust, relm4 0.11 (GTK4), zbus 5.15 (`org.margo.Tune`),
`wayle_core::Property<T>`, `grass`-compiled SCSS.

**Spec:** `docs/superpowers/specs/2026-09-05-mtune-menu-redesign-design.md`

## Global Constraints

- Commits: English, end with `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>` — no session link, no other trailer.
- Every task ends with: `cargo build`, `cargo clippy --all-targets`,
  `cargo +1.95.0 fmt -- --check`, `cargo test` for the crates it
  touched, `./scripts/panic-ratchet.sh`, `./scripts/design-lint.sh` —
  all clean before commit (mirrors this repo's `just check`).
- No feature this menu already has may be removed or made harder to
  reach — only reorganized/restyled.
- `LyricsSource`-style lesson from the last feature: when a helper
  returns data that also drives a UI label/state, keep the *shape*
  simple (a plain tuple here, not a new struct) unless a second field
  is genuinely needed — YAGNI.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `mtune/src/dbus.rs` | `QueueEntries` read-only property on `org.margo.Tune` | 1 |
| `mtune/src/bridge.rs` | `Snapshot.queue_entries: Vec<(String,String,u64)>` | 1 |
| `mtune/src/application.rs` | `refresh_bridge()` populates it from `queue.song_at(i)` | 1 |
| `mshell-crates/mshell-services/src/mtune.rs` | `MtunePlayer.queue_entries: Property<...>`; `remove_index()` proxy | 2 |
| `mshell-crates/mshell-config/src/schema/config.rs` | `mtune_menu` default size (500×760) | 3 |
| `mshell-crates/mshell-frame/src/menus/menu.rs` | drop outer `effect_max_height!` for `MenuType::Mtune`; css_class → plain `"mtune-menu"` | 3 |
| `mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs` | panel header, hero, control row, queue section | 4, 5 |
| `mshell-crates/mshell-style/scss/04-components/_mtune.scss` | panel root, header, queue-row, filter-pill styles | 6 |
| `docs/widgets.md` | `Mtune` row mentions the queue browser | 7 |

---

## Task 1: mtune exposes `QueueEntries` on `org.margo.Tune`

**Files:**
- Modify: `mtune/src/bridge.rs` (`Snapshot` struct ~L16-41, `Default` impl ~L44-68)
- Modify: `mtune/src/dbus.rs` (new `#[zbus(property)]`, next to `queue_length`/`current_index` ~L140-155)
- Modify: `mtune/src/application.rs` (`refresh_bridge()` ~L585-620, the `Snapshot { .. }` literal)

**Interfaces:**
- Consumes: `Queue::n_songs() -> u32`, `Queue::song_at(u32) -> Option<Song>`
  (`mtune/src/audio/queue.rs`, unchanged), `Song::title()/artist() -> String`,
  `Song::duration() -> u64` (`mtune/src/audio/song.rs`, unchanged).
- Produces: `Snapshot.queue_entries: Vec<(String, String, u64)>` (title,
  artist, duration_secs); `org.margo.Tune`'s `QueueEntries` property of
  the same shape. Task 2 mirrors this exact tuple shape — do not add a
  4th field or a wrapper struct without updating Task 2 too.

- [ ] **Step 1: Add the field to `Snapshot`**

In `mtune/src/bridge.rs`, add after `pub current_index: i64,`:

```rust
    /// (title, artist, duration_secs) for every song in the queue, in
    /// queue order. Rebuilt on every refresh — not diffed — since the
    /// whole snapshot already goes out together on `Changed`.
    pub queue_entries: Vec<(String, String, u64)>,
```

And in the manual `Default` impl, after `current_index: -1,`:

```rust
            queue_entries: Vec::new(),
```

- [ ] **Step 2: Populate it in `refresh_bridge()`**

In `mtune/src/application.rs`, inside `refresh_bridge()`, right after the
existing `let queue = imp.player.queue();` line, add:

```rust
        let queue_entries: Vec<(String, String, u64)> = (0..queue.n_songs())
            .filter_map(|i| queue.song_at(i))
            .map(|s| (s.title(), s.artist(), s.duration()))
            .collect();
```

Then add `queue_entries,` to the `Snapshot { .. }` literal, right after
the existing `current_index: queue.current_song_index().map(|i| i as i64).unwrap_or(-1),` line.

- [ ] **Step 3: Expose the D-Bus property**

In `mtune/src/dbus.rs`, add next to `current_index`/`queue_length`:

```rust
    /// (title, artist, duration_secs) for every song in the queue, in
    /// queue order.
    #[zbus(property)]
    async fn queue_entries(&self) -> Vec<(String, String, u64)> {
        self.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .queue_entries
            .clone()
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

Expected: clean. No test asserts on `queue_entries` directly here — it's
a straight map over three already-tested `Song` accessors and the
already-tested `Queue::song_at`; nothing new to unit-test in isolation
(the shell side's Task 2/5 will exercise the shape end-to-end).

- [ ] **Step 5: Commit**

```bash
git add mtune/src/bridge.rs mtune/src/application.rs mtune/src/dbus.rs
git commit -m "$(cat <<'EOF'
feat(mtune): expose the queue's song list on org.margo.Tune

QueueLength/CurrentIndex already told a consumer how big the queue
is and where playback sits in it, but never what's actually in it --
the shell's Tune menu had no way to render song names. New
QueueEntries: Vec<(String, String, u64)> (title, artist,
duration_secs) rebuilt from Queue::song_at() on every refresh.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `mshell-services` mirrors `QueueEntries` + `remove_index` proxy

**Files:**
- Modify: `mshell-crates/mshell-services/src/mtune.rs` (`MtunePlayer` struct
  ~L30-58, constructor ~L61-83, `refresh()` ~L285-346, method list ~L111-155)

**Interfaces:**
- Consumes: `org.margo.Tune::QueueEntries` (Task 1),
  `org.margo.Tune::RemoveIndex(u32)` method (already exists, unused by
  the shell until now).
- Produces: `MtunePlayer.queue_entries: Property<Vec<(String, String, u64)>>`;
  `MtunePlayer::remove_index(&self, index: u32)`. Task 5 consumes both by
  name — do not rename.

- [ ] **Step 1: Add the property field**

Add to the `MtunePlayer` struct, after `pub playlists: Property<Vec<String>>,`:

```rust
    /// (title, artist, duration_secs) per queue entry, in queue order.
    pub queue_entries: Property<Vec<(String, String, u64)>>,
```

In `MtunePlayer::new()`, after `playlists: Property::new(Vec::new()),`:

```rust
            queue_entries: Property::new(Vec::new()),
```

- [ ] **Step 2: Read it in `refresh()`**

In `refresh()`, after the existing `if let Some(v) = get!("Playlists", Vec<String>) { p.playlists.set(v); }` block:

```rust
    if let Some(v) = get!("QueueEntries", Vec<(String, String, u64)>) {
        p.queue_entries.set(v);
    }
```

- [ ] **Step 3: Add the `remove_index` proxy method**

Next to the existing `pub async fn play_index(&self, index: u32) { ... }`:

```rust
    pub async fn remove_index(&self, index: u32) {
        self.call("RemoveIndex", &(index,)).await;
    }
```

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mshell-services
cargo clippy -p mshell-services --all-targets
cargo +1.95.0 fmt -p mshell-services -- --check
cargo test -p mshell-services
```

Expected: clean, same reasoning as Task 1 Step 4 — this is a direct
property mirror + a one-line D-Bus call, both following the exact
pattern every other field/method in this file already uses.

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-services/src/mtune.rs
git commit -m "$(cat <<'EOF'
feat(mshell-services): mirror mtune's QueueEntries + add remove_index

Same Property<T> mirror shape as every other org.margo.Tune field.
remove_index() was already a D-Bus method and an mtune-side
AppCommand (used internally for the queue-numbers work); only the
shell-side proxy caller was missing.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Sizing — config default + outer-scroller cap exception

**Files:**
- Modify: `mshell-crates/mshell-config/src/schema/config.rs` (`default_mtune_menu()`)
- Modify: `mshell-crates/mshell-frame/src/menus/menu.rs` (`MenuType::Mtune` arm ~L607-614)

**Interfaces:** none new — this only changes numbers/wiring, no new types.

- [ ] **Step 1: Bump the default panel size**

In `default_mtune_menu()`:

```rust
fn default_mtune_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Mtune],
        minimum_width: 500,
        maximum_height: 760,
    }
}
```

(Still user-tunable from Settings → Widgets → Tune — same mechanism
just fixed for the Lyrics menu.)

- [ ] **Step 2: Drop the outer cap, drop the quick-settings-menu class**

Replace the `MenuType::Mtune` arm:

```rust
            MenuType::Mtune => {
                css_class = "mtune-menu".to_string();
                effect_widgets!(effects, base_config, sender, mtune_menu);
                effect_min_width!(effects, base_config, sender, mtune_menu);
                // NOTE: like the clipboard, notifications, and lyrics
                // menus, this does NOT cap its *outer* scroller at
                // `maximum_height`. The mtune widget applies that cap to
                // its own inner queue scroller instead (see
                // mtune.rs), so the header + hero + seek + controls
                // stay fixed while only the queue scrolls.
            }
```

`"quick-settings-menu"` is the compact card-stack/dashboard-tile family
(§5/§7); this menu is moving to the §12 panel archetype (Clipboard's
family), which never carries that class either.

- [ ] **Step 3: Build + gates**

```bash
cargo build -p mshell-config -p mshell-frame
cargo clippy -p mshell-config -p mshell-frame --all-targets
cargo +1.95.0 fmt -p mshell-config -p mshell-frame -- --check
```

Expected: this will very likely *not* compile cleanly in isolation from
Task 4/5's CSS/widget changes purely visually (the widget still expects
the old `quick-settings-menu` card look until Task 4/6 land) — but it
must still *build* (Rust-level), since nothing here changes any type or
signature. If it doesn't build, stop and re-check the match arm syntax
before proceeding; do not fix by reverting this task's css_class change.

- [ ] **Step 4: Commit**

```bash
git add mshell-crates/mshell-config/src/schema/config.rs mshell-crates/mshell-frame/src/menus/menu.rs
git commit -m "$(cat <<'EOF'
feat(mshell): move the Tune menu to the panel archetype's sizing model

Default size grows to 500x760 (still Settings-tunable) and the outer
scroller no longer caps at maximum_height -- that moves to the
queue's own inner scroller in the next commit, matching the
clipboard/notifications/lyrics menus. Drops the quick-settings-menu
card-stack class; a panel-archetype surface doesn't carry it.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Panel header + hero + consolidated control row

Pure restyle/restructure — no new data, no new D-Bus reads. Makes the
existing sections (now-playing, seek, transport, shuffle/repeat, speed)
read as a §12 panel instead of a compact card. The Library/Playlists
sections and the footer button are untouched here (Task 5 replaces the
footer button when the header's actions land).

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs`
  (`view!` macro's hero block ~L96-147, transport/toggles/speed blocks
  ~L149-256)

**Interfaces:** none new.

- [ ] **Step 1: Replace the hero block's opening with a header, enlarge the cover**

Before the existing `// ── Now playing ──` box, insert:

```rust
            // ── Panel header (DESIGN.md §12) ────────────────────
            // Hand-rolled (like Clipboard) rather than the composed
            // MenuWidget::PanelHeader, since this is one monolithic
            // component, not a widget-list menu. Reuses the *generic*
            // panel-header classes (panel_header.rs / the Dashboard
            // header), not per-widget-prefixed ones.
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("org.margo.Tune-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_xalign: 0.0,
                    set_hexpand: true,
                    set_label: "Tune",
                },
                gtk::Label {
                    add_css_class: "panel-header-meta",
                    #[watch]
                    set_label: &model.queue_count_meta(),
                    #[watch]
                    set_visible: model.running,
                },
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "folder-open-symbolic",
                    set_tooltip_text: Some("Choose a music folder"),
                    connect_clicked => MtuneMenuInput::ChooseFolder,
                },
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_icon_name: if model.running { "go-next-symbolic" } else { "media-playback-start-symbolic" },
                    #[watch]
                    set_tooltip_text: Some(if model.running { "Open Tune window" } else { "Launch Tune" }),
                    connect_clicked[sender] => move |_| {
                        sender.input(if mtune_service().player.running.get() {
                            MtuneMenuInput::OpenTune
                        } else {
                            MtuneMenuInput::Launch
                        });
                    },
                },
            },
```

Add the helper referenced above, next to `now_playing_meta()`:

```rust
    /// "12 songs" for the header's trailing meta; empty when there's no
    /// queue yet (header hides it via `set_visible` in that case, but an
    /// empty string is the harmless fallback either way).
    fn queue_count_meta(&self) -> String {
        if self.queue_len == 0 {
            String::new()
        } else {
            format!("{} songs", self.queue_len)
        }
    }
```

Then bump the cover's pixel size in the existing hero box:

```rust
                gtk::Image {
                    add_css_class: "mtune-menu-cover",
                    set_pixel_size: 88,
                    set_valign: gtk::Align::Start,
                },
```

(was `60`).

- [ ] **Step 2: Delete the old footer button**

Remove the `// ── Footer ──` block entirely (its two actions now live in
the header, Step 1). `MtuneMenuInput::OpenTune` and `::Launch` keep their
existing `update_with_view` arms unchanged — only the button that sent
them moved.

- [ ] **Step 3: Consolidate transport + toggles + speed into one row**

Replace the three separate `// ── Transport ──`, `// ── Shuffle / repeat
──`, `// ── Speed ──` boxes with a single horizontal row:

```rust
            // ── Controls (transport + shuffle/repeat + speed) ────
            gtk::Box {
                add_css_class: "mtune-menu-controls",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                set_halign: gtk::Align::Fill,
                #[watch]
                set_sensitive: model.running,

                gtk::Box {
                    add_css_class: "mtune-menu-transport",
                    set_halign: gtk::Align::Start,
                    set_spacing: 8,

                    gtk::Button {
                        set_css_classes: &["mtune-round"],
                        set_icon_name: "media-skip-backward-symbolic",
                        set_tooltip_text: Some("Previous"),
                        #[watch]
                        set_sensitive: model.running && model.queue_len > 1,
                        connect_clicked => MtuneMenuInput::Previous,
                    },
                    #[name = "play_btn"]
                    gtk::Button {
                        set_css_classes: &["mtune-round", "mtune-round-primary"],
                        #[watch]
                        set_icon_name: if model.playing {
                            "media-playback-pause-symbolic"
                        } else {
                            "media-playback-start-symbolic"
                        },
                        set_tooltip_text: Some("Play / Pause"),
                        #[watch]
                        set_sensitive: model.running && (model.has_song || model.queue_len > 0),
                        connect_clicked => MtuneMenuInput::PlayPause,
                    },
                    gtk::Button {
                        set_css_classes: &["mtune-round"],
                        set_icon_name: "media-skip-forward-symbolic",
                        set_tooltip_text: Some("Next"),
                        #[watch]
                        set_sensitive: model.running && model.queue_len > 1,
                        connect_clicked => MtuneMenuInput::Next,
                    },
                },

                gtk::Box {
                    add_css_class: "mtune-menu-toggles",
                    set_hexpand: true,
                    set_halign: gtk::Align::End,
                    set_spacing: 6,

                    #[name = "shuffle_btn"]
                    gtk::ToggleButton {
                        set_css_classes: &["mtune-toggle"],
                        set_icon_name: "media-playlist-shuffle-symbolic",
                        set_tooltip_text: Some("Shuffle"),
                        #[watch]
                        #[block_signal(shuffle_toggled)]
                        set_active: model.shuffle,
                        connect_toggled[sender] => move |_| {
                            sender.input(MtuneMenuInput::ToggleShuffle);
                        } @shuffle_toggled,
                    },
                    #[name = "repeat_btn"]
                    gtk::Button {
                        set_css_classes: &["mtune-toggle"],
                        #[watch]
                        set_icon_name: match model.repeat.as_str() {
                            "repeat-one" => "media-playlist-repeat-song-symbolic",
                            "repeat-all" => "media-playlist-repeat-symbolic",
                            _ => "media-playlist-consecutive-symbolic",
                        },
                        #[watch]
                        set_tooltip_text: Some(match model.repeat.as_str() {
                            "repeat-one" => "Repeat: one",
                            "repeat-all" => "Repeat: all",
                            _ => "Repeat: off",
                        }),
                        connect_clicked => MtuneMenuInput::CycleRepeat,
                    },
                },

                #[name = "speed_row"]
                gtk::Box {
                    add_css_class: "mtune-menu-speed",
                    set_spacing: 4,
                },
            },
```

This drops the old "Speed" section label (the row is self-evident next
to transport) but keeps `speed_row`'s name and the existing
`RATE_PRESETS` population loop in `init()` untouched — it just appends
into a differently-laid-out parent now.

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mshell-frame
cargo clippy -p mshell-frame --all-targets
cargo +1.95.0 fmt -p mshell-frame -- --check
cargo test -p mshell-frame
```

Expected: builds clean (this is pure `view!` restructuring — no new
model fields yet, so no widget-name mismatches should surface beyond
what the diff above already accounts for). The panel will look
visually broken (no panel-header/panel-action-btn CSS exists yet —
that's Task 6) until the SCSS lands; that's expected mid-plan, same
pattern as the embedded-lyrics plan's intentional cross-task breakage.

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs
git commit -m "$(cat <<'EOF'
feat(mshell-frame): Tune menu gets a §12 panel header, bigger cover art

Header (Tune + song count + choose-folder / open-Tune actions)
replaces the old footer button. Transport, shuffle/repeat, and speed
collapse into one control row instead of three stacked blocks,
making room for the queue section landing in the next commit. Visual
styling (panel-header / panel-action-btn CSS) lands with it -- this
commit is layout-only and looks unstyled until then.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Queue section — browse, filter, play, remove

The core new feature. Adds queue data to the model, a client-side
filter, the scrollable row list, and the interactions.

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs`
  (model struct, `Input`/`Cmd` enums, `read()`, `apply_dynamic()`, `view!`,
  `update_with_view()`, `init()`'s property-watch list)

**Interfaces:**
- Consumes: `MtunePlayer.queue_entries: Property<Vec<(String, String, u64)>>`,
  `MtunePlayer::remove_index(&self, index: u32)` (Task 2).
- Produces: `fn queue_row_matches(title: &str, artist: &str, query: &str) -> bool`
  (pure, unit-tested) — the filter predicate, kept free-standing so it's
  testable without constructing a `MtuneMenuWidgetModel`.

- [ ] **Step 1: Write the failing test for the filter predicate**

Add near the bottom of `mtune.rs` (new module if none exists in this
file yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_on_title_or_artist_case_insensitively() {
        assert!(queue_row_matches("Get Lucky", "Daft Punk", "lucky"));
        assert!(queue_row_matches("Get Lucky", "Daft Punk", "DAFT"));
        assert!(!queue_row_matches("Get Lucky", "Daft Punk", "acoustic"));
    }

    #[test]
    fn blank_query_matches_everything() {
        assert!(queue_row_matches("Anything", "Anyone", ""));
        assert!(queue_row_matches("Anything", "Anyone", "   "));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mshell-frame queue_row_matches`
Expected: FAIL — `queue_row_matches` not defined.

- [ ] **Step 3: Implement the predicate**

Add near `make_line_label`-style small helpers (top-level free fn):

```rust
/// Whether `title`/`artist` should show under `query` (case-insensitive
/// substring on either field; a blank query matches everything).
fn queue_row_matches(title: &str, artist: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&q) || artist.to_lowercase().contains(&q)
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p mshell-frame queue_row_matches`
Expected: PASS.

- [ ] **Step 5: Add queue state to the model + read()**

Add to `MtuneMenuWidgetModel`, after `current_index: i64,`:

```rust
    /// (title, artist, duration_secs) per entry, queue order.
    queue_entries: Vec<(String, String, u64)>,
    /// Live text from the queue filter entry.
    queue_filter: String,
```

Initialize both in `init()`'s model literal (`queue_entries: Vec::new(),`
/ `queue_filter: String::new(),`) alongside the existing `queue_len: 0,`.

In `read()`, add `m.queue_entries = p.queue_entries.get();` alongside the
existing `m.queue_len = p.queue_len.get();` line. (`queue_filter` is
user input, not server state — `read()` never touches it.)

Add the property to `init()`'s watched-stream `Vec` (next to
`Box::pin(p.playlists.watch().map(|_| ())),`):

```rust
                Box::pin(p.queue_entries.watch().map(|_| ())),
```

- [ ] **Step 6: Add the queue view section**

Insert between the control row (Task 4) and the `// ── Library ──`
section:

```rust
            // ── Queue ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "mtune-queue-section",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                set_vexpand: true,

                gtk::Label {
                    add_css_class: "mtune-menu-section-label",
                    set_xalign: 0.0,
                    set_label: "Queue",
                },

                #[name = "queue_filter_entry"]
                gtk::SearchEntry {
                    add_css_class: "mtune-queue-filter",
                    set_placeholder_text: Some("Filter queue…"),
                    #[watch]
                    set_visible: model.queue_len > 0,
                    connect_search_changed[sender] => move |e| {
                        sender.input(MtuneMenuInput::FilterQueue(e.text().to_string()));
                    },
                },

                #[name = "queue_scroller"]
                gtk::ScrolledWindow {
                    add_css_class: "mtune-queue-scroller",
                    set_hexpand: true,
                    set_vexpand: true,
                    set_propagate_natural_height: true,
                    set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),
                    #[watch]
                    set_visible: model.queue_len > 0,

                    #[name = "queue_rows"]
                    gtk::Box {
                        add_css_class: "mtune-queue-rows",
                        set_orientation: gtk::Orientation::Vertical,
                    },
                },

                gtk::Label {
                    add_css_class: "mtune-menu-status",
                    set_xalign: 0.0,
                    set_label: "Queue is empty — choose a folder or open a playlist.",
                    #[watch]
                    set_visible: model.queue_len == 0,
                },
            },
```

- [ ] **Step 7: Add the `FilterQueue` / `PlayQueueIndex` / `RemoveQueueIndex` inputs**

Add to `MtuneMenuInput`:

```rust
    /// Queue filter text changed.
    FilterQueue(String),
    /// A queue row was clicked.
    PlayQueueIndex(u32),
    /// A queue row's remove (×) button was clicked.
    RemoveQueueIndex(u32),
```

Handle them in `update_with_view`'s `match message`:

```rust
            MtuneMenuInput::FilterQueue(text) => {
                self.queue_filter = text;
                rebuild_queue_rows(widgets, self, &sender);
            }
            MtuneMenuInput::PlayQueueIndex(i) => {
                tokio_rt_spawn(async move { mtune_service().player.play_index(i).await });
            }
            MtuneMenuInput::RemoveQueueIndex(i) => {
                tokio_rt_spawn(async move { mtune_service().player.remove_index(i).await });
            }
```

- [ ] **Step 8: Rebuild + auto-scroll the row list**

Add a new free fn (mirrors `apply_dynamic`'s playlist-row rebuild, plus
scroll-to-current):

```rust
/// Rebuild the queue row list from `m.queue_entries`, filtered by
/// `m.queue_filter`. Called after every refresh and every filter
/// keystroke — the list is small enough (a folder-first personal
/// library queue, not a virtualized thousand-row history) that a full
/// rebuild is simpler and cheap enough, same call shape as the
/// existing saved-playlist rows.
fn rebuild_queue_rows(
    widgets: &MtuneMenuWidgetModelWidgets,
    m: &MtuneMenuWidgetModel,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    while let Some(c) = widgets.queue_rows.first_child() {
        widgets.queue_rows.remove(&c);
    }

    let mut current_row: Option<gtk::Widget> = None;
    for (i, (title, artist, duration)) in m.queue_entries.iter().enumerate() {
        if !queue_row_matches(title, artist, &m.queue_filter) {
            continue;
        }
        let idx = i as u32;
        let is_current = m.current_index >= 0 && m.current_index as usize == i;

        let row = gtk::Box::builder()
            .css_classes(["mtune-queue-row"])
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        if is_current {
            row.add_css_class("mtune-queue-row-current");
        }

        let num = gtk::Label::new(Some(&(i + 1).to_string()));
        num.add_css_class("mtune-queue-row-num");

        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title_label = gtk::Label::new(Some(title));
        title_label.set_xalign(0.0);
        title_label.set_ellipsize(pango::EllipsizeMode::End);
        title_label.add_css_class("mtune-queue-row-title");
        let artist_label = gtk::Label::new(Some(artist));
        artist_label.set_xalign(0.0);
        artist_label.set_ellipsize(pango::EllipsizeMode::End);
        artist_label.add_css_class("mtune-queue-row-artist");
        text.append(&title_label);
        text.append(&artist_label);
        text.set_hexpand(true);

        let dur = gtk::Label::new(Some(&format_duration(Duration::from_secs(*duration))));
        dur.add_css_class("mtune-queue-row-duration");

        let remove_btn = gtk::Button::builder()
            .css_classes(["mtune-queue-row-remove"])
            .icon_name("window-close-symbolic")
            .tooltip_text("Remove from queue")
            .valign(gtk::Align::Center)
            .build();
        let s = sender.clone();
        remove_btn.connect_clicked(move |_| s.input(MtuneMenuInput::RemoveQueueIndex(idx)));

        let click = gtk::GestureClick::new();
        let s = sender.clone();
        click.connect_released(move |_, _, _, _| s.input(MtuneMenuInput::PlayQueueIndex(idx)));
        row.add_controller(click);

        row.append(&num);
        row.append(&text);
        row.append(&dur);
        row.append(&remove_btn);
        widgets.queue_rows.append(&row);

        if is_current {
            current_row = Some(row.upcast::<gtk::Widget>());
        }
    }

    if let Some(row) = current_row {
        scroll_queue_to(&widgets.queue_scroller, &row);
    }
}

/// Smoothly centre `row` in `scroller`. Deferred to idle so its geometry
/// is laid out (freshly appended on a rebuild) — same technique as the
/// Lyrics menu's `scroll_center`.
fn scroll_queue_to(scroller: &gtk::ScrolledWindow, row: &gtk::Widget) {
    let scroller = scroller.clone();
    let row = row.clone();
    gtk::glib::idle_add_local_once(move || {
        let Some(parent) = row.parent() else { return };
        let Some(bounds) = row.compute_bounds(&parent) else {
            return;
        };
        if bounds.height() == 0.0 {
            return;
        }
        let vadj = scroller.vadjustment();
        let center = bounds.y() as f64 + bounds.height() as f64 / 2.0;
        let target = center - vadj.page_size() / 2.0;
        let max = (vadj.upper() - vadj.page_size()).max(0.0);
        vadj.set_value(target.clamp(0.0, max));
    });
}
```

Call `rebuild_queue_rows(widgets, self, &sender);` from
`update_cmd_with_view`'s `MtuneMenuCmd::Refresh` arm (after the existing
`read(self);` call, before `apply_dynamic`), and once from `init()` right
after `apply_dynamic(&widgets, &model, &sender);`.

- [ ] **Step 9: Cap the queue's inner scroller (imperative, not `#[watch]`)**

Reusing the reasoning from the Lyrics-menu height fix: a static
`#[watch] set_max_content_height` alone is fine here (only *one* bound
is driven, no min/max pin needed — Task 3 made this a pure cap, not a
pin), so it's safe as a plain `#[watch]` on `queue_scroller` — add:

```rust
                    #[watch]
                    set_max_content_height: {
                        let h = mshell_config::config_manager::config_manager()
                            .config()
                            .menus()
                            .mtune_menu()
                            .maximum_height()
                            .get();
                        if h > 0 { h } else { -1 }
                    },
```

to the `queue_scroller` block from Step 6. This needs
`use mshell_config::config_manager::config_manager;` and
`use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};`
added to the top of the file (same imports Task 3's sibling menus
already use). A single `#[watch] set_max_content_height` with no paired
min bound never asserts regardless of grow/shrink direction — the
Lyrics-menu bug was specifically about *pinning* both bounds together;
this is a cap only, matching Clipboard/Notifications' existing
`#[watch]`-only pattern exactly.

- [ ] **Step 10: Build + gates**

```bash
cargo build -p mshell-frame
cargo clippy -p mshell-frame --all-targets
cargo +1.95.0 fmt -p mshell-frame -- --check
cargo test -p mshell-frame
./scripts/panic-ratchet.sh
./scripts/design-lint.sh
```

Expected: all clean, `queue_row_matches` tests passing.

- [ ] **Step 11: Commit**

```bash
git add mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs
git commit -m "$(cat <<'EOF'
feat(mshell-frame): Tune menu gains a browsable, filterable queue

Renders every queue entry (track #, title, artist, duration), tints
the currently-playing row, and auto-scrolls to it on refresh. Click
a row to play it (PlayIndex), click x to remove it (RemoveIndex) --
both were already server-side operations, only the UI was missing.
A SearchEntry filters rows client-side (title or artist substring,
case-insensitive) over the already-fetched queue_entries, no extra
D-Bus traffic per keystroke. The scroller caps at
menus.mtune_menu.maximum_height like the outer panel used to.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: SCSS — panel root, header, control row, queue rows

**Files:**
- Modify: `mshell-crates/mshell-style/scss/04-components/_mtune.scss`

**Interfaces:** none (pure styling).

- [ ] **Step 1: Convert the root to a panel surface**

Replace:

```scss
.mtune-menu-widget {
  border-radius: var(--card-radius);
  background-color: var(--card-bg);
  color: var(--on-surface);
  padding: var(--space-3) var(--space-4);
}
```

with:

```scss
// §12 panel root: one calm tonal sheet, no separate "card" background
// (matches .clipboard-menu-widget — the compositor-level surface tint
// shows through; only rows/sections below get tonal depth).
.mtune-menu-widget {
  color: var(--on-surface);
  padding: var(--padding-xl);
}
```

- [ ] **Step 2: Header + control row**

Add (the generic `.panel-header` / `.panel-header-icon` / `.panel-title`
/ `.panel-header-meta` / `.panel-action-btn` classes are already styled
by `panel_header.rs`'s SCSS partner rules — nothing new needed for
those. This just adds the control-row layout):

```scss
// ── Control row (transport + toggles + speed, one line) ───────
.mtune-menu-controls {
  align-items: center;
}
```

- [ ] **Step 3: Queue section**

```scss
// ── Queue ───────────────────────────────────────────────────
.mtune-queue-filter {
  border-radius: var(--radius-pill);
}

.mtune-queue-scroller {
  border-radius: var(--list-radius);
  background-color: var(--surface-container);
}

.mtune-queue-rows {
  padding: var(--space-1) 0;
}

.mtune-queue-row {
  min-height: 40px;
  padding: 0 var(--space-3);
  align-items: center;
  @include state-layer(var(--surface-container));

  &:not(:last-child) {
    border-bottom: 1px solid var(--list-separator);
  }
}

.mtune-queue-row-current {
  background-color: var(--primary-container);

  .mtune-queue-row-title,
  .mtune-queue-row-num {
    color: var(--on-primary-container);
  }
}

.mtune-queue-row-num {
  min-width: 22px;
  color: var(--on-surface-variant);
  font-size: var(--font-2xs);
  font-variant-numeric: tabular-nums;
}

.mtune-queue-row-title {
  font-weight: 500;
  color: var(--on-surface);
}

.mtune-queue-row-artist {
  color: var(--on-surface-variant);
  font-size: var(--font-2xs);
}

.mtune-queue-row-duration {
  color: var(--on-surface-variant);
  font-size: var(--font-2xs);
  font-variant-numeric: tabular-nums;
}

.mtune-queue-row-remove {
  all: unset;
  min-width: 24px;
  min-height: 24px;
  border-radius: var(--radius-pill);
  color: var(--on-surface-variant);
  opacity: 0;
  transition: opacity var(--motion-fast) var(--ease-standard);
}

.mtune-queue-row:hover .mtune-queue-row-remove {
  opacity: 1;
}
```

Check `@include state-layer` and every token above (`--list-radius`,
`--list-separator`, `--primary-container`, `--on-primary-container`,
`--font-2xs`, `--padding-xl`, `--radius-pill`) already exist in
`02-functions`/`01-tokens` before using them verbatim — every one of
them is already used elsewhere in this same file or in
`_clipboard.scss`, so this should be a non-issue, but confirm via
`grep` rather than assuming if the build fails.

- [ ] **Step 4: Build + gates**

```bash
cargo build -p mshell-style
cargo build -p mshell-frame
./scripts/design-lint.sh
```

Expected: `mshell-style`'s `build.rs` fails loudly on any unknown SCSS
variable/function — a clean build here means every token above resolved.
`design-lint.sh` re-checks the "tokens only, no hardcoded hex" rule.

- [ ] **Step 5: Commit**

```bash
git add mshell-crates/mshell-style/scss/04-components/_mtune.scss
git commit -m "$(cat <<'EOF'
style(mshell-style): Tune menu panel styling — header, queue rows

Root drops its own card background (panel surfaces read as one tonal
sheet, matching Clipboard) in favour of --padding-xl per §12. Queue
rows get tonal depth (--surface-container), a --primary-container
tint for the currently-playing row, and a hover-revealed remove
button.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Docs + push

**Files:**
- Modify: `docs/widgets.md` (the `Mtune` / `Tune` row)

**Interfaces:** none.

- [ ] **Step 1: Update the widgets table**

Find the `Mtune` row in the `## Media` table (mentions "Left-click opens
the Tune menu (seek bar, transport, shuffle / repeat, speed, folder +
playlist controls)") and extend it to mention the queue browser, e.g.
append "; the menu also shows the live queue with a filter box, and can
play or remove any entry directly." to the existing sentence.

- [ ] **Step 2: Commit**

```bash
git add docs/widgets.md
git commit -m "$(cat <<'EOF'
docs(widgets): note the Tune menu's queue browser

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
- Open the Tune menu → panel reads calm/flat (no floating card edge),
  header shows "Tune", song count, folder + open/launch actions.
- Play a folder with several tracks → queue list appears, current track
  tinted, auto-scrolled into view.
- Type in the filter box → list narrows to matching title/artist;
  clearing the filter restores the full list.
- Click a queue row → that track plays (`PlayIndex`).
- Hover a row, click × → it's removed from the queue (`RemoveIndex`),
  list re-renders without it.
- Shrink/grow Settings → Widgets → Tune → Maximum Height → only the
  queue's own scroller responds; header/hero/seek/controls never move.
- Switch to a different player (Spotify/MPD) → unaffected (this menu
  only ever shows when Tune is the source, per existing wiring).

---

## Self-Review

**1. Spec coverage:** §3 (QueueEntries) → Task 1/2. §4.1 header → Task 4.
§4.4 control row → Task 4. §4.5 queue (filter, rows, tint, click, remove,
autoscroll, empty state, cap-not-pin) → Task 5. §4.6 library/playlists
"restyled, behaviour unchanged" → Task 6's SCSS covers the tonal
depth; no Rust changes needed there since their behaviour was never in
scope to change. §5 sizing → Task 3. Non-goals (§6) — no task
introduces drag-reorder or per-row art; confirmed absent from every task
above.

**2. Placeholder scan:** no TBD/TODO; every step has literal code.

**3. Type consistency:** `Vec<(String, String, u64)>` is the one shape
used end-to-end — `Snapshot.queue_entries` (Task 1) →
`org.margo.Tune::QueueEntries` (Task 1) → `MtunePlayer.queue_entries`
(Task 2) → `MtuneMenuWidgetModel.queue_entries` (Task 5) →
`queue_row_matches(title: &str, artist: &str, query: &str) -> bool`
(Task 5, consumes the tuple's first two fields by reference) →
`rebuild_queue_rows` (Task 5, consumes the tuple by destructuring). No
task introduces a competing shape.
