# App Launcher Professional Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign `mshellctl menu app-launcher` into a polished, walker-class launcher: a side **preview pane**, **per-provider rich rows**, comfortable density, refined motion/typography — all on DESIGN.md tokens.

**Architecture:** Keep the existing relm4 component + provider runtime. Add a `LauncherPreview` value + a `Provider::preview()` trait method with a sensible **default** (derives from the item), overridden by calc/clipboard/apps for richness — so no per-provider churn. Per-provider row *variants* are driven off the existing `LauncherItem::provider_name` (one match in the row builder + CSS modifier classes). The view grows a two-zone content row (list + preview); preview hides when empty so the list stays full-width.

**Tech Stack:** Rust, relm4 0.9 / GTK4, reactive_stores, the `mshell-launcher` provider trait, SCSS (grass) → baked CSS.

---

## File Structure

- `mshell-crates/mshell-launcher/src/preview.rs` — **new.** `LauncherPreview` struct + `PreviewKind` enum (Text / Mono / Color). Unit-tested pure constructors.
- `mshell-crates/mshell-launcher/src/lib.rs` — **modify.** `mod preview; pub use preview::{LauncherPreview, PreviewKind};`
- `mshell-crates/mshell-launcher/src/provider.rs` — **modify.** Add `fn preview(&self, item: &LauncherItem) -> Option<LauncherPreview>` with a default that returns a Text preview from the item's name + description.
- `mshell-crates/mshell-launcher/src/providers/{calc,clipboard}.rs` + apps provider — **modify.** Override `preview()` for richness.
- `mshell-crates/mshell-frame/src/menus/menu_widgets/app_launcher/launcher_row.rs` — **modify.** Add a `.row-<provider>` modifier class + variant layout (calc: big result; clipboard: preview text/colour swatch).
- `mshell-crates/mshell-frame/src/menus/menu_widgets/app_launcher/app_launcher.rs` — **modify.** Two-zone content row (list + preview box); update preview on selection change; hide when no preview.
- `mshell-crates/mshell-style/scss/04-components/_app_launcher.scss` — **modify.** Density bump, two-zone layout, preview pane, per-provider row variants, refined motion.

---

## Task 1: `LauncherPreview` value type

**Files:**
- Create: `mshell-crates/mshell-launcher/src/preview.rs`
- Modify: `mshell-crates/mshell-launcher/src/lib.rs`

- [ ] **Step 1: failing test** (append to new file)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_preview_carries_title_and_body() {
        let p = LauncherPreview::text("Firefox", "Web browser");
        assert_eq!(p.title, "Firefox");
        assert_eq!(p.body, "Web browser");
        assert!(matches!(p.kind, PreviewKind::Text));
    }

    #[test]
    fn color_preview_keeps_the_swatch_hex() {
        let p = LauncherPreview::color("#ff8800", "#ff8800 — copied");
        assert_eq!(p.swatch.as_deref(), Some("#ff8800"));
        assert!(matches!(p.kind, PreviewKind::Color));
    }

    #[test]
    fn mono_preview_is_monospace() {
        let p = LauncherPreview::mono("12 * 8", "96");
        assert!(matches!(p.kind, PreviewKind::Mono));
    }
}
```

- [ ] **Step 2: run, expect fail**

Run: `cargo test -p mshell-launcher preview:: 2>&1 | tail`
Expected: FAIL — `LauncherPreview` not found.

- [ ] **Step 3: implement** (prepend)

```rust
//! Detail/preview shown in the launcher's side pane for the selected
//! result. Providers return one from `Provider::preview`; the default
//! impl derives a plain `Text` preview from the item, and providers
//! that benefit (calc, clipboard, apps) override it for richness.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewKind {
    /// Body rendered as ordinary wrapped text.
    Text,
    /// Body rendered monospace (calc result, file path, code).
    Mono,
    /// Body is a colour value; the pane also paints a swatch.
    Color,
}

/// Content for the launcher preview pane.
#[derive(Debug, Clone)]
pub struct LauncherPreview {
    pub title: String,
    pub body: String,
    pub kind: PreviewKind,
    /// `#rrggbb` swatch to paint (only meaningful for `Color`).
    pub swatch: Option<String>,
}

