# Settings → Guide Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a new "Guide" page to Settings (`mshell-crates/mshell-settings`) with four tabs — Actions, Settings, Features, Tools — browsing margo's dispatch actions, config keys, feature tour, and companion-tool `--help` output, all sourced from existing single-source-of-truth data.

**Architecture:** A new Relm4 `Component` (`GuideSettingsModel` in `guide_settings.rs`) registered exactly like every existing settings page (`page!` macro arm, sidebar `Page`/`Section` entries, `PAGE_KEYWORDS` entry). Internally a plain `gtk::Stack` + `gtk::StackSwitcher` holding four tab widgets, each built by its own function in `src/guide/`. Tab 1 imports `mctl::actions::ACTIONS` directly (new path dependency). Tab 2 parses `margo/src/config.example.conf` (embedded via `include_str!`) with a small hand-written parser. Tab 3 renders a curated slice of `docs/features.md` through a small markdown-subset-to-Pango transform. Tab 4 shells out to each companion binary's `--help` off the GTK thread, mirroring `about_settings.rs`'s existing `spawn_gpu` pattern, with an added timeout.

**Tech Stack:** Rust, GTK4 (`gtk4-rs` crate re-exported as `relm4::gtk`), Relm4 (`Component` trait, `#[relm4::component]` macro), `gio::ListStore` + `glib::BoxedAnyObject` + `gtk::CustomFilter` + `gtk::SignalListItemFactory` for the two searchable lists.

**Spec:** `docs/superpowers/specs/2026-09-07-settings-guide-page-design.md`

## Global Constraints

- Every phase ends with: `cargo test -p mshell-settings`, `cargo clippy -p mshell-settings --all-targets -- -D warnings`, `cargo +1.95.0 fmt --all -- --check`, `bash scripts/panic-ratchet.sh`.
- No hand-copied documentation content anywhere — every tab derives from an existing source (`mctl::actions::ACTIONS`, `margo/src/config.example.conf`, `docs/features.md`). If a task would require typing out feature/setting descriptions by hand, that's a sign the task is wrong.
- New files follow the existing flat/`page_name.rs` + `page_name/supporting_module.rs` convention already used by `net/` + `network_settings.rs` — not a new directory-per-page convention.
- `Action` fields (from `mctl/src/actions.rs`): `pub struct Action { name: &'static str, aliases: &'static [&'static str], args: &'static str, group: Group, summary: &'static str, detail: &'static str }`, `pub const ACTIONS: &[Action]`, `pub enum Group { Tag, Focus, Layout, Scroller, Window, Scratchpad, Overview, System }` with `impl Group { pub const fn label(self) -> &'static str }`.
- Relm4 `Component` shape used throughout this codebase (verified against `about_settings.rs`): `fn init(params: Self::Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self>`, `fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root)`, a `view! { #[root] ... }` block, `let widgets = view_output!();` inside `init`.

---

## Phase 1: Actions + Settings tabs

### Task 1: `mctl` as a path dependency of `mshell-settings`

**Files:**
- Modify: `mshell-crates/mshell-settings/Cargo.toml`

**Interfaces:**
- Produces: `mctl::actions::{ACTIONS, Action, Group}` importable from `mshell-settings`.

- [ ] **Step 1: Add the dependency**

Open `mshell-crates/mshell-settings/Cargo.toml`, find the `[dependencies]` section, add a line (alphabetical position, matching this file's existing ordering convention):

```toml
mctl = { path = "../../mctl" }
```

- [ ] **Step 2: Verify it resolves**

Run: `cargo check -p mshell-settings`
Expected: succeeds (mctl and its own dependencies — wayland-client, clap, etc. — download/build; no code uses the new dependency yet so nothing else changes).

- [ ] **Step 3: Commit**

```bash
git add mshell-crates/mshell-settings/Cargo.toml Cargo.lock
git commit -m "chore(mshell-settings): depend on mctl for its dispatch-action catalogue"
```

### Task 2: `config.example.conf` section/key parser

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide/mod.rs`
- Create: `mshell-crates/mshell-settings/src/guide/config_parser.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct ConfigKey {
      pub key: String,
      pub default: String,
      pub description: String,
  }
  pub struct ConfigSection {
      pub title: String,
      pub keys: Vec<ConfigKey>,
  }
  pub fn parse(source: &str) -> Vec<ConfigSection>;
  ```
  (`guide::config_parser::{parse, ConfigSection, ConfigKey}`, used by Task 5.)

- [ ] **Step 1: Create the module scaffold**

Create `mshell-crates/mshell-settings/src/guide/mod.rs`:

```rust
//! Supporting (non-widget) logic for the Settings → Guide page.
//! `guide_settings.rs` is the page itself; this module holds the parsers
//! and data transforms each tab is built from.

pub mod config_parser;
```

- [ ] **Step 2: Write the failing tests**

Create `mshell-crates/mshell-settings/src/guide/config_parser.rs` with just the test module first (the empty `parse` stub below makes them compile but fail on assertions):

```rust
//! Turns `margo/src/config.example.conf` into structured sections/keys for
//! the Guide page's Settings tab. Deliberately ignores directive lines
//! (`bind`, `windowrule`, `tagrule`, `layerrule`, `monitorrule`,
//! `mousebind`, `gesturebind`, `axisbind`, `source`, `include`) — those are
//! reference *examples*, not standalone settings; showing all ~120 of them
//! as "settings" would bury the ~240 real keys.

/// One `key = value` line from `config.example.conf`, with the `#`-comment
/// lines that preceded it (its section header's own preceding comments if
/// none were directly above this key) as its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigKey {
    pub key: String,
    pub default: String,
    pub description: String,
}

/// One `# ── Title ──...──` divider and the keys under it, up to the next
/// divider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSection {
    pub title: String,
    pub keys: Vec<ConfigKey>,
}

const DIRECTIVE_NAMES: &[&str] = &[
    "bind", "bindr", "bindl", "bindc", "bindrl", "bindrc", "bindlc", "bindrlc",
    "mousebind", "gesturebind", "axisbind", "windowrule", "tagrule", "layerrule",
    "monitorrule", "source", "include",
];

pub fn parse(source: &str) -> Vec<ConfigSection> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_key_value_line() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
# Comment for foo.
foo = bar
";
        let sections = parse(src);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Example");
        assert_eq!(sections[0].keys.len(), 1);
        assert_eq!(sections[0].keys[0].key, "foo");
        assert_eq!(sections[0].keys[0].default, "bar");
        assert_eq!(sections[0].keys[0].description, "Comment for foo.");
    }

    #[test]
    fn multiple_comment_lines_concatenate() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
# First line.
# Second line.
foo = bar
";
        let sections = parse(src);
        assert_eq!(sections[0].keys[0].description, "First line. Second line.");
    }

    #[test]
    fn trailing_inline_comment_is_stripped_from_the_default() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
foo = bar    # not part of the value
";
        let sections = parse(src);
        assert_eq!(sections[0].keys[0].default, "bar");
    }

    #[test]
    fn directive_lines_are_skipped() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
