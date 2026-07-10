# mgreet V1 — the login screen gets a backdrop, and its theme starts working

Date: 2026-07-10
Status: approved, ready for implementation
Scope: `mgreet`, `mlogind`
Depends on: [A2](2026-07-10-mlogind-a2-unprivileged-greeter-design.md) (shipped, `24455f9d`)

## Why

Two things, and the second is a bug.

The login screen paints a flat vertical gradient. `.mgreet-scrim` is opaque on
purpose — a greeter must never let the host compositor's default wallpaper bleed
through — so the desktop's wallpaper is simply absent. Every greeter worth the
name shows one.

And `mgreet/src/main.rs:253` reads `/etc/mgreet/theme.css` in real-greeter mode.
**Nothing in this repository writes that file.** Not the code, not the
`PKGBUILD`, not `install.sh`; the directory does not exist on a running machine.
So the greeter renders the baked Dracula palette no matter what the wallpaper
theme is, and has since it was written. Any visual work has to fix that first,
or the colours stay wrong underneath whatever we put on top.

## What we take, and what we leave

From **plasma-login-manager**: nothing in this slice. Its wallpaper is an
external plugin; its blur is a QML `FastBlur` ramped on card activation. The ramp
belongs to the motion slice, not here. (Its prompt protocol —
`Login`/`Succeeded`/`Failed` with an unused `informationMessage` — is *behind*
what A1 already ships. There is nothing to take there.)

From **atrium**: the idea that a greeter has a backdrop at all, and that the
image is scaled `cover`, centred. We reject its directory-of-random-images knob:
we chose a sync-only mirror of the desktop, so there is nothing to pick from.

## The shape

`mshell` already caches the wallpaper decoded, in
`~/.cache/mshell/wallpaper.raw`, as `[u32 LE width][u32 LE height][RGBA]` —
and it is the *theme-filtered* image, which is to say exactly the pixels the
desktop puts on screen. Verified on a live machine: `1733x1080`, 7 486 568 bytes,
`8 + w*h*4` to the byte.

That means no image decoding anywhere. Not in `mlogind`, not in `mgreet`. No new
dependency in either crate. Downscale and blur are arithmetic over a byte
slice; `mgreet` hands the result to `gdk::MemoryTexture` directly.

```
~/.cache/mshell/wallpaper.raw       theme-filtered RGBA — what the desktop shows
~/.cache/mshell/last_theme.css      matugen colours
~/.config/margo/mlogind-variables.toml   the TUI greeter's palette (already synced)
        │
        │  sync step, root, after the user's session ends
        │    · area-average downscale to a 960 px long edge
        │    · 3 box-blur passes, radius 12 (≈ gaussian σ 15 at that scale)
        ▼
/var/lib/mgreet/background.raw      0644, ~2.3 MB, same [w][h][RGBA] header
/var/lib/mgreet/theme.css           0644
/etc/mlogind/variables.toml         0644, as today
        │
        │  mgreet, running as `mlogind-greeter`
        ▼
gdk::MemoryTexture → gtk::Picture, content-fit: Cover
```

What reaches the greeter is a downscaled, blurred **derivative**. The sharp
original never leaves `$HOME`. That privacy property is free — it falls out of
baking the blur rather than applying it at render time.

`theme.css` moves from `/etc/mgreet/` to `/var/lib/mgreet/`. The `/etc` path is
dead anyway, and `docs/config-conventions.md` is explicit that a machine-written
file does not belong in `/etc`.

## The privilege boundary

The sync must run as root: `/home/<user>` is `drwx--x---` (verified — the
`mlogind-greeter` user cannot even traverse it) and `/var/lib/mgreet` is root's.

But root must not *open* a path the user controls. If
`~/.cache/mshell/wallpaper.raw` were a symlink to `/etc/shadow`, root would read
it and write a derivative into a world-readable file.

So root never opens it:

```
root                              child (uid = the user)
  mkdir /var/lib/mgreet 0755
  open(background.raw.tmp) → fd
  open(theme.css.tmp)     → fd
  fork() ───────────────────────▶ alarm(5)
                                  setgroups → setgid → setuid
                                  read  ~/.cache/mshell/wallpaper.raw
                                  downscale + box_blur
                                  write(inherited fd)
                                  copy  ~/.cache/mshell/last_theme.css
                                  exit(0)
  waitpid ◀─────────────────────
  validate header vs byte count
  fsync, rename(tmp → final)
```

The child holds two already-open fds and can open nothing else that matters. A
symlink buys the attacker exactly the privilege they already had. `alarm(5)` is
the child's own watchdog, so a wedged child cannot stall the next greeter; the
parent's `waitpid` always returns.

`drop_privileges` already exists (`mlogind/src/runner/greeter_session.rs`) and
uses the same setgroups → setgid → setuid order, for the same reason.

Note this closes an existing hole as well. `sync_theme()`
(`mlogind/src/main.rs:44`) reads `~/.config/margo/mlogind-variables.toml` as
root today. Under `sudo mlogind sync-theme` the user is already root, so it is
not an escalation — but the automatic trigger runs while the user is *not* root,
and there it would be. All three source files go through the unprivileged
reader.

## Formats and validation

Header: `[u32 LE width][u32 LE height][RGBA…]`, and `len == 8 + w*h*4`.