impl LauncherPreview {
    pub fn text(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self { title: title.into(), body: body.into(), kind: PreviewKind::Text, swatch: None }
    }
    pub fn mono(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self { title: title.into(), body: body.into(), kind: PreviewKind::Mono, swatch: None }
    }
    pub fn color(swatch: impl Into<String>, body: impl Into<String>) -> Self {
        let swatch = swatch.into();
        Self { title: swatch.clone(), body: body.into(), kind: PreviewKind::Color, swatch: Some(swatch) }
    }
}
```

- [ ] **Step 4: register module** — in `lib.rs`, beside the other `mod`/`pub use`:

```rust
mod preview;
pub use preview::{LauncherPreview, PreviewKind};
```

- [ ] **Step 5: run, expect pass**

Run: `cargo test -p mshell-launcher preview:: 2>&1 | tail`
Expected: PASS (3 tests).

---

## Task 2: `Provider::preview` trait method (default + 3 overrides)

**Files:**
- Modify: `mshell-crates/mshell-launcher/src/provider.rs`
- Modify: calc provider, clipboard provider, apps provider

- [ ] **Step 1: add to the trait** (after `alt_action`)

```rust
    /// Detail shown in the launcher's preview pane for `item` when it
    /// is the selection. Default: a plain text preview from the item's
    /// name + description. Providers override for richer content
    /// (calc result, clipboard payload, colour swatch).
    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        if item.description.trim().is_empty() {
            Some(crate::LauncherPreview::text(item.name.clone(), String::new()))
        } else {
            Some(crate::LauncherPreview::text(
                item.name.clone(),
                item.description.clone(),
            ))
        }
    }
```

- [ ] **Step 2: build** — `cargo build -p mshell-launcher 2>&1 | tail`. Expected: compiles (default covers all providers).

- [ ] **Step 3: calc override** — in the calc provider, add a `preview` impl that shows the expression + big result. Locate it first: `grep -rn "fn search" mshell-crates/mshell-launcher/src/providers/calc.rs`. Add inside its `impl Provider`:

```rust
    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        // calc items carry the expression in `description`, the result in `name`.
        Some(crate::LauncherPreview::mono(item.description.clone(), item.name.clone()))
    }
```

(If calc's field mapping is reversed, swap — verify against its `search()` which builds the `LauncherItem`.)

- [ ] **Step 4: clipboard override** — colour entries get a swatch, the rest show the full payload mono. In the clipboard provider's `impl Provider`:

```rust
    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        let text = item.name.clone();
        let hex = text.trim();
        let is_hex = (hex.len() == 7 || hex.len() == 4)
            && hex.starts_with('#')
            && hex[1..].chars().all(|c| c.is_ascii_hexdigit());
        if is_hex {
            Some(crate::LauncherPreview::color(hex.to_string(), format!("{hex} — Enter to copy")))
        } else {
            Some(crate::LauncherPreview::mono("Clipboard".to_string(), text))
        }
    }
```

- [ ] **Step 5: apps override** — show comment + a hint line. In the apps provider (`apps_provider.rs` under mshell-frame, OR the launcher crate's apps provider — verify which the launcher uses) `impl Provider`:

```rust
    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        let body = if item.description.trim().is_empty() {
            "Application".to_string()
        } else {
            item.description.clone()
        };
        Some(crate::LauncherPreview::text(item.name.clone(), body))
    }
```

- [ ] **Step 6: build** — `cargo build -p mshell-launcher -p mshell-frame 2>&1 | tail`. Expected: compiles.

---

## Task 3: per-provider row variants (`launcher_row.rs`)

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/app_launcher/launcher_row.rs`

- [ ] **Step 1: add a provider modifier class** — where the row's root box is created/returned, add a CSS class derived from `provider_name` so SCSS can target it. Find the root widget construction (`grep -n "app-launcher-item" launcher_row.rs`) and add, right after the root box is built:

```rust
    // Per-provider styling hook: `.row-apps`, `.row-calc`, `.row-clipboard`…
    // Lowercased + sanitised so the class is always a valid CSS ident.
    let variant = format!(
        "row-{}",
        item.item.provider_name
            .to_ascii_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    );
    root.add_css_class(&variant);
```

(Use the actual root-widget binding name from the file; `item` is the `DisplayItem`.)

- [ ] **Step 2: calc big-result layout** — calc rows should show the result prominently. The cleanest variant-agnostic approach: give the title label a class the SCSS can size up per-variant. Add to the title `gtk::Label`:

```rust
    .add_css_class("app-launcher-item-title");
```

and the subtitle:

```rust
    .add_css_class("app-launcher-item-sub");
```

(SCSS Task 6 makes `.row-calc .app-launcher-item-title` large/mono.)

- [ ] **Step 3: build** — `cargo build -p mshell-frame 2>&1 | tail`. Expected: compiles.

---

## Task 4: preview-pane state + model

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/app_launcher/app_launcher.rs`

- [ ] **Step 1: model field** — add a current-preview field to the launcher model struct:

```rust
    /// Preview for the selected result (right pane). `None` hides the pane.
    current_preview: Option<mshell_launcher::LauncherPreview>,