bind = super,q,killclient
windowrule = isfloating:1, appid:^pavucontrol$
foo = bar
";
        let sections = parse(src);
        assert_eq!(sections[0].keys.len(), 1);
        assert_eq!(sections[0].keys[0].key, "foo");
    }

    #[test]
    fn key_with_no_preceding_comment_has_empty_description() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
foo = bar
baz = qux
";
        let sections = parse(src);
        // `foo` has no comment directly above it either (the divider isn't
        // a description) so its description is empty; `baz` likewise.
        assert_eq!(sections[0].keys[0].description, "");
        assert_eq!(sections[0].keys[1].description, "");
    }

    #[test]
    fn two_sections_split_correctly() {
        let src = "\
# ── First ───────────────────────────────────────────────────────────────────
a = 1
# ── Second ──────────────────────────────────────────────────────────────────
b = 2
";
        let sections = parse(src);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].title, "First");
        assert_eq!(sections[1].title, "Second");
    }
}
```

- [ ] **Step 3: Register the module**

Add `mod guide;` to `mshell-crates/mshell-settings/src/lib.rs`, alongside the other `mod` declarations (alphabetical position among the existing list, e.g. right before `mod general_settings;` or wherever `g` sorts).

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p mshell-settings guide::config_parser -- --nocapture`
Expected: compile error (`todo!()` panics, or the crate fails to build if `parse` isn't called — either way, not a PASS). Confirm you see `guide::config_parser::tests::...` in the test list before the panic, so you know the module wired up correctly.

- [ ] **Step 5: Implement the parser**

Replace the `todo!()` body:

```rust
pub fn parse(source: &str) -> Vec<ConfigSection> {
    let mut sections: Vec<ConfigSection> = Vec::new();
    let mut pending_comment: Vec<String> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim_end();

        if let Some(title) = section_title(trimmed) {
            sections.push(ConfigSection {
                title: title.to_string(),
                keys: Vec::new(),
            });
            pending_comment.clear();
            continue;
        }

        if let Some(comment) = trimmed.strip_prefix('#') {
            pending_comment.push(comment.trim().to_string());
            continue;
        }

        if trimmed.trim().is_empty() {
            pending_comment.clear();
            continue;
        }

        if let Some((key, default)) = key_value(trimmed) {
            if DIRECTIVE_NAMES.contains(&key.as_str()) {
                pending_comment.clear();
                continue;
            }
            if let Some(section) = sections.last_mut() {
                section.keys.push(ConfigKey {
                    key,
                    default,
                    description: pending_comment.join(" ").trim().to_string(),
                });
            }
            pending_comment.clear();
            continue;
        }

        // Anything else (shouldn't happen in practice) resets the pending
        // comment run rather than attaching stale prose to the next key.
        pending_comment.clear();
    }

    sections
}

/// `# ── Title ──...──` -> `Some("Title")`. Requires the line to actually
/// start with `# ──` (the file's own divider convention) so an ordinary
/// `#` comment starting with a dash never misparses as a header.
fn section_title(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("# ── ")?;
    let title_end = rest.find(" ──")?;
    Some(&rest[..title_end])
}

/// `key = value` (optionally with a trailing `  # inline comment`) ->
/// `Some(("key", "value"))`. Rejects anything whose "key" isn't a bare
/// identifier (covers indented / commented-out lines, which never start
/// at column 0 with a plain word).
fn key_value(line: &str) -> Option<(String, String)> {
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value = rest.split(" #").next().unwrap_or(rest).trim();
    Some((key.to_string(), value.to_string()))
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p mshell-settings guide::config_parser -- --nocapture`
Expected: all 6 tests PASS.

- [ ] **Step 7: Sanity-check against the real file**

Run a throwaway check that the parser doesn't panic on the actual 1500-line file and finds a plausible number of sections/keys — this isn't a committed test (the real file changes over time and isn't a stable fixture), just a manual gate before moving on:

```bash
cat > /tmp/parser_smoke.rs << 'EOF'
fn main() {
    let src = include_str!("/repo/archive/.kod/margo/margo/src/config.example.conf");
    let sections = mshell_settings::guide::config_parser::parse(src);
    println!("{} sections", sections.len());
    let total_keys: usize = sections.iter().map(|s| s.keys.len()).sum();
    println!("{} keys", total_keys);
    assert!(sections.len() > 30, "expected 30+ sections, got {}", sections.len());
    assert!(total_keys > 150, "expected 150+ keys, got {}", total_keys);
}
EOF
```

This requires `config_parser`'s items to be `pub` (already are) and the crate to expose `pub mod guide;` — if `mod guide;` in `lib.rs` isn't `pub`, temporarily make it `pub mod guide;` for this check, run `cargo run --example parser_smoke` (place the file at `mshell-crates/mshell-settings/examples/parser_smoke.rs` instead of `/tmp` so `cargo run --example` finds it), confirm the printed counts look sane (compare against the ~60 section headers / ~240ish keys estimated in the spec), delete the example file afterward, and revert `mod guide;` to its final intended visibility from Task 3 below.

- [ ] **Step 8: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide/ mshell-crates/mshell-settings/src/lib.rs
git commit -m "feat(mshell-settings): config.example.conf section/key parser for the Guide page"
```

### Task 3: Register the Guide page (empty shell)

Get the page appearing in Settings and reachable before either tab has real content — the smallest possible end-to-end slice, so every later task adds visible content to something that already works rather than being the first time the whole chain is exercised.

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide_settings.rs`
- Modify: `mshell-crates/mshell-settings/src/lib.rs`
- Modify: `mshell-crates/mshell-settings/src/settings.rs`

**Interfaces:**
- Produces: `pub(crate) struct GuideSettingsModel`, `pub(crate) struct GuideSettingsInit {}`, registered under route `"guide"`.
- Consumes: nothing yet (Tasks 4/5 add the real tab content).

- [ ] **Step 1: Write the page**

Create `mshell-crates/mshell-settings/src/guide_settings.rs`:

```rust
//! Settings → Guide.
//!
//! A browsable, searchable reference for what margo can do — dispatch
//! actions, config keys, a feature tour, and companion-tool `--help`
//! output. Every tab derives from an existing single-source-of-truth
//! (`mctl::actions::ACTIONS`, `margo/src/config.example.conf`,
//! `docs/features.md`) rather than hand-copied text, so it can't drift
//! the way prose docs elsewhere in this project have drifted before.
//! See `docs/superpowers/specs/2026-09-07-settings-guide-page-design.md`.

use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Default)]
pub(crate) struct GuideSettingsModel {}

#[derive(Debug)]
pub(crate) enum GuideSettingsInput {}

#[derive(Debug)]
pub(crate) enum GuideSettingsOutput {}

pub(crate) struct GuideSettingsInit {}

#[derive(Debug)]
pub(crate) enum GuideSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for GuideSettingsModel {
    type CommandOutput = GuideSettingsCommandOutput;
    type Input = GuideSettingsInput;
    type Output = GuideSettingsOutput;
    type Init = GuideSettingsInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "settings-page",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_vexpand: true,
            set_spacing: 16,

