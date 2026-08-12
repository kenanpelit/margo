# mtm — native tmux manager

## Why

The user's own `~/.local/bin/tm` (2253-line bash, `tm.sh` v2.0.0) is a
comprehensive tmux toolkit: session management, layout templates, buffer
management, clipboard integration, TPM plugin management, an fzf-based
"speed" command launcher, config backup/restore, and a KENP-named default
dev session with `anka` snapshot-restore coordination.

Separately, `~/.kod/community-plugins/tmux-provider` is a minimal Noctalia
launcher plugin: type `/tm <query>`, get a fuzzy list of running tmux
sessions (+ optional tmuxp configs), Enter spawns a terminal and
`tmux attach -t <session>`.

The user asked to fork the reference plugin's idea, use it to improve their
own `tm`, and rewrite the result as a native Rust tool (`mtm`) inside margo,
reachable both as a CLI and as `/mtm` in the app launcher. Approved scope:
**all** of `tm`'s modules, not just session management.

## Architecture

New top-level binary crate `mtm/`, structured like `mctl`/`mscreenshot`
(clap-derive subcommands, `anyhow` errors). No-arg invocation replicates
`tm`'s default: attach to (or create) the configured default session.

```
mtm/
  Cargo.toml
  src/
    main.rs      — clap CLI, dispatch, no-arg → kenp::default_session()
    config.rs    — MtmConfig (toml), load/save, defaults
    tmux.rs      — shared tmux subprocess helpers + session-name validation
    session.rs   — create/list/kill/attach/layout/term subcommands
    kenp.rs       — default-session mode + anka-restore coordination
    buffer.rs    — tmux buffer list/show (fzf)
    clip.rs      — clipboard show (cliphist/clipse backend)
    plugin.rs    — TPM plugin install/list/install-all
    speed.rs     — fzf command launcher (categorize/score/pin in Rust,
                   fzf is presentation-only; hidden `__list`/`__pin`
                   subcommands are what fzf's `--bind` hooks shell back
                   into, replacing tm's mktemp-and-source-helpers trick)
    backup.rs    — config tar backup/restore
    fzf.rs       — shared themed-fzf spawn helper
```

Config lives at `~/.config/margo/mtm.toml` — per
`docs/config-conventions.md` §1, mtm is a standalone tool (like
`mvpn`/`mpower`), not part of either `margo-config` or `mshell-config`.
Replaces `tm`'s hardcoded `DEFAULT_SESSION="KENP"` and fzf theme colors
with user-editable values (sensible defaults matching `tm`'s today).

## `/mtm` launcher provider

`mshell-crates/mshell-launcher/src/providers/tmux.rs`, following the `ssh`
provider's exact shape (`Provider` trait impl): `handles_command` gates on
`/mtm` (well, the `mtm`/`tm` prefix word after the launcher's `/`),
`search()` runs `tmux ls` directly (cheap, no shortcut through the `mtm`
binary on every keystroke — same reasoning as the SSH provider reading
`assh.yml` directly). Activating an entry spawns
`$TERMINAL -e mtm session create <name>` (create-or-attach, matching `tm`'s
own smart behavior) in the user's terminal.

## Behavior parity

Every `tm` module ports 1:1 in *behavior* (same external deps: `tmux`,
`fzf`, `git`, `tar`, `cliphist`/`wl-copy`/`clipse`), rewritten as typed,
tested Rust rather than string-parsed bash. Interactive pickers
(buffer/clip/speed) keep shelling out to `fzf` for rendering — reimplementing
that as a native TUI is out of scope and would duplicate margo's own
launcher/clipboard-popup UX for no benefit.

Explicitly not reverse-engineered: `anka`'s own file format. The
`anka_restore_pending` check (tmux option + snapshot-file existence, with a
bounded wait) is already defensive in `tm` and ports as-is without needing
to understand `anka` internals — if the file/option isn't there, mtm behaves
exactly as if anka weren't installed.

## Out of scope (v1)

- No GTK Settings page — `mtm` is a terminal tool; `mtm.toml` is the config
  surface, same tier as `mpower.toml`/`mlogind-variables.toml`.
- No native TUI replacement for `fzf`.
