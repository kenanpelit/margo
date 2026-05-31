# Config conventions

> **Binding spec — read before touching config.** This is the config-layer
> counterpart to [`mshell-crates/mshell-frame/DESIGN.md`](../mshell-crates/mshell-frame/DESIGN.md)
> (which governs UI). It covers *who owns which file*, *what is machine-written
> vs hand-edited*, *where a new knob goes*, and *what must never be merged*.
> When a task touches config — adding a setting, a Settings page, a managed
> fragment, or reorganising `~/.config/margo` — follow these rules without
> re-deriving them.
>
> User-facing docs are separate: [`configuration.md`](configuration.md) (a
> walkthrough) and [`config-reference.md`](config-reference.md) (every knob).
> This file is the *conventions*, not the knob list.

## 0. Two config worlds — pick the right one

There are **two unrelated config systems**, both exporting types named
`Config` / `Menu` / `Position`. Always verify the import path.

| | `margo-config` | `mshell-config` |
|---|---|---|
| Owner | compositor (`margo`) | shell (`mshell`) |
| Format | plain-text `key = value` `.conf` (PCRE2 window rules) | YAML profiles |
| File | `~/.config/margo/config.conf` (+ sourced fragments) | `~/.config/margo/mshell/profiles/<name>.yaml` |
| Live edit | `mctl reload` re-parses | `reactive_stores` store, hot |
| Add a knob | §4 | §5 |

A compositor behaviour → `margo-config`. A shell/bar/menu behaviour →
`mshell-config`. They never share a struct.

## 1. Who owns each file in `~/.config/margo`

Most files are **not interchangeable clutter** — each is owned by a specific
binary and several are **machine-written**. Do not move or merge across owners.

| Path | Owner | Written by | Hand-edit? |
|---|---|---|---|
| `config.conf` | margo | user (often a **dotfiles symlink**) | yes |
| `colors.conf` | margo (sourced) | **matugen** (wallpaper palette) | no — regenerated |
| `taglayouts.conf` | margo (sourced) | **mshell** (Settings → Tiling Layout) | tolerated |
| `binds.d/*.conf` | margo (sourced) | **plugin manager** (plugin keybinds) | tolerated |
| `mlayout.conf` + `layout_*.conf` | margo (sourced) / `mlayout` | `mlayout` (active = symlink) | the `layout_*` yes |
| `mlock.conf` | `mlock` | user | yes |
| `mlogind-variables.toml` | `mlogind` | user | yes |
| `twilight/` | `twilight` | `twilight` + user presets | presets yes |
| `mshell/profiles/*.yaml` | mshell | **mshell** | tolerated |
| `mshell/plugins.toml` | mshell | **mshell** (plugin manager) | no |
| `mshell/plugins/<key>/` | mshell | **plugin manager** (install) | no |

**Rule:** the file count reflects this multi-tool design. Reducing it by
merging machine-written fragments into `config.conf` **breaks the writer**
(matugen, mshell, the plugin manager hardcode their output paths). Don't.

## 2. Hand-edited vs machine-written fragments

`config.conf` pulls fragments in with `source = <relative-path>`:

```
source = colors.conf            # matugen-written
source = binds.d/<plugin>.conf  # plugin-manager-written
source = mlayout.conf           # mlayout active-layout symlink
source = taglayouts.conf        # mshell-written (Settings → Tiling Layout)
```

- **Machine-written fragments are append-or-overwrite targets for their tool.**
  Their path is hardcoded in the writer. Never inline them into `config.conf`
  and never relocate them without changing the writer too.
- **`config.conf` is frequently a symlink into a dotfiles repo** (e.g.
  `arch-config/…/margo/config.conf`). Treat it as possibly-not-writable-in-place:
  editing it edits the dotfiles repo, and the paths it `source`s are part of
  that repo's contract. Don't restructure sourced paths without coordinating.

## 3. The "managed fragment + mctl reload" pattern

When the **shell** needs to drive **compositor** config (a Settings page that
writes a `.conf`), do NOT reach into the compositor — they are separate worlds
(§0). Instead:

1. Write a managed fragment `~/.config/margo/<name>.conf` (mshell owns it).
2. Ensure `config.conf` `source`s it once (append-if-missing,
   whitespace-tolerant check — see `config_sources_us`).
3. Run `mctl reload`.

Examples: `taglayouts.conf` (Tiling Layout), `binds.d/` (plugin keybinds),
the Keybinds editor. This is the same rule DESIGN.md §8b states for Settings
pages; shell-owned settings instead go through `config_manager()` (the
reactive store), never a managed `.conf`.