            gtk::Box {
                add_css_class: "settings-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::Start,
                set_spacing: 16,
                gtk::Image {
                    add_css_class: "settings-hero-icon",
                    set_icon_name: Some("help-faq-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        add_css_class: "settings-hero-title",
                        set_label: "Guide",
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "settings-hero-subtitle",
                        set_label: "Browse everything margo can do.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },
            },

            #[name = "stack"]
            gtk::Stack {
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::Crossfade,
            },

            gtk::StackSwitcher {
                set_stack: Some(&stack),
                set_halign: gtk::Align::Center,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = GuideSettingsModel::default();
        let widgets = view_output!();
        let _ = sender;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, _message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {}
}
```

This compiles and shows an empty page with an empty (invisible, since it has no children yet) stack — Tasks 4/5 add children to `stack` by name from `init`, after `view_output!()`.

- [ ] **Step 2: Register the module**

Add `mod guide_settings;` to `mshell-crates/mshell-settings/src/lib.rs`.

- [ ] **Step 3: Register the page route**

In `mshell-crates/mshell-settings/src/settings.rs`, add to the `use` block at the top:

```rust
use crate::guide_settings::{GuideSettingsInit, GuideSettingsModel};
```

Add to `build_top_page`'s match (any position — matches existing entries are unordered by anything but readability, add it near `"about"`):

```rust
"guide" => page!(GuideSettingsModel => GuideSettingsInit {}),
```

- [ ] **Step 4: Add the sidebar entries**

In the `SIDEBAR` list (the `&[SidebarEntry]` constant ending in the `About` section/page pair), insert immediately before the existing `Section { name: "About", ... }` entry:

```rust
Section {
    name: "Guide",
    icon: "help-faq-symbolic",
    collapsed: false,
},
Page {
    route: "guide",
    icon: "help-faq-symbolic",
    label: "Guide",
},
```

- [ ] **Step 5: Add the search keyword entry**

In `PAGE_KEYWORDS`, add:

```rust
("guide", "help howto documentation actions binds keybinds config reference tutorial"),
```

- [ ] **Step 6: Build and run**

Run: `cargo build -p mshell-settings`
Expected: succeeds.

Run: `cargo test -p mshell-settings page_keyword_labels_are_unique_and_nonempty`
Expected: PASS (confirms the new `PAGE_KEYWORDS` entry didn't collide with an existing route/label).

- [ ] **Step 7: Manual verification**

Run: `just shell` (builds mshell and restarts the live systemd unit — per this repo's own dev-loop convention). Open Settings, confirm a "Guide" section with a "Guide" page appears at the bottom of the sidebar, opens to an empty page with the hero header and no visible tab switcher (expected — no tabs added yet).

- [ ] **Step 8: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide_settings.rs mshell-crates/mshell-settings/src/lib.rs mshell-crates/mshell-settings/src/settings.rs
git commit -m "feat(mshell-settings): register the Settings -> Guide page (empty shell)"
```

### Task 4: Actions tab

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide/actions_tab.rs`
- Modify: `mshell-crates/mshell-settings/src/guide/mod.rs`
- Modify: `mshell-crates/mshell-settings/src/guide_settings.rs`

**Interfaces:**
- Consumes: `mctl::actions::{ACTIONS, Action, Group}` (Task 1).
- Produces: `pub fn build() -> gtk::Widget` (`guide::actions_tab::build`), appended to `guide_settings.rs`'s stack as page name `"actions"`, title `"Actions"`.

- [ ] **Step 1: Add the submodule**

In `mshell-crates/mshell-settings/src/guide/mod.rs`, add:

```rust
pub mod actions_tab;
```

- [ ] **Step 2: Build the tab**

Create `mshell-crates/mshell-settings/src/guide/actions_tab.rs`:

```rust
//! Settings -> Guide -> Actions: every dispatch action `mctl::actions`
//! knows about, grouped, searchable, click-to-expand.

use mctl::actions::{ACTIONS, Action, Group};
use relm4::gtk::prelude::*;
use relm4::gtk::{self, gio, glib};

const GROUP_ORDER: &[Group] = &[
    Group::Tag,
    Group::Focus,
    Group::Layout,
    Group::Scroller,
    Group::Window,
    Group::Scratchpad,
    Group::Overview,
    Group::System,
];

pub fn build() -> gtk::Widget {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    // One row per action, ordered by GROUP_ORDER then by name within a
    // group -- matches `mctl actions --verbose`'s own section order, so
    // this list reads the same way the CLI output does.
    let mut sorted: Vec<&'static Action> = ACTIONS.iter().collect();
    sorted.sort_by_key(|a| {
        let group_rank = GROUP_ORDER.iter().position(|g| *g == a.group).unwrap_or(usize::MAX);
        (group_rank, a.name)
    });
    for action in sorted {
        store.append(&glib::BoxedAnyObject::new(action));
    }

    let query = std::rc::Rc::new(std::cell::RefCell::new(String::new()));

    let filter_query = query.clone();
    let filter = gtk::CustomFilter::new(move |obj| {
        let Some(bo) = obj.downcast_ref::<glib::BoxedAnyObject>() else {
            return false;
        };
        let action = bo.borrow::<&'static Action>();
        let q = filter_query.borrow();
        if q.is_empty() {
            return true;
        }
        let q = q.as_str();
        action.name.contains(q)
            || action.summary.to_lowercase().contains(q)
            || action.aliases.iter().any(|a| a.contains(q))
    });
    let filter_model = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
    let selection = gtk::NoSelection::new(Some(filter_model));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let row = build_row();
        list_item.set_child(Some(&row));
    });
    factory.connect_bind(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(item) = list_item.item() else { return };
        let bo = item.downcast_ref::<glib::BoxedAnyObject>().unwrap();
        let action: std::cell::Ref<&'static Action> = bo.borrow();
        let action = *action;
        let Some(row) = list_item.child() else { return };
        bind_row(&row, action);
    });

    let list_view = gtk::ListView::new(Some(selection), Some(factory));
    list_view.add_css_class("guide-actions-list");

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list_view)
        .build();

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search actions…")
        .build();
    let search_filter = filter.clone();
    let search_query = query;
    search.connect_search_changed(move |entry| {
        *search_query.borrow_mut() = entry.text().to_lowercase();
        search_filter.changed(gtk::FilterChange::Different);
    });

    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_top(8);
    page.append(&search);
    page.append(&scroller);
    page.upcast()
}

/// One list row's static widget structure: name + summary on top, an
/// expandable revealer underneath carrying `args` / `detail`. The
/// revealer only ever shows/hides -- its content is set once per bind in
/// `bind_row`, matching the factory's setup/bind split (structure built
/// once per pooled row in `connect_setup`, content refreshed per item in
/// `connect_bind`).
fn build_row() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.add_css_class("guide-row");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::new(None);
    name.add_css_class("guide-row-name");
    name.set_halign(gtk::Align::Start);
    name.set_xalign(0.0);
    let summary = gtk::Label::new(None);
    summary.add_css_class("guide-row-summary");
    summary.set_halign(gtk::Align::Start);
    summary.set_xalign(0.0);
    summary.set_hexpand(true);
    summary.set_wrap(true);
    header.append(&name);
    header.append(&summary);

    let revealer = gtk::Revealer::new();
    let detail = gtk::Label::new(None);
    detail.add_css_class("guide-row-detail");
    detail.set_halign(gtk::Align::Start);
    detail.set_xalign(0.0);
    detail.set_wrap(true);
    revealer.set_child(Some(&detail));

    root.append(&header);
    root.append(&revealer);

    let click = gtk::GestureClick::new();
    let revealer_for_click = revealer.clone();
    click.connect_released(move |_, _, _, _| {
        revealer_for_click.set_reveal_child(!revealer_for_click.reveals_child());
    });
    root.add_controller(click);

    root
}

