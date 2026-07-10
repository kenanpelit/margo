# Seamless greeter boot — the wallpaper that is already right

**Date:** 2026-07-10
**Status:** implemented (Flash 2 — wallpaper sync); Flash 1 (VT) deferred
**Scope:** kill the boot flash where margo shows a generic wallpaper (and the VT
shows mlogind) before mgreet draws. Binary consolidation is explicitly **out of
scope for this slice** — see the closing section.

## What shipped

- `margo/src/wallpaper.rs`: `decode_to_buffer` now recognises a `.raw` path and
  reads it as a headed `[u32 LE w][u32 LE h][RGBA]` buffer (`decode_raw` +
  `parse_raw_header`, mirroring `theme_sync.rs`'s validation) instead of routing
  it through the `image` crate. `image::ImageError: From<io::Error>` lets the raw
  failure surface through the existing `warn!(error = %e)` path. Six unit tests
  cover the header contract (round-trip, disagreeing length, truncation, zero
  dimension, overflow, extension match). The linchpin was already wired:
  `state.rs:1173` calls `WallpaperState::load(config.wallpaper.as_deref())`.
- `mlogind/src/theme_sync.rs`: new `pub fn background_path()` — the single owner
  of `/var/lib/mgreet/background.raw`, so the greeter-conf writer asks for the
  path rather than hardcoding it in a second place.
- `mlogind/src/main.rs`: `write_greeter_conf` splits into a pure
  `greeter_conf_text(xkb, backdrop)` + the IO wrapper. It appends
  `wallpaper = /var/lib/mgreet/background.raw` **only when the baked file
  exists** (self-correcting: absent on a machine's first-ever boot, present on
  every greeter after the first login, since the file persists in `/var/lib`).
  Two tests: the line appears iff the file exists, and the config never carries
  an autostart.

**Flash 1 (the VT) was deliberately not touched.** The user's chosen direction
was "Senkron zemin" (Flash 2). Clearing/blanking VT 7 before margo takes DRM is a
separate, riskier change (KD_GRAPHICS / VT ioctls on the login path) and is left
as an explicit follow-up rather than bundled in.

## The problem, measured

From one real boot (`/var/log/mlogind.log`, `/run/mlogind/margo-greeter.log`):

```
18:41:02.0  mlogind daemon starts, switches to VT 7, launches the "gui" host
18:41:03.25 margo greeter loads /usr/share/margo/wallpapers/default.jpg  ← WRONG image
18:41:03.65 margo udev backend ready (screen is now lit)
18:41:04.60 mgreet's first layer surface arrives
18:41:04.94 keyboard focus moves to mgreet
```

Two distinct flashes, both architectural:

1. **VT / mlogind (~1.6 s):** VT 7 from daemon start until margo takes the screen.
2. **Wrong wallpaper (~1.3 s):** margo greeter has no `wallpaper` in its
   `greeter.conf`, so it falls back to the packaged `default.jpg` — a generic
   image unrelated to the user's desktop — and shows it full-screen until mgreet's
   layer surface covers it.

Flash 2 is the one the user sees as "duvar kağıdı gözüküyor": not a transition
artefact but *the wrong picture*, on screen for over a second.

## Why merging mlogind + mgreet does NOT fix this

The chain is `mlogind → margo → mgreet`. mgreet is already a layer surface
*inside* margo; the thing that flashes is **margo the compositor**, a separate
process that renders its own screen for ~1.3 s before mgreet is ready. Merging
the two *binaries* (mlogind, mgreet) leaves margo untouched, so it would still
show `default.jpg` for that window. Consolidation is a worthwhile code-layout
change (the user has asked for it as a separate task) but it is not the lever
for this bug. The lever is **what margo shows before mgreet draws**.

## The solution: give margo mgreet's own backdrop

`mlogind`'s theme sync already produces `/var/lib/mgreet/background.raw` — the
user's wallpaper, downscaled and blurred, in `[u32 LE w][u32 LE h][RGBA]`. That
is *exactly* the pixels mgreet paints as its backdrop. If margo greeter loads the
same file, its first frame is identical to mgreet's, and the hand-off is
invisible: the wallpaper never changes, only the login card fades in over it.

Three small pieces:

### 1. margo learns to read the raw backdrop format

`margo/src/wallpaper.rs::decode_to_buffer` currently does `image::open(path)?`
(jpg/png/webp only). Add a branch: when the path ends in `.raw`, parse the
`[u32 LE w][u32 LE h][RGBA]` header ourselves and build the `MemoryRenderBuffer`
from the trailing bytes — no image crate, no new dependency. Validate `len == 8 +
w*h*4` and reject a zero/overflowing dimension (the file is machine-written but
we still don't index past it). RGBA maps straight to the buffer, as the existing
code notes ("No swizzle"). ~20 lines, entirely additive; every existing jpg/png
path is unchanged.

The backdrop is opaque (theme sync forces alpha 255), so there is no
premultiply question.

### 2. mlogind points greeter.conf at it

`mlogind/src/main.rs::write_greeter_conf` writes the minimal greeter config
(keyboard layout only, today). Add one line — `wallpaper = /var/lib/mgreet/
background.raw` — **but only when that file exists**, so a first boot with no
synced wallpaper still falls back to margo's default rather than pointing at a
missing path. The theme sync runs before the greeter host (`refresh_greeter_theme`
in the runner), so on any boot after the first login the file is there.

Ordering caveat to verify: the sync writes `background.raw` from the *previous*
session's wallpaper. On the very first boot on a fresh machine there is no synced
file yet, and margo shows its packaged default — acceptable, and self-correcting
after the first login.

### 3. The VT flash (secondary)

Flash 1 is smaller and separate. mlogind already switches to VT 7; before it
launches the gui host it can clear that VT and hide the cursor, so the ~1.6 s
before margo is a clean black rather than a console with a blinking caret. This
is a nice-to-have in this slice — if it turns out to need `KD_GRAPHICS`/ioctl
gymnastics that risk the login path, it is deferred rather than rushed. The
wallpaper flash (Flash 2) is the one that matters and is fully addressed by 1+2.

## Formats and validation

- `background.raw`: `[u32 LE width][u32 LE height][RGBA bytes]`, `len == 8 +
  w*h*4`. This is `mlogind/src/theme_sync.rs`'s existing output format and
  `mgreet/src/background.rs`'s existing input format — margo becomes the third
  reader of the same contract. Worth a shared note in each file so the format
  isn't changed in one place only.
- margo's raw branch rejects: short file, header/length disagreement, zero
  dimension, `w*h*4` overflow. Same checks mgreet and theme sync already make.

## Testing

- **margo**: a pure `parse_raw_header(bytes) -> Option<(w,h)>` unit test (well-formed
  round-trips; truncated / lying-header / zero-dim / overflow all reject), mirroring
  the tests already in `theme_sync.rs` and `background.rs`.
- **mlogind**: `write_greeter_conf` emits the `wallpaper` line when the file
  exists and omits it when absent — testable by pointing the existence check at a
  tempdir.
- **On hardware (user)**: rebuild margo + mlogind, reboot, confirm the login comes
  up on the user's own (blurred) wallpaper with no generic-image flash and no
  wallpaper jump when the card appears. margo takes effect on re-login, mlogind on
  the next daemon start.

## Out of scope — the binary consolidation

The user has approved merging mlogind + mgreet into one binary **as a separate
follow-up**, not part of this slice. It does not fix the flash (see above), it
touches the A2 privilege boundary (mlogind runs as root, mgreet as the
unprivileged `mlogind-greeter`), and it changes packaging (`PKGBUILD`, the
`install -Dm644` lines, the `backup=` entry). It deserves its own spec: a single
binary that dispatches by subcommand/mode into orchestrator+PAM vs. GUI greeter
(the way `mctl` is one binary with many subcommands), with the privilege
transition kept exactly where A2 put it. Deferred deliberately so the flash fix
ships small and verifiable.

## Files touched (this slice)

- `margo/src/wallpaper.rs` — raw `.raw` branch in `decode_to_buffer` + header
  parse + test.
- `mlogind/src/main.rs` — one conditional `wallpaper = …` line in
  `write_greeter_conf` + test.
- Doc note in `theme_sync.rs` / `background.rs` / margo's raw branch that the
  three share one format.
- No new dependencies; `Cargo.lock` and `PKGBUILD` untouched.
