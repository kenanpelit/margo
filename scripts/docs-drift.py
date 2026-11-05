#!/usr/bin/env python3
"""docs-drift.py — hold hand-written docs to the code they describe.

Checks, each a "this list must match the source" rule:

  * docs/widgets.md lists every `BarWidget` variant
    (mshell-crates/mshell-config/src/schema/bar_widgets.rs): a unit variant
    as `Name`, a variant with data as `!Name …` (the YAML tag form).

The dispatch-action tables (mctl's tests), the man pages (mctl/mshellctl/
mdots tests) and the mkdocs nav (`mkdocs build --strict` in docs.yml) have
their own gates.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
ENUM_FILE = ROOT / 'mshell-crates/mshell-config/src/schema/bar_widgets.rs'
WIDGETS_MD = ROOT / 'docs/widgets.md'


def bar_widget_variants(source):
    """(name, has_data) for each variant of `pub enum BarWidget`."""
    match = re.search(r'^pub enum BarWidget \{\n(.*?)^\}', source, re.M | re.S)
    if not match:
        return None
    variants = []
    for line in match.group(1).splitlines():
        m = re.match(r'^    ([A-Z][A-Za-z0-9]*)\s*([(,{])', line)
        if m:
            variants.append((m.group(1), m.group(2) != ','))
    return variants


def main():
    try:
        source = ENUM_FILE.read_text()
        widgets = WIDGETS_MD.read_text()
    except OSError as e:
        print(f'docs-drift: {e}')
        return 1
    variants = bar_widget_variants(source)
    if not variants:
        print(f'docs-drift: no `pub enum BarWidget` in {ENUM_FILE.relative_to(ROOT)}')
        return 1
    missing = []
    for name, has_data in variants:
        token = f'`!{name} ' if has_data else f'`{name}`'
        if token not in widgets:
            missing.append(f'!{name} <…>' if has_data else name)
    if missing:
        print('FAIL docs/widgets.md does not list these bar widgets: ' + ', '.join(missing))
        return 1
    print(f'docs-drift OK: docs/widgets.md lists all {len(variants)} bar widgets.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