fn bind_row(row: &gtk::Widget, action: &'static Action) {
    let root = row.downcast_ref::<gtk::Box>().unwrap();
    let header = root.first_child().and_downcast::<gtk::Box>().unwrap();
    let name = header.first_child().and_downcast::<gtk::Label>().unwrap();
    let summary = name.next_sibling().and_downcast::<gtk::Label>().unwrap();
    let revealer = header.next_sibling().and_downcast::<gtk::Revealer>().unwrap();
    let detail = revealer.child().and_downcast::<gtk::Label>().unwrap();

    name.set_label(action.name);
    summary.set_label(action.summary);

    let has_extra = !action.args.is_empty() || !action.detail.is_empty();
    if has_extra {
        let mut text = String::new();
        if !action.args.is_empty() {
            text.push_str("Args: ");
            text.push_str(action.args);
        }
        if !action.detail.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(action.detail);
        }
        detail.set_label(&text);
    }
    revealer.set_reveal_child(false);
    root.set_can_target(true);
    root.set_sensitive(true);
    let _ = has_extra; // row stays clickable either way; an empty revealer just toggles open on nothing
}
```

- [ ] **Step 3: Wire the tab into the stack**

In `guide_settings.rs`, add the import:

```rust
use crate::guide::actions_tab;
```

In `init`, after `let widgets = view_output!();`, add:

```rust
widgets.stack.add_titled(&actions_tab::build(), Some("actions"), "Actions");
```

- [ ] **Step 4: Build and verify**

Run: `cargo build -p mshell-settings`
Expected: succeeds.

Run: `cargo clippy -p mshell-settings --all-targets -- -D warnings`
Expected: no warnings (pay particular attention to the `has_extra`/`let _ = has_extra;` line above -- if clippy flags it as dead, remove the unused binding rather than suppress the lint; it was left in only as a readability aid while writing `bind_row` and may not survive clippy).

- [ ] **Step 5: Manual verification**

`just shell`, open Settings -> Guide, confirm the "Actions" tab shows a searchable, scrollable list of every dispatch action, typing in the search box filters live, clicking a row expands/collapses its args/detail (or does nothing visible for actions with neither -- acceptable, not a bug).

- [ ] **Step 6: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide/
git commit -m "feat(mshell-settings): Guide page Actions tab"
```

### Task 5: Settings tab

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide/config_tab.rs`
- Modify: `mshell-crates/mshell-settings/src/guide/mod.rs`
- Modify: `mshell-crates/mshell-settings/src/guide_settings.rs`

**Interfaces:**
- Consumes: `guide::config_parser::{parse, ConfigSection, ConfigKey}` (Task 2).
- Produces: `pub fn build() -> gtk::Widget` (`guide::config_tab::build`), appended to the stack as page name `"config"`, title `"Settings"`.

- [ ] **Step 1: Add the submodule**

Add `pub mod config_tab;` to `guide/mod.rs`.

- [ ] **Step 2: Build the tab**

Create `mshell-crates/mshell-settings/src/guide/config_tab.rs`. This mirrors `actions_tab.rs`'s list/filter/row shape exactly, substituting `ConfigKey` for `Action` and grouping by `ConfigSection` instead of `Group`:

```rust
//! Settings -> Guide -> Settings: every `config.conf` key from
//! `margo/src/config.example.conf`, grouped by its own section dividers,
//! searchable, click-to-expand.

use super::config_parser::{self, ConfigKey};
use relm4::gtk::prelude::*;
use relm4::gtk::{self, gio, glib};

/// One row's data: the key itself plus which section it's under (shown as
/// a prefix so search results out of context are still legible).
struct Row {
    section: String,
    key: ConfigKey,
}

pub fn build() -> gtk::Widget {
    let source = include_str!("../../../../margo/src/config.example.conf");
    let sections = config_parser::parse(source);

    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for section in sections {
        for key in section.keys {
            store.append(&glib::BoxedAnyObject::new(Row {
                section: section.title.clone(),
                key,
            }));
        }
    }

    let query = std::rc::Rc::new(std::cell::RefCell::new(String::new()));

    let filter_query = query.clone();
    let filter = gtk::CustomFilter::new(move |obj| {
        let Some(bo) = obj.downcast_ref::<glib::BoxedAnyObject>() else {
            return false;
        };
        let row: std::cell::Ref<Row> = bo.borrow();
        let q = filter_query.borrow();
        if q.is_empty() {
            return true;
        }
        let q = q.as_str();
        row.key.key.to_lowercase().contains(q) || row.key.description.to_lowercase().contains(q)
    });
    let filter_model = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
    let selection = gtk::NoSelection::new(Some(filter_model));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        list_item.set_child(Some(&build_row()));
    });
    factory.connect_bind(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(item) = list_item.item() else { return };
        let bo = item.downcast_ref::<glib::BoxedAnyObject>().unwrap();
        let row: std::cell::Ref<Row> = bo.borrow();
        let Some(widget) = list_item.child() else { return };
        bind_row(&widget, &row);
    });

    let list_view = gtk::ListView::new(Some(selection), Some(factory));
    list_view.add_css_class("guide-config-list");

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list_view)
        .build();

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search settings…")
        .build();
    let search_filter = filter.clone();
    let search_query = query;
    search.connect_search_changed(move |entry| {
        *search_query.borrow_mut() = entry.text().to_lowercase();
        search_filter.changed(gtk::FilterChange::Different);
    });

    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_top(8);
    page.append(&search);
    page.append(&scroller);
    page.upcast()
}

fn build_row() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.add_css_class("guide-row");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let key_value = gtk::Label::new(None);
    key_value.add_css_class("guide-row-name");
    key_value.set_halign(gtk::Align::Start);
    key_value.set_xalign(0.0);
    let section = gtk::Label::new(None);
    section.add_css_class("guide-row-summary");
    section.set_halign(gtk::Align::End);
    section.set_hexpand(true);
    header.append(&key_value);
    header.append(&section);

    let revealer = gtk::Revealer::new();
    let description = gtk::Label::new(None);
    description.add_css_class("guide-row-detail");
    description.set_halign(gtk::Align::Start);
    description.set_xalign(0.0);
    description.set_wrap(true);
    revealer.set_child(Some(&description));

    root.append(&header);
    root.append(&revealer);

    let click = gtk::GestureClick::new();
    let revealer_for_click = revealer.clone();
    click.connect_released(move |_, _, _, _| {
        revealer_for_click.set_reveal_child(!revealer_for_click.reveals_child());
    });
    root.add_controller(click);

    root
}

