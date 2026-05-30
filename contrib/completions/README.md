# margo shell completions

Hand-curated completion scripts for `mctl` (the compositor-control CLI)
**and** `mshellctl` (the shell-control CLI). Both static subcommand
surface + live, runtime-pulled lookups (dispatch actions for mctl, plugin
keys for mshellctl).

The clap-derived `mctl completions <shell>` generator covers the
subcommand and flag layer but can't enumerate margo's dispatch action
names — those live in `margo-ipc/src/actions.rs` and are exposed at
runtime through `mctl actions --names`. The scripts in this directory
extend the basic completion with:

* The full dispatch action list (cached on first tab-press, refreshed
  per shell session — when you upgrade margo and a new action lands,
  open a new shell or unset the cache var).
* `setlayout` argument completion against the static layout-name list.
* `--output` / `-o` value completion against live `wl_output` names
  pulled from `mctl status`.
* `--group` value completion for `mctl actions --group <NAME>`.

## Install

```sh
# bash (XDG path picked up by bash-completion 2.x)
mkdir -p ~/.local/share/bash-completion/completions
cp contrib/completions/mctl.bash      ~/.local/share/bash-completion/completions/mctl
cp contrib/completions/mshellctl.bash ~/.local/share/bash-completion/completions/mshellctl

# zsh (any directory in $fpath works; this one is a common choice)
mkdir -p ~/.local/share/zsh/site-functions
cp contrib/completions/_mctl ~/.local/share/zsh/site-functions/_mctl
# Make sure ~/.local/share/zsh/site-functions is in your $fpath.

# fish (auto-loaded from the standard completions dir)
mkdir -p ~/.config/fish/completions
cp contrib/completions/mctl.fish      ~/.config/fish/completions/mctl.fish
cp contrib/completions/mshellctl.fish ~/.config/fish/completions/mshellctl.fish
```

## System-wide install (PKGBUILD / packagers)

```sh
install -Dm644 contrib/completions/mctl.bash      "$pkgdir/usr/share/bash-completion/completions/mctl"
install -Dm644 contrib/completions/mshellctl.bash "$pkgdir/usr/share/bash-completion/completions/mshellctl"
install -Dm644 contrib/completions/_mctl          "$pkgdir/usr/share/zsh/site-functions/_mctl"
install -Dm644 contrib/completions/mctl.fish      "$pkgdir/usr/share/fish/vendor_completions.d/mctl.fish"
install -Dm644 contrib/completions/mshellctl.fish "$pkgdir/usr/share/fish/vendor_completions.d/mshellctl.fish"
```

## Refreshing the action cache

Bash and zsh both cache the action list per shell session to keep
tab-press latency under one mctl invocation. After a margo upgrade
that adds a new dispatch action, either open a new shell or:

```sh
# bash
unset _MCTL_ACTIONS_CACHE

# zsh
unset _mctl_actions_cache

# fish
set --erase __mctl_actions_cache
```

## Extending

The action catalogue lives in `margo-ipc/src/actions.rs` and is the
single source of truth for both `mctl actions` and these completion
scripts. Add an entry there and rebuild — the next shell session
picks up the new spelling automatically.
