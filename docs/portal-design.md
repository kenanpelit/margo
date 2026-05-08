# Built-in xdg-desktop-portal backend — design notes

> Status: **planning + scaffolding**. The portals.conf shipped at
> `assets/margo-portals.conf` currently routes ScreenCast / Screenshot
> / RemoteDesktop to `none`; users who need them have to install
> `xdg-desktop-portal-wlr`. This document is the migration plan toward
> hosting these implementations natively inside `margo-portal`, a
> dedicated D-Bus daemon shipped alongside the compositor.

## Why a built-in backend

Margo already implements the Wayland-side capture protocols
(`wlr-screencopy-v1`, `linux-dmabuf-v1`, `wp-presentation`) and runs
its own dispatch loop. Forcing the portal layer through xdp-wlr means
two extra D-Bus hops, an external process the user has to remember to
keep updated, and policy splitting: the portal-wlr restriction list
doesn't see margo's window-rule-driven `block_out_from_screencast`
flag, so blackout-on-screencast clients leak through portal-routed
capture.

Goals:

1. **Single policy**: window-rule `block_out_from_screencast:1`
   suppresses content in capture flows the user goes through portals
   (browser screen-share, screenshot tools using xdp Screenshot).
2. **No xdp-wlr install**: `pacman -Rdd xdg-desktop-portal-wlr`
   should leave a working session.
3. **Region / window picker**: Screenshot's interactive region
   chooser should integrate with margo's overview / focus state
   instead of spawning slurp blindly.

## Architecture

```
                      ┌─────────────────────┐
   browser  ────────► │   xdg-desktop-portal │  (xdp main daemon)
                      └─────────┬───────────┘
                                │ dbus method call
                                ▼
                      ┌─────────────────────┐
                      │   margo-portal      │  ← new crate, this doc
                      │  (separate binary)  │
                      └────┬───────┬────────┘
                           │       │
                           │       │ wayland (wl_display)
                           │       ▼
                           │  ┌─────────────┐
                           │  │   margo     │  (compositor)
                           │  │             │
                           │  └─────────────┘
                           │       ▲
                           │       │ wlr-screencopy / dmabuf
                           │       │
                           │       │
                           ▼       │
                    spawn grim / wf-recorder
                    (where helpful)
```

`margo-portal` runs as a user-systemd service activated lazily by D-Bus
("org.freedesktop.impl.portal.desktop.margo"), implements the relevant
`org.freedesktop.impl.portal.*` interfaces, and where it can't answer a
request natively (e.g. arbitrary file rec) shells out to `grim` /
`wf-recorder` against the live Wayland connection.

## Scoped milestones

The full portal surface is too large for a single sprint. Ship in
priority order, each milestone independently usable.

### M1 — Screenshot (region + full screen)

* `org.freedesktop.impl.portal.Screenshot.Screenshot()` →
  margo-portal opens a wlr-screencopy frame, encodes PNG, returns the
  file URI to xdp.
* `Screenshot.PickColor()` → wlr-screencopy single-pixel sample.
* Interactive flag: when true, spawn slurp for region selection then
  resolve as above.

Estimated size: ~400 LOC + zbus + image crate. Replaces 80 % of what
xdp-wlr is needed for.

### M2 — ScreenCast (browser screen-share)

* `org.freedesktop.impl.portal.ScreenCast.CreateSession()`,
  `SelectSources()`, `Start()`.
* Source picker: native dialog showing margo's outputs + visible
  windows (already enumerable via dwl-ipc-v2).
* Pipewire stream: re-publish the existing wlr-screencopy frames into
  pipewire's portal node. The `block_out_from_screencast` flag is
  honoured by the renderer already; the portal just inherits the
  filter for free.

Estimated size: ~600 LOC + pipewire-rs.

### M3 — RemoteDesktop

Out of scope until M1+M2 settle; remote-desktop adds an input-injection
surface that needs careful design (margo's keyboard / pointer paths
don't currently take synthetic events). Defer.

### M4 — File chooser polish

xdp-gtk handles FileChooser fine and probably always will. We don't
re-implement; we just ensure margo's window-rule for the
`xdg-desktop-portal-(gtk|gnome|wlr)` toplevels (already in
config.example) keeps the dialog floating + sized + focused on first
map.

## Why not just use xdp-wlr?

* xdp-wlr is a `tinywl`-shaped reference impl maintained on a thin
  resource budget — its bug tracker has open year-old issues.
* The `block_out_from_screencast` window-rule margo carries is invisible
  to xdp-wlr (it doesn't look at any compositor state beyond the wlr
  protocols themselves).
* The "select region" UX in xdp-wlr is "spawn slurp, hope the user
  knows what slurp is".

## Crate layout

```
margo-portal/
├── Cargo.toml
├── src/
│   ├── lib.rs            # zbus bus connection, service registration
│   ├── interfaces/
│   │   ├── mod.rs
│   │   ├── screenshot.rs # M1
│   │   └── screencast.rs # M2
│   ├── wayland.rs        # connection to margo via dwl-ipc-v2 +
│   │                     # screencopy capture
│   └── bin/
│       └── margo-portal.rs   # main, wires the bus
├── assets/
│   └── margo-portal.service  # systemd user unit
└── docs/
    └── interfaces.md     # auto-generated from xml stubs
```

A single binary keeps deployment simple. `margo-portal` activates from
`org.freedesktop.impl.portal.desktop.margo` at first xdp call and
sticks around for the session.

## Build deps

* `zbus = "5"` — D-Bus client + service
* `tokio = "1"` (rt-multi-thread + macros) — zbus 5 requires async
* `image = "0.25"` — PNG / JPEG encoding for Screenshot output
* `pipewire = "0.8"` — for M2 only; gate behind `--features
  screencast`
* `slurp` + `grim` runtime dependencies (already optdepends on
  margo-git)

## How packagers ship this

Once M1 lands:

* margo-git PKGBUILD installs `/usr/lib/margo/margo-portal` and
  `/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.margo.service`
* Updates `/usr/share/xdg-desktop-portal/margo-portals.conf` to
  switch `org.freedesktop.impl.portal.Screenshot=margo`.
* Drops `xdg-desktop-portal-wlr` from optdepends — only listed for
  user systems still on the legacy path.

## Why this is `[ ]` and not done

The work to make this real is well scoped (~1500 LOC for M1+M2), but
multiplied by the testing matrix (browser screen share, third-party
screenshot tools, sandboxed apps via xdp's session token flow) it's a
multi-week effort. This document exists so the next person picking it
up doesn't re-derive the architecture from scratch.

Implementation tracker: see issue [TBD] on the GitHub repo.
