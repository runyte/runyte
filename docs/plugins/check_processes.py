# SPDX-License-Identifier: MPL-2.0
"""Bounded binary helper SDK and deterministic native controller checks."""
import base64
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import unittest
from application import Application, PluginError
from check_queries import shutdown, HeldWorker


class ProcessSdkTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Helper test', [], ['processes'])
        self.calls = []
        self.result = {}
        def request(method, **params):
            self.calls.append((method, params))
            return self.result
        self.app.request = request

    def tearDown(self):
        shutdown(self.app)

    def test_start_preserves_vector_and_only_explicit_cwd(self):
        self.app.start_process('Echo', '/usr/bin/python3', ['a b', '$(literal)'])
        self.assertEqual(self.calls[-1], ('process.start', {
            'label': 'Echo', 'executable': '/usr/bin/python3', 'args': ['a b', '$(literal)'],
            'capture_stderr': False}))
        self.app.start_process('Echo', 'python3', cwd='subdir', capture_stderr=True)
        self.assertEqual(self.calls[-1][1]['cwd'], 'subdir')
        self.assertTrue(self.calls[-1][1]['capture_stderr'])

    def test_binary_write_and_eof_are_exact_and_bounded(self):
        data = bytes(range(256)) * 256
        self.result = {'process': 'pr:1', 'written': len(data), 'stdin_closed': True}
        self.app.write_process('pr:1', data, eof=True)
        self.assertEqual(base64.b64decode(self.calls[-1][1]['data']), data)
        self.assertTrue(self.calls[-1][1]['eof'])
        with self.assertRaises(PluginError):
            self.app.write_process('pr:1', data + b'x')
        self.assertEqual(len(self.calls), 1)
        with self.assertRaises(PluginError):
            self.app.write_process('pr:1', 'not bytes')
        self.assertEqual(len(self.calls), 1)

    def test_noncontiguous_memoryview_is_bounded_before_copy(self):
        self.result = {'process': 'pr:1', 'written': 3, 'stdin_closed': False}
        self.app.write_process('pr:1', memoryview(b'abcdef')[::2])
        self.assertEqual(base64.b64decode(self.calls[-1][1]['data']), b'ace')

    def test_incomplete_write_ack_is_unknown_without_retry(self):
        for result in [{'process': 'pr:2', 'written': 3, 'stdin_closed': True},
                       {'process': 'pr:1', 'written': 2, 'stdin_closed': True},
                       {'process': 'pr:1', 'written': 3, 'stdin_closed': False}]:
            self.result = result
            self.calls.clear()
            with self.assertRaises(PluginError) as error:
                self.app.write_process('pr:1', b'abc', eof=True)
            self.assertEqual(error.exception.code, 'outcome_unknown')
            self.assertEqual(len(self.calls), 1)

    def test_reads_validate_binary_offsets_and_empty_nonterminal_chunk(self):
        self.result = {'process': 'pr:1', 'stream': 'stdout', 'offset': 9,
                       'data': '/wA=', 'next': 11, 'eof': False}
        self.assertEqual(self.app.read_process('pr:1', 'stdout', 9, 2)['data'], b'\xff\0')
        self.result.update(data='', next=9)
        result = self.app.read_process('pr:1', 'stdout', 9)
        self.assertEqual(result['data'], b'')
        self.assertFalse(result['eof'])
        for changes in [{'data': '!!!!'}, {'data': 'YQ==', 'next': 9},
                        {'stream': 'stderr'}, {'offset': 8}, {'process': 'pr:2'}]:
            original = self.result.copy()
            self.result.update(changes)
            with self.assertRaises(PluginError):
                self.app.read_process('pr:1', 'stdout', 9)
            self.result = original

    def test_invalid_read_limit_never_sends(self):
        for limit in (0, 65537):
            with self.assertRaises(PluginError):
                self.app.read_process('pr:1', 'stdout', 0, limit)
        self.assertEqual(self.calls, [])

    def test_process_fixtures_match_schema(self):
        from jsonschema import Draft202012Validator
        directory = Path(__file__).parent
        schema = json.loads((directory / 'runyte-experimental-2.schema.json').read_text())
        fixtures = json.loads((directory / 'epoch2-fixtures.json').read_text())
        checked = 0
        for fixture in fixtures:
            if (fixture['message'].get('id') in {f'p:{i}' for i in range(1000, 1005)}
                    or fixture['message'].get('sequence') == 'e:1000'):
                Draft202012Validator({'$defs': schema['$defs'], '$ref': f'#/$defs/{fixture["direction"]}Message'}).validate(fixture['message'])
                checked += 1
        self.assertEqual(checked, 9)

    def test_checked_in_backend_echo_flood_and_eof(self):
        result = subprocess.run([sys.executable, str(Path(__file__).with_name('echo_helper.py'))],
                                input=b'hello\nflood\n', capture_output=True, timeout=5, check=True)
        self.assertIn(b'Echo: hello\n', result.stdout)
        self.assertTrue(result.stdout.endswith(b'Flood record 32767: bounded retained output.\n'))
        self.assertGreater(len(result.stdout), 1024 * 1024)
        self.assertLess(len(result.stdout), 2 * 1024 * 1024)
        self.assertEqual(result.stderr, b'')


class HelperExampleTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('helper_example_test', Path(__file__).with_name('helper.py'))
        self.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.helper)
        self.real_worker = self.helper.worker
        self.helper.worker = HeldWorker()

    def tearDown(self):
        self.real_worker.shutdown(wait=True, cancel_futures=True)
        shutdown(self.helper.app)

    def test_output_controls_are_data_not_terminal_sequences(self):
        self.assertEqual(self.helper.safe_text(b'\x1b[31m\0\n\xff'), '\\u001b[31m\\u0000\n�')

    def test_output_flood_retains_one_refresh_task_and_intent(self):
        self.helper.process = 'pr:1'
        for i in range(1000):
            self.helper.observed('event.changed', f'e:{i}', {})
        self.assertEqual(len(self.helper.worker.calls), 1)
        self.assertTrue(self.helper.refresh_pending)

    def test_cancelled_or_old_prompt_never_writes_to_new_helper(self):
        calls = []
        self.helper.app.write_process = lambda *args: calls.append(args)
        self.helper.process = 'pr:2'
        self.helper.pending_input = ('u:1', 'pr:1')
        self.helper.submitted({'surface': 'u:1', 'accepted': True, 'values': {'line': 'old'}})
        self.helper.pending_input = ('u:2', 'pr:2')
        self.helper.submitted({'surface': 'u:2', 'accepted': False, 'values': {}})
        self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
