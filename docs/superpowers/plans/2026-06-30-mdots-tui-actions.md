# mdots TUI — from read-only viewer to a fully-functional tool — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]` checkboxes.

**Goal:** Turn `mdots tui` from a read-only dashboard (overview/modules/packages/sync-preview) into a fully-functional TUI: act on the system (enable/disable modules, run sync, manage services/hooks/secrets), with discoverable navigation, confirmation-gated destructive actions, mouse support, and palette theming.

**Background:** The TUI (`mdots/src/tui/`) was just finished as a read-only viewer. Architecture (already in place):
- `mod.rs::run` — main loop: render → poll events (250 ms tick) → `handle_global_key` then `current_screen.handle_key` → match `ScreenAction`.
- `terminal.rs` — `init()` (raw mode + alt screen) / `restore()` (the basis for suspend/restore).
- `events.rs` — `TuiEvent::{Key,Mouse,Resize,Tick}`; **Mouse events are captured but ignored, and mouse capture is NOT enabled in `init()`**.
- `ui.rs` — renders titlebar + sidebar + content + statusbar + **`render_dialog` overlay (already wired to `app.dialog`)**.
- `app.rs` — `App` holds `dialog: Option<Dialog>` (incl. `Dialog::Confirm{title,message,confirmed}`), `status_message`, `screen_history`; sidebar index→`Screen` mapping lives in `handle_global_key`. `ScreenAction` = `{None, Back, Refresh}`.
- Screens implement `ScreenTrait { handle_key, render, on_activate }`; the `Screen` enum + `mod.rs` delegation + sidebar items must be updated together when adding a screen.

## Key architecture decisions (apply throughout)

1. **Destructive / external commands use SUSPEND→RUN→RESTORE, not an in-TUI async pane.** On a confirmed action, leave the alt screen + disable raw mode (reuse/extend `terminal.rs`), run the existing `commands::*` function inheriting the real terminal (so its normal stdout/progress/prompts work exactly as the CLI), then re-enter the alt screen + `terminal.clear()` and refresh. This is the lazygit/gitui pattern — it eliminates UI-freeze/partial-render complexity and lets us REUSE the CLI command functions verbatim. (A streamed in-pane progress view is an explicit future enhancement, NOT required here.)
2. **Every system-modifying action is gated behind an explicit `Dialog::Confirm`.** The TUI NEVER auto-runs a mutating action. The screen requests it; the loop shows the confirm; only on y/Enter does it run.
3. **Action plumbing:** extend `ScreenAction` with a `Request(Action)` variant (or add an `Action` enum + `App.pending_action`). A screen returns `ScreenAction::Request(Action::X)`; the main loop sets up the confirm dialog, and on confirm dispatches `X` via suspend→`commands::*`→restore→`show_message(result)`→refresh. Screens stay decoupled from terminal/suspend logic (which lives in `mod.rs::run`, where the `Terminal` is owned).
4. **Reuse CLI command functions** (`commands::module::{enable,disable}`, `commands::sync::run`, `commands::service::*`, `commands::secrets::{edit,sync,status}`, hooks, `commands::doctor::run`) — do NOT re-implement their logic in the TUI.

## Global Constraints

