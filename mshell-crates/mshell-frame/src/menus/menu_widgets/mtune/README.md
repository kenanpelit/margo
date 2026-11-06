# Native Tune menu

## Responsibilities

- `mtune.rs`: Relm4 component, native view, watched service state and command dispatch.
- `actions.rs`: transport, file choosers, queue/library actions and volume.
- `presentation.rs`: service snapshots, artwork/seek/rate presentation and saved playlists.
- `queue.rs`: keyboard-activatable queue rows, filtering and arrow navigation.
- `radio.rs`: native directory/favorites/custom-URL browser and async service calls.
- `radio_state.rs`: pure directory request-generation guard and current-station logic.

Radio uses only `mshell_services::mtune::MtunePlayer`. Search, preview, play,
save and removal run through `tokio_rt_spawn`; completions return through Relm4
commands. Preview does not save. A late directory response cannot overwrite a
newer request. Service errors are shown verbatim, including older-player errors.
The current station is matched by `station_name` while `is_live` is true, since
that is the identity exposed by the agreed service contract.

LIVE mode removes seek, speed and repeat/shuffle, keeps next/previous available
for navigating a saved station queue, and shows LIVE instead of track/queue duration. Local transport, repeat-each count,
rate presets, folder chooser, playlist chooser/saved playlists and queue removal
remain available. Folder dialogs deliberately have no layer-shell parent.

All three browser scrollers use `list_fit::mark_menu_list` with a 320px fallback
and no reserved minimum height. `MenuWidget::Mtune` is included in
`menus/list_fit.rs::lists_follow_menu_height`, so the standalone Tune menu
follows configured heights above that fallback.

## Verification

Targeted logic tests (no display, player, HTTP, or live desktop required):

```sh
CARGO_TARGET_DIR=/home/kenan/.cache/margo-targets/cp2-integ RUSTC_WRAPPER=sccache \
  cargo test -p mshell-frame menus::menu_widgets::mtune --lib -- --test-threads=1
```

`render_tests.rs` is an opt-in real GTK fixture, not a mockup. It sets in-memory
service properties without starting the player or service watcher. Run its test
binary under `dbus-run-session -- xvfb-run -a`, with `GDK_BACKEND=x11`,
`GSK_RENDERER=cairo`, `GTK_A11Y=none`, `TUNE_RENDER_DIR` pointing to a scratch
output directory and `TUNE_RENDER_CSS` pointing to the freshly built
`mshell-style-*/out/mshell-style.css`. Filter for
`native_tune_menu_render_fixture --ignored --test-threads=1 --nocapture`.

The fixture writes Queue, Library, Radio, small Radio and expanded/scrolled custom
URL screenshots, asserting width bounds (440px and 360px) and visible list
allocation. These are fixture renders, not proof of real stream playback or
production layer-shell placement. End-to-end D-Bus/audio integration belongs to
the parent integration verification. No installation or live shell restart is
needed for either test path.