fn bind_row(widget: &gtk::Widget, row: &Row) {
    let root = widget.downcast_ref::<gtk::Box>().unwrap();
    let header = root.first_child().and_downcast::<gtk::Box>().unwrap();
    let key_value = header.first_child().and_downcast::<gtk::Label>().unwrap();
    let section = key_value.next_sibling().and_downcast::<gtk::Label>().unwrap();
    let revealer = header.next_sibling().and_downcast::<gtk::Revealer>().unwrap();
    let description = revealer.child().and_downcast::<gtk::Label>().unwrap();

    key_value.set_label(&format!("{} = {}", row.key.key, row.key.default));
    section.set_label(&row.section);
    description.set_label(&row.key.description);
    revealer.set_reveal_child(false);
}
```

Note the `include_str!` path: `config_tab.rs` lives at
`mshell-crates/mshell-settings/src/guide/config_tab.rs`, one directory
deeper than `guide_settings.rs`, so it needs **four** `..` segments
(not the three verified for `guide_settings.rs`'s own depth in the
spec) to reach the workspace root: `../../../../margo/src/config.example.conf`.
Verify this before running the build:

```bash
realpath --relative-to=mshell-crates/mshell-settings/src/guide margo/src/config.example.conf
```

Expected output: `../../../../margo/src/config.example.conf` — if it differs, use whatever this command actually prints instead of the path shown above.

- [ ] **Step 3: Wire the tab into the stack**

In `guide_settings.rs`, add:

```rust
use crate::guide::config_tab;
```

In `init`, after the `actions_tab` line:

```rust
widgets.stack.add_titled(&config_tab::build(), Some("config"), "Settings");
```

- [ ] **Step 4: Build and verify**

Run: `cargo build -p mshell-settings`
Expected: succeeds.

Run: `cargo clippy -p mshell-settings --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Manual verification**

`just shell`, open Settings -> Guide, confirm a "Settings" tab now appears next to "Actions" (the `StackSwitcher` added in Task 3 now has two real buttons), lists every config key grouped/searchable, clicking expands the description.

- [ ] **Step 6: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide/ mshell-crates/mshell-settings/src/guide_settings.rs
git commit -m "feat(mshell-settings): Guide page Settings tab"
```

### Task 6: Phase 1 full verification gate

**Files:** none (verification only).

- [ ] **Step 1: Full test suite**

Run: `cargo test -p mshell-settings`
Expected: all tests PASS, including the 6 `config_parser` tests and `page_keyword_labels_are_unique_and_nonempty`.

- [ ] **Step 2: Clippy**

Run: `cargo clippy -p mshell-settings --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Formatting**

Run: `cargo +1.95.0 fmt --all -- --check`
Expected: clean (if not, run `cargo +1.95.0 fmt --all` and commit the formatting fix separately).

- [ ] **Step 4: Panic ratchet**

Run: `bash scripts/panic-ratchet.sh`
Expected: `OK: at baseline.` If it regressed, find and remove the new `.unwrap()`/`.expect()` call the ratchet flagged — the `.downcast::<...>().unwrap()` calls in `actions_tab.rs`/`config_tab.rs` are all on values this code just constructed itself (never on external/attacker-controlled input), so if the ratchet counts them, replace the pattern with an early-return (`let Some(x) = ... else { return };`) instead of suppressing the count.

- [ ] **Step 5: Push and watch CI**

```bash
git push origin main
```

Watch the GitHub Actions run to completion (per this repo's established `gh run view` polling convention) before starting Phase 2.

---

## Phase 2: Features tab

### Task 7: Markdown-subset -> Pango markup transform

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide/markdown.rs`
- Modify: `mshell-crates/mshell-settings/src/guide/mod.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum Block {
      Paragraph(String),  // Pango markup, one logical paragraph
      Bullet(String),     // Pango markup, one bullet item (no leading "•")
  }
  pub fn transform(markdown: &str) -> Vec<Block>;
  ```

- [ ] **Step 1: Add the submodule**

Add `pub mod markdown;` to `guide/mod.rs`.

- [ ] **Step 2: Write the failing tests**

Create `mshell-crates/mshell-settings/src/guide/markdown.rs`:

```rust
//! A markdown *subset* -> Pango markup transform, covering exactly the
//! constructs `docs/features.md`'s Compositor / Desktop shell sections use
//! today: **bold**, `code`, [links](url), and `- ` bullets. Not a general
//! CommonMark parser -- anything else (tables, nested lists, images,
//! headers inside the slice) passes through as literal escaped text
//! rather than being silently dropped, which is what the last test below
//! pins down.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(String),
    Bullet(String),
}

pub fn transform(markdown: &str) -> Vec<Block> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paragraph() {
        let out = transform("Just plain text.");
        assert_eq!(out, vec![Block::Paragraph("Just plain text.".to_string())]);
    }

    #[test]
    fn bold_becomes_pango_b() {
        let out = transform("Some **bold** text.");
        assert_eq!(out, vec![Block::Paragraph("Some <b>bold</b> text.".to_string())]);
    }

    #[test]
    fn code_becomes_pango_tt() {
        let out = transform("Run `mctl reload`.");
        assert_eq!(out, vec![Block::Paragraph("Run <tt>mctl reload</tt>.".to_string())]);
    }

    #[test]
    fn link_becomes_pango_anchor() {
        let out = transform("See [Configuration](configuration.md).");
        assert_eq!(
            out,
            vec![Block::Paragraph(
                "See <a href=\"configuration.md\">Configuration</a>.".to_string()
            )]
        );
    }

    #[test]
    fn bullet_lines_become_bullet_blocks() {
        let out = transform("- First item\n- Second item");
        assert_eq!(
            out,
            vec![
                Block::Bullet("First item".to_string()),
                Block::Bullet("Second item".to_string()),
            ]
        );
    }

    #[test]
    fn multiline_bullet_continuation_joins_with_a_space() {
        // features.md wraps long bullets onto an indented continuation
        // line, matching normal markdown paragraph-wrap conventions.
        let out = transform("- First line of the bullet\n  continues here.");
        assert_eq!(
            out,
            vec![Block::Bullet("First line of the bullet continues here.".to_string())]
        );
    }

    #[test]
    fn blank_line_separates_paragraphs() {
        let out = transform("First paragraph.\n\nSecond paragraph.");
        assert_eq!(
            out,
            vec![
                Block::Paragraph("First paragraph.".to_string()),
                Block::Paragraph("Second paragraph.".to_string()),
            ]
        );
    }

    #[test]
    fn unhandled_construct_passes_through_as_literal_text() {
        // A table row -- not one of the four handled constructs. Must not
        // panic or vanish; it renders as plain (Pango-escaped) text.
        let out = transform("| Tool | Role |");
        assert_eq!(out, vec![Block::Paragraph("| Tool | Role |".to_string())]);
    }

    #[test]
    fn angle_brackets_in_plain_text_are_escaped() {
        // Guards against literal "<" / ">" / "&" in the source breaking
        // the Pango markup the transform hands to GtkLabel::set_markup.
        let out = transform("Use `<tag>` syntax & such.");
        assert_eq!(
            out,
            vec![Block::Paragraph("Use <tt>&lt;tag&gt;</tt> syntax &amp; such.".to_string())]
        );
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p mshell-settings guide::markdown -- --nocapture`
Expected: fails on the `todo!()`.

- [ ] **Step 4: Implement the transform**

```rust
pub fn transform(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph_lines: Vec<String> = Vec::new();
    let mut bullet_lines: Vec<String> = Vec::new();

    let flush_paragraph = |blocks: &mut Vec<Block>, lines: &mut Vec<String>| {
        if !lines.is_empty() {
            blocks.push(Block::Paragraph(inline(&lines.join(" "))));
            lines.clear();
        }
    };
    let flush_bullet = |blocks: &mut Vec<Block>, lines: &mut Vec<String>| {
        if !lines.is_empty() {
            blocks.push(Block::Bullet(inline(&lines.join(" "))));
            lines.clear();
        }
    };

    for raw_line in markdown.lines() {
        let line = raw_line.trim_end();

        if line.trim().is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            flush_bullet(&mut blocks, &mut bullet_lines);
            continue;
        }

        if let Some(rest) = line.trim_start().strip_prefix("- ") {
            // A new bullet starts: flush whatever bullet was accumulating
            // (a previous item), then start this one. A bare paragraph
            // never continues into a bullet or vice versa.
            flush_bullet(&mut blocks, &mut bullet_lines);
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            bullet_lines.push(rest.trim().to_string());
            continue;
        }

        if !bullet_lines.is_empty() && line.starts_with("  ") {
            // Indented continuation of the current bullet.
            bullet_lines.push(line.trim().to_string());
            continue;
        }

        flush_bullet(&mut blocks, &mut bullet_lines);
        paragraph_lines.push(line.trim().to_string());
    }
    flush_bullet(&mut blocks, &mut bullet_lines);
    flush_paragraph(&mut blocks, &mut paragraph_lines);

    blocks
}