- **No `dcli`** anywhere — `grep -rni dcli mdots/src` stays 0.
- **No new non-test panics** — panic-ratchet ceiling is **364**; any non-test `.unwrap()/.expect(/panic!(`/panic-able indexing breaks the gate. Use `Result`/`?`/`unwrap_or`. (Suspend/restore must restore the terminal even on the command's error path — use a guard/`finally` style so a failed command never leaves the user in a broken raw-mode terminal.)
- **CI gate per task:** mdots-scoped `cargo fmt --all` + `cargo clippy -p mdots --all-targets -- -D warnings` + `cargo test -p mdots` + `bash scripts/panic-ratchet.sh`; controller runs full `just check` at the final review.
- **Confirmation-gated mutations** (decision #2). Read-only screens stay read-only until their action task.
- English code/comments; conventional commits; footer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- Follow the existing screen/idiom patterns (`overview.rs`); panic-free like the current code.

---

### Task 1: Navigation & discoverability (no actions)

Pure-UX pass, zero system mutation. Make the TUI navigable and self-documenting.

**Files:** `mdots/src/tui/app.rs`, `mdots/src/tui/components/statusbar.rs`, `mdots/src/tui/components/` (maybe a new `help.rs` overlay), `mdots/src/tui/screens/{modules,packages,overview}.rs`, `mdots/src/tui/ui.rs`.

**Requirements:**
- **Help overlay** — a global `?` key toggles a centered overlay listing the keybindings (global + current-screen). Render it like `render_dialog` (on top of everything); `?`/Esc closes it. Source the key list from a single place so it can't drift.
- **Per-screen footer hints** — the statusbar (`render_statusbar`, bottom 3 rows) shows the context keys for the active screen (e.g. Overview: `[r] refresh  [?] help  [q] quit`; Modules later adds `[space] toggle`). Keep it one line, dim style.
- **Consistent scroll + selection on every list** — `modules`, `packages`, and the `overview` config-tree should support `j/k`/↑↓ scrolling and (where a list is selectable) a highlighted selection, matching what `sync.rs` already does (`scroll` offset + clamp + a scroll hint). Use `ratatui` `ListState`/`TableState` or the existing manual-offset pattern consistently.
- **Modules filter** — add a `/`-to-filter text field over the modules list (mirror the packages screen's existing filter).

**Acceptance:** `?` shows a keybind overlay on every screen; the footer shows context keys; all lists scroll; modules has a `/` filter. No mutation. No new non-test panics. mdots-scoped gate green.

- [ ] Help overlay (`?`)
- [ ] Footer context-key hints
- [ ] Consistent scroll/selection across lists
- [ ] Modules `/` filter

---

### Task 2: Action infrastructure + Modules enable/disable

Build the confirm + suspend/restore action plumbing (decisions #1–#3), and prove it with the first real action: toggling a module from the Modules screen.

**Files:** `mdots/src/tui/screens/mod.rs` (`ScreenAction`), `mdots/src/tui/app.rs` (`Action` enum + `pending_action`, confirm state), `mdots/src/tui/mod.rs` (`run` loop: confirm-flow + suspend/restore dispatch), `mdots/src/tui/screens/modules.rs`, `mdots/src/tui/terminal.rs` (a `suspend`/`resume` helper or a `with_suspended` wrapper).

**Requirements:**
- Add `ScreenAction::Request(Action)` and an `Action` enum (start with `ToggleModule { name: String, enable: bool }`).
- The Modules screen: `space` (or `enter`) on the selected module returns `ScreenAction::Request(Action::ToggleModule{..})` with `enable` = !currently-enabled.
- The loop: on a `Request`, set `app.dialog = Some(Dialog::Confirm{ title, message })` describing exactly what will happen (e.g. "Enable module `zsh`? This will run a sync."), and stash the pending `Action`. On `y`/Enter → dispatch; on `n`/Esc → clear, no-op.
- **Dispatch** = suspend the TUI (leave alt screen + disable raw mode), call the existing `commands::module::{enable,disable}` (whatever the CLI uses — reuse it, it may run a sync and print normally), then **restore** (re-enter alt screen + `terminal.clear()`), then `show_message(result)` (Success/Error via `StatusMessage`) and refresh the screen so the new enabled-state shows.
- **Terminal safety:** restore MUST run even if the command returns `Err` or panics-in-the-callee path is impossible — wrap so the terminal is never left in raw/alt state. No new non-test panics.

**Acceptance:** selecting a module + `space` → confirm dialog → on confirm the module is enabled/disabled (real `commands::module` call, visible in normal terminal), then the TUI returns and reflects the change; cancel does nothing. mdots-scoped gate green. Unit-test the pure parts (e.g. the confirm-message builder, the enable-toggle decision) where present.

- [ ] `ScreenAction::Request(Action)` + `Action` enum + `pending_action`
- [ ] Confirm-flow in the loop (y/Enter/n/Esc)
- [ ] `suspend → commands::module → restore → show_message → refresh`, terminal-safe
- [ ] Modules screen `space` toggle wired

---

### Task 3: Run `sync` from the Sync screen

The headline destructive action. The Sync screen already previews install/prune; let the user execute it.

**Files:** `mdots/src/tui/screens/sync.rs`, plus the `Action` enum + dispatch (from Task 2).

**Requirements:**
- `s` (or Enter) on the Sync screen returns `ScreenAction::Request(Action::RunSync)`.
- Confirm dialog summarizes the plan counts (e.g. "Run sync? +N native, +M flatpak to install, −K to prune.").
- Dispatch: suspend → `commands::sync::run(...)` (the real sync the CLI runs — reuse it; it prints progress/prompts normally in the restored terminal) → restore → `show_message(result)` → refresh the preview (it should now show "in sync").
- Respect existing sync semantics (it may itself prompt/`--prune` based on config) — do NOT pass new flags or change sync behavior; just invoke the standard path. Terminal-safe restore.

**Acceptance:** Sync screen `s` → confirm with plan counts → real sync runs in the terminal → returns to TUI → preview refreshes to "in sync". mdots-scoped gate green.

- [ ] `s` → `Action::RunSync` + plan-count confirm
- [ ] Dispatch via `commands::sync::run`, terminal-safe
- [ ] Refresh preview after

---

### Task 4: Services screen

New sidebar screen for service profiles (the `mdots service` surface).

**Files:** new `mdots/src/tui/screens/services.rs`; `screens/mod.rs` (`Screen` enum + delegation); `app.rs` (sidebar items + index→screen mapping); `mod.rs` if needed; `Action` enum (enable/disable service).

**Requirements:**
- Add a `Services` sidebar entry + screen. List service profiles (reuse `commands::service`/the service listing logic — do not duplicate) with their enabled state.
- `space`/Enter on a profile → `Action::{EnableService,DisableService}{name}` → confirm → suspend→`commands::service::*`→restore→refresh.
- Lazy load on activate; scroll/select consistent with Task 1; footer hints.

**Acceptance:** Services screen lists profiles; toggling one runs the real service enable/disable behind a confirm; reflects the change. mdots-scoped gate green.

- [ ] Services screen + sidebar wiring
- [ ] enable/disable action (confirm + suspend/restore)

---

### Task 5: Hooks screen

New sidebar screen for hooks (the `mdots hooks` surface).

**Files:** new `mdots/src/tui/screens/hooks.rs`; `screens/mod.rs`; `app.rs` (sidebar); `Action` (run hook).

**Requirements:**
- Add a `Hooks` sidebar entry + screen. List hooks with their executed/last-run status (reuse the hooks listing logic the `hooks list` command uses).
- `r`/Enter on a hook → `Action::RunHook{name}` → confirm → suspend→run the hook (reuse the `hooks` run path)→restore→refresh status.
- Lazy load, consistent scroll/select, footer hints.

**Acceptance:** Hooks screen lists hooks + status; running one executes the real hook behind a confirm; status refreshes. mdots-scoped gate green.

- [ ] Hooks screen + sidebar wiring
- [ ] run-hook action (confirm + suspend/restore)

---

### Task 6: Secrets screen

New sidebar screen for secrets (the `mdots secrets` surface).

**Files:** new `mdots/src/tui/screens/secrets.rs`; `screens/mod.rs`; `app.rs` (sidebar); `Action` (edit/sync secret).

**Requirements:**
- Add a `Secrets` sidebar entry + screen. List declared secrets with their `SecretState` (reuse `secrets::classify_secret_status` etc. — same logic `mdots doctor`/`secrets status` use; do NOT duplicate).
- `e`/Enter on a secret → `Action::EditSecret{name}` → (edit is not destructive to the system but launches `sops`/$EDITOR) suspend→`commands::secrets::edit`→restore→refresh status. `s` → `Action::SyncSecrets` → confirm → suspend→`commands::secrets::sync`→restore→refresh.
- `edit` MUST go through suspend/restore (sops opens an interactive editor needing the full terminal).

**Acceptance:** Secrets screen shows per-secret status; `e` opens sops for the selected secret via suspend/restore and returns cleanly; `s` runs secrets sync behind a confirm. mdots-scoped gate green.

- [ ] Secrets screen + sidebar wiring
- [ ] edit (suspend→sops) + sync (confirm) actions

---

### Task 7: Doctor overlay

Run the health check from inside the TUI.

**Files:** `mdots/src/tui/app.rs` (global key), `mdots/src/tui/components/` (a doctor/report overlay or a screen), `mod.rs` dispatch; reuse `commands::doctor`.

**Requirements:**
- A global key (e.g. `D`) runs the doctor checks (reuse `commands::doctor`'s check-gathering — expose a function returning the `Vec<Check>` if it isn't already, rather than only the print path) and shows the grouped pass/warn/fail report in a scrollable overlay (read-only; no suspend needed since doctor is read-only and quick — render its results inside the TUI). Esc closes it.
- If gathering checks is only available via the print path, refactor minimally so the TUI can render the structured results (no behavior change to `mdots doctor` itself).

**Acceptance:** `D` shows the doctor report inside the TUI; Esc closes. mdots-scoped gate green.

- [ ] Doctor results exposed as data (if needed) + overlay
- [ ] global `D` wired

---

### Task 8: Polish — mouse support + palette theming

**Files:** `mdots/src/tui/terminal.rs` (`EnableMouseCapture`/`DisableMouseCapture`), `mdots/src/tui/mod.rs` (handle `TuiEvent::Mouse`), `mdots/src/tui/app.rs` (sidebar hit-testing), a theming module + the screens' color usage.

**Requirements:**
- **Mouse:** enable mouse capture in `init()` (and disable in `restore()`); handle `TuiEvent::Mouse` — click a sidebar entry to navigate, scroll-wheel to scroll the focused list. Keep keyboard fully working.
- **Theming:** replace the hardcoded `Color::Blue/Cyan/Green/Yellow` with a small theme module. If margo's matugen palette is readily readable (e.g. `~/.cache/margo/mshell-colors.toml` per the project's theme-sync, see the mlogind theme-sync convention), map it to the TUI accent/severity colors with a sane built-in fallback; otherwise define a coherent fixed palette in one module and route all screens through it. Don't hardcode colors scattered across screens.

**Acceptance:** clicking sidebar items + wheel-scroll work; colors come from one theme module (palette-derived if available). mdots-scoped gate green; controller runs full `just check`.

- [ ] Mouse capture + click/scroll handling
- [ ] Central theme module (palette-derived if available, fallback otherwise)
