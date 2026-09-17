# SPDX-License-Identifier: MPL-2.0
"""Pure harness checks; no editor or timed workload is started."""
import hashlib
import json
import os
from pathlib import Path
import stat
import shlex
import tempfile
import unittest
from unittest.mock import patch

import context_access


class ContextAccessHarnessTests(unittest.TestCase):
    def test_noisy_command_fits_the_native_prompt(self):
        command = 'terminal /bin/sh -c ' + shlex.quote(context_access.NOISE)
        self.assertLessEqual(len(command.encode()), 100)
        self.assertNotIn('\n', command)

    def test_seeded_grants_match_exact_canonical_root_and_private_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project = root / 'project'
            project.mkdir()
            storage = root / 'ctx'
            credential = context_access.seed_grant(storage, project)
            self.assertEqual(len(credential), 64)
            identity = json.loads((storage / 'identity.json').read_text())
            self.assertEqual(identity, {'name': 'agent', 'credential': credential})
            fingerprint = hashlib.sha256(credential.encode()).hexdigest()
            project_bytes = os.fsencode(project.resolve())
            key = hashlib.sha256(project_bytes + b'\0' + fingerprint.encode()).hexdigest()
            grant = json.loads((storage / ('grant-' + key + '.json')).read_text())
            self.assertEqual(bytes(grant['root']), project_bytes)
            self.assertEqual(grant['identity'], fingerprint)
            self.assertEqual(grant['scopes'], ['terminal_read', 'editor_context_read'])
            self.assertNotIn(credential, json.dumps(grant))
            self.assertEqual(stat.S_IMODE(storage.stat().st_mode), 0o700)
            for record in storage.iterdir():
                self.assertEqual(stat.S_IMODE(record.stat().st_mode), 0o600)

    def test_phase_evidence_excludes_warmup_and_saved_file_verification(self):
        points = [[(1, 'r:1'), (2, 'r:2'), (9, 'r:3'), (10, 'r:4')]]
        with self.assertRaises(ValueError):
            context_access.phase_progress(points, 3, 8)
        with self.assertRaises(ValueError):
            context_access.phase_progress([[(4, 'r:3'), (5, 'r:3')]], 3, 8)
        self.assertEqual(context_access.phase_progress([[(1, 'r:1'), (4, 'r:2'), (7, 'r:3'), (9, 'r:4')]], 3, 8),
                         [[(4, 'r:2'), (7, 'r:3')]])

    def test_session_environment_overrides_inherited_context_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.dict(os.environ, {'RUNYTE_CONTEXT_HOME': '/must-not-write', 'RUNYTE_PARENT_CONTEXT': 'parent-context', 'RUNYTE_BENCH_EVENTS': '/must-not-log', 'RUNYTE_INPUT_TRACE': '/must-not-trace'}):
                with patch.object(context_access.plugins, 'spawn', return_value=(123, 456)):
                    with patch.object(context_access.plugins, 'Terminal'):
                        editor = context_access.Session(Path('/not-started'), root, False)
                self.assertEqual(editor.env['RUNYTE_CONTEXT_HOME'], str(root / 'ctx'))
                self.assertNotIn('RUNYTE_PARENT_CONTEXT', editor.env)
                self.assertNotIn('RUNYTE_BENCH_EVENTS', editor.env)
                self.assertNotIn('RUNYTE_INPUT_TRACE', editor.env)
                self.assertFalse((root / 'ctx').exists())
                for key in ('XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
                    self.assertTrue(Path(editor.env[key]).is_relative_to(root))
                self.assertEqual(os.environ['RUNYTE_CONTEXT_HOME'], '/must-not-write')


if __name__ == '__main__':
    unittest.main()
