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
                                stdin=subprocess.DEVNULL, env={**os.environ, 'PANIC_RATCHET_BASE': self.base})
        return result

    def test_design_scans_production_after_test_items(self):
        # Exercise each affected rule separately so one hit cannot hide another.
        for violation, rule in [('let c = "#ff0000";', 'L10'),
                                ('std::env::var("XDG_RUNTIME_DIR");', 'L11'),
                                ('let b = Button::from_icon_name("x");', 'L12')]:
            for test_item in ('#[cfg(test)]\nfn helper() {\n}\n',
                              '#[cfg(test)] fn helper() {}\n',
                              '#[cfg(test)]\nuse foo::bar;\n',
                              '#[cfg(test)]\nuse foo::{bar, baz};\n',
                              '#[cfg(test)]\nmod tests;\n',
                              '#[cfg(test)]\nmod tests {\nfn helper() {}\n}\n'):
                with self.subTest(rule=rule, item=test_item):
                    self.write('mshell-crates/mshell-frame/src/probe.rs',
                               test_item + 'fn production() {\n' + violation + '\n}\n')
                    result = self.gate('design-lint.sh')
                    self.assertIn('FAIL ' + rule, result.stdout)

    def test_design_still_skips_test_modules(self):
        for violation in ('let c = "#ff0000";', 'std::env::var("XDG_RUNTIME_DIR");',
                          'let b = Button::from_icon_name("x");'):
            for wrapper in ('#[cfg(test)]\nmod tests {\nfn t() {\n%s\n}\n}\n',
                            '#[cfg(test)]\nmod tests\n{\n%s\n}\n',
                            '#[test]\nfn t() {\n%s\n}\n'):
                with self.subTest(violation=violation, wrapper=wrapper):
                    self.write('mshell-crates/mshell-frame/src/probe.rs',
                               'fn production() {}\n' + wrapper % violation)
                    result = self.gate('design-lint.sh')
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_panic_scans_after_inline_test_item(self):
        self.write('demo.rs', '#[cfg(test)] fn helper() {}\nfn f() { a.unwrap(); }\n')
        self.assertNotEqual(self.gate('panic-ratchet.sh').returncode, 0)

    def docs_drift(self, widgets_md):
        self.write('mshell-crates/mshell-config/src/schema/bar_widgets.rs',
                   'pub enum BarWidget {\n    Clock,\n    /// doc\n    Translate,\n'
                   '    Custom(String),\n    Spacer(u32),\n}\n')
        self.write('docs/widgets.md', widgets_md)
        return subprocess.run(['python3', 'scripts/docs-drift.py'], cwd=self.root,
                              text=True, capture_output=True, stdin=subprocess.DEVNULL)

    def test_docs_drift_flags_an_undocumented_widget(self):
        result = self.docs_drift('| `Clock` |\n| `!Custom <name>` |\n| `!Spacer <px>` |\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Translate', result.stdout + result.stderr)

    def test_docs_drift_flags_a_tuple_widget_without_bang(self):
        result = self.docs_drift('| `Clock` |\n| `Translate` |\n| `!Spacer <px>` |\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Custom', result.stdout + result.stderr)

    def test_docs_drift_passes_when_every_widget_is_listed(self):
        result = self.docs_drift('| `Clock` |\n| `Translate` |\n| `!Custom <name>` |\n'
                                 '| `!Spacer <px>` |\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_docs_drift_fails_when_the_enum_is_missing(self):
        self.write('docs/widgets.md', '')
        result = subprocess.run(['python3', 'scripts/docs-drift.py'], cwd=self.root,
                                text=True, capture_output=True, stdin=subprocess.DEVNULL)
        self.assertNotEqual(result.returncode, 0)

    def test_panic_excludes_path_attribute_test_modules(self):
        for attrs in ('#[cfg(test)]\n#[path = "t_tests.rs"]\n',
                      '#[path = "t_tests.rs"]\n#[cfg(test)]\n',
                      '#[cfg(test)]\n#[allow(dead_code)]\n#[path = "t_tests.rs"]\n',
                      '#[path = "t_tests.rs"]\n#[cfg(test)] '):
            with self.subTest(attrs=attrs):
                self.write('src/t.rs', 'fn f() {}\n' + attrs + 'mod t_tests;\n')
                self.write('src/t_tests.rs', 'fn helper() { a.unwrap(); }\n')
                result = self.gate('panic-ratchet.sh')
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_panic_resolves_test_modules_of_non_mod_files(self):
        # `mod tests;` in src/foo.rs is src/foo/tests.rs, not src/tests.rs.
        self.write('src/foo.rs', 'fn f() {}\n#[cfg(test)]\nmod tests;\n')
        self.write('src/foo/tests.rs', 'fn helper() { a.unwrap(); }\n')
        result = self.gate('panic-ratchet.sh')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.write('src/tests.rs', 'fn production() { a.unwrap(); }\n')
        self.assertNotEqual(self.gate('panic-ratchet.sh').returncode, 0)

    def test_size_rejects_malformed_rules(self):
        for rule in ('demo.rs abc', 'demo.rs -1', 'demo.rs 2 extra',
                     'demo.rs 2\ndemo.rs 3'):
            with self.subTest(rule=rule):
                self.write('scripts/file-size-limits.txt', rule + '\n')
                self.assertNotEqual(self.gate('file-size-gate.sh').returncode, 0)

    def test_size_deleted_rule_requires_trailer(self):
        self.write('scripts/file-size-limits.txt', '')
        self.commit('remove rule')
        self.assertNotEqual(self.gate('file-size-gate.sh').returncode, 0)

    def test_size_default_ceiling(self):
        self.write('new.rs', '// line\n' * 2001)
        self.assertNotEqual(self.gate('file-size-gate.sh').returncode, 0)

    def test_size_historical_malformed_rule(self):
        self.write('scripts/file-size-limits.txt', 'demo.rs invalid\n')
        self.commit('bad limits')
        self.write('scripts/file-size-limits.txt', 'demo.rs 2\n')
        self.commit('repair limits')
        self.assertNotEqual(self.gate('file-size-gate.sh').returncode, 0)

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
