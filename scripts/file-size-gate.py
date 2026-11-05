#!/usr/bin/env python3
"""Validate explicit file-size ratchets and the first-party Rust default cap."""
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

LIMITS = 'scripts/file-size-limits.txt'
DEFAULT = 2000
EXCLUDED = {'.git', 'target', 'vendor', 'fuzz', 'mplugin-sdk'}


def git(*args):
    return subprocess.run(['git', *args], text=True, capture_output=True, check=True).stdout


def parse(text, source):
    limits = {}
    for number, line in enumerate(text.splitlines(), 1):
        fields = line.partition('#')[0].split()
        if not fields:
            continue
        if len(fields) != 2 or not re.fullmatch(r'0|[1-9][0-9]{0,8}', fields[1]):
            raise ValueError(f'{source}:{number}: expected <path> <decimal limit>')
        path, limit = fields
        if (PurePosixPath(path).is_absolute() or '..' in PurePosixPath(path).parts
                or path in limits):
            raise ValueError(f'{source}:{number}: duplicate or unsafe path {path}')
        limits[path] = int(limit)
    return limits


def at(commit):
    return parse(git('show', f'{commit}:{LIMITS}'), commit)


def main():
    os.chdir(Path(__file__).resolve().parent.parent)
    limits = parse(Path(LIMITS).read_text(), LIMITS)
    paths = set(limits)
    for root, dirs, files in os.walk('.'):
        dirs[:] = [d for d in dirs if d not in EXCLUDED]
        paths.update(str(Path(root) / name) for name in files if name.endswith('.rs'))
    bad = False
    for path in sorted(paths):
        limit = limits.get(path, DEFAULT)
        try:
            n = len(Path(path).read_bytes().splitlines())
        except OSError as error:
            print(f'FAIL: {path}: {error}')
            bad = True
            continue
        if n > limit:
            print(f'FAIL: {path}: {n} lines > {limit}; split the owning module or justify FILE-SIZE-RAISE.')
            bad = True
    base = (sys.argv[1] if len(sys.argv) > 1 else
            os.environ.get('FILE_SIZE_GATE_BASE') or os.environ.get('PANIC_RATCHET_BASE'))
    if not base:
        try:
            base = git('merge-base', 'HEAD', 'origin/main').strip()
        except subprocess.CalledProcessError:
            pass
    try:
        git('rev-parse', '--verify', f'{base or "HEAD~1"}^{{commit}}')
    except subprocess.CalledProcessError:
        base = 'HEAD~1'
        try:
            git('rev-parse', '--verify', base)
        except subprocess.CalledProcessError:
            return int(bad)
    base = base or 'HEAD~1'
    for commit in git('rev-list', '--reverse', '--first-parent', f'{base}..HEAD', '--', LIMITS).split():
        new, old = at(commit), at(f'{commit}^')
        removed = old.keys() - new.keys()
        raised = {p for p in new if new[p] > old.get(p, DEFAULT)}
        if removed or raised:
            reason = git('log', '-1', '--format=%(trailers:key=FILE-SIZE-RAISE,valueonly)', commit).strip()
            if not reason:
                print(f'FAIL: {commit} removes/raises limits for {sorted(removed | raised)} without FILE-SIZE-RAISE: <reason>')
                bad = True
    if not bad:
        print(f'file-size gate OK (default Rust cap: {DEFAULT} lines).')
    return int(bad)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'FAIL: {error}', file=sys.stderr)
        sys.exit(1)
