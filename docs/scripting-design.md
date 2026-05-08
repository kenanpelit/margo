# Plugin / scripting system — design notes

> Status: **foundation landed**. `~/.config/margo/init.rhai` is
> evaluated at compositor startup with one bound function (`spawn`)
> and three no-op event-hook stubs. This document captures the
> roadmap toward a full event-driven plugin system.

## Why this exists

Two recurring user requests sit just outside what the config file
plus `mctl` can do:

1. **Reactive automation** — "when Spotify opens, send it to tag 8";
   "when I focus a fullscreen video, dim other monitors"; "when the
   external monitor disconnects, move all clients to eDP-1". The
   config's `windowrule` covers the simplest cases at map time, but
   nothing fires on focus/unmap/output-change events.
2. **User-defined dispatch** — combining several actions into one
   bind ("toggle this layout AND set this gap AND fullscreen the
   focused window"). Today this needs a shell wrapper around
   `mctl dispatch`, with one IPC roundtrip per call.

A scripting layer fixes both: events call into user code, and the
script can call back into compositor actions in-process.

## Why Rhai

Evaluated three options:

| Option   | Embed cost | Bindings | Sandbox | Familiarity |
|----------|-----------|----------|---------|-------------|
| **Rhai** | pure Rust, ~300KB | type-safe via `register_fn` | strict-by-default, opt-in | small but growing |
| **Lua (mlua)** | C dep (lua5.4), ~200KB + libc | manual stack juggling, less type safety | needs explicit `package.path` clamping | huge — every WM user knows Lua |
| **Bespoke DSL** | none | trivial | trivial | nobody knows it |

Rhai wins on integration cost — adding it doesn't require a build-
time C toolchain change, the sandbox is opt-in tight by default,
and bindings register with normal Rust function signatures (no
manual lua_pushstring / lua_tonumber dance).

The cost: Lua's familiarity. A user who already writes AwesomeWM /
Neovim configs reaches for Lua first. Rhai's syntax is close
enough to JS/Rust that the learning curve is shallow, but it's
a real ergonomic hit. Mitigated long-term by example scripts in
`contrib/scripts/` showing common patterns.

A bespoke DSL would have minimal embed cost but no ecosystem,
no editor support, and we'd own every parser bug forever. Hard pass.

## Phased rollout

### Phase 1 — Engine boot + spawn (this commit)

* `margo/src/scripting.rs` adds Rhai dependency.
* `init_engine()` builds a sandboxed engine. Limits: 64-deep
  call stack, 32-deep expression nesting, 1024-element arrays,
  64KB strings, strict variables.
* `run_user_init()` evaluates `~/.config/margo/init.rhai`
  (or `$XDG_CONFIG_HOME/margo/init.rhai`) once at startup.
  Errors are logged with line numbers, never panic.
* Single binding: `spawn(cmd)` — equivalent to the config-file
  `spawn` action.
* Forward-compat stubs: `on_focus_change(fn)`, `on_tag_switch(fn)`,
  `on_window_open(fn)` are accepted (logged) but no-op. Users can
  write scripts today that will start firing once Phase 2 lands.

### Phase 2 — Action bindings (next sprint)

* `dispatch(action: string, args: [...])` — call any registered
  margo action. Replaces shell scripts that wrap `mctl dispatch`.
* `tag(n: int)` / `tagview(n: int)` — convenience wrappers.
* `current_tag() -> int`, `focused_appid() -> string?`,
  `monitors() -> [Monitor]` — read-only state introspection.
* Threading: scripts run on the compositor thread, synchronously.
  Heavy work in scripts blocks the event loop; document this.

### Phase 3 — Event hooks (multi-sprint)

* Engine moves into `MargoState` so callbacks survive past startup.
* `on_focus_change(fn(client))` fires from the focus-update site
  in `state.rs::activate_window`.
* `on_tag_switch(fn(tag))` fires from `view_tag`.
* `on_window_open(fn(client))` fires from `finalize_initial_map`.
* `on_window_close(fn(client))` fires from `unmap`.
* `on_output_change(fn(monitor))` fires from output add/remove.
* Each callback receives a frozen state snapshot (Rust struct
  exposed as Rhai object). Mutating snapshots does not mutate
  compositor state — to make a change, the script calls a
  binding. (Mirrors how Wayland clients can't mutate compositor
  state except via protocol calls.)

### Phase 4 — `mctl run <script>` (small)

* `mctl run path/to/script.rhai` sends the script source over IPC.
* Compositor receives via dwl-ipc-v2 extension command, evals in a
  fresh scope. Useful for one-shot automation without touching
  init.rhai.
* Caveat: only available locally (Unix socket); remote eval would
  be a security hole.

### Phase 5 — Plugin packaging (deferred)

* `~/.config/margo/plugins/<name>/init.rhai` auto-loaded.
* `mctl plugin {list,enable,disable,install <git-url>}` for
  package management.
* Plugins can register their own dispatch actions which then show
  up in `mctl actions`.
* Most experimental WMs that ship plugin systems regret it within
  a year — third-party plugins drift, mis-handle compositor API
  changes, get blamed on the compositor for crashes. So this
  phase is gated on real user demand, not delivered speculatively.

## Sandbox posture

Even Phase 1 ships with these clamps already on:

* `set_strict_variables(true)` — typoed variable names error
  rather than silently equal `()`.
* No `import` / no module resolver — scripts cannot pull in code
  from arbitrary paths.
* No filesystem / network / process bindings except `spawn` (which
  is what the user already has via `bind = …,spawn,…`).
* Operator overloading disabled, custom syntax disabled.

Scripts run with the user's privileges — a malicious script can
do anything the user can. The sandbox protects against typo-grade
mistakes (a script that goes infinite-loop blows past the call-stack
limit), not adversarial inputs. If you sync your dotfiles from an
untrusted source, your bind = …,spawn,rm -rf ~ already does this
to you; the scripting engine doesn't widen the attack surface.

## Why a placeholder isn't enough

Two of the P5 items shipped as just design docs (HDR, portal). For
scripting, foundation code is small enough (~150 LOC) that we ship
it now alongside the doc. The risk of "bind a half-baked API and
then break users on iteration" is real, so Phase 1 deliberately
exposes only `spawn` — a function that already has stable
semantics from the config file. Future phases extend the surface
deliberately.

## Build deps

| Phase | New deps |
|-------|---------|
| 1     | `rhai = "1"` (no_optimize, std features only) |
| 2     | none — uses existing dispatch table |
| 3     | none — wires into existing event sites |
| 4     | extends dwl-ipc-v2 with a `run_script` opcode |
| 5     | `serde_yaml` or `toml` for plugin manifests |

## Example user script (forward-looking)

```rhai
// ~/.config/margo/init.rhai
//
// Auto-tag Spotify into tag 8 when it opens; jump back to tag 1
// after dismissing it.
on_window_open(|client| {
    if client.appid == "spotify" {
        spawn("mctl dispatch tagmon 8");
    }
});

// Toggle a "focus mode": kills bar, sets gaps to 0.
fn focus_mode_on() {
    spawn("pkill waybar");
    dispatch("setgaps", [0, 0, 0, 0]);
}

on_tag_switch(|tag| {
    if tag == 9 { focus_mode_on(); }
});
```

This script would not fire today (Phase 1 stubs the hooks), but it
parses, evaluates, and registers without error — so users can
write it now and have it light up automatically when Phase 3 ships.
