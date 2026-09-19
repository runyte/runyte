# SPDX-License-Identifier: MPL-2.0
"""Stable schema, public examples and real uppercase wire behavior.

The jsonschema dependency is development-only. Child processes and test data
remain isolated; plugin runtime dependencies are Python standard library only.
"""
import copy
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import sys
import tempfile
import unittest

from jsonschema import Draft202012Validator

DIRECTORY = Path(__file__).resolve().parent
SCHEMA = json.loads((DIRECTORY / 'runyte-1.schema.json').read_text())
FIXTURES = json.loads((DIRECTORY / 'stable-fixtures.json').read_text())
HOST = [fixture['message'] for fixture in FIXTURES if fixture['direction'] == 'host']
VALIDATOR = Draft202012Validator(SCHEMA)
PLUGIN_VALIDATOR = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]})


class PluginSchemaTests(unittest.TestCase):
    def test_schema_and_directional_fixtures(self):
        Draft202012Validator.check_schema(SCHEMA)
        self.assertTrue(FIXTURES)
        for fixture in FIXTURES:
            direction = fixture['direction'] + 'Message'
            validator = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/' + direction}]})
            with self.subTest(direction=direction, type=fixture['message']['type']):
                errors = list(validator.iter_errors(fixture['message']))
                self.assertFalse(errors, errors[0].message if errors else '')

    def test_row_action_override_shape_in_both_directions(self):
        for definition in ('row', 'host_row'):
            validator = Draft202012Validator({'$defs': SCHEMA['$defs'], '$ref': '#/$defs/' + definition})
            row = {'id': 'row', 'text': 'Row', 'role': 'ordinary'}
            self.assertTrue(validator.is_valid(row))
            for actions in ([], ['refresh', 'disconnect']):
                self.assertTrue(validator.is_valid({**row, 'actions': actions}))
            for actions in (None, ['duplicate', 'duplicate'], ['bad name'], ['x'] * 65):
                self.assertFalse(validator.is_valid({**row, 'actions': actions}))

    def test_event_name_selects_payload_with_additive_host_fields(self):
        validator = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/hostMessage'}]})
        events = [f['message'] for f in FIXTURES if f['direction'] == 'host' and f['message']['type'] == 'event']
        self.assertTrue(events)
        for original in events:
            message = copy.deepcopy(original)
            message['future_host_field'] = True
            message['data']['future_payload_field'] = True
            with self.subTest(event=message['event']):
                self.assertTrue(validator.is_valid(message))
        changed = {'type': 'event', 'sequence': 'e:1', 'event': 'event.changed',
                   'data': {'subscription': 'o:1', 'sources': [], 'coalesced': 0, 'future': True}}
        self.assertTrue(validator.is_valid(changed))
        # A subscription alone is a resync payload, never a changed payload.
        self.assertFalse(validator.is_valid({**changed, 'data': {'subscription': 'o:1'}}))
        self.assertTrue(validator.is_valid({**changed, 'event': 'event.resync_required'}))
        self.assertFalse(validator.is_valid({**changed, 'event': 'future.event'}))

    def test_untagged_results_allow_overlap_but_empty_acknowledgement_is_exact(self):
        validator = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/hostMessage'}]})
        reply = {'type': 'response', 'id': 'p:1', 'result': {}}
        self.assertTrue(validator.is_valid(reply))
        self.assertFalse(validator.is_valid({**reply, 'result': {'unknown_result': True}}))
        self.assertFalse(validator.is_valid({**reply, 'result': {'revision': 17}}))
        # Selection also has the buffer/revision fields of a local-open result.
        selection = {'pane': 'p:1', 'buffer': 'b:1', 'revision': 'q:1', 'primary': 0,
                     'ranges': [{'anchor': 0, 'head': 0, 'future': True}],
                     'spans': [{'from': 0, 'to': 1, 'future': True}], 'future': True}
        self.assertTrue(validator.is_valid({**reply, 'result': selection}))
        self.assertFalse(validator.is_valid({**reply, 'error': {'code': 'stale', 'message': 'Changed'}}))

    def test_guide_json_blocks(self):
        guide = (DIRECTORY.parent / 'plugins.md').read_text()
        blocks = re.findall(r'^```json\n(.*?)^```', guide, re.M | re.S)
        self.assertTrue(blocks)
        for block in blocks:
            try:
                messages = [json.loads(block)]
            except json.JSONDecodeError:
                messages = [json.loads(line) for line in block.splitlines() if line.strip()]
            for message in messages:
                with self.subTest(type=message.get('type')):
                    errors = list(VALIDATOR.iter_errors(message))
                    self.assertFalse(errors, errors[0].message if errors else '')

    def test_experimental_protocols_and_missing_stable_metadata_are_rejected(self):
        registration = next(f['message'] for f in FIXTURES if f['message']['type'] == 'register')
        for version in ('runyte-experimental-1', 'runyte-experimental-2'):
            self.assertFalse(PLUGIN_VALIDATOR.is_valid({**registration, 'version': version}))
        for field in ('runyte', 'required_features', 'optional_features'):
            incomplete = {key: value for key, value in registration.items() if key != field}
            self.assertFalse(PLUGIN_VALIDATOR.is_valid(incomplete), field)


class UppercaseTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='runyte-uppercase-')
        self.addCleanup(directory.cleanup)
        self.child = subprocess.Popen([sys.executable, str(DIRECTORY / 'uppercase.py')],
            cwd=directory.name, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, bufsize=0)
        self.addCleanup(self.close)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.child.stdout, selectors.EVENT_READ)
        self.addCleanup(self.selector.close)
        self.buffer = bytearray()
        self.send(HOST[0])
        registration = self.read()
        self.assertEqual(registration['version'], 'runyte-1')
        self.assertEqual(registration['runyte'], '>=0.3.0, <0.4.0')
        self.assertEqual(registration['required_capabilities'], ['text', 'selections'])
        self.assertEqual(registration['commands'][0]['context'], 'buffer')
        self.send({**HOST[1], 'commands': ['plugin.case.uppercase'],
                   'capabilities': ['text', 'selections']})

    def close(self):
        if not self.child.stdin.closed:
            self.child.stdin.close()
        try:
            self.child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.child.kill()
            self.child.wait(timeout=3)
            self.fail('Uppercase did not stop at EOF')
        finally:
            self.child.stdout.close()
            self.child.stderr.close()

    def send(self, message):
        self.child.stdin.write((json.dumps(message, ensure_ascii=False) + '\n').encode())
        self.child.stdin.flush()

    def read(self):
        while b'\n' not in self.buffer:
            self.assertTrue(self.selector.select(3), 'Uppercase response timed out')
            chunk = os.read(self.child.stdout.fileno(), 65536)
            self.assertTrue(chunk, 'Uppercase exited before responding')
            self.buffer.extend(chunk)
            self.assertLessEqual(len(self.buffer), 1048576)
        line, _, self.buffer = self.buffer.partition(b'\n')
        result = json.loads(line)
        errors = list(PLUGIN_VALIDATOR.iter_errors(result))
        self.assertFalse(errors, errors[0].message if errors else '')
        return result

    def reply(self, request, result=None, error=None):
        self.send({'type': 'response', 'id': request['id'],
                   **({'error': error} if error else {'result': result})})

    def invoke(self):
        message = copy.deepcopy(HOST[2])
        message['id'] = 'h:uppercase'
        message['params'].update(command='uppercase', context='buffer', pane='p:g:1',
                                 buffer='b:g:1', buffer_revision='r:1', selection_revision='q:1')
        self.send(message)
        request = self.read()
        self.assertEqual(request['method'], 'selection.get')
        self.assertEqual(request['params'], {'pane': 'p:g:1'})
        return request

    def selection(self, request, spans, ranges=None, **overrides):
        self.reply(request, {'pane': 'p:g:1', 'buffer': 'b:g:1', 'revision': 'q:1',
            'ranges': ranges or [{'anchor': s['from'], 'head': s['to']} for s in spans],
            'spans': spans, 'primary': 0, **overrides})

    def test_unicode_directions_and_empty_spans_become_one_transaction(self):
        selection = self.invoke()
        spans = [{'from': 0, 'to': 3}, {'from': 3, 'to': 4}, {'from': 4, 'to': 4}]
        self.selection(selection, spans, ranges=[{'anchor': 2, 'head': 0},
                                                {'anchor': 3, 'head': 3},
                                                {'anchor': 4, 'head': 4}])
        for span, text in zip(spans, ['e\u0301ß', '😀', '']):
            request = self.read()
            self.assertEqual(request['method'], 'buffer.read')
            self.assertEqual(request['params'], {'buffer': 'b:g:1', 'expected_revision': 'r:1', **span})
            self.reply(request, {'revision': 'r:1', **span, 'text': text})
        edit = self.read()
        self.assertEqual(edit['method'], 'buffer.edit')
        self.assertEqual(edit['params'], {'buffer': 'b:g:1', 'expected_revision': 'r:1', 'changes': [
            {'from': 0, 'to': 3, 'text': 'E\u0301SS'},
            {'from': 3, 'to': 4, 'text': '😀'}, {'from': 4, 'to': 4, 'text': ''}]})
        self.reply(edit, {'buffer': 'b:g:1', 'revision': 'r:2', 'changes': 3})
        self.assertEqual(self.read(), {'type': 'response', 'id': 'h:uppercase', 'result': {'job': None}})
        self.assertFalse(self.selector.select(0.1), 'Uppercase emitted another transaction')

    def test_changed_selection_refuses_before_reading(self):
        self.selection(self.invoke(), [{'from': 0, 'to': 1}], revision='q:2')
        self.assertEqual(self.read()['error']['code'], 'stale')
        self.assertFalse(self.selector.select(0.1))

    def test_changed_pane_buffer_refuses_before_reading(self):
        self.selection(self.invoke(), [{'from': 0, 'to': 1}], buffer='b:g:other')
        self.assertEqual(self.read()['error']['code'], 'stale')
        self.assertFalse(self.selector.select(0.1))

    def test_stale_text_read_never_edits(self):
        self.selection(self.invoke(), [{'from': 0, 'to': 1}])
        request = self.read()
        self.assertEqual(request['method'], 'buffer.read')
        self.reply(request, error={'code': 'stale', 'message': 'Buffer changed'})
        self.assertEqual(self.read()['error']['code'], 'stale')
        self.assertFalse(self.selector.select(0.1))

    def test_edit_refusal_is_not_replayed(self):
        self.selection(self.invoke(), [{'from': 0, 'to': 1}])
        self.reply(self.read(), {'revision': 'r:1', 'from': 0, 'to': 1, 'text': 'ß'})
        edit = self.read()
        self.assertEqual(edit['method'], 'buffer.edit')
        self.reply(edit, error={'code': 'stale', 'message': 'Buffer changed'})
        self.assertEqual(self.read()['error']['code'], 'stale')
        self.assertFalse(self.selector.select(0.1))


if __name__ == '__main__':
    unittest.main(verbosity=2)
