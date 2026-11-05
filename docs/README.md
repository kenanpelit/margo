# docs/

This folder is the source of the documentation site
(<https://kenanpelit.github.io/margo/>, built by `mkdocs build --strict` in
`.github/workflows/docs.yml`; the page list is `nav:` in `mkdocs.yml`). This
README is the one file the site leaves out — it is the "what's in this folder"
index GitHub shows when you browse `/docs/`.

## What lives here

| Kind | Files |
| --- | --- |
| User guide | `index.md`, `install.md`, `features.md`, `widgets.md`, `configuration.md`, `config-reference.md`, `mdock.md`, `companion-tools.md`, `mdots.md`, `scripting.md`, `ipc.md`, `manual-checklist.md` |
| Design notes | `hdr-design.md`, `portal-design.md`, `scripting-design.md`, `install-script.md`, `protocol-comparison.md`, `protocol-matrix.md`, `mplugins-wasm-design.md`, `community-plugins-review.md`, `code-quality-roadmap.md`, `config-conventions.md` (binding for config work) |
| Copies of root files | `roadmap.md`, `changelog.md`, `contributing.md` — `docs.yml` re-copies `road_map.md`, `CHANGELOG.md` and `CONTRIBUTING.md` before every build; edit the root files |
| Release notes | `release-notes/v1.0.0.md`, `v1.0.1.md`. Later releases are described in `CHANGELOG.md` and the GitHub releases only |
| Not on the site | `agents/` (issue-tracker and triage notes for coding agents), `superpowers/` (working plans and specs) |

## Generated or gated content

- The dispatch-action tables in `configuration.md` (and `wiki/Configuration.md`)
  are generated from `margo-config/src/actions.rs`; `cargo test -p mctl` fails
  when they drift — run `just docs-actions`.
- `widgets.md` must list every `BarWidget` variant (`scripts/docs-drift.py`,
  part of `just check` and CI).
- Links to files outside `docs/` (also in the root files copied here) use
  absolute GitHub URLs, so they work both on GitHub and on the site; the
  strict build fails on a broken link or anchor, or a page missing from the nav.
