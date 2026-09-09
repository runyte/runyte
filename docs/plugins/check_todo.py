# SPDX-License-Identifier: MPL-2.0
"""Run identical public-wire behavior checks against Python, Rust and C todos.

Build compiled programs with todo/build.py before running this suite. Tests never
write executable fixtures, configuration or editor state into a real workspace.
"""
import argparse
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import tempfile
import time
import unittest

from jsonschema import Draft202012Validator

DIRECTORY = Path(__file__).resolve().parent
OUTPUT = Path(os.environ.get('RUNYTE_TODO_BIN_DIR', DIRECTORY.parents[1] / 'target' / 'todo-showcase'))
SCHEMA = json.loads((DIRECTORY / 'runyte-experimental-2.schema.json').read_text())
FIXTURES = json.loads((DIRECTORY / 'epoch2-fixtures.json').read_text())
HOST = [f['message'] for f in FIXTURES if f['direction'] == 'host']
VALIDATOR = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]})
LIMIT = 1048576
VARIANTS = {
    'Python': [sys.executable, str(DIRECTORY / 'todo' / 'python' / 'todo.py')],
    'Rust': [str(OUTPUT / 'rust' / 'release' / 'runyte-todo-example')],
    'C': [str(OUTPUT / 'todo-c')],
}


