#!/usr/bin/env python3
"""Exercise gates in isolated repositories, never the developer's checkout."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent


class GateTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        shutil.copytree(SCRIPTS, self.root / 'scripts')
        self.git('init', '-q', '-b', 'main')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'user.name', 'test')
        self.write('scripts/panic-baseline.txt', '0\n')
        self.write('scripts/file-size-limits.txt', 'demo.rs 2\n')
        self.write('demo.rs', '// fixture\n')
        self.commit('base')
        self.base = self.git('rev-parse', 'HEAD').stdout.strip()

    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.root, text=True,
                              capture_output=True, check=True)

    def write(self, path, text):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def commit(self, message):
        self.git('add', '.')
        self.git('commit', '-qm', message)

    def gate(self, script):
        result = subprocess.run(['bash', f'scripts/{script}', self.base],
                                cwd=self.root, text=True, capture_output=True,
                                env={**os.environ, 'PANIC_RATCHET_BASE': self.base})
        return result

    def test_panic_malformed_baselines(self):
        for value in ('', 'x', '1#comment', '-1', '1 0'):
            with self.subTest(value=value):
                self.write('scripts/panic-baseline.txt', value + '\n')
                result = self.gate('panic-ratchet.sh')
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_panic_guard_rejects_malformed_commit(self):
        self.write('scripts/panic-baseline.txt', 'x\n')
        self.commit('bad baseline')
        self.assertNotEqual(self.gate('panic-baseline-guard.sh').returncode, 0)

    def test_panic_counts_calls_not_lines(self):
        self.write('demo.rs', 'fn f() { a.unwrap(); b.unwrap(); }\n')
        self.write('scripts/panic-baseline.txt', '1\n')
        self.assertNotEqual(self.gate('panic-ratchet.sh').returncode, 0)

    def test_panic_excludes_external_trees(self):
        for tree in ('vendor', 'fuzz', 'mplugin-sdk'):
            self.write(f'{tree}/lib.rs', 'fn f() { a.unwrap(); }\n')
        result = self.gate('panic-ratchet.sh')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_panic_guard_checks_merge_resolution(self):
        self.git('checkout', '-qb', 'side')
        self.write('side.txt', 'side\n')
        self.commit('side')
        self.git('checkout', 'main')
        self.write('main.txt', 'main\n')
        self.commit('main')
        self.git('merge', '--no-commit', '--no-ff', 'side')
        self.write('scripts/panic-baseline.txt', '1\n')
        self.commit('merge raises baseline')
        self.assertNotEqual(self.gate('panic-baseline-guard.sh').returncode, 0)


if __name__ == '__main__':
    unittest.main()
