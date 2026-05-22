# §12 panel-header rollout across 11 menus

**Date:** 2026-05-22
**Spec:** `mshell-crates/mshell-frame/DESIGN.md` §12 (panel archetype),
§0.6 (one icon family), §1 (tokens).

## 1. Intent

Apply the established §12 panel header to eleven more menus so they
share the clipboard/dashboard language: a leading symbolic glyph + a
SemiBold title + (where there's a clear primary action) a circular
action button, via the shared `.panel-header` / `.panel-title` /
`.panel-header-icon` / `.panel-action-btn` SCSS.

**Chrome consistency only** (user decision): standardize the header,
remove decorative accents, audit tonal tiers / metadata / accent
discipline. **No behaviour changes, no new search fields.**

## 2. Decisions (locked via brainstorming)

- Depth: chrome consistency (not search-add, not re-layout).
- Compact menus (Bluetooth, Audio): get the full header too.
- Delivery: all eleven in one commit + one rebuild (user accepted the
  large-diff risk over batching).
- Missing icons: authored fresh in MargoMaterial (user authorised) to
  keep §0.6 one-family — no system-theme fallbacks.

## 3. New icons (MargoMaterial, MDI-style, 24×24, single path)

`assets/icons/MargoMaterial/symbolic/`, installed by the PKGBUILD's
`cp -a` of the whole tree (so the user's `rebuild.sh` picks them up):

- `view-refresh-symbolic` — circular refresh arrow
- `edit-copy-symbolic` — two stacked sheets
- `open-in-new-symbolic` — box + out-arrow
- `cube-symbolic` — 3D cube (Podman / containers)

All rasterise-verified to render as the intended glyph.

## 4. Per-menu header

| Menu | Leading icon | Title | Header action(s) | Notes |
|---|---|---|---|---|
| UFW | `firewall-symbolic` | UFW Firewall | (status badge + toggle kept) | title-row restyle |
| DNS / VPN | `network-vpn-symbolic` | DNS / VPN | — (subtitle + badge kept) | was `vpn-symbolic` px28 |
| Podman | `cube-symbolic` | Podman | ↻ `view-refresh` (was text Refresh) | counter kept |
| Notes | `notes-symbolic` | Notes Hub | — (+Add stays in body) | |
| Power | `power-profile-balanced-symbolic` | Power | — | header injected above battery hero |
| Valent | `phone-symbolic` | Valent Connect | (switcher/refresh kept) | title→panel-title |
| Network | `network-wired-symbolic` | Network | — | header injected above connection hero |
| Bluetooth | `bluetooth-active-symbolic` | Bluetooth | — | header injected (was header-less) |
| Audio | `audio-volume-high-symbolic` | Audio | — | header injected (was header-less) |
| CPU | `cpu-symbolic` | CPU | — | header injected above identity line |
| Public IP | `globe-symbolic` | Public IP | — | header injected; duplicate globe + caption removed from hero; footer Copy/Refresh/Open kept as labelled buttons (clearer than icons) |

`.panel-title` is 24px SemiBold (bespoke, §12). Bluetooth + Audio
needed `use relm4::gtk::prelude::*;` so the new header widgets'
`set_orientation` / `set_label` / `set_icon_name` resolve.

## 5. What stayed the same

All menu behaviour, body content, footers, revealer rows, sliders,
device pickers, the standalone clock menu, and `MenuWidget::Clock`.
Decorative accents removed only where the header replaced them.

## 6. Verification

`cargo check -p mshell-frame` passes (compile-verified). Visual is
pending the user's `rebuild.sh` (per the build-workflow split): then
open each via `mshellctl menu <name>` and confirm the header reads
`[icon] Title …`, the new icons render, accents are calm, and
`journalctl --user-unit mshell` is panic-free. The single big diff
means several may need follow-up nudges.

## 7. Follow-up risk

This is the riskiest delivery shape (11 heterogeneous components, one
unverified diff). Hero/header-less menus (Power, Network, Bluetooth,
Audio, CPU, IP) got a header *injected* at the top — those are the most
likely to need a spacing/placement nudge after the first look.
