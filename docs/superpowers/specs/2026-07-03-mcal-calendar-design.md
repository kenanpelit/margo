# mcal — calendar for margo (design)

Status: **approved to plan** (user delegated the open decisions: "sen karar ver haydi", 2026-07-03)
Port source: `~/.kod/dankcalendar` (danklinux/DankCalendar — Go core + Quickshell QML)

## 1. Goal

Bring real calendar **events** into margo. Today margo's clock pill opens a
calendar menu (`mshell-frame/.../menus/menu_widgets/calendar.rs`) that is a
hero card + a bare `gtk::Calendar` month grid — **no events, no accounts, no
sync**. mcal grows that surface into an event-aware calendar backed by a
GTK-free calendar core.

dankcalendar is ~24.6k lines of hand-written Go (+40k generated Ent ORM) plus
71 QML files (~18.9k lines): providers (local, ical, evolution) + OAuth
(Google/Microsoft) + CalDAV/iCloud + sync engine + reminder daemon + HTTP API
+ keyring + RFC 5545 recurrence + tray. **This spec is NOT a 1:1 port.** It is
the first, self-contained slice that delivers a working event calendar without
the daemon/OAuth/tray machinery.

## 2. Scope

### In (slice 1)
- **`mcal`** — new GTK-free crate: domain types, ICS parsing, RRULE
  recurrence expansion, local + remote-ICS providers, a reactive event store.