/// Apply the four inline constructs (bold, code, link) plus Pango-markup
/// escaping, in one left-to-right pass over the line. Escaping happens
/// character-by-character as literal text is copied through; the three
/// constructs below emit their own literal `<...>` tags directly (not
/// escaped), which is safe because none of their *content* is re-escaped
/// after being placed inside the tag -- it goes through this same
/// character loop first.
fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(end) = find_closing(&chars, i + 2, "**") {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<b>");
                out.push_str(&inline(&inner));
                out.push_str("</b>");
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '`' {
            if let Some(end) = find_closing(&chars, i + 1, "`") {
                let inner: String = chars[i + 1..end].iter().collect();
                out.push_str("<tt>");
                out.push_str(&escape(&inner));
                out.push_str("</tt>");
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(close_bracket) = find_char(&chars, i + 1, ']') {
                if chars.get(close_bracket + 1) == Some(&'(') {
                    if let Some(close_paren) = find_char(&chars, close_bracket + 2, ')') {
                        let label: String = chars[i + 1..close_bracket].iter().collect();
                        let url: String = chars[close_bracket + 2..close_paren].iter().collect();
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape(&url),
                            escape(&label)
                        ));
                        i = close_paren + 1;
                        continue;
                    }
                }
            }
        }
        escape_char_into(chars[i], &mut out);
        i += 1;
    }
    out
}

fn find_closing(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    let mut i = from;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()] == needle[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        escape_char_into(c, &mut out);
    }
    out
}

fn escape_char_into(c: char, out: &mut String) {
    match c {
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '&' => out.push_str("&amp;"),
        _ => out.push(c),
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p mshell-settings guide::markdown -- --nocapture`
Expected: all 8 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide/
git commit -m "feat(mshell-settings): markdown-subset to Pango markup transform for the Guide page"
```

### Task 8: Features tab

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide/features_tab.rs`
- Modify: `mshell-crates/mshell-settings/src/guide/mod.rs`
- Modify: `mshell-crates/mshell-settings/src/guide_settings.rs`

**Interfaces:**
- Consumes: `guide::markdown::{transform, Block}` (Task 7).
- Produces: `pub fn build() -> gtk::Widget`, appended to the stack as page name `"features"`, title `"Features"`.

- [ ] **Step 1: Add the submodule**

Add `pub mod features_tab;` to `guide/mod.rs`.

- [ ] **Step 2: Slice the two sections out of `docs/features.md` and render them**

Create `mshell-crates/mshell-settings/src/guide/features_tab.rs`:

```rust
//! Settings -> Guide -> Features: a curated tour, sliced from
//! `docs/features.md`'s "Compositor" and "Desktop shell (mshell)"
//! sections (its other two `##` sections -- a table, and a list of
//! website links -- don't fit this renderer or this page; see the design
//! spec).

use super::markdown::{self, Block};
use relm4::gtk::prelude::*;
use relm4::gtk;

const FEATURES_MD: &str = include_str!("../../../../docs/features.md");
const INCLUDED_HEADINGS: &[&str] = &["Compositor", "Desktop shell (mshell)"];

pub fn build() -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 16);
    page.set_margin_top(8);
    page.set_margin_start(4);
    page.set_margin_end(4);

    for heading in INCLUDED_HEADINGS {
        let Some(section_text) = slice_section(FEATURES_MD, heading) else {
            continue;
        };

        let heading_label = gtk::Label::new(None);
        heading_label.set_markup(&format!("<span size=\"large\" weight=\"bold\">{}</span>", heading));
        heading_label.set_halign(gtk::Align::Start);
        page.append(&heading_label);

        for block in markdown::transform(&section_text) {
            let label = gtk::Label::new(None);
            label.set_wrap(true);
            label.set_halign(gtk::Align::Start);
            label.set_xalign(0.0);
            match block {
                Block::Paragraph(markup) => label.set_markup(&markup),
                Block::Bullet(markup) => label.set_markup(&format!("•  {}", markup)),
            }
            page.append(&label);
        }
    }

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&page)
        .build();
    scroller.upcast()
}

