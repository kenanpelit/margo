# Clipboard A+B+C+D Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans. Checkbox steps.

**Goal:** Richer `mshellctl menu clipboard`: a `mshellctl clipboard` CLI (A), smart content typing with colour swatch + per-type icons (B), source-free row polish — counts / empty-state / motion (C minus the side preview), and every knob exposed under Settings → Widgets → Clipboard incl. a new image-size cap (D).

**Architecture:** Keep the existing watcher/history + relm4 menu. Add content detection in `mshell-clipboard::entry` (pure, TDD). Add IPC verbs + bus reply for the CLI, driven against the shell's `clipboard_service()`. Surface new + existing config knobs in the Settings page. SCSS for type icons/swatch/counts/empty-state.

**Tech Stack:** Rust, GTK4/relm4, reactive_stores config, zbus IPC, SCSS.

---

## File map
- `mshell-clipboard/src/entry.rs` — extend `ClipCategory` (Url/Color/Code/Email) + content detection + `color_hex()` + tests.
- `mshell-clipboard/src/settings.rs` + `history.rs`/`watcher.rs` — `image_max_kb` gate.
- `mshell-config/src/schema/clipboard.rs` — `image_max_kb: u32` field (+ default).
- `mshell-core/src/lib.rs` — map `image_max_kb` in `clipboard_settings_from_config`.
- `mshell-core/src/ipc.rs` — `ClipboardAction(String)` (fire) + `ClipboardList` (reply) IPC verbs + bus methods.
- `mshellctl/src/subcommands/clipboard.rs` (new) + `app.rs` — `clipboard list|copy|pin|unpin|delete|clear|wipe`.
- `mshell-frame/.../clipboard/clipboard.rs` — category tabs/badge/icon + colour swatch + char/line count + empty-state; honour `ClipboardAction`/`ClipboardList`.
- `mshell-settings/src/clipboard_settings.rs` — ensure all knobs + image_max_kb row.
- `mshell-style/scss/04-components/_clipboard.scss` — type icon/swatch/count/empty-state + motion.

---

## A. `mshellctl clipboard` CLI

### Task A1: IPC verbs + bus methods
- [ ] In `mshell-core/src/ipc.rs`: add `ClipboardAction(String)` to `IPCCommand` (fire: `"copy <id>"|"pin <id>"|"unpin <id>"|"delete <id>"|"clear"|"wipe"`) routed to a new `ShellInput::ClipboardAction(spec)`, and `ClipboardList` reply verb returning a JSON string (`id\tcategory\tpreview` lines or JSON array). Add the zbus methods `clipboard_action(&self, spec)` and `clipboard_list(&self) -> String` mirroring the existing `screenshot_capture` / reply pattern.
- [ ] In the shell handler (`relm_app.rs`): `ClipboardAction(spec)` parses the verb and calls `mshell_clipboard::clipboard_service().history()` ops (`get`+copy, `toggle_pin`, `remove`, `clear`, `clear`/wipe). `ClipboardList` serialises `entries()` to JSON.
- [ ] Build `-D warnings`.

### Task A2: mshellctl subcommand
- [ ] Create `mshellctl/src/subcommands/clipboard.rs`: clap `ClipboardCommands { List{json}, Copy{id}, Pin{id}, Unpin{id}, Delete{id}, Clear, Wipe }`; `list` uses `bus_command_with_reply("ClipboardList")` and prints; the rest use `bus_command_with_arg("ClipboardAction", spec)`.
- [ ] Register in `mshellctl/src/app.rs` (subcommand enum + dispatch).
- [ ] Build `-D warnings` + commit.

---

## B. Smart content typing

### Task B1: detection (pure, TDD)
- [ ] In `entry.rs`: extend `ClipCategory` with `Url, Color, Code, Email` (keep Text/Image/File). Add `fn detect_text_category(s: &str) -> ClipCategory` (Color: `#rgb`/`#rrggbb`/`rgb(...)`; Url: starts `http://`/`https://`/`www.`; Email: one `@` + a dot in domain, no spaces; Code: multi-line with `{};`/indentation or leading `$ `; else Text) + `fn color_hex(&self) -> Option<String>`. Wire `category()` to call `detect_text_category` for text. Tests for each.
- [ ] Build + test.

### Task B2: menu typing UI
- [ ] In `clipboard.rs`: per-row type icon (Url→`web-browser-symbolic`, Color→swatch box painted via a per-row CssProvider, Code→`text-x-script-symbolic`, Email→`mail-message-symbolic`, Image→thumbnail, Text→`edit-paste-symbolic`); add Url/Color tabs (extend `ClipTab` + `matches_cat`). Colour rows show a swatch.
- [ ] Build `-D warnings` + commit (A+B together if cohesive).

---

## C (minus preview). Row polish

### Task C1: counts + empty-state + motion
- [ ] `clipboard.rs`: show a dim `N chars · M lines` caption for text rows (compute in `ClipRow::from_entry`); render an empty-state placeholder (icon + "Clipboard history is empty / No matches") when the filtered list is empty.
- [ ] `_clipboard.scss`: style the type icon, swatch, count caption, empty-state; add row hover/selection motion (`--motion-fast`).
- [ ] Build `-D warnings` + commit.

(Source-app badge is **out**: entries don't track the source app — would need watcher plumbing beyond this scope.)

---

## D. Settings → Widgets → Clipboard — all knobs

### Task D1: image_max_kb knob
- [ ] `schema/clipboard.rs`: add `pub image_max_kb: u32` (default `0` = no cap) + Default.
- [ ] `mshell-clipboard/src/settings.rs`: add `image_max_kb: u32`; `watcher.rs`/`history.rs` skips image entries whose `data.len() > image_max_kb*1024` when cap > 0.
- [ ] `mshell-core/src/lib.rs` `clipboard_settings_from_config`: map it.
- [ ] Build.

### Task D2: expose every knob in the Settings page
- [ ] `clipboard_settings.rs`: confirm rows exist for max_entries, persist, clear_policy, clear_after_hours, skip_sensitive, image_history, density; add a SpinRow for `image_max_kb` ("Max image size (KB, 0 = no limit)"). Each writes `config.clipboard.*` via `config_manager().update_config` + `apply_clipboard_config()`.
- [ ] Build `-D warnings` + commit.

---

## Final
- [ ] `cargo fmt --all`; `RUSTFLAGS="-D warnings" cargo build -p mshell-clipboard -p mshell-config -p mshell-core -p mshell-frame -p mshell-settings -p mshell-style -p mshellctl`; `cargo test -p mshell-clipboard`; `cargo build -p mshell -p mshellctl`; push.

## Self-Review
Covers A (A1-A2), B (B1-B2 + swatch reusing the launcher technique), C-minus-preview (C1; source badge explicitly deferred with reason), D (D1-D2, all knobs + new cap). Types: `ClipCategory{Text,Image,File,Url,Color,Code,Email}`, `detect_text_category`, `color_hex`, `image_max_kb` consistent across crate/schema/core/settings.
