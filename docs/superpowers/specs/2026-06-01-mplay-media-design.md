# mplay media — smart multi-player media controller (design)

**Date:** 2026-06-01
**Status:** approved (brainstorm), pending implementation plan

## Goal

Fold the user's `osc-media.sh` (805-line smart media controller) into `mplay`
as a `media` subcommand group, replacing the external script. It routes a
transport command (toggle/play/pause/stop/next/prev/status) to the best active
player across **MPRIS** (Spotify/VLC/browsers via `playerctl`), **MPD** (`mpc`),
and **mpv** (the IPC socket mplay already speaks), or to an explicitly named
player, and shows a rich album-art notification.

## Scope

In scope (one delivery, `durmadan bitir`):
- `media` subcommand group on `mplay`.
- Backends: MPRIS (`playerctl` subprocess), MPD (`mpc` subprocess), mpv (reuse
  `crate::mpv_ipc`).
- Auto-detect with the osc-media scoring model + last-player memory.
- Rich notification (player name + status + title/artist/album, album-art icon,
  coalesced via the synchronous hint).
- **Spotify autostart** when explicitly targeted and not running (incl. the
  wait-for-ready loop). *(included per user)*
- man page `media` section, PKGBUILD/AUR `optdepends += playerctl, mpc`, config
  keybind migration.

Out of scope: VLC/other-player autostart; persistent daemon; GUI.

## Command surface

```
mplay media toggle [PLAYER]
mplay media play|pause|stop|next|prev|status [PLAYER]
# PLAYER ∈ spotify | vlc | mpv | mpd (alias mpc) | browser ; omitted = auto-detect
```
Alias: `mplay m …`.

## Architecture

New module tree `mplay/src/media/` (declared `mod media;` in main):

```
media/mod.rs      run(cmd, player) → resolve active player, execute, notify
media/player.rs   Kind { Mpris(String), Mpd, Mpv }; PURE candidate_score();
                  autodetect + explicit-target resolution; last-player file
media/status.rs   PURE: Status enum + normalize() + Turkish labels; Command enum
media/mpris.rs    playerctl subprocess: list/status/metadata/control
media/mpd.rs      mpc subprocess: status/metadata/control
media/mpv.rs      thin adapter over crate::mpv_ipc (status/metadata/control)
media/notify.rs   rich notify-send (icon = album art or player icon)
media/spotify.rs  autostart + wait-for-ready (explicit spotify target only)
```

### Player model (`player.rs`, `status.rs`)
- `Status { Playing, Paused, Stopped, Unknown }` + `normalize(&str)` (pure).
- `Command { Toggle, Play, Pause, Stop, Next, Prev, Status }`.
- `Kind { Mpris(String), Mpd, Mpv }`.
- `candidate_score(kind, name, status, command, last_player_id) -> i32` — **pure**,
  a faithful port of osc-media's `candidate_score` (status base; kind bonus
  mpv/mpd +40, mpris +35 / browser +8; name bonus spotify+35 vlc+28 mpv+24
  browser+10; last-player +90; "Playing" context +18; play+Paused +15).
- `pick_active(target: Option<&str>) -> Option<(Kind, String)>` — explicit target
  resolution (incl. Spotify autostart) or scored auto-detect over mpv + mpd +
  all MPRIS players.
- last-player memory: `$XDG_RUNTIME_DIR/mplay/last-player` (`<kind>:<name>`),
  read for scoring, written after a successful command.

### Backends
- **mpris.rs**: `playerctl -l` (list, drop `playerctld`), `playerctl -p P status`,
  `playerctl -p P metadata <field>`, control via `playerctl -p P play-pause|play|
  pause|stop|next|previous`.
- **mpd.rs**: `mpc status` (parse `[playing]`/`[paused]`), `mpc current -f …`,
  control via `mpc toggle|play|pause|stop|next|prev`.
- **mpv.rs**: status from `idle-active`/`pause` (via `mpv_ipc::get_bool`); metadata
  from `media-title` / `metadata/by-key/Artist|Album` / `path`; control via
  `set_property pause`, `playlist-next/prev`, `stop`, `cycle pause`.

### Notification (`notify.rs`)
`notify-send -a osc-media -u <urgency> -t 3200 -h string:x-canonical-private-synchronous:osc-media -i <icon> "<Player · Action/Status>" "<body>"` where body = `Durum:` + `Parça/Sanatçı/Albüm`; icon = local album-art file (from `mpris:artUrl` `file://`) else a per-player themed icon name. (Same sync id as the script so existing OSDs coalesce.)

### Spotify autostart (`spotify.rs`)
When `PLAYER=spotify`, the command is one of toggle/play/next/prev/status, and no
Spotify MPRIS player is present: notify "starting", spawn `spotify` detached,
poll `pick_best_mpris_for_target("spotify")` every 250 ms up to
`SPOTIFY_START_TIMEOUT` (default 12 s, env-overridable), then proceed.

## Error handling
- No controllable player found → error notification (`dialog-error`, critical) +
  non-zero exit, matching the script.
- Missing `playerctl`/`mpc` → that backend is simply skipped (no players from it);
  mpv still works via IPC.

## Testing
- **Unit (pure):** `Status::normalize`, `candidate_score` ranking (Playing beats
  Paused; last-player bonus wins ties; browser penalty), `Command` parse, mpc
  status-line parse, playerctl list filtering (drop `playerctld`).
- **Live control:** manual (needs real players).
- Gates: `cargo clippy -p mplay --all-targets -D warnings`, `cargo test -p mplay`.

## Packaging / integration
- No new binary. PKGBUILD + AUR + install.sh: `optdepends += playerctl, mpc`.
- `man/mplay.1`: add a `media` command section + the PLAYER list.
- Dotfiles `config.conf`: `alt,u` → `mplay media toggle mpv`, `alt+ctrl,e` →
  `mplay media toggle mpd`; keep them working (separate dotfiles commit).
- `osc-media.sh` dotfiles script → thin compat shim → `mplay media` (separate
  commit), mirroring the `margo-mpv.sh` shim.

## Risks
- playerctl/mpc output format drift → parsing is defensive (status normalized,
  empty → Unknown).
- Spotify autostart timing varies → bounded wait loop, then best-effort proceed.
