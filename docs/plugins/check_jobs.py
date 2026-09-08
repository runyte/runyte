# SPDX-License-Identifier: MPL-2.0
"""Background-job cancellation at registration/completion boundaries and on wire."""
import importlib.util
import json
from pathlib import Path
import queue
import selectors
import subprocess
import sys
import threading
import unittest
from unittest.mock import Mock, patch

from application import PluginError
from check_queries import shutdown

DIRECTORY = Path(__file__).resolve().parent


class JobsTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('jobs_example', DIRECTORY / 'jobs.py')
        self.jobs = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.jobs)
        self.addCleanup(shutdown, self.jobs.app)

    def test_cancellation_before_create_returns_is_acknowledged_without_waiting(self):
        finished = queue.Queue()
        def request(method, **params):
            if method == 'job.create':
                self.jobs.event('job.cancel_requested', {'job': 'j:1'})
                return {'job': 'j:1'}
            if method == 'job.finish':
                finished.put(params)
            return {}
        self.jobs.app.request = request
        self.assertEqual(self.jobs.start({}), {'job': 'j:1'})
        self.assertEqual(finished.get(timeout=1), {'job': 'j:1', 'state': 'cancelled'})

    def test_cancellation_winning_success_is_acknowledged(self):
        calls, workers = [], []
        def request(method, **params):
            if method == 'job.create':
                return {'job': 'j:1'}
            calls.append((method, params))
            if params.get('state') == 'succeeded':
                raise PluginError('cancelled', 'Cancellation won')
            return {}
        self.jobs.app.request = request
        with patch.object(threading, 'Event', return_value=Mock(wait=lambda _: False)), \
                patch.object(threading, 'Thread', side_effect=lambda **kw: workers.append(kw['target']) or Mock()):
            self.jobs.start({})
            workers[0]()
        self.assertEqual([params['state'] for _, params in calls], ['succeeded', 'cancelled'])
        self.assertFalse(self.jobs.cancellations)

    def test_worker_start_failure_finishes_the_accepted_job(self):
        calls = []
        self.jobs.app.request = lambda method, **params: calls.append((method, params)) or {'job': 'j:1'}
        with patch.object(threading.Thread, 'start', side_effect=RuntimeError('No worker')):
            with self.assertRaises(RuntimeError):
                self.jobs.start({})
        self.assertEqual(calls[-1], ('job.finish', {'job': 'j:1', 'state': 'failed'}))
        self.assertFalse(self.jobs.cancellations)

    def test_early_cancellation_memory_is_bounded_and_preserves_recent_handles(self):
        for number in range(100):
            self.jobs.event('job.cancel_requested', {'job': f'j:{number}'})
        self.assertEqual(len(self.jobs.early_cancellations), 16)
        self.assertIn('j:99', self.jobs.early_cancellations)
        self.assertNotIn('j:0', self.jobs.early_cancellations)

    def test_real_example_reader_handles_start_and_cancellation_without_a_frontend(self):
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'jobs.py')],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        self.addCleanup(lambda: child.poll() is None and child.kill())
        def send(value):
            child.stdin.write((json.dumps(value) + '\n').encode())
        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(2), 'Background job response stalled')
            return json.loads(child.stdout.readline(1048577))
        fixtures = json.loads((DIRECTORY / 'epoch2-fixtures.json').read_text())
        hello, registered, invocation = [item['message'] for item in fixtures if item['direction'] == 'host'][:3]
        try:
            send(hello)
            self.assertEqual(receive()['required_capabilities'], ['jobs'])
            send(registered)
            send({**invocation, 'params': {**invocation['params'], 'command': 'start'}})
            create = receive()
            self.assertEqual(create['method'], 'job.create')
            send({'type': 'response', 'id': create['id'], 'result': {'job': 'j:1'}})
            send({'type': 'event', 'event': 'job.cancel_requested', 'data': {'job': 'j:1'}})
            messages = [receive(), receive()]
            finish = next(value for value in messages if value.get('method') == 'job.finish')
            self.assertEqual(finish['params'], {'job': 'j:1', 'state': 'cancelled'})
            self.assertTrue(any(value.get('id') == invocation['id'] for value in messages))
            send({'type': 'response', 'id': finish['id'], 'result': {}})
            child.stdin.close()
            self.assertEqual(child.wait(timeout=2), 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=2)
            for stream in (child.stdin, child.stdout, child.stderr):
                stream.close()


if __name__ == '__main__':
    unittest.main()
