# HDR + colour management — design notes

> Status: **planning**. Margo currently runs a single 8-bit-per-channel
> sRGB pipeline. HDR / WCG content is delivered tone-mapped at the
> client (Chromium / mpv) and presents as SDR. This document is the
> migration plan toward `wp_color_management_v1` + KMS HDR scan-out.

## Why this is hard

HDR is not "set a pixel format and go". It needs four things in
lock-step:

1. **Protocol surface**: `wp_color_management_v1` (staging) lets a
   client describe its surface's colour space + transfer function
   (sRGB, BT.2020, Rec.709, PQ, HLG…). Smithay 0.7 doesn't ship a
   handler — we have to wayland-scan the XML and write our own.
2. **Render pipeline**: composite happens in linear light, not in the
   transfer-encoded domain. Means swapping the GLES2 pipeline for a
   linear-FP16 framebuffer with per-surface decoding before mixing.
3. **KMS scan-out**: feed the right `HDR_OUTPUT_METADATA` blob and
   the right pixel format (BT.2020 + PQ for HDR10) to the DRM
   atomic commit. Smithay's `DrmCompositor` doesn't expose HDR
   metadata APIs in 0.7; we'd interface with `drm-rs` directly.
4. **ICC profile loading**: read `~/.config/colord/icc/<output>.icc`
   (or `xdg-desktop-portal-gnome` Settings query), bake it into the
   per-output 3D LUT used during composition.

Skipping any of the four = wrong colours. There's no MVP that cuts
two of them out.

## Target surface

What a "done" release looks like, user-side:

* mpv playing an HDR10 file: framebuffer flips to 10-bit BT.2020/PQ
  on the targeted output, the OS desktop dims to SDR luminance to
  preserve highlight headroom in the video, the rest of the screen
  composites correctly in HDR space.
* Chromium reporting an HDR-capable display (`screen.colorGamut === "p3"`,
  `window.matchMedia('(dynamic-range: high)').matches === true`).
* `colormgr get-display-profile <output>` returns a profile, and
  margo applies it as a pre-scan-out 3D LUT. Switching profiles
  takes effect within one frame.
* Clients that don't speak `wp_color_management_v1` keep getting
  the same sRGB experience they have today — no regressions.

## Phased rollout

### Phase 1 — Protocol scaffolding + advertise capability

* Hand-generate Rust bindings for `wp_color_management_v1` from
  `/usr/share/wayland-protocols/staging/color-management/color-management-v1.xml`
  using `wayland-scanner` (margo already does this for dwl-ipc-v2 and
  several other protocols — same machinery applies).
* Register the manager global. For now, accept every well-known
  named primary set (sRGB, BT.2020, Display-P3) and every transfer
  function (sRGB, PQ, HLG, linear). Reject ICC-blob params with
  `unsupported_feature` until phase 4.
* Per-surface state stores the requested colour space + transfer
  function but does NOT change rendering. Clients see "the
  compositor accepted my preference" without behaviour shifting.

  This gives Chromium / mpv enough handshake to enable their
  internal HDR paths *as if* the compositor were colour-managed,
  even though the actual composition is still SDR. Useful as a
  capability advertisement step ahead of phase 2.

  Estimated size: ~300 LOC.

### Phase 2 — Linear-light composite path

* Add a `MARGO_COLOR_LINEAR=1` env-var (eventually a config knob).
  When on:
  * Allocate the swapchain in fp16 RGBA (`DrmFourcc::Abgr16161616F`)
    instead of `DrmFourcc::Argb8888`.
  * Each surface's render element decodes its declared transfer
    function before sampling — sRGB clients go through the inverse
    sRGB curve, PQ clients through inverse-PQ, etc.
  * Final framebuffer encodes back to the output's transfer
    function before scan-out.
* Per-pass cost: a fragment-shader hop. Measurable on lower-end iGPUs;
  acceptable on anything with hardware fp16 mixing (Intel/AMD/NV
  iGPUs from 2018+).

  Estimated size: ~500 LOC + a notable shader-test matrix.

### Phase 3 — KMS HDR scan-out

* Negotiate the output's preferred HDR format via DRM
  `EDR_PROPERTIES` blob queries.
* When the focused-output has an HDR-capable surface visible, set
  `HDR_OUTPUT_METADATA` with the surface's PQ / HLG metadata,
  flip to a 10-bit pixel format, drive the right colorimetry +
  EOTF.
* When no HDR client is visible, the output stays in SDR mode (no
  metadata blob). Avoids the "everything's dim" complaint when a
  user has an HDR monitor but a tab loses HDR focus.

  Estimated size: ~400 LOC + heavy hardware-test matrix.

### Phase 4 — ICC profile per output

* Read `colord` per-output ICC profile via D-Bus (or
  `xdg-desktop-portal Settings` colour scheme query).
* Bake the profile's colour table into a 3D LUT loaded into a per-
  output sampler.
* Sample the LUT in the final encode pass, after composition.
* `mctl color show <output>` / `mctl color load <icc>` for
  scripting.

  Estimated size: ~300 LOC + colord D-Bus adapter.

## Why this is a multi-month project

Each phase is itself a multi-sprint effort. Hardware testing matrix:

| GPU         | sRGB | linear comp | HDR flip | ICC |
|-------------|------|-------------|----------|-----|
| Intel TGL+  | ok   | ok          | ok       | ok  |
| Intel ICL-  | ok   | ok          | partial  | ok  |
| AMD GCN5+   | ok   | ok          | ok       | ok  |
| AMD Polaris | ok   | ok          | partial  | ok  |
| NVIDIA      | ok   | varies      | varies   | varies |

…and that's just the desktop case; the user's eDP-1 panel on the
mobile setup is a separate matrix entry. Every HDR-emitting client
(mpv, Chromium, gamescope, OBS) has its own fall-back path.

## Why the placeholder is `[ ]`

The smaller P5 items (adaptive layout, spatial canvas, drop shadow)
fit in single sprints because their state machines are local. HDR
is end-to-end: Wayland protocol → render path → KMS → output
metadata. The four phases above are real work blocks, each measured
in weeks not days. Until they all land, the user sees no benefit.

This document exists so the next person picking it up doesn't burn
a week re-discovering that smithay 0.7 doesn't expose HDR in the
DrmCompositor and they need to drop down to drm-rs.

## Build deps for each phase

| Phase | New deps |
|-------|---------|
| 1     | wayland-scanner generated bindings (already vendored) |
| 2     | Shader source for inverse-sRGB / inverse-PQ / inverse-HLG; no new crate |
| 3     | drm-rs upgrade (current 0.x doesn't expose all HDR props) |
| 4     | colord-cli (subprocess) or zbus client for `org.gnome.Settings` |

## What we ship in this commit

Just this document — design + rollout plan + hardware support
matrix. No code; the surface is too large to half-build. The
protocol XML lives at `/usr/share/wayland-protocols/staging/...`
and is the entry point for whoever picks up Phase 1 first.

Implementation tracker: GitHub issue [TBD].
