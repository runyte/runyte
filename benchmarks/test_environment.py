# SPDX-License-Identifier: MPL-2.0
"""Child-process environment isolation for PTY benchmark harnesses."""

import json
import os
from pathlib import Path
import select
import sys
import tempfile
import time
import unittest
from unittest import mock

import plugins
import ptybench
import run
import session_navigation


AMBIENT_SETTINGS = ('RUNYTE_INPUT_TRACE', 'RUNYTE_STARTUP_TIMING_FILE',
                    'RUNYTE_BENCH_EVENTS', 'RUNYTE_PARENT_CONTEXT')


class EnvironmentTests(unittest.TestCase):
    def test_pty_child_drops_ambient_settings_but_keeps_explicit_instrumentation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inherited = {name: str(root / name) for name in AMBIENT_SETTINGS}
            explicit = str(root / 'measured-events')
            script = ('import json,os; print(json.dumps({k:os.environ[k] for k in '
                      + repr(AMBIENT_SETTINGS) + ' if k in os.environ}))')
            with mock.patch.dict(os.environ, inherited):
                pid, fd = ptybench._spawn([sys.executable, '-c', script],
                                          {'RUNYTE_BENCH_EVENTS': explicit}, None)
                try:
                    data = bytearray()
                    deadline = time.monotonic() + 3
                    while b'\n' not in data and time.monotonic() < deadline:
                        if select.select([fd], [], [], .05)[0]:
                            data.extend(os.read(fd, 4096))
                    child = json.loads(data)
                finally:
                    ptybench._reap(pid)
                    os.close(fd)
                for name in AMBIENT_SETTINGS:
                    self.assertEqual(os.environ[name], inherited[name])
                    self.assertEqual(child.get(name), explicit if name == 'RUNYTE_BENCH_EVENTS' else None)

    def test_example_and_session_environments_drop_ambient_settings(self):
        with tempfile.TemporaryDirectory() as temporary:
            inherited = {name: str(Path(temporary) / name) for name in AMBIENT_SETTINGS}
            with mock.patch.dict(os.environ, inherited):
                for name, environment in (('plugins', plugins.environment),
                                          ('sessions', session_navigation.environment)):
                    root = Path(temporary) / name
                    root.mkdir()
                    child = environment(root)
                    self.assertFalse(set(AMBIENT_SETTINGS) & child.keys())
                    self.assertEqual(Path(child['XDG_CONFIG_HOME']).parent, root)

    def test_version_probe_drops_ambient_settings(self):
        with mock.patch.dict(os.environ, {name: 'caller-setting' for name in AMBIENT_SETTINGS}), \
                mock.patch('run.subprocess.run') as process:
            process.return_value.stdout = 'runyte 0.3.0\n'
            self.assertEqual(run.version_of(['editor'], {}, '.'), 'runyte 0.3.0')
            self.assertFalse(set(AMBIENT_SETTINGS) & process.call_args.kwargs['env'].keys())


if __name__ == '__main__':
    unittest.main()
