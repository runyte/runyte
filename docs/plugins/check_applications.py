# SPDX-License-Identifier: MPL-2.0
"""Epoch 2 structural conformance; runtime preconditions are covered in Rust."""
import json
import selectors
import subprocess
import sys
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator

DIRECTORY = Path(__file__).resolve().parent
SCHEMA = json.loads((DIRECTORY / 'runyte-experimental-2.schema.json').read_text())
FIXTURES = json.loads((DIRECTORY / 'epoch2-fixtures.json').read_text())

class ApplicationSchemaTests(unittest.TestCase):
    def test_all_fixtures(self):
        Draft202012Validator.check_schema(SCHEMA)
        for fixture in FIXTURES:
            validator = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': f"#/$defs/{fixture['direction']}Message"}]})
            with self.subTest(message=fixture['message']):
                validator.validate(fixture['message'])
                malformed = {**fixture['message'], 'extra': True}
                if fixture['direction'] == 'plugin':
                    self.assertFalse(validator.is_valid(malformed))
                if malformed['type'] == 'response':
                    malformed['result'] = {'job': None}
                    malformed['error'] = {'code': 'internal', 'message': 'Failed'}
                    self.assertFalse(validator.is_valid(malformed))

    def test_task_application_runs_through_public_messages(self):
        host = [f['message'] for f in FIXTURES if f['direction'] == 'host']
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'tasks.py')],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)
        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()
        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(3), 'application response timed out')
            data = child.stdout.readline(1048577)
            self.assertTrue(data.endswith(b'\n'))
            message = json.loads(data)
            Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]}).validate(message)
            return message
        try:
            send(host[0])
            self.assertEqual(receive()['type'], 'register')
            send({**host[1], 'capabilities': ['views']})
            send(host[2])
            create = receive()
            self.assertEqual(create['method'], 'view.create')
            model = create['params']['model']
            send({'type': 'response', 'id': create['id'], 'result': {'view': 'v:g:1', 'revision': 'm:1', 'model': model}})
            show = receive()
            self.assertEqual(show['params'], {'invocation': 'h:1', 'view': 'v:g:1'})
            send({'type': 'response', 'id': show['id'], 'result': {}})
            self.assertEqual(receive(), {'type': 'response', 'id': 'h:1', 'result': {'job': None}})
            send({'type': 'request', 'id': 'h:2', 'method': 'command.invoke',
                  'params': {'command': 'toggle', 'context': 'view', 'view': 'v:g:1', 'model_revision': 'm:1', 'rows': ['read'], 'arguments': {}}})
            publish = receive()
            self.assertEqual(publish['method'], 'view.publish')
            model = publish['params']['model']
            self.assertTrue(model['rows'][0]['text'].startswith('[x]'))
            send({'type': 'response', 'id': publish['id'], 'result': {'view': 'v:g:1', 'revision': 'm:2', 'model': model}})
            self.assertEqual(receive()['id'], 'h:2')
            child.stdin.close()
            self.assertEqual(child.wait(timeout=3), 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)
            child.stdout.close()
            child.stderr.close()
            if not child.stdin.closed:
                child.stdin.close()

if __name__ == '__main__':
    unittest.main(verbosity=2)