The parent validates the child's output before it renames anything:

- `w > 0`, `h > 0`
- `w * h * 4` computed with `checked_mul`; overflow → reject
- `len == 8 + w*h*4` exactly
- `w <= MAX_EDGE`, `h <= MAX_EDGE` (the child was supposed to downscale)

The child validates its input the same way, and refuses an input over
`MAX_INPUT_BYTES`. A file that fails any check means no background, not a broken
one.

Constants, in one place, documented where they sit:

| Name | Value | Why |
|---|---|---|
| `MAX_EDGE` | 960 | a blurred 960 px image upscaled to 4K is indistinguishable from a blurred 4K one, and costs 2.3 MB instead of 33 MB |
| `BLUR_RADIUS` | 12 | ≈ 60 px at 4K after `Cover` upscales it |
| `BLUR_PASSES` | 3 | three box passes approximate a gaussian; the fourth is not visible |
| `MAX_INPUT_BYTES` | 64 MiB | 4K RGBA is 33 MB; twice that is generous and bounds the child |
| `CHILD_TIMEOUT` | 5 s | `alarm(5)`; the work is ~10 ms |

Alpha: the wallpaper is opaque. Each channel is blurred independently and the
output alpha is forced to 255, so no premultiplication question arises.

## Rendering

`build_window`'s overlay gains a bottom layer:

```
Overlay
├── child     GtkPicture (the backdrop)   ← or the scrim, when there is no image
└── overlays  dim box, card, battery, power footer
```

- `Picture::for_paintable(&texture)`, `content_fit = Cover`, `can_shrink = true`.
- The dim is its own box: `background: var(--bg); opacity: .55`. Not
  `alpha(var(--bg), …)` — whether GTK's colour functions accept `var()` is not
  something this should rest on. `opacity` on a box is unambiguous.
- Both the picture and the dim get `set_can_target(false)` so neither eats input.
- The window keeps `background: var(--bg)`, so an uncovered edge is never
  transparent.

The dim tracks the palette, so the matugen overlay recolours it without a
re-bake. Only the blur is baked.

**No image, no change.** With `/var/lib/mgreet/background.raw` absent, the class
`.has-background` is never added, the overlay's child stays the scrim, and the
screen is byte-for-byte what it renders today. This is the rule the whole slice
answers to: a greeter that cannot find its wallpaper still logs you in.

### Preview

`mgreet --preview` (and a bare `mgreet` under a live session) reads the same
`/var/lib/mgreet/background.raw` — it is world-readable, and a preview that
showed a different backdrop from the real thing would be worth less than no
preview. Its colours keep coming from `~/.cache/mshell/last_theme.css`, exactly
as today, so a dry run matches the desktop it was launched from even before the
first sync has ever happened.

## Triggers

1. **The runner, after the user's session ends**, before it exits. It knows the
   user, it is root, and nothing is waiting on it. The next greeter shows the
   wallpaper the user just left.
2. **`sudo mlogind sync-theme`**, extended to do the same work. The escape hatch,
   and what an admin runs after a fresh install.

One function, two call sites. A sync failure is logged at `WARN` and never
changes the runner's exit code — the login path does not fail because a
decoration could not be prepared.

A session that ends by `poweroff` may be killed before the sync completes. Then
the backdrop is one wallpaper change stale until the next logout. That is the
correct trade: nothing should delay a shutdown to update a login screen.

## Testing

The core is pure and lives away from root, fds and GTK:

- `parse_header(&[u8]) -> Option<(u32, u32)>` and its round trip
- `downscale(rgba, w, h, max_edge) -> (rgba, w, h)` — area average; identity when
  the image is already small; aspect preserved; a 1×1 input
- `box_blur(rgba, w, h, radius, passes)` — a single bright pixel spreads
  symmetrically; a uniform image is unchanged; radius 0 is identity
- `validate(len, w, h)` — the overflow, zero-dimension and size-mismatch cases

`mgreet`'s `background::parse` gets the same treatment: a truncated file, a
lying header, and a good one.

## Packaging

Nothing. `/var/lib/mgreet` is created by the sync at runtime, `0755`, the way
`/run/mlogind` already is. No new dependency in either crate means `Cargo.lock`
does not move, so `PKGBUILD` and the AUR package are untouched by this slice.

## Out of scope

Queued, deliberately not here: the blur ramp on card activation, the reject
shake, the busy state, the card following the active monitor, the avatar, the
keyboard-layout indicator, hiding the session picker when there is only one
session, idle blank. Each is its own slice.

Also rejected: atrium's directory-of-random-images. A sync-only mirror has one
image by definition.

## Hardware, and only hardware

None of this can be seen from here. `mgreet` is not even reachable until A2's
first successful boot, and driving the live greeter with synthetic input is
forbidden.

After it lands: log in, log out, and look at the greeter — it should carry a
blurred version of the wallpaper and the matugen colours, both for the first
time. Then check `ls -l /var/lib/mgreet` (0644, root), `od -An -tu4 -N8` on
`background.raw` (a sane `w×h`, both ≤ 960), and that
`sudo mlogind sync-theme` prints what it wrote. Delete
`/var/lib/mgreet/background.raw` and confirm the greeter falls back to today's
gradient rather than to a black screen.
