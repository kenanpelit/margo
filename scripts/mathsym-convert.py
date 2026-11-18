#!/usr/bin/env python3
"""Convert the mathsym plugin's tables into the launcher's symbols_math.tsv.

usage: scripts/mathsym-convert.py <community-plugins/mathsym> > \
       mshell-crates/mshell-launcher/src/providers/symbols_math.tsv

Input (generated upstream by SturgeonInc/noctalia-mathsym-plugin
tools/build_data.py): data.tsv (char, display, subtitle), names.tsv
(name, row), desc.tsv (description, row). Output, one row per symbol:
char, description, LaTeX commands (with `\\`), Lean input names (without
`\\`), first code point (hex). Lean names are the row's names that are
not one of its LaTeX commands (normalised: no `\\`, braces, spaces).
"""
import sys
from collections import defaultdict
from pathlib import Path

src = Path(sys.argv[1])
names = defaultdict(list)
for line in (src / 'names.tsv').read_text().splitlines():
    n, row = line.rsplit('\t', 1)
    names[int(row)].append(n)
desc = {}
for line in (src / 'desc.tsv').read_text().splitlines():
    d, row = line.rsplit('\t', 1)
    desc[int(row)] = d


def norm(s):
    return s.replace('\\', '').replace('{', '').replace('}', '').replace(' ', '')


print('# Math symbols for the launcher `.` provider (symbols_math.rs).')
print('# From the mathsym Noctalia plugin by fisherman (MIT,')
print('# github.com/SturgeonInc/noctalia-mathsym-plugin), whose data comes from')
print('# unicode-math-table.tex of the LaTeX unicode-math package (LPPL 1.3c) and')
print('# the Lean 4 VS Code abbreviation table (leanprover/vscode-lean4,')
print('# Apache-2.0). Regenerate with scripts/mathsym-convert.py.')
print('# char\tdescription\tlatex\tlean\tcodepoint')
for i, line in enumerate((src / 'data.tsv').read_text().splitlines(), start=1):
    ch, _display, sub = line.split('\t', 2)
    latex = []
    for seg in sub.split('  ·  '):
        if seg.startswith('\\'):
            latex = seg.split()
    latex_n = {norm(x) for x in latex}
    lean = [n for n in names[i] if n not in latex_n]
    cp = f'{ord(ch[0]):04X}'
    d = desc.get(i, '').replace('\t', ' ')
    print('\t'.join([ch, d, ' '.join(latex), ' '.join(lean), cp]))
