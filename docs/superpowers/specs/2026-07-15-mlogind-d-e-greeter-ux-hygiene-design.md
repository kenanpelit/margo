# mlogind phase D + E — greeter UX and code hygiene

Date: 2026-07-15. Follows phase B (spec
`2026-07-15-mlogind-b-daemon-vt-hardening-design.md`, shipped c1949508).
Ideas from atrium (GPL-2, ideas only — no code transfer) adapted to the
A1/A2 architecture: one PAM conversation in the forked session runner,
unprivileged greeter, three hosts (gui → cage → tty).

## What D ships

### D1 — background directory (`[display] background_dir`)

atrium picks a random image from a directory at every greeter start; we
currently show only the baked blurred copy of the last user's wallpaper
(`/var/lib/mgreet/background.raw`).

- New knob `background_dir` (string, default `""` = off). When set and the
  directory contains at least one decodable image (`png/jpg/jpeg/webp/bmp`),
  the **runner** picks one at random per greeter start — one pick, used by
  both consumers, so the compositor wallpaper and the mgreet backdrop can
  never disagree:
  - `write_greeter_conf` writes `wallpaper = <picked>` into the throwaway
    margo config (instead of the baked raw), and
  - `spawn_greeter` exports `MLOGIND_BACKGROUND=<picked>` for mgreet.
- mgreet's `background::load()` tries `MLOGIND_BACKGROUND` first via
  `gdk::Texture::from_filename` (GTK's own decoders — mgreet still links no
  image crate), falling back to the raw cache on any failure. A directory
  image is **not** blurred; the existing `.mgreet-dim` overlay carries
  legibility, as it does for atrium.
- Empty dir / unreadable dir / undecodable file all fall back to the baked
  raw backdrop. A login screen that cannot find its wallpaper still logs
  you in.

### D2 — external CSS theme (`[display] greeter_css`)

atrium ships themable greeters; mgreet's palette is matugen-or-Dracula.

- New knob `greeter_css` (string, default `""` = off): absolute path to a
  CSS file. The runner exports `MLOGIND_CSS=<path>` when the file exists.
- mgreet layers it as a third `CssProvider` at USER priority, **after** the
  matugen overlay, so it wins over both the baked palette and the synced
  theme. The natural content is `:root { --primary: …; }` token overrides,
  but any GTK CSS is accepted — it is the admin's file.
- Shipping atrium's 10 themes as content is out of scope; the mechanism is
  the deliverable.

### D3 — keyboard layout switcher + indicator (mgreet)

The badge next to the password field currently *states* group 0 of
`XKB_DEFAULT_LAYOUT` and nothing can change it. margo already has a
`cyclekblayout` dispatch, and the greeter conf already carries the full
comma list (`xkb_rules_layout = tr,us`), so the compositor side has been
switchable all along — only the greeter had no way to ask.

- `keyboard::layouts()` returns every group formatted (`tr(f)`, `us`, …),
  lock-step with variants, exactly like the existing group-0 logic.
- With >1 layout the badge becomes a button: click → spawn
  `mctl dispatch cyclekblayout` (mgreet inherits `MARGO_SOCKET` from the
  greeter compositor's startup shell — the same channel the existing
  `mctl dispatch quit` uses) → advance a local index → relabel the badge.
  margo and the badge both start at group 0 and both step +1, so they stay
  in lock step. (A `grp:` XKBOPTIONS hotkey switching behind our back can
  stale the badge; accepted and documented — the badge resyncs on the next
  click.)
- Gated on the real greeter: in `--preview` there is no throwaway
  compositor, and dispatching `cyclekblayout` would switch the *user's*
  session layout underneath them. Preview cycles the label only.
- The TUI greeter keeps the kernel keymap it booted with (switching would
  mean shelling out to `loadkeys` as root per keypress) — out of scope,
  same as atrium's txt frontend.

### D4 — autologin (once per boot)

- New section `[autologin]`: `user` (default `""`), `session` (default
  `""`), `pam_service` (default `"mlogind-autologin"`). Both `user` and
  `session` non-empty → enabled.