/// Extract the body of the first `## {heading}` section (everything up to
/// the next `## ` line or end of file), excluding the heading line itself.
fn slice_section(markdown: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}");
    let start = markdown.find(&marker)?;
    let after_heading = &markdown[start + marker.len()..];
    let body_start = after_heading.find('\n').map(|i| i + 1).unwrap_or(0);
    let body = &after_heading[body_start..];
    let end = body.find("\n## ").unwrap_or(body.len());
    Some(body[..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_a_known_section_out_of_a_fixture() {
        let fixture = "\
# Title

## First

Body of first.

## Second

Body of second.
";
        assert_eq!(slice_section(fixture, "First"), Some("Body of first.".to_string()));
        assert_eq!(slice_section(fixture, "Second"), Some("Body of second.".to_string()));
    }

    #[test]
    fn missing_heading_returns_none() {
        let fixture = "## Only\n\nbody\n";
        assert_eq!(slice_section(fixture, "Nonexistent"), None);
    }

    #[test]
    fn the_two_included_headings_exist_in_the_real_file() {
        // Guards against `docs/features.md` being restructured (headings
        // renamed/removed) without this tab being updated to match --
        // fails loudly here instead of silently rendering an empty tab.
        for heading in INCLUDED_HEADINGS {
            assert!(
                slice_section(FEATURES_MD, heading).is_some(),
                "docs/features.md no longer has a \"## {heading}\" section \
                 -- update INCLUDED_HEADINGS in features_tab.rs"
            );
        }
    }
}
```

Path check (same reasoning as Task 5 — `features_tab.rs` is one directory deeper than `guide_settings.rs`):

```bash
realpath --relative-to=mshell-crates/mshell-settings/src/guide docs/features.md
```

Expected: `../../../../docs/features.md` — use whatever this actually prints.

- [ ] **Step 3: Run the new tests**

Run: `cargo test -p mshell-settings guide::features_tab -- --nocapture`
Expected: all 3 tests PASS (the third one only passes as long as `docs/features.md` still has both headings — if it fails, that's the test doing its job, not a bug in the test).

- [ ] **Step 4: Wire the tab into the stack**

In `guide_settings.rs`:

```rust
use crate::guide::features_tab;
```

In `init`, after the `config_tab` line:

```rust
widgets.stack.add_titled(&features_tab::build(), Some("features"), "Features");
```

- [ ] **Step 5: Build and verify**

Run: `cargo build -p mshell-settings && cargo clippy -p mshell-settings --all-targets -- -D warnings`
Expected: both clean.

- [ ] **Step 6: Manual verification**

`just shell`, Settings -> Guide -> Features tab shows the Compositor and Desktop shell bullet lists with bold/code/link formatting rendering correctly (not literal `**`/`` ` ``/`[...]`  characters).

- [ ] **Step 7: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide/ mshell-crates/mshell-settings/src/guide_settings.rs
git commit -m "feat(mshell-settings): Guide page Features tab"
```

### Task 9: Phase 2 full verification gate

Identical to Task 6's five steps, run again after Task 8. Push and watch CI before starting Phase 3.

---

## Phase 3: Tools tab

### Task 10: Background `--help` spawner with timeout

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide/tools.rs`
- Modify: `mshell-crates/mshell-settings/src/guide/mod.rs`

**Interfaces:**
- Produces:
  ```rust
  pub const TOOLS: &[&str] = &[ /* the 14 binary names */ ];
  pub enum HelpResult {
      Output(String),
      NotFound,
      TimedOut,
  }
  pub fn spawn_help<F>(binary: &'static str, on_done: F)
  where
      F: FnOnce(HelpResult) + Send + 'static;
  ```

- [ ] **Step 1: Add the submodule**

Add `pub mod tools;` to `guide/mod.rs`.

