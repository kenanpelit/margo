# Tune bar-pill menu redesign — design spec

**Status:** approved 2026-09-05 (in chat; no separate review round — scope
was fixed via two rounds of structured questions during brainstorming).

## 1. Problem

The `MtuneMenuWidgetModel` panel (`mshell-crates/mshell-frame/src/menus/menu_widgets/mtune/mtune.rs`)
covers now-playing, seek, transport, shuffle/repeat, speed, library-root
picking, and playlists — but it has no view into the *queue* mtune is
actually playing. mtune's own window is queue-centric (folder-first
playback, a live "up next" list); the shell's quick panel can't show or
act on that at all today. The user asked for a menu "as capable as Tune's
own interface," without losing anything the panel already does.

## 2. Decisions (from brainstorming)

1. **Add a real queue browser** — title + artist + duration per entry,
   current track highlighted, click-to-play, remove-from-queue, and a
   filter box (search-as-you-type over title + artist).
2. **Promote the panel to the §12 "panel" archetype** (DESIGN.md) — same
   family as Clipboard History: its own header, generous padding, tonal
   depth for rows — instead of the current compact `quick-settings-menu`
   card. Anchors the extra content without the panel feeling cramped.
3. Every existing feature (cover/title/artist, seek, transport,
   shuffle/repeat, speed, library root + rescan, playlists, launch/open
   Tune) is kept — reorganized and restyled, nothing dropped.

## 3. New cross-process surface

`org.margo.Tune` currently exposes `QueueLength` (count) and
`CurrentIndex`, but never the queue's actual contents — the shell has no
way to render song names. New read-only property:

```
QueueEntries: Vec<(String, String, u64)>   // (title, artist, duration_secs)
```

Populated in mtune's `refresh_bridge()` by enumerating
`queue.song_at(i)` for `i in 0..queue.n_songs()` (the same `Queue`
already read there for `queue_len`/`current_index`) and mapping each
`Song` through its existing `title()`/`artist()`/`duration()`
accessors — no new Song API needed. Delivered whole on every `Changed`
(same cadence as every other mtune property; not fired on the 1 Hz
position tick), mirrored into `mshell-services`'s
`MtunePlayer.queue_entries: Property<Vec<(String, String, u64)>>` the
same way every other property already is.

`MtunePlayer` also gains a `remove_index(&self, index: u32)` proxy
method (the D-Bus method and `AppCommand::RemoveIndex` already exist on
the mtune side — Task 5 of the queue-numbers work earlier this cycle
used `RemoveIndex` internally; only the shell-side caller was missing).

## 4. Panel layout (top to bottom)

1. **Header** (§12) — hand-rolled (the widget is one monolithic
   component, same shape as Clipboard, which also hand-rolls its
   header rather than composing the separate `MenuWidget::PanelHeader`).
   Reuses the *generic* `.panel-header` / `.panel-header-icon` /
   `.panel-title` / `.panel-header-meta` / `.panel-action-btn` classes
   already shipped for the Dashboard header (`panel_header.rs`) — not
   Clipboard's own `.clipboard-*`-prefixed set, since those predate the
   generic classes and aren't shared. Leading note glyph, "Tune" title,
   "N songs" as the quiet trailing meta, two circular actions: choose
   folder, open/launch Tune (replaces the current footer button).
2. **Hero** — cover art enlarged (60px → 88px) to read at panel scale;
   title/artist/album unchanged in substance.
3. **Seek bar** — unchanged.
4. **Control row** — transport + shuffle/repeat + speed consolidated
   into one row instead of three stacked blocks (less vertical chrome
   now that the queue needs the room).
5. **Queue (new)** — the main scroll area:
   - A `--radius-pill` filter entry (§12 "Panel search") above the list;
     typing filters rows by substring match on title *or* artist
     (case-insensitive), client-side, over the already-fetched
     `queue_entries` — no new D-Bus call per keystroke.
   - One row per (filtered) entry: track number, title, artist,
     `mm:ss` duration; the row at `current_index` gets the `--primary`
     tint (same convention as every other "this one is active" surface
     in the app, §3). Click anywhere on a row → `PlayIndex`. Trailing
     small remove (×) button → `RemoveIndex`.
   - Auto-scrolls to the current row when the track changes or the
     panel is revealed (mirrors the Lyrics menu's `scroll_center`).
   - Height is a *cap*, not a pin (§ below) — a short queue does not
     force the panel tall; a long one scrolls within the configured
     max instead of growing the whole panel. This is the Clipboard /
     Notifications pattern, not the Lyrics-menu "pin to target size"
     one from the previous fix — the queue is one section among
     several fixed-height ones above it, not the entire panel's content.
   - Empty state ("Queue is empty — choose a folder or open a
     playlist") per §17.
6. **Library** and **Playlists** — unchanged behaviour, restyled to the
   panel's tonal row language (`--surface-container` / `-high`).

## 5. Sizing

`menus.mtune_menu` defaults move from `(minimum_width: 380,
maximum_height: 0)` to `(500, 760)` — both still user-tunable from
Settings → Widgets → Tune, unchanged mechanism. `maximum_height` caps
the **queue's inner scroller** only; `menu.rs`'s outer
`effect_max_height!` is dropped for `MenuType::Mtune` (added to the
Clipboard/Notifications "cap the inner list, not the outer scroller"
exception, with the same NOTE comment convention), so header + hero +
seek + controls stay put and only the queue scrolls — exactly the
mechanism already proven for Clipboard and Notifications (and, in its
"pin" variant, just fixed for Lyrics).

## 6. Non-goals

- No drag-to-reorder (mtune's own window doesn't offer this either as
  far as this menu is concerned — out of scope, not a regression).
- No per-row cover art thumbnails (queue rows are text; the single
  hero cover above already carries the current track's art).
- No changes to `org.margo.Tune`'s existing properties/methods beyond
  the one addition in §3.
