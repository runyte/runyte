# SPDX-License-Identifier: MPL-2.0
"""Lease correlation, cancellation dispatch, and structural conformance."""
import json
import queue
import threading
import unittest
from pathlib import Path
from application import Application, PluginError
from check_queries import shutdown


class ActivitySdkTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Continuing work', [], ['activity'])
        self.calls = []
        self.app.request = lambda method, **params: self.calls.append((method, params))

    def tearDown(self):
        shutdown(self.app)

    def test_methods_keep_explicit_lease_and_duration_without_starting_renewal(self):
        self.app.acquire_activity('Playback')
        self.app.renew_activity('a:g:1', 45)
        self.app.get_activity('a:g:1')
        self.app.cancel_activity('a:g:1')
        self.app.release_activity('a:g:1')
        self.assertEqual(self.calls, [
            ('activity.acquire', {'title': 'Playback', 'duration_seconds': 600}),
            ('activity.renew', {'lease': 'a:g:1', 'duration_seconds': 45}),
            ('activity.get', {'lease': 'a:g:1'}),
            ('activity.cancel', {'lease': 'a:g:1'}),
            ('activity.release', {'lease': 'a:g:1'})])

    def test_failed_renewal_never_replays_or_acquires_replacement_protection(self):
        def fail(method, **params):
            self.calls.append((method, params))
            raise PluginError('cancelled', 'Activity cleanup has started')
        self.app.request = fail
        with self.assertRaises(PluginError) as error:
            self.app.renew_activity('a:g:1')
        self.assertEqual(error.exception.code, 'cancelled')
        self.assertEqual(self.calls, [('activity.renew', {'lease': 'a:g:1', 'duration_seconds': 600})])

    def test_cancellation_cleans_before_release_while_all_command_workers_are_busy(self):
        gate = threading.Event()
        started = queue.Queue()
        cleaned = threading.Event()
        received = queue.Queue()
        def command():
            started.put(True)
            gate.wait(timeout=5)
        workers = [self.app._executor.submit(command) for _ in range(4)]
        try:
            for _ in workers:
                started.get(timeout=2)
            def event(name, data):
                received.put((name, data))
                cleaned.set()
                self.app.release_activity(data['lease'])
            def request(method, **params):
                self.assertTrue(cleaned.is_set(), 'release must acknowledge completed cleanup')
                received.put((method, params))
            self.app.on_event = event
            self.app.request = request
            self.app._submit({'type': 'event', 'sequence': 'e:1',
                              'event': 'activity.cancel_requested',
                              'data': {'lease': 'a:g:1', 'reason': 'expired'}})
            self.assertEqual(received.get(timeout=2), ('activity.cancel_requested',
                                                      {'lease': 'a:g:1', 'reason': 'expired'}))
            self.assertEqual(received.get(timeout=2), ('activity.release', {'lease': 'a:g:1'}))
            self.assertTrue(all(not future.done() for future in workers))
        finally:
            gate.set()

    def test_all_lease_shapes_and_cancel_event_match_schema(self):
        from jsonschema import Draft202012Validator
        directory = Path(__file__).parent
        schema = json.loads((directory / 'runyte-experimental-2.schema.json').read_text())
        fixtures = json.loads((directory / 'epoch2-fixtures.json').read_text())
        checked = 0
        for fixture in fixtures:
            message = fixture['message']
            if (message.get('id') in {f'p:{i}' for i in range(1200, 1205)}
                    or message.get('sequence') == 'e:1200'):
                validator = Draft202012Validator({'$defs': schema['$defs'],
                                                  '$ref': f'#/$defs/{fixture["direction"]}Message'})
                self.assertEqual(list(validator.iter_errors(message)), [], message)
                checked += 1
        self.assertEqual(checked, 11)

    def test_schema_rejects_unbounded_or_spoofed_activity_requests(self):
        from jsonschema import Draft202012Validator
        schema = json.loads(Path(__file__).with_name('runyte-experimental-2.schema.json').read_text())
        validator = Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/activity.acquire'})
        message = {'type': 'request', 'id': 'p:1', 'method': 'activity.acquire',
                   'params': {'title': 'Playback'}}
        self.assertTrue(validator.is_valid(message), 'omitted duration means 600 seconds')
        for params in ({'title': ''}, {'title': 'x' * 161},
                       {'title': 'Playback', 'owner': 'another-plugin'},
                       *({'title': 'Playback', 'duration_seconds': duration}
                         for duration in (0, 601, -1, '600', True))):
            with self.subTest(params=params):
                self.assertFalse(validator.is_valid({**message, 'params': params}))


if __name__ == '__main__':
    unittest.main()
