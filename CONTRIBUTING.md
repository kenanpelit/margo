# Contributing to margo

Thanks for stopping by. margo is a Rust + Smithay Wayland compositor in
the dwm/dwl tradition — actively developed as a daily driver, not a
museum piece. The bar for contributions is moderate: read this guide,
poke around the code, ship something small first, expand from there.

## Quick start

```bash
git clone https://github.com/kenanpelit/margo
cd margo
cargo build --workspace
cargo test --workspace
cargo run --bin margo -- --winit         # nested mode for dev
```

System dependencies (Arch / pacman names — translate for your distro):

```
wayland libinput libxkbcommon mesa seatd pixman libdrm pcre2
xorg-xwayland libnotify grim slurp wl-clipboard
```

The PKGBUILD at the repo root is the canonical install recipe. For
local dev, `cargo run -- --winit` runs margo in a nested wayland window
inside whatever compositor you're already in — the fastest iteration
loop. `scripts/smoke-winit.sh` exercises the nested-mode end-to-end.

## Code layout

```
margo/             compositor binary; bulk of the work lives here
margo-config/      config parser crate
margo-ipc/         mctl + the dispatch action catalogue
mlayout/           named monitor-arrangement profiles
mscreenshot/       screenshot helper (grim/slurp/wl-copy orchestration)
contrib/           shell completions, example init.rhai, plugins
docs/              design docs (HDR, scripting, portal)
scripts/           smoke tests + post-install validation
.github/workflows/ ci.yml (build/test/clippy/check-config) + smoke.yml
```

`margo/src/state.rs` is the heart — `MargoState` holds every long-lived
piece. `margo/src/backend/{udev,winit}.rs` are the two backends.
`margo/src/protocols/` is hand-rolled or smithay-extension wayland
protocols. `margo/src/render/` holds custom GLES render elements
(border, shadow, resize, open/close, linear-light HDR scaffolding).

## Lint posture

`cargo clippy --workspace --all-targets -- -D warnings` is a CI gate.
Before pushing, run it locally — fixing a new clippy finding takes 30
seconds at PR-author time and doesn't block reviewers.

`clippy.toml` documents the project-level overrides (interior-mutability
allowlist for smithay's `Window` / `Output` handles). Categories
deliberately suppressed across the crate root in `margo/src/main.rs`:
`too_many_arguments` (render helpers), `doc_overindented_list_items`,
`collapsible_if`, etc. Don't add new `#[allow(clippy::...)]` without a
reason in the surrounding comment.

## Testing

```bash
cargo test --workspace             # unit + snapshot tests (62 today)
cargo insta review                 # accept new layout snapshots
scripts/smoke-winit.sh             # end-to-end (nested mode)
scripts/post-install-smoke.sh      # post-install validation
mctl check-config <conf>           # offline config validation
```

`margo/src/layout/snapshot_tests.rs` locks the geometry output of every
layout algorithm in committed `.snap` files. Adding a new layout means
adding new snapshot fixtures alongside it; see the module docs for the
add-a-scenario walkthrough.

`render/hdr_metadata.rs` and `render/linear_composite.rs` contain CPU-
side reference implementations of every transfer function so the GLSL
math can be verified without a live GLES context.

## Style

The codebase has a particular feel: heavy module-level docstrings
explaining *why*, not *what*; line-level comments only where a hidden
constraint or past bug makes the code surprising; dense in-line
explanation of trade-offs at function-level. Match it. Don't write
docstrings that paraphrase the function name.

Commit messages follow the conventional-commit shape
(`feat(render): ...`, `fix(ipc): ...`, `chore(lint): ...`). The body
explains *why* the change was made and what the alternative was —
"add foo" is a bad subject; "feat(foo): wire X into Y so Z stops
flickering" is a good one. Bodies use plain English paragraphs, not
bullet-list manifests.

If you're touching a hot path, add a `tracy_client::span!` so the
profiler picks it up. The macro is no-op without `--features
profile-with-tracy`, so cost is zero in normal builds.

## Pull requests

Keep PRs focused: one feature or one bug fix per PR. Multiple changes
that happen to land in the same week belong in separate PRs. When
addressing review comments, squash into the relevant commit rather than
piling fix-up commits on top.

Before opening:

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all` (when in doubt)
- [ ] Manual test in `--winit` nested mode for anything render- /
      input-related; manual test on real DRM hardware for
      output-management / DRM-format / HDR / cursor-plane changes.
- [ ] Roadmap / docs updated when behaviour changes meaningfully.

Big features need a roadmap entry first (open an issue or update
`road_map.md`). Don't write 1000 lines of code chasing a feature
nobody agreed to ship.

## Licensing

GPL-3.0-or-later. By submitting code you agree to license it under the
same terms. Original portions of dwl/dwm/sway/tinywl/wlroots are
preserved under their respective licences — see `LICENSE.*`.

## Communication

- **Bug reports**: GitHub issues. Include `mctl status --json` output,
  `journalctl --user -u margo -n 200`, and the smallest config that
  reproduces the issue.
- **Feature ideas**: GitHub issues with the `enhancement` label.
  Discuss before coding so we don't end up with overlapping PRs.
- **Security issues**: open a private security advisory via GitHub's
  security tab, not a public issue.

## On AI-generated contributions

If you used an LLM to draft a contribution, *you* are responsible for
checking that it's correct, idiomatic, and trimmed to just what's
needed. Don't paste verbatim output. Don't open PRs that nobody
including yourself has read end-to-end. The bar for review effort is
the same regardless of how the patch was authored — make it easy to
review by reading what you submit.