class TodoChecks:
    def setUp(self):
        executable = VARIANTS[self.language]
        if not Path(executable[0]).is_file():
            self.skipTest('Build all variants with docs/plugins/todo/build.py')
        directory = tempfile.TemporaryDirectory(prefix='runyte-todo-check-')
        self.addCleanup(directory.cleanup)
        self.child = subprocess.Popen(executable, cwd=directory.name, stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        self.addCleanup(self.close)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.child.stdout, selectors.EVENT_READ)
        self.addCleanup(self.selector.close)
        self.buffer = bytearray()
        self.serial = self.last_request = 0
        self.view, self.revision = 'v:g:todo', 'm:1'

    def close(self):
        if not self.child.stdin.closed:
            self.child.stdin.close()
        try:
            self.child.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self.child.kill()
            self.child.wait(timeout=2)
            self.fail('Plugin did not stop at EOF')
        finally:
            self.child.stdout.close()
            self.child.stderr.close()

    def send(self, message):
        self.child.stdin.write((json.dumps(message, ensure_ascii=False) + '\n').encode())
        self.child.stdin.flush()

    def read(self):
        deadline = time.monotonic() + 3
        while b'\n' not in self.buffer:
            self.assertTrue(self.selector.select(max(0, deadline - time.monotonic())), 'Response timed out')
            data = os.read(self.child.stdout.fileno(), 8192)
            self.assertTrue(data, 'Plugin exited before responding')
            self.buffer.extend(data)
            self.assertLessEqual(len(self.buffer), LIMIT + 8192)
        line, _, self.buffer = self.buffer.partition(b'\n')
        self.assertLessEqual(len(line) + 1, LIMIT)
        message = json.loads(line)
        VALIDATOR.validate(message)
        if message['type'] == 'request':
            serial = int(message['id'].split(':')[1])
            self.assertEqual(serial, self.last_request + 1)
            self.last_request = serial
        return message

    def reply(self, request, result=None, error=None):
        self.send({'type': 'response', 'id': request['id'],
                   **({'error': {'code': error, 'message': 'Refused'}} if error else {'result': result})})

    def handshake(self):
        self.send(HOST[0])
        registration = self.read()
        self.assertEqual(registration['type'], 'register')
        self.assertEqual(registration['required_capabilities'], ['views'])
        commands = registration['commands']
        self.assertEqual([c['name'] for c in commands], ['open', 'add', 'toggle', 'remove', 'filter'])
        self.assertEqual(commands[1]['arguments'], [{'name': 'title', 'type': 'string'}])
        self.assertEqual([c['name'] for c in commands if c.get('primary')], ['toggle'])
        self.send({**HOST[1], 'capabilities': ['views']})

    def invoke(self, command='open', **params):
        self.serial += 1
        context = {'command': command, 'context': 'workspace' if command == 'open' else 'view',
                   'arguments': {}, 'rows': ['task-1'], 'view': self.view, 'model_revision': self.revision}
        context.update(params)
        self.send({'type': 'request', 'id': f'h:{self.serial}', 'method': 'command.invoke', 'params': context})
        return f'h:{self.serial}'

    def completed(self, id, error=None):
        response = self.read()
        self.assertEqual(response['id'], id)
        if error:
            self.assertEqual(response['error']['code'], error)
        else:
            self.assertEqual(response['result'], {'job': None})

    def show(self, id):
        show = self.read()
        self.assertEqual(show['method'], 'pane.show')
        self.assertEqual(show['params'], {'invocation': id, 'view': self.view})
        self.reply(show, {'pane': 'p:g:1'})
        self.completed(id)

    def open(self):
        self.handshake()
        id = self.invoke()
        create = self.read()
        self.assertEqual(create['method'], 'view.create')
        self.assertEqual(create['params']['model']['rows'], [
            {'id': 'task-1', 'text': '[ ] Read the plugin guide', 'role': 'ordinary'}])
        self.reply(create, {'view': self.view, 'revision': self.revision})
        self.show(id)

    def publish(self, command, error=None, **params):
        id = self.invoke(command, **params)
        request = self.read()
        self.assertEqual(request['method'], 'view.publish')
        self.assertEqual(request['params']['view'], self.view)
        self.assertEqual(request['params']['expected_revision'], self.revision)
        if error:
            self.reply(request, error=error)
        else:
            self.revision = f'm:{self.serial}'
            self.reply(request, {'view': self.view, 'revision': self.revision})
        self.completed(id, error)
        return request['params']['model']['rows']

    def test_add_unicode_multiselect_toggle_filter_remove_and_reopen(self):
        self.open()
        title = 'Buy "żółć" \\ tea 😀 e\u0301'
        rows = self.publish('add', arguments={'title': title})
        self.assertEqual(rows[1], {'id': 'task-2', 'text': '[ ] ' + title, 'role': 'ordinary'})
        rows = self.publish('toggle', rows=['task-1', 'task-2'])
        self.assertTrue(all(r['text'].startswith('[x]') and r['role'] == 'muted' for r in rows))
        self.assertEqual(self.publish('filter'), [])
        rows = self.publish('add', arguments={'title': 'Visible while filtered'})
        self.assertEqual([r['id'] for r in rows], ['task-3'])
        self.assertEqual(self.publish('remove', rows=['task-3']), [])
        rows = self.publish('filter')
        self.assertEqual([r['id'] for r in rows], ['task-1', 'task-2'])
        self.publish('remove', rows=['task-1', 'task-2'])
        self.assertEqual(self.publish('add', arguments={'title': 'New'})[0]['id'], 'task-4')
        self.show(self.invoke())  # Reuse the one retained view.

    def test_stale_context_and_invalid_rows_never_publish(self):
        self.open()
        for override in [{'view': 'v:g:other'}, {'model_revision': 'm:old'}, {'context': 'workspace'}]:
            self.completed(self.invoke('toggle', **override), 'stale')
        for rows in [[], ['missing'], ['task-1', 'task-1'], [1]]:
            self.completed(self.invoke('toggle', rows=rows), 'invalid_argument')
        self.publish('toggle')
        self.publish('filter')
        self.completed(self.invoke('remove'), 'invalid_argument')  # Hidden row.

    def test_title_validation_uses_utf8_bytes_and_rejects_control_characters(self):
        self.open()
        for title in ['', '   ', 'x' * 257, '😀' * 65, 'a\nb', 'a\tb', '\x1b[31m',
                      'a\x00b', 'a\x7fb', 'a\u0085b', 'a\u2028b', 'a\u2029b', 12]:
            self.completed(self.invoke('add', arguments={'title': title}), 'invalid_argument')
        self.assertEqual(self.publish('add', arguments={'title': '😀' * 64})[-1]['text'], '[ ] ' + '😀' * 64)

    def test_refusal_rolls_back_task_data_filter_and_ids(self):
        self.open()
        self.publish('add', error='busy', arguments={'title': 'Refused'})
        rows = self.publish('add', arguments={'title': 'Accepted'})
        self.assertEqual(rows[-1]['id'], 'task-2')
        self.publish('toggle', error='stale')
        self.assertTrue(self.publish('toggle')[0]['text'].startswith('[x]'))
        self.publish('filter', error='busy')
        self.assertEqual([r['id'] for r in self.publish('filter')], ['task-2'])
        self.publish('remove', error='busy', rows=['task-2'])
        self.assertEqual(self.publish('remove', rows=['task-2']), [])

    def test_busy_command_and_close_during_publication(self):
        self.open()
        active = self.invoke('toggle')
        pending = self.read()
        self.completed(self.invoke('filter'), 'busy')
        self.send({'type': 'event', 'event': 'view.closed', 'data': {'view': self.view}})
        self.reply(pending, {'view': self.view, 'revision': 'm:2'})
        self.completed(active, 'stale')
        id = self.invoke()
        create = self.read()
        self.assertTrue(create['params']['model']['rows'][0]['text'].startswith('[ ]'))
        self.view, self.revision = 'v:g:new', 'm:new'
        self.reply(create, {'view': self.view, 'revision': self.revision})
        self.show(id)

    def test_closed_view_recreated_with_retained_tasks_and_opaque_tokens(self):
        self.open()
        self.publish('add', arguments={'title': 'Keep after view close'})
        self.send({'type': 'event', 'event': 'view.closed', 'data': {'view': self.view}})
        id = self.invoke()
        create = self.read()
        self.assertEqual(len(create['params']['model']['rows']), 2)
        self.view, self.revision = 'opaque-view', 'opaque-revision'
        self.reply(create, {'view': self.view, 'revision': self.revision})
        self.show(id)
        self.publish('toggle')

    def test_ambiguous_publication_blocks_new_mutations(self):
        self.open()
        self.publish('toggle', error='outcome_unknown')
        self.completed(self.invoke('toggle'), 'unavailable')
        self.completed(self.invoke(), 'unavailable')

    def test_bad_publication_acknowledgement_blocks_new_mutations(self):
        self.open()
        id = self.invoke('toggle')
        request = self.read()
        self.reply(request, {'view': self.view, 'revision': self.revision})
        self.completed(id, 'outcome_unknown')
        self.completed(self.invoke('toggle'), 'unavailable')

    def test_failed_presentation_reuses_view_without_new_authority(self):
        self.handshake()
        id = self.invoke()
        create = self.read()
        self.reply(create, {'view': self.view, 'revision': self.revision})
        show = self.read()
        self.reply(show, error='context_changed')
        self.completed(id, 'context_changed')
        self.show(self.invoke())

    def test_limit_releases_capacity_without_reusing_ids(self):
        self.open()
        for index in range(99):
            self.publish('add', arguments={'title': f'Task {index}'})
        self.completed(self.invoke('add', arguments={'title': 'Too many'}), 'limit_exceeded')
        self.publish('remove', rows=['task-2'])
        rows = self.publish('add', arguments={'title': 'Room again'})
        self.assertEqual(len(rows), 100)
        self.assertEqual(rows[-1]['id'], 'task-101')

    def test_fragmented_frames_and_unknown_host_fields(self):
        frame = (json.dumps({**HOST[0], 'future_field': {'ignored': True}}) + '\n').encode()
        for piece in [frame[:3], frame[3:11], frame[11:]]:
            self.child.stdin.write(piece)
        self.assertEqual(self.read()['type'], 'register')
        self.send({**HOST[1], 'capabilities': ['views']})
        self.send({'type': 'event', 'event': 'future.event', 'data': {}})
        self.invoke()
        self.assertEqual(self.read()['method'], 'view.create')

    def test_eof_settles_pending_request(self):
        self.handshake()
        self.invoke()
        self.read()
        self.child.stdin.close()
        self.assertEqual(self.child.wait(timeout=2), 0)

    def test_missing_acknowledgement_times_out_without_replay(self):
        self.open()
        self.invoke('toggle')
        self.assertEqual(self.read()['method'], 'view.publish')
        self.assertNotEqual(self.child.wait(timeout=10), 0)
        self.assertEqual(self.child.stdout.read(), b'')

    def test_oversized_unterminated_frame_fails(self):
        try:
            self.child.stdin.write(b' ' * LIMIT)
        except BrokenPipeError:
            pass
        self.assertNotEqual(self.child.wait(timeout=2), 0)

    def test_malformed_json_fails(self):
        self.child.stdin.write(b'{invalid}\n')
        self.assertNotEqual(self.child.wait(timeout=2), 0)

    def test_invalid_utf8_fails(self):
        self.child.stdin.write(b'{"type":"\xff"}\n')
        self.assertNotEqual(self.child.wait(timeout=2), 0)

    def test_missing_required_capability_fails(self):
        self.send({**HOST[0], 'capabilities': []})
        self.assertNotEqual(self.child.wait(timeout=2), 0)

    def test_truncated_frame_fails(self):
        self.child.stdin.write(b'{"type":')
        self.child.stdin.close()
        self.assertNotEqual(self.child.wait(timeout=2), 0)

    def test_wrong_epoch_fails(self):
        self.send({**HOST[0], 'version': 'runyte-experimental-1'})
        self.assertNotEqual(self.child.wait(timeout=2), 0)

    def test_unknown_response_fails(self):
        self.handshake()
        self.send({'type': 'response', 'id': 'p:999', 'result': {}})
        self.assertNotEqual(self.child.wait(timeout=2), 0)


for language in VARIANTS:
    globals()[f'{language}TodoTests'] = type(f'{language}TodoTests', (TodoChecks, unittest.TestCase), {'language': language})

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--require-all', action='store_true')
    options, remaining = parser.parse_known_args()
    if options.require_all:
        for command in VARIANTS.values():
            if not Path(command[0]).is_file():
                parser.error('Build all variants with docs/plugins/todo/build.py first')
    unittest.main(argv=[sys.argv[0]] + remaining)
