# mplay — native MPV companion for margo (design)

**Date:** 2026-06-01
**Status:** approved (brainstorm), pending implementation plan

## Goal

Replace the user's `margo-mpv.sh` helper with a first-party Rust binary,
`mplay`, in margo's `m*` tool family. `mplay` controls an mpv window through
margo (`mctl`) + mpv's own JSON IPC, **and** plays video wallpapers natively —
porting `mpvpaper`'s engine in-tree so there is no external `mpvpaper`
dependency.

## Scope

In scope (one delivery, no phasing — user directive):

- **Controller** subsystem — port the 8 commands of `margo-mpv.sh`.
- **Wallpaper engine** subsystem — native wlr-layer-shell + EGL + libmpv
  render-context video wallpaper (mpvpaper port, core feature set).
- man page, PKGBUILD/install.sh wiring, dotfiles `margo-mpv.sh` reduced to a
  thin alias (or removed).

Out of scope (YAGNI, deferred — not in v1):

- mpvpaper's slideshow mode, the "holder" fast-resume helper process, and the
  auto-pause/stop-when-a-window-is-visible automation.

## Command surface

```
mplay start                       launch mpv (pseudo-gui) + JSON IPC socket
mplay toggle                      play/pause (cycle pause)
mplay play [URL]                  load file/URL/clipboard (ytdl auto-detect)
mplay download [URL]   (alias dl) yt-dlp → ~/Downloads
mplay snap                        cycle the floating mpv window across 4 corners
mplay pin                         toggle sticky / all-tags (toggle)
mplay focus                       focus the mpv window (monitor + tag hop)
mplay stop                        quit mpv
mplay wallpaper <SRC> [--output NAME] [--mute] [--no-loop]
                                  [--scale fit|fill|stretch] [--daemon]
mplay wallpaper stop [--output NAME]   (alias wall)
```

Defaults: mpv JSON socket `/tmp/mpvsocket` (overridable via `$MARGO_MPV_SOCKET`);
window size 640×360, corner margins 32/96 (env `MARGO_MPV_*`, kept for
compatibility).

## Architecture

New workspace member `mplay/` (binary crate), mirroring `mpicker`/`mscreenshot`.

```
mplay/
  Cargo.toml
  build.rs            # cargo:rustc-link-lib=mpv (system libmpv)
  src/
    main.rs           # clap dispatch → control / wallpaper
    cli.rs            # clap command/argument definitions
    margo.rs          # margo state/commands via `mctl` subprocess
    mpv_ipc.rs        # mpv JSON IPC socket client (loadfile, cycle pause, …)
    control.rs        # start/toggle/play/download/snap/pin/focus/stop
    geometry.rs       # PURE helpers: corner-cycle math, clamp, scale-mode parse
    ytdl.rs           # PURE helpers: youtube URL detection, clipboard resolve
    paper/
      mod.rs          # engine entry: run(src, opts) / stop(output)
      wayland.rs      # wl registry, wlr-layer-shell background surface per output
      egl.rs          # EGL display/context/surface (khronos-egl + wl_egl_window)
      render.rs       # mpv_render_context create/render loop (calloop)
      mpv_sys.rs      # hand-written FFI: the ~10 mpv client + render_gl symbols
```

### Controller (small, mostly logic)

- **margo.rs** shells out to `mctl get clients|monitors|focused` (the new socket
  topics → pure JSON) and `mctl dispatch <action> [args…]`. Client objects are
  flat: `app_id`, `monitor`, `tags`, `x/y/width/height`, `floating`; outputs
  expose `name`, `x/y/width/height`, `active`, `active_tag_mask`. Dispatch
  actions used: `focusmon`, `view`, `focusstack`, `togglefloating`, `movewin`
  (delta), `resizewin` (delta), `togglesticky`.
- **mpv_ipc.rs** opens the mpv JSON socket (`std::os::unix::net::UnixStream`),
  writes one `{"command":[…]}` line per request (`loadfile`, `cycle pause`).
- **control.rs** ports `focus_mpv` (monitor/tag hop + focusstack scan),
  `ensure_floating`, the 4-corner `snap`, sticky toggle, start/play/download/stop.
- Pure, unit-tested pieces live in `geometry.rs` / `ytdl.rs`.

### Wallpaper engine (large, graphics/systems code)

Per selected output (one named output, or all):

