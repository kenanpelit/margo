# In-app usage & settings guide (Settings → Guide)

## Problem

margo's user-facing documentation lives in three places a user never
opens from inside the desktop itself: `docs/features.md` (the site),
`docs/configuration.md` / `config-reference.md` (rendered from
`margo/src/config.example.conf`), and `mctl actions --verbose` (a
terminal command). Discovering "what can I bind Super+Tab to" or "what
does `mru_scope` do" means leaving the shell for a browser or a
terminal. There is no in-shell, clickable, searchable way to browse
what margo can do.

## Goal

A new Settings page — **Guide** — at the very bottom of the sidebar
(right after **About**), with four tabs:

1. **Actions** — every dispatch action (`mctl actions`'s catalogue),
   grouped, searchable, click to expand a summary + example.
2. **Settings** — every `config.conf` key from
   `margo/src/config.example.conf`, grouped by its `# ── Section ──`
   divider, searchable, click to expand its comment/example.
3. **Features** — a short curated tour, sourced from `docs/features.md`.
4. **Tools** — the companion binaries' own `--help` output
   (`mlock --help`, `mvpn --help`, `mcal --help`, …), one per tool,
   selected from a side list.

All four tabs derive their content from existing single-source-of-truth
data (Rust structs, the shipped example config, the site's own
markdown) — nothing is hand-copied into the new page, so the guide
can't drift the way `config-reference.md`'s stale "15 layouts" claim
did before this session's earlier fix.

Delivered in three phases (each independently shippable and testable):

- **Phase 1** — Actions + Settings tabs (cheapest, highest value: both
  read structured data that already exists).
- **Phase 2** — Features tab (a small markdown-subset renderer).
- **Phase 3** — Tools tab (the only tab that shells out to a
  subprocess at runtime; most exposure to "the tool isn't installed /
  hangs / doesn't support `--help` cleanly").

## Non-goals

- Not a replacement for the website — the guide is a subset framed for
  "what can I click/bind today", not the full prose docs (roadmap,
  protocol comparisons, design docs stay web-only).
- Not editable from this page. It's a reference — the existing
  Keybinds / Window Rules / etc. settings pages remain the place you
  actually change something. (A "copy this bind line" affordance is a
  reasonable future add-on, out of scope for v1.)
- Not a markdown engine. The Features tab renders a **subset**
  (bold, links, unordered lists, paragraph breaks) via Pango markup,
  not a general CommonMark parser.

## Architecture

### Where it lives

New module `mshell-crates/mshell-settings/src/guide_settings.rs`,
registered exactly like every existing page:

- `mod guide_settings;` in `lib.rs`.
- `"guide" => page!(GuideSettingsModel => GuideSettingsInit {})` added
  to `build_top_page`'s match in `settings.rs`.
- One new `Page { route: "guide", icon: "help-faq-symbolic", label:
  "Guide" }` entry in the sidebar list, inserted **before** the
  existing `Section { name: "About", ... }` entry so Guide gets its
  own section header (reads better than nesting it under "About",
  since it isn't about the app, it's about margo itself) — a new
  `Section { name: "Guide", icon: "help-faq-symbolic", collapsed:
  false }` immediately above it.
- One `PAGE_KEYWORDS` entry: `("guide", "help howto documentation
  actions binds keybinds config reference tutorial")` — this alone is
  what makes Guide content reachable from the existing global Settings
  search without any further work.

`GuideSettingsModel` is a Relm4 `Component` like every other page here
(`about_settings.rs` at 346 lines is the closest sibling in shape and
scale — a `Component` mixing static data with one background-thread
probe, `spawn_gpu`, reused directly below for the Tools tab). Internally
it's a plain `gtk::Stack` + `gtk::StackSwitcher` (both core GTK4, no
Adwaita-specific widget needed) with the four tabs as child widgets,
each its own small internal struct — not sub-components, since none of
the four needs its own Relm4 message loop; plain GTK builder code per
tab is enough and keeps the file navigable.

### Tab 1 — Actions (Phase 1)

**Data**: `mctl::actions::{ACTIONS, Group}` (the existing `pub static
ACTIONS: &[Action]` / `pub struct Action { name, aliases, args, group,
summary, detail }` in `mctl/src/actions.rs`), imported directly.
`mctl/src/lib.rs` already declares `pub mod actions;` and
`mctl/src/bin/mctl.rs` already consumes it as `mctl::actions::ACTIONS`
— Cargo already builds `src/lib.rs` as an implicit library target for
the `mctl` package (no `[lib]` section needed, and none exists today),
so this works by adding one new workspace-path dependency:

```toml
# mshell-crates/mshell-settings/Cargo.toml
mctl = { path = "../../mctl" }
```

Trade-off worth naming: `mctl`'s own dependencies include
`wayland-client` / `wayland-backend` / `wayland-scanner` / `clap` /
`clap_complete`, none of which `mshell-settings` currently pulls in.
This is extra one-time build weight on a crate that otherwise has none
of that in its graph. Accepted for v1 because it's still a normal,
bounded dependency (not a cycle — `mctl` depends on nothing in
`mshell-*`) and extracting `actions.rs` into its own leaf crate is
pure yak-shaving unless the build-time cost turns out to matter in
practice; if it does, that extraction is a clean, mechanical follow-up
(move the module, point both `mctl` and `mshell-settings` at it) that
doesn't touch this page's own code at all.

**UI**: a `gtk::ListView` + `gtk::SignalListItemFactory` +
`gtk::CustomFilter` over a `gio::ListStore<BoxedAnyObject>` holding
`&'static Action` — the same virtualized-list technique already used
for a searchable list in this workspace (`mshell-frame`'s clipboard
history widget, `menus/menu_widgets/clipboard/clipboard.rs`; not
reused by dependency — `mshell-settings` doesn't and shouldn't depend
on `mshell-frame` — reimplemented locally following the same shape).
Grouped by `Action::group` (the existing 8-variant `Group` enum —
`Tag`, `Focus`, `Layout`, `Scroller`, `Window`, `Scratchpad`,
`Overview`, `System`) via a `gtk::CustomSorter` sorting by group then
name, group boundaries
drawn as inline section labels in the factory's `bind` step (same
technique `mctl actions --verbose` uses to print sections, mirrored in
GTK instead of stdout).

