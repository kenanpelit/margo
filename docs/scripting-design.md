# Plugin / scripting system — design notes

> Status: **Phase 3 landed**. `~/.config/margo/init.rhai` is
> evaluated at compositor startup with full dispatch invocation,
> state introspection, AND event hooks that actually fire mid-
> event-loop. `on_focus_change`, `on_tag_switch`, and
> `on_window_open` registered handlers now run from the matching
> compositor event sites with the live state reachable through
> the same binding surface as Phase 2.

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

### Phase 2 — Action bindings + state introspection ✓ shipped

* `dispatch(action: string, args: [...])` — calls any registered
  margo action. Args array maps positionally onto the `Arg` struct
  (strings → v/v2/v3, ints → i/ui & i2/ui2, floats → f/f2).
  Zero-arg overload `dispatch(action)` for `killclient`,
  `togglefloating`, etc.
* `spawn(cmd)` — convenience for `dispatch("spawn", [cmd])`.
* `tag(n: int) → int` — converts 1-based tag number to bitmask.
* Read-only state: `current_tag()`, `current_tagmask()`,
  `focused_appid()`, `focused_title()`, `focused_monitor_name()`,
  `monitor_count()`, `monitor_names()`, `client_count()`.
* Threading: scripts run on the compositor thread, synchronously,
  during startup only. Heavy work blocks the event loop — keep it
  cheap or `spawn` it as a subprocess.
* State-access pattern: thread-local raw pointer to `MargoState`
  set for the duration of the eval, cleared via RAII guard. Same
  contract reused in Phase 3.

  Example user script (works today):

  ```rhai
  if monitor_count() >= 2 {
      // External monitor present — start in scroller layout
      dispatch("setlayout", ["scroller"]);
  }
  if focused_appid() == "" {
      // Cold start, no focused window — pop a terminal
      dispatch("spawn", ["kitty"]);
  }
  ```

### Phase 3 — Event hooks ✓ shipped

* `ScriptingState { engine, ast, hooks }` lives on `MargoState`
  as `Option<Box<...>>`, so callbacks survive past startup.
* `on_focus_change(fn())` fires from `focus_surface` after
  the focus broadcast, gated on `prev != new` so the speculative
  refresh path doesn't trigger no-op hooks.
* `on_tag_switch(fn())` fires from `view_tag` after the
  arrange + IPC broadcast, so handlers reading
  `current_tag()` / `focused_appid()` see post-switch state.
* `on_window_open(fn())` fires from `finalize_initial_map`
  after window rules + focus, so handlers see the final
  app_id + title and `dispatch(...)` calls apply to the
  just-opened window.
* Recursion guard: hooks fire by *taking* `ScriptingState` out
  of `MargoState`, running the FnPtrs on the still-owned
  engine/AST, then putting it back. A re-entrant `fire_*`
  finds `None` and is a no-op — so a hook that calls
  `dispatch(...)` triggering another event doesn't re-fire
  itself.
* Hooks registered during a hook body are appended to the
  matching list when the outer fire returns; no handler is
  silently dropped.
* Hooks today take no args. Reading state via the Phase 2
  binding surface (`focused_appid()`, `current_tag()`, …) is
  the user-facing API. Future iterations may pass typed
  argument structs, but the no-arg version covers ~all real
  patterns and avoids exposing a struct surface that has to
  evolve compatibly across compositor versions.

Out-of-scope (future):

* `on_window_close(fn())` — needs a stable identity for
  closed windows so a handler can react before the client
  dies. Tracked separately.
* `on_output_change(fn())` — fires on output add/remove
  with the monitor name; useful for "auto-relayout when
  external display plugged in".
* Per-client filter sugar (`on_window_open("spotify", fn() { ... })`)
  — today filters are `if focused_appid() == "spotify" { ... }`
  inside the handler body. Sugar is small Phase 3.5 if
  user demand surfaces.

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