- **Local provider** — read `~/.config/margo/calendars/*.ics` and directories
  of `.ics` files (mirrors dankcalendar's `internal/providers/local`).
- **Remote ICS provider** — fetch subscription URLs (e.g. a Google Calendar
  "secret iCal address", a shared `.ics`) over `ureq`, disk-cached (mirrors
  `internal/providers/ical`). This is what makes the calendar non-empty on a
  real machine without OAuth.
- **UI** — extend `calendar.rs`: mark days that have events on the month grid,
  and render a per-day **agenda list** (title · time · location) below it.
- **Config** — a `calendars` block in the shell profile (local dir + list of
  subscription URLs + refresh interval).

### Out (later slices / separate specs)
Google/Microsoft OAuth · CalDAV/iCloud · keyring · background sync daemon ·
HTTP API · system tray · tasks / VTODO · RSVP / invitations · event
**create/edit/delete** (write-back) · desktop reminder notifications. The last
two (write-back, reminders) are the natural slice-2 candidates.

## 3. Architecture

```
                    ~/.config/margo/calendars/*.ics   (local files/dirs)
                    subscription URLs (config)         (remote .ics over ureq)
                              │
                              ▼
   ┌──────────────────────────────────────────────┐
   │ mcal   (top-level crate, GTK-free)       │
   │  model::{Event, Calendar, Account, Attendee}  │  ← ported from models/*.go
   │  ics::parse   (icalendar crate)               │
   │  recur::expand (rrule crate)                  │
   │  provider::{Local, RemoteIcs} : Provider      │  ← trait, dankcalendar shape
   │  store::CalendarStore (reactive_graph)        │  ← RwSignal<Vec<Event>>
   └──────────────────────────────────────────────┘
                              │ reactive read
                              ▼
   ┌──────────────────────────────────────────────┐
   │ mshell-frame  calendar.rs  (GTK / relm4)      │
   │  hero + gtk::Calendar (existing)              │
   │  + day marks (mark_day)                       │
   │  + agenda list (selected day's events)        │
   └──────────────────────────────────────────────┘
```

**Why the core is GTK-free** (the decision the user delegated): testability
(`cargo test --workspace` — the CI gate — exercises ICS/RRULE headlessly),
reuse (a future `mcal` CLI or daemon links the core without GTK), and margo's
existing non-GTK-core / GTK-UI split. The reactive glue is `reactive_graph`
(already a workspace dep, used across mshell) — not GTK — so the GTK UI
subscribes to it the same way other menu widgets subscribe to services.

### 3.1 Crate placement & naming
- `mcal/` — **top-level** crate (peer to `mctl`, `mvpn`, `mdots`), keeping the
  `mshell-crates/mshell-*` prefix consistent. It is a **library** in slice 1;
  the GTK-free-ness is an internal property, not part of the name (no `-core`
  suffix). A future `mcal` CLI is added as a `[[bin]]` in this same crate.
- Workspace registration: add `"mcal"` to `members`, and
  `mcal = { path = "mcal" }` to `[workspace.dependencies]`.

### 3.2 New third-party deps (add to `[workspace.dependencies]`)
- `icalendar` — RFC 5545 parse/serialize (replaces hand-porting the Go ICS code).
- `rrule` — RRULE recurrence expansion over a date window (replaces
  `internal/recurrence`). Pulls `chrono` + `chrono-tz`.

Both use `chrono`, so **`mcal` uses `chrono`** for its time types.
`calendar.rs` currently uses the `time` crate; it converts chrono → `time` /
raw fields at the UI boundary (small, localized).

## 4. mcal internals

### 4.1 Domain model (`model.rs`) — ported from `models/event.go`
```rust
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub uid: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub url: Option<String>,
    pub status: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub recurrence: Vec<String>,   // raw RRULE/RDATE/EXDATE lines
    pub attendees: Vec<Attendee>,
    pub categories: Vec<String>,
}
pub struct Attendee { pub email: String, pub display_name: Option<String>, pub status: Option<String> }
pub struct Calendar { pub account_id: String, pub remote_id: String, pub name: String, pub color: Option<String> }
pub struct Account  { pub id: String, pub kind: AccountKind, pub name: String }
pub enum AccountKind { Local, RemoteIcs }
```
Recurrence-write (`EventCreate`/`EventUpdate`) is **out of scope** for slice 1
(read-only), so those Go structs are not ported yet.

### 4.2 ICS parsing (`ics.rs`)
`fn parse_ics(text: &str, calendar_id: &str) -> Result<Vec<Event>, McalError>`
using `icalendar`. Maps `VEVENT` → `Event`: `SUMMARY`, `DTSTART`/`DTEND`
(honouring `VALUE=DATE` → `all_day`, and `TZID`), `LOCATION`, `URL`, `STATUS`,
`UID`, `RRULE`/`RDATE`/`EXDATE` (kept raw for the recur pass), `ATTENDEE`,
`CATEGORIES`. `VTODO` is skipped in slice 1.

### 4.3 Recurrence (`recur.rs`)
`fn expand(event: &Event, window: (DateTime<Utc>, DateTime<Utc>)) -> Vec<Event>`
— non-recurring events pass through; recurring events are expanded via `rrule`
into concrete instances clamped to the query window, each carrying the master's
`uid` (so the UI can dedupe/trace). This bounds unbounded RRULEs.

### 4.4 Provider trait (`provider/mod.rs`) — dankcalendar shape, trimmed
```rust
#[async_trait]
pub trait Provider {
    fn kind(&self) -> AccountKind;
    async fn list_calendars(&self) -> Result<Vec<Calendar>, McalError>;
    async fn list_events(&self, cal: &Calendar, window: (DateTime<Utc>, DateTime<Utc>))
        -> Result<Vec<Event>, McalError>;
}
```
- `provider/local.rs` — read `.ics` files/dirs under the configured root
  (create the root if missing, like the Go local provider).
- `provider/remote_ics.rs` — `ureq` GET each subscription URL, cache the body
  under `mshell-cache` (or `~/.cache/margo/mcal/`), parse, expand. Network runs
  off the GTK thread (spawned), results pushed into the store.

### 4.5 Reactive store (`store.rs`)
A process-global `CalendarStore` (init-once, like `mshell-services`
singletons) holding `RwSignal<Vec<Event>>` + `RwSignal<Vec<Calendar>>` +
load/refresh state. `refresh()` reloads local (cheap, sync) and kicks remote
fetches (async). The UI reads a derived "events for day D" via `reactive_graph`.

### 4.6 Config (`mshell-config`)
```yaml
calendars:
  local_dir: ~/.config/margo/calendars   # default
  subscriptions:
    - name: Work
      url: https://calendar.google.com/…/basic.ics
      color: "#4285F4"
  refresh_secs: 900
```
serde defaults; `skip_serializing_if` per the profile-rebake trap
(`reference_profile_rebake_serde_default`).

## 5. UI changes (`calendar.rs`)

Keep the hero + grid. Add:
1. **Day marks** — after (re)load, call `gtk::Calendar::mark_day(d)` for days in
   the visible month that have ≥1 event; `clear_marks()` on month change.
   Subscribe to the store so marks repaint when a remote fetch lands.
2. **Agenda list** — a scrolling `GtkListBox`/`gtk::Box` under the grid showing
   the selected day's events (time range · summary · location; all-day pinned
   top). `connect_day_selected` → filter store by that date.
   Empty state: "No events." Respect `GtkScrolledWindow` min-content-height
   (`reference_gtk_scroller_collapse`).