A `gtk::SearchEntry` above the list drives `CustomFilter::set_filter`,
matching against `name`, every string in `aliases`, and `summary`
(case-insensitive substring).

Clicking a row expands it in place (a `gtk::Revealer`, matching the
established revealer-row pattern from `DESIGN.md`) to show `args` and
`detail` — `detail` is empty for most actions (only the ones `mctl
actions --verbose` bothers to elaborate on), so the row's expand
affordance itself is hidden when both `args` and `detail` are empty
and only `summary` shows.

### Tab 2 — Settings (Phase 1)

**Data**: `margo/src/config.example.conf`, embedded via
`include_str!("../../../margo/src/config.example.conf")` — `include_str!`
resolves relative to the file it's used in
(`mshell-crates/mshell-settings/src/guide_settings.rs`), and three `..`
segments from there does land on the workspace root; verified directly
with `realpath --relative-to` rather than assumed.

A small parser (new, ~60-80 lines, no dependency) walks the embedded
string line by line:

- A line matching `^# ── (.+?) ─+$` starts a new **section** (title
  captured group 1) — mirrors the exact divider convention the file
  already uses everywhere (confirmed consistent across all ~60
  section headers checked while building the `config-reference.md`
  fix earlier this session).
- Consecutive `# ...` lines immediately following are that section's
  (or that key's, once one is seen) **description** lines, concatenated.
- A line matching `^(\w+)\s*=\s*(.*)$` is a **key**: `(key, default_value,
  accumulated_preceding_# lines since the last key or section header)`.
  `default_value` strips a trailing ` # inline comment` if present (e.g.
  `mru_scope            = all    # default scope: ...` → value `all`,
  the trailing comment discarded — it's the divider/preceding-lines
  prose that becomes the shown description, not this fragment).
- Everything else (blank lines, `bind = …` / `windowrule = …` /
  `gesturebind = …` / etc. bulk example blocks) is skipped for this
  tab — those are reference *examples*, not standalone settings, and
  showing every one of the ~120 example binds as its own "setting"
  would bury the ~240 real keys. (`bind`/`windowrule`/`tagrule`/
  `layerrule`/`monitorrule`/`mousebind`/`gesturebind`/`axisbind`/
  `source`/`include` are the exact directive names the parser skips —
  the same list `docs/config-reference.md`'s own key-diff script used
  this session to separate "keys" from "directives".)

Parsed once, lazily, on first tab activation (not at settings-app
startup — matches the existing "container routes... built eagerly"
vs. lazy-page pattern already documented in `build_top_page`'s own doc
comment), cached in the model for the process's lifetime (the embedded
string is `'static` and the file only changes with a margo update,
which restarts mshell anyway).

**UI**: same `ListView` + `CustomFilter` + `Revealer`-expand shape as
the Actions tab, grouped by section, search matching `key` and the
description text. A key's row shows `key = <default from the file>`
as the collapsed line (exactly what a user would type), the
description underneath when expanded.

### Tab 3 — Features (Phase 2)

**Data**: `docs/features.md`, embedded via
`include_str!("../../../docs/features.md")`, but only its first two
`##` sections — **Compositor** and **Desktop shell (mshell)** — sliced
out at parse time by heading. The file's other two `##` sections don't
belong here: **Companion binaries** is a table (this renderer doesn't
handle tables — see below — and it's redundant with the live Tools
tab anyway), and **Configuration & IPC** is just a list of links out
to the website, of no use inside the app. Slicing by heading means a
future `features.md` edit that adds content to Compositor/Desktop
shell shows up automatically; a new `##` section added after them is
excluded by default (matches the "curated tour" intent — an explicit
opt-in, not "dump the whole file", is the correct default here) and
one line added to the slice's end-heading list opts it in.

