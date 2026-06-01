# mplay media Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans. Steps use `- [ ]`.

**Goal:** `mplay media <cmd> [player]` — port osc-media.sh into mplay: route transport commands to the best active player (MPRIS via playerctl, MPD via mpc, mpv via IPC) with scoring, last-player memory, Spotify autostart, and rich notifications.

**Architecture:** New `mplay/src/media/` module tree. Pure logic (status normalize, scoring, parse) is unit-tested; backends shell out to playerctl/mpc (mpv reuses `crate::mpv_ipc`).

**Tech Stack:** Rust, clap, serde_json, std::process, existing mpv_ipc.

---

### Task 1: `media/status.rs` — Status + Command enums (TDD)
**Files:** Create `mplay/src/media/status.rs`, `mplay/src/media/mod.rs` (`pub mod status;`), `mod media;` in main.rs.
- [ ] Test: `Status::normalize("Playing")==Playing`, `"paused"==Paused`, `"x"==Unknown`; `Status::label()` Turkish; `Command::parse("toggle")==Some(Toggle)`, `"prev"/"previous"==Prev`, `"x"==None`; `Command::mpris_action(Toggle)=="play-pause"`, `Prev=="previous"`.
- [ ] Implement `enum Status{Playing,Paused,Stopped,Unknown}` (+`normalize(&str)`,`label()->&str`), `enum Command{Toggle,Play,Pause,Stop,Next,Prev,Status}` (+`parse`,`mpris_action`,`is_status`).
- [ ] `cargo test -p mplay media::status` green. Commit.

### Task 2: `media/player.rs` — Kind + candidate_score (TDD)
**Files:** Create `mplay/src/media/player.rs` (`pub mod player;`).
- [ ] Test `candidate_score`: Playing>Paused>Stopped same kind/name; mpv bonus > mpris browser; spotify name bonus; last-player id adds +90 (flips a tie); `play`+Paused bonus.
- [ ] Implement `enum Kind{Mpris(String),Mpd,Mpv}` (+`id()->String` as `"kind:name"`, `is_browser(name)`), and pure `candidate_score(kind:&Kind, name:&str, status:Status, cmd:Command, last_id:&str)->i32` (faithful port).
- [ ] Test green. Commit.

### Task 3: `media/mpris.rs` — playerctl backend (TDD on parse)
**Files:** Create `mplay/src/media/mpris.rs`.
- [ ] Test pure `parse_player_list("vlc\nplayerctld\nfirefox\n")==["vlc","firefox"]` (drop playerctld + blanks).
- [ ] Implement: `list()->Vec<String>` (`playerctl -l`, filter via the tested parser), `status(p)->Status` (`playerctl -p p status`→normalize), `metadata(p,field)->Option<String>`, `control(p, cmd)` (`playerctl -p p <action>`), `art_url(p)`.
- [ ] Test green. Commit.

### Task 4: `media/mpd.rs` — mpc backend (TDD on parse)
**Files:** Create `mplay/src/media/mpd.rs`.
- [ ] Test pure `parse_mpc_status` extracting `[playing]`→Playing, `[paused]`→Paused from a sample `mpc status` block; `available` reflects exit.
- [ ] Implement `available()->bool` (`mpc status` ok), `status()->Status`, `current(fmt)->Option<String>`, `control(cmd)` (`mpc toggle|play|...`).
- [ ] Test green. Commit.

### Task 5: `media/mpv.rs` — mpv adapter
**Files:** Create `mplay/src/media/mpv.rs`.
- [ ] Implement over `crate::mpv_ipc`: `available()` (`socket_ready()||pgrep mpv`), `status()` (idle-active→Stopped, pause→Paused, else Playing), `metadata()` (media-title/Artist/Album/path), `control(cmd)` (toggle=`cycle pause`, play/pause=`set_property pause`, next/prev=`playlist-next/prev`, stop=`stop`).
- [ ] `cargo build -p mplay`. Commit.

### Task 6: `media/spotify.rs` — autostart
**Files:** Create `mplay/src/media/spotify.rs`.
- [ ] Implement `ensure_ready(cmd) -> Option<String>`: if a spotify MPRIS player exists → return it; else if cmd ∈ {toggle,play,next,prev,status} and `spotify` on PATH and not running → notify "starting", spawn detached, poll `mpris::list()`/match "spotify" every 250 ms up to `SPOTIFY_START_TIMEOUT` (env, default 12s), return the found player or None.
- [ ] `cargo build -p mplay`. Commit.

### Task 7: `media/notify.rs` + `media/mod.rs` orchestration
**Files:** Create `mplay/src/media/notify.rs`; finish `mplay/src/media/mod.rs`.
- [ ] `notify.rs`: `media(player_pretty, action_or_status, body, icon)` → `notify-send -a osc-media -t 3200 -h string:x-canonical-private-synchronous:osc-media -i <icon> title body`. Helpers `player_pretty(name)`, `player_icon(name)`, `resolve_icon(art_url, name)`.
- [ ] `mod.rs`: `pub fn run(cmd: Command, target: Option<&str>) -> Result<()>`:
  resolve `last_id` from file; `pick_active(target)` (explicit→resolve incl. spotify::ensure_ready; else scored auto over mpv/mpd/mpris); bail with error notify if none; if cmd != Status → execute on the backend; read status + metadata; write last-player; send notification.
- [ ] `cargo build -p mplay` + `cargo clippy -p mplay -- -D warnings`. Commit.

### Task 8: CLI wiring
**Files:** `mplay/src/cli.rs`, `mplay/src/main.rs`.
- [ ] cli: add `#[command(alias="m", subcommand)] Media(MediaCmd)` where `MediaCmd` = the 7 transport verbs, each `{ player: Option<String> }`. (Or a single `Media { action: String, player: Option<String> }` parsed via Command::parse — simpler; use that.)
- [ ] main: `Command::Media{action,player} => media::run(media::status::Command::parse(&action).ok_or_else(...)?, player.as_deref())`.
- [ ] `cargo run -p mplay -- media --help` works; `cargo test -p mplay` green. Commit.

### Task 9: docs + packaging + config + shim
**Files:** `man/mplay.1`, `PKGBUILD`, `~/Work/aur/margo-git/PKGBUILD` (+`.SRCINFO`), `install.sh`, dotfiles `config.conf` + `osc-media.sh`.
- [ ] man: add `media` to COMMANDS + PLAYER list.
- [ ] PKGBUILD (repo + AUR) + install.sh: `optdepends += playerctl, mpc`; AUR bump pkgver + regen .SRCINFO.
- [ ] dotfiles config.conf: `alt,u`→`mplay media toggle mpv`, `alt+ctrl,e`→`mplay media toggle mpd`; add `super,m`→`mplay media toggle` (auto) if free. `mctl check-config` clean.
- [ ] dotfiles `osc-media.sh` → thin shim `exec mplay media "$@"` (map `[player] cmd`).
- [ ] Commit (margo) + commit/push (AUR) + commit/push (dotfiles).

### Task 10: Final gates
- [ ] `cargo fmt -p mplay`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test -p mplay`; `cargo build --release -p mplay`. Push margo.

## Self-review
- Spec coverage: status/scoring (T1-2), backends (T3-5), spotify (T6), notify+orchestration (T7), CLI (T8), docs/pkg/config/shim (T9), gates (T10). ✓
- Placeholders: none. Type names (Status/Command/Kind/candidate_score/run/pick_active) consistent across tasks. ✓
