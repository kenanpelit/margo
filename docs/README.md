# Margo design docs + manual checklist

This folder is **not** end-user documentation — that lives in the
top-level `README.md` and the bundled `mctl --help` / `mctl actions
--verbose` output. The four files here are working notes for
contributors:

| File | Purpose | Status |
|---|---|---|
| [`portal-design.md`](portal-design.md) | Four-milestone rollout for a built-in `xdg-desktop-portal` backend (screencast, screenshot, file chooser, activation policy). Smithay 0.7 ships no full xdp handler; this doc is the spec the next contributor follows when the work starts. | Design only, no code shipped. |
| [`hdr-design.md`](hdr-design.md) | Four-phase rollout for HDR + colour management — `wp_color_management_v1` scaffolding (Phase 1 — code in `margo/src/protocols/color_management.rs`, currently global is gated off until Phase 2 lands), linear-light fp16 composite, KMS HDR scan-out, ICC profile per output. Hardware-test matrix included. | Phase 1 code partial; Phase 2-4 design only. |
| [`scripting-design.md`](scripting-design.md) | Five-phase rollout for the embedded Rhai scripting engine. Phases 1–3 (engine + dispatch bindings + state introspection + event hooks) shipped in `margo/src/scripting.rs`; Phases 4–5 (`mctl run`, plugin packaging) tracked here. | Phases 1-3 shipped; 4-5 design only. |
| [`manual-checklist.md`](manual-checklist.md) | 13-section validation checklist run after a fresh install or before tagging a release. Covers session bring-up, layer shells, notifications, clipboard, multi-monitor, lock, scratchpad, animations, gamma, portal, recording, XWayland, idle behaviour. | Live; update when adding new features users should sanity-check. |

The folder is `.gitignore`-listed by default so rustdoc output
doesn't pollute the tree. Each design doc is `git add -f`'d
intentionally — see the commits that introduced them
(`1f5b6b0` portal, `15b1e38` HDR, `562b5f7`/`13bdd57`/`769141e`
scripting, `f5b8d71` manual checklist).

## Why these four and not more

Three categories of documentation deliberately don't live here:

* **End-user docs** — README.md handles install, basic
  configuration, and the per-action reference is generated
  from `margo-ipc/src/actions.rs` via `mctl actions --verbose`.
* **Per-feature deep dives** — code-adjacent comments + the
  module-level docstring (`//!` block at the top of each
  `.rs` file) are the source of truth. Duplicating into
  markdown drifts.
* **API reference** — `cargo doc --workspace --no-deps`
  generates from the source.

So this folder stays small and contributor-facing. Adding a
new file here is a signal that the work is multi-month and
needs a written plan before code starts.