**Rendering**: a minimal markdown-subset → Pango-markup line
transform (new, small, no dependency) handling exactly what those two
sections actually use today (checked): `**bold**` → `<b>`,
`` `code` `` → `<tt>`, `[text](url)` → `<a href="url">`, and `- `
bullet lines → a leading "•  " with a `GtkLabel` per bullet — no `##`
headers inside the slice itself (the two section titles become the
tab's own two static subheadings, not part of the transformed text),
no tables, no nested lists, no images. If a future edit introduces one
of those inside the two included sections, it renders as literal text
(safe degradation, not a crash or a silent drop) — and the fixture
test set (below) includes exactly this "unhandled construct" case so
that degradation path itself is covered, not just assumed.

**UI**: a single scrollable `GtkBox` of labels, no search (this tab is
short-form prose, not a lookup table — search would be over-engineering
for ~15 bullet points).

### Tab 4 — Tools (Phase 3)

**Data**: none of the companion binaries expose a structured catalogue
like `mctl actions` does, and several (`mlock`, `mvpn`, `mcal`, …)
aren't confirmed to use `clap` — their `--help` text shape isn't
guaranteed consistent. Rather than parse, this tab runs each binary
with `--help` via `std::process::Command`, captures stdout, and shows
it verbatim in a monospace `GtkTextView` (read-only). The exact binary
list to probe: `mctl`, `mshellctl`, `mlock`, `mlogind`, `mlayout`,
`mscreenshot`, `mkeys`, `mvpn`, `mcal`, `mtune`, `mpicker`, `mdots`,
`mpower`, `mwizard` — 14 of the 21 rows in README's own "Binaries"
table. Excluded: `margo` and `mshell` (the compositor and shell
themselves, not CLI tools you run standalone), `margo-portal` and
`mshellshare` (backend D-Bus/portal daemons, no user-facing CLI at
all), `start-margo` (a supervisor process, same reason), `mgreet` (a
graphical greeter `mlogind` launches — there's no logged-in desktop
session in which a user would run it directly), and `mvisual` (an
internal renderer-debugging helper, not a tool aimed at end users).

**Execution model**: a `gtk::ListBox` on the left (binary names) drives
a right-hand `GtkTextView`; each `--help` is run **lazily on first
selection** of that tool (not all 14 up front) and cached in the model
for the session. `mshell-settings` already has the exact right
precedent for this: `about_settings.rs`'s `spawn_gpu` runs
`gpu_name()` (which shells out to `lspci`) on a `std::thread::spawn`
background thread and delivers the result back to the GTK thread via
`ComponentSender::input(AboutSettingsInput::GpuLoaded(...))` — Relm4's
own sender is `Clone` + thread-safe, so no separate `glib::MainContext`
channel is needed. `guide_settings.rs` follows the identical shape per
tool: a background thread runs the child process and sends
`GuideSettingsInput::ToolHelpLoaded(name, result)` when done. The one
addition beyond that baseline: `lspci` is a base-system tool that's
always present and well-behaved, but these 14 are optional companions
that might not be installed or might hang (e.g. one waiting on stdin
for input that will never come) — a 2-second `try_wait` poll loop
around the spawned child enforces a timeout and kills it on expiry
before the background thread reports back, which `spawn_gpu` doesn't
need and doesn't have. A binary that isn't installed, exits non-zero,
or times out shows a one-line explanatory placeholder
("`mvpn` not found on PATH" / "`mcal --help` didn't respond") in the
text view instead of an error dialog — this tab must never be able to
hang or crash the settings window over a missing
optional tool.

## Testing

- **Parser unit tests** (Settings tab): the `config.example.conf`
  section/key parser gets its own `#[cfg(test)]` module with fixture
  strings (not the real 1500-line file) covering: a plain `key =
  value` line, a key preceded by multiple `#` comment lines, a section
  divider, a `bind = …` directive line correctly skipped, and a key
  with no preceding comment (empty description, not a panic).
- **Markdown-subset unit tests** (Features tab): fixture strings in,
  expected Pango markup out, for each of the five constructs the
  transform handles plus one "unhandled construct passes through
  literally" case.
- **Keyword uniqueness**: the existing
  `page_keyword_labels_are_unique_and_nonempty` test in `settings.rs`
  already covers the new `PAGE_KEYWORDS` entry for free — no new test
  needed there, just don't break it.
- **Tools tab**: no automated test spawns the real binaries (they may
  not be installed in CI) — covered by manual verification only, per
  this workspace's existing convention for anything that shells out to
  optional runtime tools.
- Every phase ends with the project's standard gate: `cargo test -p
  mshell-settings`, `cargo clippy -p mshell-settings --all-targets -D
  warnings`, `cargo +1.95.0 fmt --all -- --check`, `bash
  scripts/panic-ratchet.sh`.

## Open questions

None — the four tabs' data sources, parsing rules, and UI patterns are
all pinned above against real, already-read source (not left for
implementation time to decide). The one deliberate deferral is the
"copy this bind line" / "jump to the Keybinds page" affordance
mentioned under Non-goals — a natural v2, not needed for the guide to
be useful on day one.