- [ ] **Step 2: Write it (no TDD split here — the only interesting logic, the timeout loop, isn't unit-testable without a real slow/hanging subprocess, which isn't a fixture worth building; correctness is covered by Task 11's manual verification instead)**

Create `mshell-crates/mshell-settings/src/guide/tools.rs`:

```rust
//! Settings -> Guide -> Tools: live `--help` output for margo's companion
//! binaries. None of these expose a structured catalogue the way
//! `mctl::actions` does, and several aren't confirmed to use `clap`, so
//! this runs the real binary and shows its output verbatim rather than
//! trying to parse a shape that isn't guaranteed consistent.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The companion binaries a user would actually run themselves --
/// excludes `margo`/`mshell` (not standalone CLI tools), `margo-portal`/
/// `mshellshare` (backend daemons, no user-facing CLI), `start-margo` (a
/// supervisor process), `mgreet` (a graphical greeter with no desktop
/// session to run it from), and `mvisual` (an internal debugging helper).
pub const TOOLS: &[&str] = &[
    "mctl",
    "mshellctl",
    "mlock",
    "mlogind",
    "mlayout",
    "mscreenshot",
    "mkeys",
    "mvpn",
    "mcal",
    "mtune",
    "mpicker",
    "mdots",
    "mpower",
    "mwizard",
];

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub enum HelpResult {
    Output(String),
    NotFound,
    TimedOut,
}

/// Runs `<binary> --help` on a background thread and calls `on_done` with
/// the result. `on_done` is called from that background thread, NOT the
/// GTK main thread -- callers must marshal back via
/// `ComponentSender::input` (or equivalent) themselves, same as
/// `about_settings.rs`'s `spawn_gpu`/`GpuLoaded`.
pub fn spawn_help<F>(binary: &'static str, on_done: F)
where
    F: FnOnce(HelpResult) + Send + 'static,
{
    std::thread::spawn(move || {
        on_done(run_help(binary));
    });
}

fn run_help(binary: &str) -> HelpResult {
    let mut child = match Command::new(binary)
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return HelpResult::NotFound,
    };

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                let Ok(output) = child.wait_with_output() else {
                    return HelpResult::NotFound;
                };
                // Some tools print --help to stderr instead of stdout;
                // show whichever stream actually has content, preferring
                // stdout.
                let stdout = String::from_utf8_lossy(&output.stdout);
                let text = if stdout.trim().is_empty() {
                    String::from_utf8_lossy(&output.stderr).into_owned()
                } else {
                    stdout.into_owned()
                };
                return HelpResult::Output(text);
            }
            Ok(None) => {
                if start.elapsed() >= TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return HelpResult::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return HelpResult::NotFound,
        }
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p mshell-settings`
Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide/
git commit -m "feat(mshell-settings): background --help spawner (with timeout) for the Guide page"
```

### Task 11: Tools tab

**Files:**
- Create: `mshell-crates/mshell-settings/src/guide/tools_tab.rs`
- Modify: `mshell-crates/mshell-settings/src/guide/mod.rs`
- Modify: `mshell-crates/mshell-settings/src/guide_settings.rs`

Unlike the other three tabs, this one needs its own `Component` (Relm4) rather than a plain built-once widget, because it has live state (which tool is selected, whether its `--help` has loaded yet) that changes after the tab is built. It becomes a child component of `GuideSettingsModel`.

**Interfaces:**
- Consumes: `guide::tools::{TOOLS, HelpResult, spawn_help}` (Task 10).
- Produces: `pub(crate) struct ToolsTabModel`, embedded into `GuideSettingsModel` as a Relm4 controller.

- [ ] **Step 1: Add the submodule**

Add `pub mod tools_tab;` to `guide/mod.rs` (this one stays private to the crate like the other page-adjacent modules — no `pub` needed beyond `pub(crate)` on the type itself, matching every other page's visibility).

- [ ] **Step 2: Build the child component**

Create `mshell-crates/mshell-settings/src/guide/tools_tab.rs`:

```rust
//! Settings -> Guide -> Tools: a `gtk::ListBox` of companion binaries on
//! the left, a monospace `--help` dump on the right. Its own small Relm4
//! `Component` (unlike the other three tabs, which are plain
//! build-once-and-forget widgets) because tool selection and the
//! per-tool loading state genuinely change after the tab is first shown.

use super::tools::{self, HelpResult};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;

#[derive(Debug, Clone)]
enum ToolState {
    Loading,
    Output(String),
    NotFound,
    TimedOut,
}

#[derive(Debug, Default)]
pub(crate) struct ToolsTabModel {
    selected: Option<&'static str>,
    cache: HashMap<&'static str, ToolState>,
}

#[derive(Debug)]
pub(crate) enum ToolsTabInput {
    Select(&'static str),
    HelpLoaded(&'static str, HelpResult),
}

#[derive(Debug)]
pub(crate) enum ToolsTabOutput {}

pub(crate) struct ToolsTabInit {}

#[derive(Debug)]
pub(crate) enum ToolsTabCommandOutput {}

#[relm4::component(pub(crate))]
impl Component for ToolsTabModel {
    type CommandOutput = ToolsTabCommandOutput;
    type Input = ToolsTabInput;
    type Output = ToolsTabOutput;
    type Init = ToolsTabInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_hexpand: true,
            set_vexpand: true,
            set_spacing: 8,

            gtk::ScrolledWindow {
                set_width_request: 180,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                #[name = "tool_list"]
                gtk::ListBox {
                    add_css_class: "guide-tools-list",
                    connect_row_selected[sender] => move |_, row| {
                        if let Some(row) = row {
                            let name = tools::TOOLS[row.index() as usize];
                            sender.input(ToolsTabInput::Select(name));
                        }
                    },
                },
            },

            gtk::ScrolledWindow {
                set_hexpand: true,
                #[name = "output_view"]
                gtk::TextView {
                    add_css_class: "guide-tools-output",
                    set_editable: false,
                    set_cursor_visible: false,
                    set_monospace: true,
                    set_wrap_mode: gtk::WrapMode::WordChar,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ToolsTabModel::default();
        let widgets = view_output!();
        for name in tools::TOOLS {
            let row = gtk::Label::new(Some(name));
            row.set_halign(gtk::Align::Start);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row.set_margin_start(8);
            row.set_margin_end(8);
            widgets.tool_list.append(&row);
        }
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ToolsTabInput::Select(name) => {
                self.selected = Some(name);
                if !self.cache.contains_key(name) {
                    self.cache.insert(name, ToolState::Loading);
                    let sender = sender.clone();
                    tools::spawn_help(name, move |result| {
                        let mapped = match result {
                            HelpResult::Output(text) => ToolState::Output(text),
                            HelpResult::NotFound => ToolState::NotFound,
                            HelpResult::TimedOut => ToolState::TimedOut,
                        };
                        sender.input(ToolsTabInput::HelpLoaded(name, mapped));
                    });
                }
            }
            ToolsTabInput::HelpLoaded(name, result) => {
                let mapped = match result {
                    HelpResult::Output(text) => ToolState::Output(text),
                    HelpResult::NotFound => ToolState::NotFound,
                    HelpResult::TimedOut => ToolState::TimedOut,
                };
                self.cache.insert(name, mapped);
            }
        }
    }

    fn update_view(&self, widgets: &mut Self::Widgets, _sender: ComponentSender<Self>) {
        let buffer = widgets.output_view.buffer();
        let text = match self.selected.and_then(|name| self.cache.get(name)) {
            None => String::new(),
            Some(ToolState::Loading) => "Loading…".to_string(),
            Some(ToolState::Output(text)) => text.clone(),
            Some(ToolState::NotFound) => format!(
                "\"{}\" not found on PATH.",
                self.selected.unwrap_or("")
            ),
            Some(ToolState::TimedOut) => format!(
                "\"{} --help\" didn't respond within 2 seconds.",
                self.selected.unwrap_or("")
            ),
        };
        buffer.set_text(&text);
    }
}
```

`ToolsTabModel::update_view` is Relm4's post-`update` widget refresh
hook (distinct from `view!`'s declarative bindings, needed here because
setting a `GtkTextBuffer`'s text isn't expressible as a simple property
binding the macro can track automatically) — it runs after every
`update` call, reading `self` to decide what the text view should show.

- [ ] **Step 3: Embed it as a child controller in `GuideSettingsModel`**

`guide_settings.rs` needs a place to keep `ToolsTabModel`'s controller alive (Relm4 components have to be held somewhere for their lifetime, same reasoning `about_settings.rs`... wait, `about_settings.rs` has no children — the closer precedent is `settings.rs`'s own `build_top_page`, which returns `vec![Box::new(ctrl) as Box<dyn std::any::Any>]` as a keep-alive token. `GuideSettingsModel` needs the same shape, but for an *embedded* child rather than a whole separate page):

Add a field to `GuideSettingsModel`:

```rust
#[derive(Debug)]
pub(crate) struct GuideSettingsModel {
    tools_tab: Option<relm4::Controller<crate::guide::tools_tab::ToolsTabModel>>,
}
```

(Drop the earlier `#[derive(Debug, Default)]` unit-struct version from Task 3 — this field means `Default` no longer applies; construct it explicitly in `init` instead.)

In `guide_settings.rs`, add the import:

```rust
use crate::guide::tools_tab::{ToolsTabInit, ToolsTabModel};
use relm4::ComponentController;
```

In `init`, after the `features_tab` line:

```rust
let tools_tab = ToolsTabModel::builder().launch(ToolsTabInit {}).detach();
widgets.stack.add_titled(tools_tab.widget(), Some("tools"), "Tools");
let model = GuideSettingsModel { tools_tab: Some(tools_tab) };
```

Move this whole block (and the `let model = ...` line) so it comes
*after* all four `add_titled` calls but *before* `ComponentParts { model, widgets }` — `model` has to be constructed once, after every tab (including the child controller) exists, replacing the earlier
`let model = GuideSettingsModel::default();` line from Task 3.

- [ ] **Step 4: Build and verify**

Run: `cargo build -p mshell-settings && cargo clippy -p mshell-settings --all-targets -- -D warnings`
Expected: both clean.

- [ ] **Step 5: Manual verification**

`just shell`, Settings -> Guide -> Tools tab: click through several tools in the left list. Confirm: a tool that's installed shows its real `--help` text; try temporarily renaming/hiding one binary from `PATH` (or just trust `mwizard`/whichever tool you don't have built locally) to confirm the "not found" placeholder appears instead of a frozen UI; confirm switching between tools rapidly doesn't crash or show the wrong tool's output (the cache keyed by tool name should make repeat visits instant).

- [ ] **Step 6: Commit**

```bash
git add mshell-crates/mshell-settings/src/guide/ mshell-crates/mshell-settings/src/guide_settings.rs
git commit -m "feat(mshell-settings): Guide page Tools tab"
```

### Task 12: Phase 3 full verification gate

Identical to Task 6's five steps, run again after Task 11. Push and watch CI. This closes out the feature — all four tabs are live.

---

## Self-review notes (fixed inline while writing this plan)

- Confirmed via `realpath --relative-to` that `guide_settings.rs`
  (three `..` segments) and everything under `guide/` (four `..`
  segments) resolve to genuinely different depths — Tasks 5 and 8 both
  call this out explicitly with the verification command, rather than
  assuming Task 3's three-segment path applies everywhere.
- `Action`'s catalogue is `pub const ACTIONS`, not `pub static` as an
  earlier draft of the design spec said — corrected in this plan's
  Global Constraints.
- `Group` has 8 variants including `System` (confirmed against
  `mctl/src/bin/mctl.rs`'s own print-order list) — Task 4's
  `GROUP_ORDER` includes all 8, not the 7 the spec's prose mentioned.