```

Initialise it `None` wherever the model is constructed (the `init`/default site — find with `grep -n "current_selection\|selected_index\|fn init" app_launcher.rs`).

- [ ] **Step 2: recompute on selection change** — find where the selected index changes (search/nav handlers update a `selected`/`current` index). Add a helper on the model:

```rust
    fn refresh_preview(&mut self) {
        self.current_preview = self
            .selected_display_item()
            .and_then(|di| self.runtime.borrow().preview_for(&di.item));
    }
```

where `selected_display_item()` returns the currently highlighted `&DisplayItem` (adapt to the existing selection accessor). Call `self.refresh_preview();` at the end of every handler that changes the selection or the result list (nav up/down, query change, category change, quick-key focus).

- [ ] **Step 3: runtime helper** — the runtime owns providers; add a `preview_for` that asks the item's provider. In `mshell-launcher/src/runtime.rs` `impl Runtime` (find with `grep -n "pub fn " runtime.rs`):

```rust
    /// Preview for `item`, asked of the provider that produced it.
    pub fn preview_for(&self, item: &crate::LauncherItem) -> Option<crate::LauncherPreview> {
        self.providers
            .iter()
            .find(|p| p.name() == item.provider_name)
            .and_then(|p| p.preview(item))
    }
```

(Adapt `self.providers` to the runtime's actual provider collection field; if providers are `RefCell`-wrapped, borrow accordingly.)

- [ ] **Step 4: build** — `cargo build -p mshell-launcher -p mshell-frame 2>&1 | tail`. Expected: compiles.

---

## Task 5: preview-pane UI (two-zone content)

**Files:**
- Modify: `mshell-crates/mshell-frame/src/menus/menu_widgets/app_launcher/app_launcher.rs`

- [ ] **Step 1: wrap list + preview in a horizontal box** — find the `gtk::ScrolledWindow` that holds the result list in the `view!` macro. Wrap it so the scrolled list and a new preview box are siblings of a horizontal container:

```rust
            gtk::Box {
                add_css_class: "app-launcher-content",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 0,
                set_vexpand: true,

                // existing ScrolledWindow (result list) goes here, with
                // set_hexpand: true so it fills when the preview is hidden.

                gtk::Box {
                    add_css_class: "app-launcher-preview",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                    #[watch]
                    set_visible: model.current_preview.is_some(),
                    set_width_request: 220,

                    gtk::Label {
                        add_css_class: "app-launcher-preview-title",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_label: model.current_preview.as_ref().map(|p| p.title.as_str()).unwrap_or(""),
                    },
                    #[name = "preview_swatch"]
                    gtk::Box {
                        add_css_class: "app-launcher-preview-swatch",
                        set_height_request: 44,
                        #[watch]
                        set_visible: model
                            .current_preview
                            .as_ref()
                            .map(|p| matches!(p.kind, mshell_launcher::PreviewKind::Color))
                            .unwrap_or(false),
                    },
                    gtk::Label {
                        add_css_class: "app-launcher-preview-body",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_vexpand: true,
                        set_valign: gtk::Align::Start,
                        #[watch]
                        set_css_classes: match model.current_preview.as_ref().map(|p| &p.kind) {
                            Some(mshell_launcher::PreviewKind::Mono) => &["app-launcher-preview-body", "mono"],
                            _ => &["app-launcher-preview-body"],
                        },
                        #[watch]
                        set_label: model.current_preview.as_ref().map(|p| p.body.as_str()).unwrap_or(""),
                    },
                },
            },
```

- [ ] **Step 2: paint the colour swatch** — `#[watch]` can't set an inline style, so paint via a draw func or inline CSS provider in `update`/a post-init hook. Simplest: in the handler after `refresh_preview()`, set the swatch box's background through a per-widget CSS provider. Add a helper using a stored `gtk::CssProvider`:

```rust
    fn apply_swatch(&self, widgets: &AppLauncherWidgets) {
        let hex = self
            .current_preview
            .as_ref()
            .and_then(|p| p.swatch.clone())
            .unwrap_or_default();
        let css = if hex.is_empty() {
            String::new()
        } else {
            format!(".app-launcher-preview-swatch {{ background-color: {hex}; }}")
        };
        self.swatch_css.load_from_data(&css);
    }
```

with `swatch_css: gtk::CssProvider` on the model, added once to the swatch widget's style context in `init` at `gtk::STYLE_PROVIDER_PRIORITY_APPLICATION`. (If threading `widgets` into `update` is awkward in this component shape, instead store the swatch `gtk::Box` on the model and mutate it directly in `refresh_preview`.)

