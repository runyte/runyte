# SPDX-License-Identifier: MPL-2.0
"""Real Node epoch-2 conformance; Node uses no SDK or third-party dependencies."""
import copy
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import time
import unittest

from jsonschema import Draft202012Validator

DIRECTORY = Path(__file__).resolve().parent
SCHEMA = json.loads((DIRECTORY / 'runyte-experimental-2.schema.json').read_text())
FIXTURES = json.loads((DIRECTORY / 'epoch2-fixtures.json').read_text())
HOST = [fixture['message'] for fixture in FIXTURES if fixture['direction'] == 'host']
VALIDATOR = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]})
NODE = os.environ.get('NODE') or shutil.which('node')
LIMIT = 1048576


@unittest.skipUnless(NODE, 'Install Node.js or set NODE')
class NodeTasksTests(unittest.TestCase):
    def setUp(self):
        self.child = subprocess.Popen([NODE, str(DIRECTORY / 'tasks.mjs')],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        self.addCleanup(self.close)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.child.stdout, selectors.EVENT_READ)
        self.addCleanup(self.selector.close)
        self.buffer = bytearray()
        self.messages = []
        self.last_request = 0

    def close(self):
        if self.child.poll() is None:
            self.child.stdin.close()
            try:
                self.child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait(timeout=2)
                self.fail('Node failed to settle pending requests at EOF')
        for stream in (self.child.stdin, self.child.stdout, self.child.stderr):
            if not stream.closed:
                stream.close()

    def send(self, message):
        self.child.stdin.write((json.dumps(message, ensure_ascii=False) + '\n').encode())
        self.child.stdin.flush()

    def read(self):
        deadline = time.monotonic() + 3
        while not self.messages:
            self.assertTrue(self.selector.select(max(0, deadline - time.monotonic())), 'Node response timed out')
            data = os.read(self.child.stdout.fileno(), LIMIT + 1)
            self.assertTrue(data, 'Node exited before a response')
            self.buffer.extend(data)
            self.assertLessEqual(len(self.buffer), 2 * LIMIT)
            while b'\n' in self.buffer:
                line, _, remaining = self.buffer.partition(b'\n')
                self.buffer = bytearray(remaining)
                self.assertLessEqual(len(line) + 1, LIMIT)
                value = json.loads(line)
                VALIDATOR.validate(value)
                self.messages.append(value)
        result = self.messages.pop(0)
        if result['type'] == 'request':
            serial = int(result['id'].split(':')[1])
            self.assertGreater(serial, self.last_request)
            self.last_request = serial
        return result

    def handshake(self):
        self.send(HOST[0])
        registered = self.read()
        self.assertEqual(registered['type'], 'register')
        self.assertEqual(registered['required_capabilities'], ['views'])
        self.assertEqual([command['name'] for command in registered['commands']], ['open', 'toggle'])
        self.send({**HOST[1], 'commands': ['plugin.node-tasks.open', 'plugin.node-tasks.toggle'],
                   'capabilities': ['views']})

    def invoke(self, serial, command='open', **params):
        base = copy.deepcopy(HOST[2])
        base['id'] = f'h:{serial}'
        base['params'].update(command=command, **params)
        self.send(base)

    def reply(self, request, result=None, error=None):
        self.send({'type': 'response', 'id': request['id'],
                   **({'error': error} if error else {'result': result})})

    def opened(self):
        self.handshake()
        self.invoke(1)
        create = self.read()
        self.assertEqual(create['method'], 'view.create')
        self.reply(create, {'view': 'v:g:1', 'revision': 'm:1', 'model': create['params']['model']})
        show = self.read()
        self.assertEqual(show['method'], 'pane.show')
        self.assertEqual(show['params'], {'invocation': 'h:1', 'view': 'v:g:1'})
        self.reply(show, {'pane': 'p:g:1'})
        self.assertEqual(self.read(), {'type': 'response', 'id': 'h:1', 'result': {'job': None}})

    def toggle(self, serial, revision='m:1', view='v:g:1', rows=None):
        self.invoke(serial, 'toggle', context='view', view=view, model_revision=revision,
                    rows=['read'] if rows is None else rows)

    def test_real_wire_success_uses_exact_revision_and_retains_stable_task_rows(self):
        self.opened()
        self.toggle(2)
        publish = self.read()
        self.assertEqual(publish['method'], 'view.publish')
        self.assertEqual(publish['params']['expected_revision'], 'm:1')
        rows = publish['params']['model']['rows']
        self.assertEqual([row['id'] for row in rows], ['read', 'edit', 'split'])
        self.assertTrue(rows[0]['text'].startswith('[x]'))
        self.reply(publish, {'view': 'v:g:1', 'revision': 'm:2', 'model': publish['params']['model']})
        self.assertEqual(self.read()['result'], {'job': None})
        self.toggle(3, revision='m:2')
        publish = self.read()
        self.assertTrue(publish['params']['model']['rows'][0]['text'].startswith('[ ]'))
        self.reply(publish, {'view': 'v:g:1', 'revision': 'm:3', 'model': publish['params']['model']})
        self.assertEqual(self.read()['id'], 'h:3')
        self.assertFalse(self.selector.select(0.2), 'Quiescent example produced output')

    def test_failed_publication_preserves_revision_and_candidate_state_for_retry(self):
        self.opened()
        self.toggle(2)
        first = self.read()
        self.reply(first, error={'code': 'busy', 'message': 'private diagnostic canary'})
        failed = self.read()
        self.assertEqual(failed['error']['code'], 'busy')
        self.assertNotIn('canary', str(failed))
        self.toggle(3)
        retry = self.read()
        self.assertEqual(retry['params'], first['params'])
        self.reply(retry, {'view': 'v:g:1', 'revision': 'm:2', 'model': retry['params']['model']})
        self.assertEqual(self.read()['id'], 'h:3')

    def test_stale_view_revision_unknown_duplicate_and_empty_rows_never_publish(self):
        self.opened()
        for serial, params in enumerate([{'revision': 'm:0'}, {'view': 'v:other'},
                                         {'rows': ['unknown']}, {'rows': ['read', 'read']}, {'rows': []}], 2):
            self.toggle(serial, **params)
            result = self.read()
            self.assertEqual(result['type'], 'response')
            self.assertIn(result['error']['code'], ('stale', 'invalid_argument'))

    def test_foreground_denial_never_retries_or_replaces_the_captured_invocation(self):
        self.handshake()
        self.invoke(1)
        create = self.read()
        self.reply(create, {'view': 'v:g:1', 'revision': 'm:1', 'model': create['params']['model']})
        show = self.read()
        self.reply(show, error={'code': 'context_changed', 'message': 'Focus changed'})
        self.assertEqual(self.read()['error']['code'], 'context_changed')
        self.invoke(2)
        show = self.read()
        self.assertEqual(show['method'], 'pane.show')
        self.assertEqual(show['params']['invocation'], 'h:2')
        self.reply(show, {'pane': 'p:g:1'})
        self.assertEqual(self.read()['id'], 'h:2')

    def test_reader_handles_concurrent_request_and_close_while_publication_waits(self):
        self.opened()
        self.toggle(2)
        publish = self.read()
        self.invoke(3)
        self.assertEqual(self.read()['error']['code'], 'busy')
        self.send({'type': 'event', 'sequence': 'e:1', 'event': 'view.closed', 'data': {'view': 'v:g:1'}})
        self.reply(publish, {'view': 'v:g:1', 'revision': 'm:2', 'model': publish['params']['model']})
        self.assertEqual(self.read()['error']['code'], 'stale')
        self.invoke(4)
        create = self.read()
        self.assertEqual(create['method'], 'view.create')
        self.assertTrue(create['params']['model']['rows'][0]['text'].startswith('[ ]'))

    def test_close_overtaking_create_reply_cannot_resurrect_a_retired_view(self):
        self.handshake()
        self.invoke(1)
        create = self.read()
        messages = [{'type': 'response', 'id': create['id'], 'result':
                     {'view': 'v:g:1', 'revision': 'm:1', 'model': create['params']['model']}},
                    {'type': 'event', 'sequence': 'e:1', 'event': 'view.closed', 'data': {'view': 'v:g:1'}}]
        self.child.stdin.write((''.join(json.dumps(message) + '\n' for message in messages)).encode())
        self.child.stdin.flush()
        self.assertEqual(self.read()['error']['code'], 'stale')

    def test_old_view_close_does_not_invalidate_replacement_publication(self):
        self.opened()
        self.send({'type': 'event', 'sequence': 'e:1', 'event': 'view.closed', 'data': {'view': 'v:g:1'}})
        self.invoke(2)
        create = self.read()
        self.reply(create, {'view': 'v:g:2', 'revision': 'm:1', 'model': create['params']['model']})
        show = self.read()
        self.reply(show, {'pane': 'p:g:1'})
        self.assertEqual(self.read()['id'], 'h:2')
        self.toggle(3, view='v:g:2')
        publish = self.read()
        self.send({'type': 'event', 'sequence': 'e:2', 'event': 'view.closed', 'data': {'view': 'v:g:1'}})
        self.reply(publish, {'view': 'v:g:2', 'revision': 'm:2', 'model': publish['params']['model']})
        self.assertEqual(self.read()['result'], {'job': None})

    def test_unknown_publication_requires_restart_and_never_replays(self):
        self.opened()
        self.toggle(2)
        publish = self.read()
        self.reply(publish, error={'code': 'outcome_unknown', 'message': 'private canary'})
        self.assertEqual(self.read()['error']['code'], 'outcome_unknown')
        self.toggle(3)
        self.assertEqual(self.read()['error']['code'], 'unavailable')
        self.invoke(4)
        failed = self.read()
        self.assertEqual(failed['error']['code'], 'unavailable')
        self.assertIn('restart', failed['error']['message'])
        self.assertNotIn('canary', str(failed))

    def test_actual_publication_deadline_leaves_host_headroom_and_ignores_late_success(self):
        self.opened()
        self.toggle(2)
        publish = self.read()
        started = time.monotonic()
        self.assertTrue(self.selector.select(9), 'Callback did not honor its bounded deadline')
        self.assertEqual(self.read()['error']['code'], 'timeout')
        self.assertLess(time.monotonic() - started, 9)
        self.reply(publish, {'view': 'v:g:1', 'revision': 'm:2', 'model': publish['params']['model']})
        self.toggle(3)
        self.assertEqual(self.read()['error']['code'], 'unavailable')

    def test_eof_rejects_outstanding_wait_without_waiting_for_request_deadline(self):
        self.handshake()
        self.invoke(1)
        self.assertEqual(self.read()['method'], 'view.create')
        self.child.stdin.close()
        self.assertEqual(self.child.wait(timeout=2), 0)
        self.assertEqual(self.child.stderr.read(), b'')

    def test_malformed_utf8_and_oversized_line_exit_without_echoing_input(self):
        for raw in [b'{not json secret}\n', b'\xff\n', b'x' * LIMIT]:
            child = subprocess.Popen([NODE, str(DIRECTORY / 'tasks.mjs')], stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                output, errors = child.communicate(raw, timeout=3)
                self.assertEqual(child.returncode, 1)
                self.assertEqual(output, b'')
                self.assertEqual(errors, b'')
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=2)
                for stream in (child.stdin, child.stdout, child.stderr):
                    if not stream.closed:
                        stream.close()


if __name__ == '__main__':
    unittest.main(verbosity=2)
