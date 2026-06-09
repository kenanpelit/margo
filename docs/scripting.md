# Scripting

Margo embeds a [Rhai](https://rhai.rs/) interpreter — pure-Rust, sandboxed by default. Drop a script at `~/.config/margo/init.rhai`; margo evaluates it at startup and keeps any registered hooks alive across the session.

For the design rationale (why Rhai over Lua, the recursion-guard pattern, phase rollout history), see [Scripting engine — design notes](scripting-design.md).

## Init script

```rhai
// Auto-tag Spotify into tag 8
on_window_open(|| {
    if focused_appid() == "spotify" {
        dispatch("tagview", [tag(8)]);
    }
});

// Tell the bar when entering tag 9
on_tag_switch(|| {
    if current_tag() == 9 {
        spawn("pkill -SIGUSR1 waybar");
    }
});

// Keep a notepad ready on the side monitor
if monitor_count() >= 2 {
    spawn("kitty --class scratch-notes -e nvim ~/notes.md");
}
```

A complete annotated example lives at [`contrib/scripts/init.example.rhai`](https://github.com/kenanpelit/margo/blob/main/contrib/scripts/init.example.rhai).

## Bindings

### Dispatch

```rhai
dispatch("action_name");                    // zero-arg
dispatch("action_name", [arg1, arg2]);      // with args (mirrors `bind = ...`)

spawn("kitty");                             // shell-style spawn helper
tag(n);                                     // tag bitmask helper — tag(3) == 4
```

Anything in `mctl actions` is callable. `mctl actions --verbose` enumerates every action with example arg shapes.

### Read-only state

```rhai
current_tag()              // active tag index (1..=9 for typical configs)
current_tagmask()          // bitmask form
focused_appid()            // String, "" if nothing focused
focused_title()
focused_monitor_name()
monitor_count()
monitor_names()            // Array<String>
client_count()
```

### Event hooks

Each hook fires from a well-defined event site; the body runs synchronously on the compositor mainloop, so keep it cheap.

| Hook | Fires from | Args |
|---|---|---|
| `on_focus_change(fn())` | `focus_surface`, post-IPC-broadcast, gated on `prev != new` | none |
| `on_tag_switch(fn())` | `view_tag`, after arrange + IPC | none |
| `on_window_open(fn())` | `finalize_initial_map`, after window-rules + focus | none |
| `on_window_close(fn())` | after the client is gone, focus has shifted, arrange has run | `(app_id: String, title: String)` |

Re-entrancy is guarded automatically: a hook that calls `dispatch(...)` and triggers another event will see the inner hook as a no-op rather than recursing. (Implementation: thread-local Option-take/restore.)

## Live edit

```bash
mctl run ~/.config/margo/test.rhai
```

Eval a script against the live engine — handy for prototyping. Hook registrations inside the script persist after the run, so you can iterate on a hook without restarting margo. Falls back to standing up a fresh engine if `init.rhai` was never loaded.

## Plugin packaging

```
~/.config/margo/plugins/
├── auto-monocle/
│   ├── plugin.toml          # name, version, description, enabled
│   └── init.rhai
├── focus-history-osd/
│   ├── plugin.toml
│   └── init.rhai
└── tag-1-no-anim/
    ├── plugin.toml
    └── init.rhai
```

Each plugin's `init.rhai` runs against the same engine `init.rhai` uses — so plugins can layer hooks on top of (and alongside) your own. `plugin.toml` is a minimal manifest:

```toml
name        = "auto-monocle"
version     = "0.1.0"
description = "Switch to monocle layout when only one window is on the focused tag."
enabled     = true
```

Compile / runtime errors per-plugin don't take down the loader — bad plugins log a warning and the rest still load.

Manage installed plugins with `mctl plugin`:

```bash
mctl plugin list                 # discovered plugins + on-disk enabled state
mctl plugin disable auto-monocle # flip enabled=false (takes effect next start)
mctl plugin enable  auto-monocle
```

### Example: `app-workspaces`

A complete, copy-pasteable plugin — when a chosen app opens, move its window
to a fixed "home" tag, so your media / chat windows always land in the same
place. Two files in `~/.config/margo/plugins/app-workspaces/`:

```toml title="plugin.toml"
name        = "app-workspaces"
version     = "1.0.0"
description = "Send chosen apps to their home tag when they open"
enabled     = true
```

```rhai title="init.rhai"
// Edit the table (app-id → tag). Find an app's id with `mctl clients`.
// Set a tag to 0 to leave that app where it is.
fn home_tag(app) {
    if app == "Spotify"  { return 8; }   // music   → tag 8
    if app == "ferdium"  { return 9; }   // chats   → tag 9
    if app == "webcord"  { return 5; }   // Discord → tag 5
    return 0;
}

on_window_open(|| {
    let t = home_tag(focused_appid());
    if t > 0 {
        // Moves the just-opened window to tag `t` (you stay put). Add the
        // `view` line if you'd rather FOLLOW it to its home tag.
        dispatch("tag", [tag(t)]);
        // dispatch("view", [tag(t)]);
    }
});
```

After `mctl plugin list` shows it, restart margo (relogin) to load it.

## Output

`print(...)` and `debug(...)` from inside a script land in `journalctl -u margo` at info / debug level respectively. Useful for "why didn't my hook fire?" debugging.

```rhai
on_window_open(|| {
    print(`opened: ${focused_appid()}`);
});
```

```bash
journalctl --user -u margo -f | grep margo::scripting
```

See [Roadmap → Scripting & plugins](roadmap.md#7-scripting--plugins) for the full rollout history (Phase 1 → 3 shipped, plugin packaging shipped, `mctl run` + `mctl plugin list/enable/disable` shipped, `on_output_change` shipped).