- [ ] **Step 3: list fills when preview hidden** — ensure the result `ScrolledWindow` has `set_hexpand: true` so it expands to full width whenever the preview box is `set_visible: false`.

- [ ] **Step 4: build** — `cargo build -p mshell-frame 2>&1 | tail`. Expected: compiles.

---

## Task 6: SCSS — density, two-zone, preview pane, variants, motion

**Files:**
- Modify: `mshell-crates/mshell-style/scss/04-components/_app_launcher.scss`

- [ ] **Step 1: comfortable row density** — replace the cramped `.app-launcher-item` padding:

```scss
.app-launcher-item {
    padding: var(--space-2) var(--space-3);
    margin: 2px 0;
    image { -gtk-icon-size: var(--icon-md); }
}
.app-launcher-item.row-apps image,
.app-launcher-item.row-windows image { -gtk-icon-size: var(--icon-lg); }
```

- [ ] **Step 2: title/subtitle classes**

```scss
.app-launcher-item-title { font-weight: 600; }
.app-launcher-item-sub {
    color: var(--on-surface-variant);
    opacity: 0.7;
    font-size: var(--font-xs);
}
```

- [ ] **Step 3: calc variant (big result)**

```scss
.app-launcher-item.row-calc {
    .app-launcher-item-title {
        font-size: var(--font-xl);
        font-family: monospace;
        font-feature-settings: "tnum";
    }
    .app-launcher-item-sub { font-family: monospace; }
}
```

- [ ] **Step 4: two-zone content + preview pane**

```scss
.app-launcher-content { gap: var(--space-3); }

.app-launcher-preview {
    background-color: var(--surface-container-lowest);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    box-shadow: inset 0 1px 3px rgba(0, 0, 0, 0.22);

    .app-launcher-preview-title {
        font-size: var(--font-md);
        font-weight: 600;
        color: var(--on-surface);
    }
    .app-launcher-preview-body {
        color: var(--on-surface-variant);
        font-size: var(--font-sm);
        &.mono { font-family: monospace; font-feature-settings: "tnum"; }
    }
    .app-launcher-preview-swatch {
        border-radius: var(--radius-sm);
        box-shadow: inset 0 0 0 1px unquote("color-mix(in srgb, var(--on-surface) 12%, transparent)");
    }
}
```

- [ ] **Step 5: build + restart note** — SCSS is baked at build time. `cargo build -p mshell-style -p mshell 2>&1 | tail`. Expected: compiles.

---

## Task 7: Final — verify, fmt, clippy (-D warnings), commit + push

- [ ] **Step 1:** `cargo fmt --all`
- [ ] **Step 2:** `cargo test -p mshell-launcher preview:: 2>&1 | tail` → preview tests pass.
- [ ] **Step 3:** `RUSTFLAGS="-D warnings" cargo build -p mshell-launcher -p mshell-frame -p mshell-style 2>&1 | tail` → clean (mirrors CI; catches deprecations).
- [ ] **Step 4:** `cargo clippy -p mshell-launcher -p mshell-frame 2>&1 | grep -E "warning:|error:" | grep -iE "launcher|preview" | head` → no new warnings.
- [ ] **Step 5:** `cargo build -p mshell 2>&1 | tail` → shell binary links.
- [ ] **Step 6:** `cargo fmt --all --check && echo FMT-OK`
- [ ] **Step 7:** Commit + push:

```bash
git add -A
git commit -m "feat(app-launcher): preview pane + per-provider rich rows + density/motion polish"
git push
```

---

## Self-Review

**Spec coverage (full package A–H):**
- A principles (tokens/surfaces/shape/density) → Task 6 throughout. ✓
- B two-zone shell + preview → Task 5. ✓
- C per-provider rich rows → Task 3 + Task 6 variants. ✓
- D preview pane → Tasks 1/2/4/5. ✓
- E density/type/icons → Task 6 steps 1-3. ✓
- F motion → existing transitions kept; row transitions in `.app-launcher-item` (already present) retained. ✓ (no new motion regressions)
- G a11y/keyboard → existing keybinds unchanged; preview is read-only and label-based (accessible names inherit). ✓
- H SCSS → Task 6. ✓

**Placeholder scan:** the only soft spots are "adapt to the existing accessor/field name" notes in Tasks 3-5 — unavoidable because exact bindings live in 1249-line files; each names the exact `grep` to find the binding. No TBD logic.

**Type consistency:** `LauncherPreview{title,body,kind,swatch}` + `PreviewKind{Text,Mono,Color}` used identically in preview.rs, provider.rs default + overrides, runtime `preview_for`, model `current_preview`, and the view. `preview_for` returns `Option<LauncherPreview>`; model holds `Option<LauncherPreview>`. Consistent.
