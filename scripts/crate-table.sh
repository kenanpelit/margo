#!/usr/bin/env bash
# crate-table.sh — print the workspace crate table from `cargo metadata`.
#
# CLAUDE.md and other docs list every workspace crate with its role; a hand
# list drifts (crates get added/removed, counts go stale). Regenerate it:
#
#   scripts/crate-table.sh            # markdown: top-level crates, then mshell-crates/
#   scripts/crate-table.sh --count    # just the member counts
#
# The "Role" column is the crate's Cargo.toml `description` — give a new crate
# one, and it shows up here. Needs cargo + python3.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo metadata --no-deps --format-version 1 --offline 2>/dev/null \
    | python3 -c '
import json, os, sys

count_only = len(sys.argv) > 1 and sys.argv[1] == "--count"
meta = json.load(sys.stdin)
root = meta["workspace_root"]
members = set(meta["workspace_members"])
rows = []
for pkg in meta["packages"]:
    if pkg["id"] not in members:
        continue
    path = os.path.relpath(os.path.dirname(pkg["manifest_path"]), root)
    desc = " ".join((pkg.get("description") or "").split()) or "(no description)"
    rows.append((path, pkg["name"], desc))
rows.sort()
top = [r for r in rows if "/" not in r[0]]
helpers = [r for r in rows if r[0].startswith("mshell-crates/")]
other = [r for r in rows if r not in top and r not in helpers]

if count_only:
    print(f"workspace_members\t{len(rows)}")
    print(f"top_level\t{len(top)}")
    print(f"mshell_crates\t{len(helpers)}")
    sys.exit(0)

def table(title, items):
    print(f"### {title} ({len(items)})")
    print("| Crate | Role |")
    print("|---|---|")
    for path, name, desc in items:
        label = f"`{path}/`" if path == name or path.endswith("/" + name) else f"`{path}/` ({name})"
        print(f"| {label} | {desc.replace("|", "\\|")} |")
    print()

table("Top-level crates", top)
table("Shell helper crates (`mshell-crates/`)", helpers)
if other:
    table("Other members", other)
' "$@"