3. **Lazy load** — reuse the existing `ParentRevealChanged` gate: trigger the
   first `store.refresh()` on first reveal, not at login
   (`project_menu_lazy_polling`, `project_startup_main_thread_burn`).

No new menu, no new pill in slice 1 — the clock/calendar menu is the surface.

## 6. Optional sub-slices (same spec, gated by time)
- **1b — Settings page**: Settings → Calendar to edit `local_dir` +
  subscription list (follow `mshell-settings` conventions; inline entry rows).
- **1c — `mcal` CLI**: top-level `mcal` binary (`mcal today`, `mcal agenda
  [--days N]`) printing the agenda from `mcal`. Satisfies the `m*`-CLI
  convention (`mctl`/`mvpn`) without a daemon.

## 7. Testing
`mcal` ships unit tests (the point of the GTK-free split):
- `ics.rs`: fixture `.ics` (all-day, timed, TZID, multi-VEVENT) → asserted `Event`s.
- `recur.rs`: a weekly RRULE expanded over a 30-day window → N instances, EXDATE honoured.
- `provider/local.rs`: temp dir with `.ics` files + a subdir → calendars + events.
- `store.rs`: refresh merges providers; day-filter derivation.
All headless → run in `cargo test --workspace` (CI gate).

## 8. CI / conventions
Must pass `just check`: fmt, clippy `--all-targets -D warnings`, panic-ratchet
(no new `unwrap/expect/panic` in `mcal` — use `thiserror` `McalError` +
`Result`), design-lint, `mctl check-config`, `cargo test`. New crate gets
`[lints] workspace = true`. Adheres to `docs/config-conventions.md` for the
config knob and `DESIGN.md` for any agenda-row styling.

## 9. Risks / open questions
- **Timezones**: `icalendar` + `rrule` + `chrono-tz` must agree on `TZID` →
  offset. Covered by a TZID fixture test; store everything as `Utc`, render in
  local.
- **`chrono` vs `time`**: core is `chrono`, existing UI is `time`. Convert at
  the boundary; do not migrate `calendar.rs` off `time` in this slice.
- **Empty-machine UX**: with no local files and no subscriptions the calendar
  looks unchanged (no events) — acceptable; the Settings page (1b) is how the
  user adds a subscription.
- **Remote fetch failure**: surface as a quiet non-fatal state (stale cache +
  no crash); never block the menu.

## 10. Slice order (for the plan)
1. `mcal` crate skeleton + workspace registration + `model.rs`.
2. `ics.rs` + tests. 3. `recur.rs` + tests. 4. `provider/local.rs` + tests.
5. `store.rs` + config schema. 6. `provider/remote_ics.rs` (async fetch/cache).
7. `calendar.rs` UI: marks + agenda + lazy refresh.
8. (1b) Settings page. 9. (1c) `mcal` CLI.
Ship-gate after step 7 (a working local+remote event calendar in the menu).