- Runs **once, before the host ladder**, host-independently: the daemon
  forks a runner in autologin mode (no greeter, no socketpair), waits via
  `daemon::Events` (a termination signal still stops everything cleanly and
  never falls down any ladder). Whatever the exit code, the flag is spent:
  when the autologin session ends — or fails to start — the normal greeter
  ladder takes over. Logout therefore re-greets instead of looping straight
  back in (SDDM's `Relogin=false` semantics).
- The autologin runner: reset the inherited sigmask (B invariant), resolve
  the session name, `auth::lookup` the user, then
  `Authenticator::with_password(autologin.pam_service)` +
  `set_credentials(user, "")`. The stack authenticates with `pam_permit`
  (nothing is typed, so nothing can be replayed), and the **session** phase
  is the real `login` include — logind session, loginuid, limits — so an
  autologin session is a first-class session. Then the existing
  `start_session` path, generified over `pam::Converse` (it never touched
  the handler; only the signature pinned it to `GreeterConv`).
- Ships `extra/mlogind-autologin.pam` (auth `pam_permit`, account/session/
  password `include login`-shaped like the interactive stack), wired into
  PKGBUILD (install + `backup`) and install.sh. A missing PAM service file
  fails `pam_start`/`authenticate` → `EXIT_SESSION_FAILED` → normal greeter;
  autologin can never lock the machine.
- The last-login cache is **not** written by autologin: it is config-driven,
  and the cache pre-fills the *greeter*, whose next appearance means the
  user just logged out on purpose.

### D5 — mkeys on-screen keyboard (`[display] osk`)

- New knob `osk` (bool, default `false`). Gui host only. When on and
  `mkeys` is installed, the greeter compositor's startup command becomes
  `mkeys & OSK=$!; mgreet; kill $OSK 2>/dev/null; mctl dispatch quit` —
  mkeys (layer-shell + zwp_virtual_keyboard, both already served by margo)
  floats over the greeter for touch login, and is torn down with it.
  `preflight` is untouched: a missing mkeys just drops the OSK
  (`which` check at spawn), never the host.

### D6 — echo-on prompt rendering

The proto has carried `Prompt { echo }` since A1; both greeters ignore it
and mask everything. An OTP code or a `pam_exec` challenge is not a secret,
and masking it doubles the typo rate on a field you cannot re-read.

- mgreet: `auth::Action::AskUser` grows `echo`; the password entry flips
  `set_visibility(echo)` for the duration of that answer and back to masked
  the moment the answer is taken (and on failure / blank / runner loss —
  every path that clears the field also re-masks it).
- TUI: `InputFieldWidget` remembers its configured mask and gains
  `set_echo(bool)` (Echo ↔ the remembered `Replace`); `pump()` sets it from
  the prompt's `echo` flag and every reset path (answer sent, failure)
  restores the mask. The username field never masks, so `set_echo(false)`
  on it is a no-op by construction.

### Already shipped, noted for completeness

Idle blank (`[display] blank_timeout`) shipped with the gui host and is
gui-only by design — the cage/tty hosts have no surface of their own to
blank (the console blanks itself via the kernel's own timer).

## What E ships

- **E1 — edition 2024.** mlogind adopts `edition.workspace` +
  `rust-version.workspace` (1.93), dropping its private 2021/1.84 pins (the
  fork-era carve-out). Mechanical consequences: every `env::set_var` /
  `remove_var` needs an `unsafe` block (mlogind is single-threaded — the
  SAFETY argument is the same one `fork` already relies on), and
  `unsafe fn chvt` needs explicit unsafe blocks inside
  (`unsafe_op_in_unsafe_fn`). Let-chains become available but existing code
  is not churned to use them.
- **E2 — packaging tidy.** PKGBUILD installs the pam/sysusers/polkit
  extras; install.sh (the cross-distro path) installs only the binary.
  Bring install.sh up to PKGBUILD parity for mlogind's system files
  (`/etc/pam.d/mlogind{,-greeter,-autologin}`, sysusers.d + a
  `systemd-sysusers` run, polkit rules, `/etc/mlogind/config.toml` +
  `variables.toml`, the service unit), each guarded with "don't clobber an
  existing file" for the `/etc` ones.
- **E3 — tests.** Every D knob lands with unit tests (random pick filter,
  css env gating, layouts() grouping, autologin config gate, echo flip
  round-trip, startup-command shapes), plus the baked-config parse pin that
  keeps `Config::default()` honest.

## Out of scope

- TUI keyboard-layout switching (kernel keymap; would shell to `loadkeys`).
- Shipping a theme catalogue (mechanism only).
- Autologin re-login loops (`Relogin=true`), autologin for X11 sessions is
  untested but not blocked.
- Touchscreen auto-detection for the OSK (explicit knob only).
- Blurring `background_dir` images (the dim overlay is the contrast story).

## Testing

- `cargo test -p mlogind -p mgreet`, clippy `--all-targets -D warnings`,
  `cargo +1.95.0 fmt`, panic-ratchet, Cargo.lock untouched (no new deps).
- Hardware (user):
  1. `background_dir` pointed at a photo dir → greeter shows a random photo,
     compositor wallpaper matches it, next logout shows a different one.
  2. `greeter_css` with a loud override (`--primary: red`) → visible.
  3. Two layouts in vconsole (`tr,us`) → badge clickable, password field
     obeys the switched layout.
  4. `[autologin] user/session` set → boot lands in the session with no
     greeter; logout shows the greeter; reboot autologs again. `systemctl
     stop mlogind` during the autologin session behaves like B.
  5. `osk = true` → mkeys floats over the greeter and dies with it.
  6. An `echo=true` PAM prompt (e.g. `pam_exec expose_authtok=0` asking a
     question) renders readable in both greeters, and the next password
     prompt is masked again.

## Files touched

`mlogind/src/{config.rs, main.rs, runner/mod.rs, ui/mod.rs,
ui/input_field.rs, post_login/env_variables.rs, runner/greeter_session.rs,
chvt.rs, cli.rs?}`, `mlogind/extra/{config.toml, mlogind-autologin.pam}`,
`mlogind/Cargo.toml`, `mgreet/src/{main.rs, ui.rs, auth.rs, background.rs,
keyboard.rs, style.rs}` + `mgreet/scss`, `PKGBUILD`, `install.sh`.
