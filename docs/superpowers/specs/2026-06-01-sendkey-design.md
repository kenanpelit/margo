# sendkey — synthetic key injection dispatch action (design)

**Date:** 2026-06-01
**Status:** approved (brainstorm), pending implementation plan

## Goal

A native margo dispatch action, `sendkey`, that injects a synthetic key combo
(e.g. `ctrl+Tab`) into the focused window — optionally only when the focused
`app_id` matches a regex, with an optional fallback action otherwise. Lets a
3-finger touchpad gesture switch browser tabs (the user's old fusuma +
fusuma-plugin-sendkey workflow) without any external tool, uinput, or daemon.

## Why native (not ydotool/wtype)

margo owns the seat keyboard, so it can forward synthetic key events straight to
the focused surface. It does **not** implement the virtual-keyboard protocol
(only virtual_pointer), so `wtype` can't work; `ydotool` would need a daemon +
uinput permissions. A native action is self-contained, reusable for any
"send a key to the focused app" bind, and matches margo's first-party trajectory
(margo-mpv/osc-media/power-profile were all brought in-house).

## Config grammar

`sendkey` reuses the standard bind/gesturebind arg slots (`build_arg`):

```
sendkey,<combo>[,<appid-regex>][,<fallback>]
```
- **combo** → `arg.v`: `+`-joined modifiers + one key, xkb key names matching the
  bind grammar (`ctrl+Tab`, `ctrl+shift+Tab`, `ctrl+Page_Up`). Modifiers:
  `ctrl|shift|alt|super` (aliases `control`, `meta`, `mod`, `logo`).
- **appid-regex** → `arg.v2` (optional): inject only when the focused window's
  `app_id` matches (Rust `regex`, anchored as the user writes it, no lookahead —
  same engine as windowrule). Empty/absent → always inject.
- **fallback** → `arg.v3` (optional): `action[:arg]` to run when the regex is
  present and does NOT match (e.g. `focusdir:up`). Absent → no-op on no match.

Examples (the user's fusuma muscle memory):
```
gesturebind = NONE,down,3,sendkey,ctrl+Tab,^(Kenp|Ai|CompecTA|Nil|webcord)$,focusdir:down
gesturebind = NONE,up,3,sendkey,ctrl+shift+Tab,^(Kenp|Ai|CompecTA|Nil|webcord)$,focusdir:up
```
Also usable as a keybind: `bind = ctrl,Prior,sendkey,ctrl+Page_Up,^firefox$`.

## Architecture

New module `margo/src/dispatch/sendkey.rs`:

- **Pure (unit-tested):**
  - `parse_combo(&str) -> Option<KeyCombo>` where `KeyCombo { mods: Vec<u32>,
    key: u32 }` are **evdev keycodes**. Splits on `+`; last token = key, others =
    modifiers. Modifier names → their left-variant evdev codes
    (ctrl→29, shift→42, alt→56, super→125). Key name → evdev code via
    `key_name_to_evdev`.
  - `key_name_to_evdev(&str) -> Option<u32>`: a built-in table of
    **layout-independent** keys — Tab(15), Return(28), Escape(1), space(57),
    Page_Up/Prior(104), Page_Down/Next(109), Home(102), End(107),
    Left/Right/Up/Down(105/106/103/108), F1-F12(59-88), BackSpace(14),
    Delete(111), Insert(110), digits 0-9, plus the modifier names. Case-tolerant.
    (Letter symbols are layout-dependent; out of scope for v1 — tab navigation
    needs none of them.)
  - `appid_matches(focused_app_id: Option<&str>, regex: &str) -> bool` (compiles
    the regex; non-match / bad regex → false).
- **Impure (on `MargoState`):**
  - `fn send_key(&mut self, arg: &Arg)`:
    1. Parse `arg.v` combo → bail (log) if unparseable.
    2. If `arg.v2` is a non-empty regex: get the focused window's `app_id`; if it
       doesn't match → run the `arg.v3` fallback (`dispatch_action`) and return.
       (Empty regex → skip the check, always inject.)
    3. Inject the sequence to the focused surface (see below).
  - `fn inject_combo(&mut self, combo: &KeyCombo)`: press each modifier, press the
    key, release the key, release each modifier (reverse order). For each step
    call `keyboard.input::<(), _>(self, (code+8).into(), state, serial, time,
    |_,_,_| FilterResult::Forward)` — Forward so the event reaches the client and
    does NOT re-trigger compositor binds. Fresh `SERIAL_COUNTER` serial + a
    monotonic `time_msec` per event. Mirrors `handle_keyboard`'s real path.

`dispatch/mod.rs`: add arm `"sendkey" => state.send_key(arg)`.

### Fallback parsing
`arg.v3` = `"action[:arg]"`. Split once on `:` → `(action, opt_arg)`. Build a sub
`Arg` with `v = opt_arg` (covers `focusdir:up`, `spawn:...` etc.), then
`dispatch::dispatch_action(self, action, &sub_arg)`. Self-reference guard: if
`action == "sendkey"`, ignore (no recursion).

## Data flow
```
gesture/key → dispatch_action("sendkey", arg)
  → send_key: parse combo; (regex? match focused app_id)
     ├─ match / no regex → inject_combo → keyboard.input ×N (Forward) → focused client
     └─ no match → dispatch_action(fallback action, sub_arg)
```

## Error handling
- Bad combo / unknown key name → log `tracing::warn!`, no-op.
- No focused keyboard / no focus → no-op.
- Bad regex → treated as no-match (runs fallback / no-op).

## Testing
- Unit (pure): `parse_combo` (mods+key → codes; bad input → None; case), the
  key-name table (Tab/Page_Up/F-keys), `appid_matches` (match, no-match, None,
  invalid regex). Fallback-spec split (`focusdir:up` → ("focusdir","up")).
- Injection + gesture wiring: verified live on a margo session.
- Gates: `cargo clippy -p margo --all-targets -D warnings`, `cargo test -p margo`.

## Integration
- `mctl actions` catalogue: add `sendkey` (so `mctl actions` lists it and
  `mctl check-config` doesn't flag it — update the action list/validator if one
  exists; verify during impl).
- Docs: a short note in the keybind/action docs + `mctl dispatch sendkey
  ctrl+Tab` works for scripting.
- Dotfiles `config.conf`: add the two 3-finger gesturebinds (browser app-id
  regex from the user's clients: Kenp|Ai|CompecTA|Nil|webcord + common browser
  ids), fallback `focusdir:up`/`down`. Separate dotfiles commit; `mctl
  check-config` clean.

## Risks
- Layout-dependent letter keys excluded in v1 (tab nav doesn't need them; can add
  a layout-aware path later).
- Synthetic events use Forward + fresh serials so margo's own binds never
  re-fire; if a client tracks key-repeat by device this is a discrete tap (fine
  for Ctrl+Tab).
