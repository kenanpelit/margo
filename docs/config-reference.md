# Config reference

The complete annotated reference config — every knob margo exposes, with
inline commentary explaining both syntax and the *why* behind each
setting. This is the same file you get at
`/usr/share/doc/margo-git/config.example.conf` after installing the
package, and it lives in source at
[`margo/src/config.example.conf`](https://github.com/kenanpelit/margo/blob/main/margo/src/config.example.conf).

Strip what you don't need; everything has a sane built-in default, so
blank entries fall back to the compiled-in value.

!!! tip "How to use this page"
    The example mixes **declarative settings** (look, animations,
    input, layout) with **rule-based matchers** (window rules, layer
    rules, tag rules) and **bindings** (keyboard, mouse, gesture).
    The first half covers settings; the second half is keys + rules.
    Every section header below maps 1:1 to a `# ── ... ──` divider in
    the source file, so `Ctrl+F` from either side hits the same spot.

For a quick-start curated subset, see [Configuration](configuration.md)
instead.

---

## Reload + validation

```bash
mctl reload          # re-read the config in place (no logout)
mctl check-config    # validate without applying — exits 1 on any error
```

Reloading replays *every* setting. Removing a key reverts to its
default; removing a `bind = …` line unbinds it; removing a
`windowrule = …` line removes the rule effect from already-mapped
windows on the next arrange.

`mctl check-config` runs offline (no Wayland connection needed). It
catches: unknown fields, regex compile errors, **duplicate bind
detection** (caught real shadowing in the maintainer's own config),
and include-resolution loops. Wire it into your editor on save or
your pre-commit hook.

## Discoverability tools

```bash
mctl actions --verbose   # every dispatch action with examples
mctl actions --names     # bare list (handy for shell completion)
mctl rules --appid X --title Y   # which rules a hypothetical client hits
mctl status / clients / outputs  # live state from the running compositor
```

`mctl rules` is the right tool for *"why didn't my windowrule fire?"*
— offline, runs against the same rule engine as the compositor.

## The full file

```ini title="margo/src/config.example.conf"
--8<-- "margo/src/config.example.conf"
```

---

## Where to next

- [Configuration overview](configuration.md) — curated walkthrough of
  the high-traffic options.
- [`mctl actions --verbose`](companion-tools.md#mctl) — the full
  enumerated dispatch catalogue (40+ actions).
- [Scripting](scripting.md) — when window rules and keybinds aren't
  expressive enough, reach for `~/.config/margo/init.rhai`.