1. **wayland.rs** — connect display, bind `wl_compositor`, `zwlr_layer_shell_v1`,
   enumerate `wl_output`s (reuse the `mpicker` client pattern; deps
   `wayland-client 0.31`, `wayland-protocols-wlr 0.3` already in the workspace).
   Create a `wl_surface` → `zwlr_layer_surface_v1` on the **background** layer,
   anchored to all edges, exclusive zone -1, input region empty.
2. **egl.rs** — `khronos-egl` for EGL display/config/context; `wl_egl_window`
   from `wayland-egl` to get an `EGLSurface` sized to the output. (If the
   `wayland-egl 0.32` ↔ `wayland-client 0.31` interop doesn't line up, fall back
   to a minimal hand-rolled `wl_egl_window_create` FFI — same pattern as the mpv
   FFI.)
3. **render.rs** — create an `mpv` handle (`mpv_create`, set `loop`, `mute`,
   `hwdec`, etc.), then `mpv_render_context_create` with
   `MPV_RENDER_API_TYPE_OPENGL` and a `get_proc_address` backed by
   `eglGetProcAddress`. Drive it from a **calloop** loop integrating: the
   wayland fd, an mpv render-update wakeup (eventfd set via
   `mpv_render_context_set_update_callback`), and the surface frame callback.
   Each frame: `eglMakeCurrent` → `mpv_render_context_render` (fbo 0, output
   w/h) → `eglSwapBuffers`. Load the source with `loadfile`.
4. Lifecycle: write `$XDG_RUNTIME_DIR/mplay/<output>.pid` on start; `wallpaper
   stop [--output]` reads the pidfile(s) and signals SIGTERM; clean teardown
   frees the render context and the mpv handle. `--daemon` forks + disowns.

### mpv FFI (`mpv_sys.rs`)

Hand-written `extern "C"` declarations for exactly the symbols used (avoids any
external libmpv crate → builds offline; mirrors mpvpaper's direct use of
`<mpv/client.h>` + `<mpv/render_gl.h>`):

```
mpv_create, mpv_initialize, mpv_set_option_string, mpv_command (loadfile),
mpv_render_context_create, mpv_render_context_set_update_callback,
mpv_render_context_render, mpv_render_context_free, mpv_terminate_destroy
```

`build.rs` emits `cargo:rustc-link-lib=mpv` (libmpv is a runtime requirement of
mpv itself, already present).

## Data flow

```
CLI ─► control.rs ─► margo.rs ──(mctl get/dispatch subprocess)──► margo socket
                  └► mpv_ipc.rs ──(JSON line over unix socket)───► mpv

CLI ─► paper::run ─► wayland.rs (layer surface) ─► egl.rs (GL ctx)
                  └► render.rs ─► libmpv render ctx ─► eglSwapBuffers (calloop)
```

## Error handling

- Controller: missing tool (`mctl`/`mpv`/`yt-dlp`/`socat`→ now native) or no mpv
  window → clear stderr message + `notify-send` (best-effort), non-zero exit.
- Engine: wayland/EGL/mpv init failure → log + exit non-zero; a failed output is
  skipped (others continue) when targeting all outputs.

## Testing

- **Unit (pure):** corner-cycle distance/selection, `clamp`, scale-mode parse,
  `is_youtube_url`, clipboard/URL resolution, mctl-JSON → window-rect mapping.
- **Engine:** GPU + wayland session required → manual verification (run on a
  margo session, confirm video on the background; `wallpaper stop` tears down).
  Only pure helpers (output selection, option string building) are unit-tested.
- Gates: `cargo clippy -p mplay --all-targets -D warnings`, `cargo test -p mplay`.

## Install / integration

- `cargo build --release -p mplay` → `/usr/bin/mplay`.
- `man/mplay.1` (roff, same style as `margo.1`/`mctl.1`).
- PKGBUILD + `install.sh`: build + install the binary and man page.
- Dotfiles: `~/.cachy/modules/scripts/bin/margo-mpv.sh` becomes a thin
  `exec mplay "$@"`-style shim (or is removed) — done in the dotfiles repo,
  separate commit.

## Known risks

1. **libmpv render-gl FFI** — hand-rolled; must match the installed libmpv ABI.
   Mitigation: only stable, long-lived symbols; verified against the system
   `mpv/render_gl.h`.
2. **wayland-egl ↔ wayland-client version interop** — fall back to a minimal
   `wl_egl_window` FFI shim if the crate versions don't compose.
3. **mctl subcommand/JSON drift** — confirm `get clients|monitors|focused` shapes
   and the dispatch action names against the live `mctl` during implementation.
