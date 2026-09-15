# SPDX-License-Identifier: MPL-2.0
"""Offline tests for candidate staging and immutable compatibility admission."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch

import candidate
import check_frozen
import check_inventory


MANIFEST = '[package]\nname = "runyte"\nversion = "0.2.4"\n\n[dependencies]\nother = "0.2.4"\n'
LOCK = 'version = 4\n\n[[package]]\nname = "other"\nversion = "0.2.4"\n\n[[package]]\nname = "runyte"\nversion = "0.2.4"\ndependencies = ["other"]\n'


class CandidateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='runyte-candidate-test-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / 'source'
        self.source.mkdir()
        (self.source / 'Cargo.toml').write_text(MANIFEST)
        (self.source / 'Cargo.lock').write_text(LOCK)
        (self.source / '.gitignore').write_text('target/\n.runyte/\nignored\n')
        subprocess.run(['git', 'init', '-q', str(self.source)], check=True)
        subprocess.run(['git', '-C', str(self.source), 'add', '.'], check=True)
        self.calls = []

    def cargo(self, command, **kwargs):
        if command[0] == 'git':
            return subprocess.run(command, **kwargs)
        self.calls.append(command)
        self.assertEqual(command, ['cargo', 'check', '--lib'])
        staged = kwargs['cwd']
        (staged / 'Cargo.lock').write_text(candidate.replace_lock_version(LOCK, '0.2.4', '0.3.0'))
        return types.SimpleNamespace(returncode=0)

    def test_bootstrap_copies_untracked_source_and_preserves_checkout_and_modes(self):
        (self.source / 'new.rs').write_text('new source')
        executable = self.source / 'fixture'
        executable.write_text('checked-in stand-in fixture')
        executable.chmod(0o755)
        (self.source / 'fixture-link').symlink_to('fixture')
        for name in ('target', '.runyte', '__pycache__'):
            (self.source / name).mkdir()
            (self.source / name / 'private').write_text('not source')
        (self.source / 'ignored').write_text('not source')
        result = candidate.prepare(self.source, self.root / 'candidate', self.cargo)
        staged = Path(result['source'])
        self.assertEqual(result['host_version'], '0.3.0')
        self.assertEqual(result['mode'], 'bootstrap-candidate')
        self.assertEqual((staged / 'new.rs').read_text(), 'new source')
        self.assertTrue((staged / 'fixture-link').is_symlink())
        self.assertEqual((staged / 'fixture').stat().st_mode & 0o777, 0o755)
        for name in ('.git', 'target', '.runyte', '__pycache__', 'ignored'):
            self.assertFalse((staged / name).exists(), name)
        self.assertEqual((self.source / 'Cargo.toml').read_text(), MANIFEST)
        self.assertEqual((self.source / 'Cargo.lock').read_text(), LOCK)
        self.assertEqual(len(self.calls), 1)

    def test_stable_and_prerelease_builds_use_the_exact_source_without_rewriting(self):
        for version in ('0.3.0', '0.3.1', '0.4.0', '1.0.0', '0.3.0-rc.1'):
            manifest = candidate.replace_package_version(MANIFEST, version)
            (self.source / 'Cargo.toml').write_text(manifest)
            result = candidate.prepare(self.source, self.root / 'unused', self.cargo)
            self.assertEqual(result['source'], str(self.source.resolve()))
            self.assertEqual(result['host_version'], version)
            self.assertEqual(result['mode'], 'exact')
            self.assertFalse((self.root / 'unused').exists())
            self.assertEqual((self.source / 'Cargo.toml').read_text(), manifest)
        self.assertEqual(self.calls, [])

    def test_dependency_drift_fails_and_removes_only_the_owned_staging_directory(self):
        def drift(command, **kwargs):
            result = self.cargo(command, **kwargs)
            if command[0] == 'cargo':
                path = kwargs['cwd'] / 'Cargo.lock'
                path.write_text(path.read_text().replace('name = "other"', 'name = "changed"'))
            return result
        destination = self.root / 'candidate'
        with self.assertRaisesRegex(ValueError, 'changed dependencies'):
            candidate.prepare(self.source, destination, drift)
        self.assertFalse(destination.exists())
        self.assertEqual((self.source / 'Cargo.lock').read_text(), LOCK)

    def test_destination_inside_source_and_escaping_symlinks_are_refused(self):
        with self.assertRaises(ValueError):
            candidate.prepare(self.source, self.source / 'candidate', self.cargo)
        (self.source / 'escape').symlink_to('../outside')
        with self.assertRaisesRegex(ValueError, 'symlink escapes'):
            candidate.prepare(self.source, self.root / 'candidate', self.cargo)
        self.assertFalse((self.root / 'candidate').exists())

    def test_only_root_package_version_changes_and_outputs_reject_line_breaks(self):
        self.assertIn('other = "0.2.4"', candidate.replace_package_version(MANIFEST, '0.3.0'))
        self.assertIn('name = "other"\nversion = "0.2.4"', candidate.replace_lock_version(LOCK, '0.2.4', '0.3.0'))
        with self.assertRaises(ValueError):
            candidate.write_outputs({'source': 'bad\ninjected=1'}, self.root / 'outputs')
        self.assertFalse((self.root / 'outputs').exists())


class InventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='runyte-inventory-test-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = self.root / 'inventory.json'
        # Synthetic offline identities belong only to unit fixtures.
        source = hashlib.sha1(b'offline-sdk-fixture').hexdigest()
        plugin = hashlib.sha1(b'offline-plugin-fixture').hexdigest()
        files = {}
        for role, name in (('sdk', 'application.py'), ('schema', 'runyte-1.schema.json'), ('fixtures', 'stable-fixtures.json')):
            data = ('offline ' + role).encode()
            (self.root / name).write_bytes(data)
            files[role] = {'path': name, 'sha256': hashlib.sha256(data).hexdigest()}
        profile = {'id': 'python-v1', 'repository': 'runyte/runyte', 'revision': source,
                   'release': 'unit-candidate', 'protocol': 'runyte-1', 'runyte': '>=0.3.0, <0.4.0', 'features': [], 'files': files}
        external = {**profile, 'id': 'ru-time-v1', 'repository': 'runyte/ru-time', 'revision': plugin,
                    'profile': 'python-v1', 'sdk_revision': source,
                    'files': {role: copy.deepcopy(files[role]) for role in ('sdk', 'schema')},
                    'native_tests': ['test_native.NativeTests.test_behavior']}
        self.inventory = {'version': 1, 'stable_floor': '0.3.0', 'profiles': [profile], 'plugins': [external]}

    def load(self, inventory=None):
        self.path.write_text(json.dumps(inventory if inventory is not None else self.inventory))
        return check_inventory.load_inventory(self.path)

    def test_complete_pins_and_matching_artifacts_are_accepted(self):
        loaded = self.load()
        check_inventory.verify_files(self.root, loaded['profiles'][0]['files'])
        self.assertEqual(loaded['plugins'][0]['sdk_revision'], loaded['profiles'][0]['revision'])

    def test_empty_bootstrap_missing_or_floating_pins_and_wrong_digests_fail(self):
        for mutate in (
            lambda value: value.update(profiles=[]),
            lambda value: value.update(plugins=[]),
            lambda value: value['plugins'][0].update(revision='main'),
            lambda value: value['profiles'][0].update(revision='0' * 40),
            lambda value: value['plugins'][0].pop('sdk_revision'),
            lambda value: value['plugins'][0]['files']['sdk'].update(sha256='a' * 64),
            lambda value: value['plugins'][0].update(native_tests=[]),
            lambda value: value['profiles'][0]['files']['sdk'].update(path='../application.py'),
        ):
            value = copy.deepcopy(self.inventory)
            mutate(value)
            with self.assertRaises(ValueError):
                self.load(value)
        self.load()
        (self.root / 'application.py').write_text('changed SDK')
        with self.assertRaisesRegex(ValueError, 'digest mismatch'):
            check_inventory.verify_files(self.root, self.inventory['profiles'][0]['files'])

    def test_duplicate_fields_and_unsafe_artifacts_are_refused(self):
        self.path.write_text('{"version":1,"version":1}')
        with self.assertRaisesRegex(ValueError, 'Duplicate inventory field'):
            check_inventory.load_inventory(self.path)
        sdk = self.root / 'application.py'
        sdk.unlink()
        sdk.symlink_to(self.root / 'runyte-1.schema.json')
        with self.assertRaisesRegex(ValueError, 'unsafe compatibility artifact'):
            check_inventory.verify_files(self.root, self.inventory['profiles'][0]['files'])

    def test_bad_checkout_identity_or_provenance_cannot_be_used_as_a_baseline(self):
        row = self.inventory['plugins'][0]
        with patch.object(check_inventory, 'git', return_value='wrong'):
            with self.assertRaisesRegex(ValueError, 'immutable revision'):
                check_inventory.verify_checkout(self.root, row)
        (self.root / 'VENDOR.md').write_text('Source: pending implementation')
        with patch.object(check_inventory, 'git', side_effect=[row['revision'], '']):
            with self.assertRaisesRegex(ValueError, 'VENDOR.md'):
                check_inventory.verify_checkout(self.root, row)

    def test_missing_and_skipped_native_tests_are_gate_failures(self):
        class Native(unittest.TestCase):
            @unittest.skip('missing native dependency')
            def test_required(self):
                pass
        case = Native('test_required')
        suite = unittest.TestSuite([case])
        with self.assertRaisesRegex(ValueError, 'missing'):
            check_frozen.require_native_tests(suite, ['test_native.NativeTests.test_absent'])
        check_frozen.require_native_tests(suite, [case.id()])
        result = unittest.TestResult()
        suite.run(result)
        with self.assertRaisesRegex(ValueError, 'skipped'):
            check_frozen.check_test_result(result, [case.id()])
        with self.assertRaises(ValueError):
            check_frozen.check_test_result(unittest.TestResult(), [])


if __name__ == '__main__':
    unittest.main(verbosity=2)
