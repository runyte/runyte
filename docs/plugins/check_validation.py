# SPDX-License-Identifier: MPL-2.0
"""Validation callback isolation, correlation and secret-safe SDK failures."""
import queue
import json
from pathlib import Path
import selectors
import subprocess
import sys
import threading
import unittest
from application import Application, PluginError


class ValidationTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Validation', [], ['interaction'])
        self.messages = queue.Queue()
        self.app._write = self.messages.put
        self.release = threading.Event()

    def tearDown(self):
        self.release.set()
        with self.app._lock:
            for future in self.app._pending.values():
                future.set_result({})
        for executor in (self.app._executor, self.app._resource_executor,
                         self.app._control, self.app._observations, self.app._validation_executor):
            executor.shutdown(wait=True, cancel_futures=True)

    def request(self, request_id='h:1', revision='i:1'):
        return {'type': 'request', 'id': request_id, 'method': 'ui.validate',
                'params': {'surface': 'u:1', 'revision': revision,
                           'fields': ['name', 'password'],
                           'values': {'name': 'example', 'password': 'private-test-value'}}}

    def test_validation_returns_typed_exact_revision_without_echoing_values(self):
        contexts = []
        def validate(context):
            contexts.append(context)
            return {'password': 'invalid', 'name': 'valid'}
        self.app.on_validate = validate
        self.app._submit(self.request())
        reply = self.messages.get(timeout=2)
        self.assertEqual(reply, {'type': 'response', 'id': 'h:1', 'result': {
            'kind': 'validation', 'surface': 'u:1', 'revision': 'i:1',
            'fields': [{'field': 'name', 'status': 'valid'}, {'field': 'password', 'status': 'invalid'}]}})
        self.assertEqual(contexts[0]['values']['password'], 'private-test-value')
        self.assertNotIn('private-test-value', str(reply))

    def test_default_validator_refuses_unimplemented_validation(self):
        self.app._submit(self.request())
        self.assertEqual([field['status'] for field in self.messages.get(timeout=2)['result']['fields']],
                         ['unavailable', 'unavailable'])

    def test_invalid_return_and_exceptions_never_echo_secrets(self):
        for value in ({'name': 'valid'}, {'name': 'valid', 'password': 'private-test-value'}, None):
            with self.subTest(value=value):
                self.app.on_validate = lambda _: value
                self.app._submit(self.request())
                reply = self.messages.get(timeout=2)
                self.assertEqual(reply['error']['code'], 'invalid_argument')
                self.assertNotIn('private-test-value', str(reply))
        def fail(_):
            raise PluginError('unavailable', 'private-test-value')
        self.app.on_validate = fail
        self.app._submit(self.request())
        self.assertEqual(self.messages.get(timeout=2)['error'],
                         {'code': 'unavailable', 'message': 'Application validation failed'})

    def test_waiting_commands_cannot_starve_validation_or_cancellation(self):
        self.app.handlers['wait'] = lambda _: self.app.request('workspace.info')
        for index in range(4):
            self.app._submit({'type': 'request', 'id': f'h:{index + 10}',
                              'method': 'command.invoke', 'params': {'command': 'wait'}})
        for _ in range(4):
            self.assertEqual(self.messages.get(timeout=2)['method'], 'workspace.info')
        entered, cancelled = threading.Event(), threading.Event()
        def validate(_):
            entered.set()
            self.release.wait(2)
            return {'name': 'valid', 'password': 'valid'}
        self.app.on_validate = validate
        self.app.on_event = lambda *_: cancelled.set()
        self.app._submit(self.request())
        self.assertTrue(entered.wait(2))
        self.app._submit({'type': 'event', 'event': 'ui.validation_cancelled',
                          'data': {'request': 'h:1', 'surface': 'u:1', 'revision': 'i:1'}})
        self.assertTrue(cancelled.wait(2))

    def test_validation_dispatch_is_bounded(self):
        entered = threading.Event()
        def validate(_):
            entered.set()
            self.release.wait(2)
            return {'name': 'valid', 'password': 'valid'}
        self.app.on_validate = validate
        self.app._submit(self.request())
        self.assertTrue(entered.wait(2))
        self.app._submit(self.request('h:2'))
        with self.assertRaises(PluginError) as error:
            self.app._submit(self.request('h:3'))
        self.assertEqual(error.exception.code, 'busy')

    def test_example_runs_through_the_public_validation_wire(self):
        directory = Path(__file__).resolve().parent
        child = subprocess.Popen([sys.executable, str(directory / 'validation.py')],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)
        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()
        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(3), 'Example response timed out')
            return json.loads(child.stdout.readline(1048577))
        try:
            send({'type': 'hello', 'version': 'runyte-experimental-2'})
            self.assertEqual(receive()['required_capabilities'], ['interaction'])
            send({'type': 'registered', 'capabilities': ['interaction']})
            send({'type': 'request', 'id': 'h:10', 'method': 'command.invoke',
                  'params': {'command': 'open', 'context': 'workspace'}})
            form = receive()
            self.assertEqual(form['method'], 'ui.form')
            self.assertTrue(all(field['validate'] for field in form['params']['fields']))
            send({'type': 'response', 'id': form['id'], 'result': {'surface': 'u:1'}})
            self.assertEqual(receive()['id'], 'h:10')
            request = self.request()
            request['params'].update(fields=['name', 'code'], values={'name': 'taken', 'code': 'demo-code'})
            send(request)
            result = receive()['result']
            self.assertEqual(result['fields'], [{'field': 'name', 'status': 'invalid'}, {'field': 'code', 'status': 'valid'}])
            request['id'] = 'h:2'
            request['params']['revision'] = 'i:2'
            request['params']['values']['name'] = 'available'
            send(request)
            result = receive()['result']
            self.assertEqual(result['revision'], 'i:2')
            self.assertTrue(all(field['status'] == 'valid' for field in result['fields']))
            child.stdin.close()
            self.assertEqual(child.wait(timeout=3), 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)
            for pipe in (child.stdin, child.stdout, child.stderr):
                if not pipe.closed:
                    pipe.close()


if __name__ == '__main__':
    unittest.main(verbosity=2)