`mctl reload` re-parses config **in the running binary** — it does **not**
pick up new *compositor code*. A compositor behaviour change needs a margo
rebuild + relogin; only config values are live via reload.

## 4. Adding a compositor knob (`margo-config`)

Touch all four or the knob silently no-ops / fails validation:

1. `margo-config/src/types.rs` — field on the `Config` struct **and** its
   `Default`.
2. `margo-config/src/parser.rs` — a `match` arm in `parse_line` **and** add the
   key to the `KNOWN` keys array.
3. `margo-config/src/validator.rs` — add the key to the known-keys `matches!`
   list (else `mctl check-config` flags it and **`reload_config` returns early
   before applying anything** — a real failure mode).
4. Consume it in `margo/src/…` (and re-apply on `reload_config` if it should be
   live, not only at startup — see §6).

## 5. Adding a shell knob (`mshell-config`)

1. `mshell-config/src/schema/…` — field on the struct. It derives
   `Store`/`Patch`, so reactive accessors (`config.menus().x_menu().field()`)
   are generated automatically.
2. `#[serde(default)]` (container) fills a *missing* field from the struct's
   `Default` — **not** from a per-field intent. For a non-`false`/non-zero
   default on a bool/scalar, add `#[serde(default = "fn")]`, or older saved
   profiles won't pick it up. (This is why a default-value flip can't reach
   existing profiles by editing `Default` alone.)
3. Read it via `config_manager()`; writes go through `update_config`. There is
   **no migration layer** — design new fields to default safely.

## 6. Precedence & live-apply rules

- **Per-tag tiling layout precedence: `taglayout` > `tagrule layout_name` >
  `default_layout`.** Enforced in `apply_tag_rules_to_monitor` (skip a tag's
  rule layout when it has a `taglayout`) + `reload_config` (seed taglayouts,
  then apply `default_layout` to tags with neither an override nor a live
  user-picked layout). A manual `setlayout` sets `user_picked_layout` and is
  never clobbered.
- **If a setting should take effect on `mctl reload`, re-apply it in
  `reload_config`** — seeding only at output/Pertag creation means it lands at
  the next start, not on reload (the bug behind "Apply does nothing").
- **Before adding a knob, check for an existing mechanism on the same axis.**
  Two ways to set one thing (e.g. `tagrule layout_name` vs `taglayout`, or the
  reverted menu min/max vs auto vs plugin-panel min/max) need an explicit
  precedence or they fight. Define it up front.

## 7. mlayout (monitor arrangement) convention

- Named layouts are `layout_<slug>.conf` files in
  `~/.config/margo/layouts/`; `mlayout` scans that subdir (`gather_layouts`).
- The **active** layout is the root-level `mlayout.conf` **symlink** → the
  chosen `layouts/layout_<slug>.conf` (relative target, dotfiles-portable);
  `config.conf` `source = mlayout.conf` picks it up. The symlink *is* the
  runtime selection; the `layouts/layout_*` files are the catalogue. (Keeping
  `mlayout.conf` at the root means the `source` line never changes.)
- This is idiomatic — `mlayout.conf → layout_*.conf` is the design, not a
  redundant chain. Distinct from §6's *tiling* layout (`taglayout`), which is a
  different concept (per-tag tiling algorithm, not monitor arrangement).

## 8. Plugin install = runtime files only

Installed plugins live in `~/.config/margo/mshell/plugins/<key>/`. The shell
runs only the `manifest.toml`, the wasm `entry`, and any scripts/assets the
manifest references (`*.sh`, `sounds/`, …). The install copy
(`mshell-plugins/src/git.rs::copy_dir_all`) **skips plugin source** (`src/`,
`Cargo.toml`/`lock`, `README`, `.gitignore`, `target/`, `.git`) — the source
lives in the plugin repo, not the config dir. Keep it that way; don't
reintroduce a whole-dir copy.

## 9. Quick checklist

- Compositor setting → §4 (types + parser + KNOWN + validator). Shell setting
  → §5. Never the wrong world (§0).
- Shell driving compositor config → managed fragment + `source` + `mctl reload`
  (§3); shell-only → `config_manager()`.
- Should it be live on reload? Re-apply in `reload_config` (§6).
- New per-tag / per-menu knob → is there already a mechanism on that axis?
  Define precedence (§6).
- Don't merge/relocate machine-written fragments (§1/§2); `config.conf` may be
  a dotfiles symlink (§2).
